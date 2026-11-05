//! A manager and two workers in one process, against a mock server.

use super::run::{LogsFault, PrepareFault, TestFaults, LOG_FAULT_AT};
use crate::config::GooseConfiguration;
use crate::goose::{Scenario, Transaction};
use crate::metrics::GooseMetrics;
use crate::GooseAttack;
use httpmock::{Method::GET, MockServer};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// A transaction that GETs `path`.
pub(super) fn get(path: &'static str) -> Transaction {
    Transaction::new(Arc::new(move |user| {
        Box::pin(async move {
            let _ = user.get(path).await;
            Ok(())
        })
    }))
}

/// Each user, first: GET /pre, and GET /data/ with its worker data; then a
/// wait until its worker's metrics reset, at most 10 seconds, so every later
/// request is measured. With the ramp at 4 users a second, a worker's users
/// start 500 ms apart, so a worker's start may slip that much before more
/// than its last user's /pre races the reset.
fn on_start() -> Transaction {
    Transaction::new(Arc::new(|user| {
        Box::pin(async move {
            let _ = user.get("/pre").await;
            let data = String::from_utf8_lossy(user.worker_data()).into_owned();
            let _ = user.get(&format!("/data/{data}")).await;
            // The run's start makes the epoch 1, and the reset 2.
            let reset = || {
                user.metrics_epoch
                    .as_ref()
                    .is_some_and(|epoch| epoch.load(std::sync::atomic::Ordering::Relaxed) >= 2)
            };
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !reset() && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(())
        })
    }))
    .set_on_start()
}

/// The load test: scenarios A (weight 2, GET /a) and B (weight 1, GET /b),
/// test_start GET /start, test_stop GET /stop, and each worker's data its
/// index.
fn load_test(configuration: GooseConfiguration) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("A")
                .set_weight(2)
                .unwrap()
                .register_transaction(on_start())
                .register_transaction(get("/a")),
        )
        .register_scenario(
            Scenario::new("B")
                .register_transaction(on_start())
                .register_transaction(get("/b")),
        )
        .test_start(get("/start"))
        .test_stop(get("/stop"))
        .set_worker_data(|share| format!("w{}", share.index).into_bytes())
}

pub(super) fn configuration(args: &[&str]) -> GooseConfiguration {
    GooseConfiguration::parse_args_default(args).unwrap()
}

/// A fresh folder for a test's runs, removed once dropped, however the test
/// ends.
pub(super) struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh folder for this test's runs.
pub(super) fn scratch(name: &str) -> Scratch {
    let folder = std::env::temp_dir().join(format!(
        "goose-gaggle-{name}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    Scratch(folder)
}

/// The lines in `file`.
pub(super) fn lines(file: &Path) -> usize {
    std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("{}: {e}", file.display()))
        .lines()
        .count()
}

/// The users of each scenario.
pub(super) fn scenario_users(metrics: &GooseMetrics) -> Vec<BTreeSet<usize>> {
    metrics
        .scenarios
        .iter()
        .map(|scenario| scenario.users.iter().copied().collect())
        .collect()
}

/// No token from the environment: each test gives its own, if any.
fn no_token_from_the_environment() {
    std::env::remove_var(super::options::TOKEN_ENV);
}

/// A Gaggle in this process, against `host`: a manager given
/// `manager_args` and `count` workers given `worker_args`, each a load test
/// `build` makes from its configuration and its place, `None` for the
/// manager and `Some(n)` for worker `n`, which is also given `faults[n]`.
/// The manager listens on a port the system picks, and each process keeps
/// its run folder under `folder`. The manager's result, and each worker's.
pub(super) async fn gaggle(
    build: impl Fn(GooseConfiguration, Option<usize>) -> GooseAttack,
    count: usize,
    host: String,
    folder: &Path,
    manager_args: &[&str],
    worker_args: &[&str],
    faults: &[TestFaults],
) -> (
    Result<GooseMetrics, crate::GooseError>,
    Vec<Result<GooseMetrics, crate::GooseError>>,
) {
    let ((manager, _), workers) = gaggle_timed(
        build,
        count,
        host,
        folder,
        manager_args,
        worker_args,
        faults,
    )
    .await;
    (manager, workers)
}

/// [`gaggle`], with when the manager's `execute()` returned.
async fn gaggle_timed(
    build: impl Fn(GooseConfiguration, Option<usize>) -> GooseAttack,
    count: usize,
    host: String,
    folder: &Path,
    manager_args: &[&str],
    worker_args: &[&str],
    faults: &[TestFaults],
) -> (
    (Result<GooseMetrics, crate::GooseError>, std::time::Instant),
    Vec<Result<GooseMetrics, crate::GooseError>>,
) {
    no_token_from_the_environment();
    let count_text = count.to_string();
    let manager_folder = folder.join("manager").display().to_string();
    let mut args = vec![
        "--manager",
        "--expect-workers",
        &count_text,
        "--manager-bind-port",
        "0",
        "--quiet",
        "--host",
        &host,
    ];
    // The telnet Controller stays off unless a test names its port.
    if !manager_args.contains(&"--telnet-port") {
        args.push("--no-telnet");
    }
    // A folder for the manager's run when it writes anything: one that would
    // hold nothing is refused.
    if manager_args
        .iter()
        .any(|arg| *arg == "--report-file" || arg.ends_with("-log"))
    {
        args.extend_from_slice(&["--output-dir", &manager_folder]);
    }
    args.extend_from_slice(manager_args);
    let (bound, address) = tokio::sync::oneshot::channel();
    let manager = build(configuration(&args), None).report_manager_address(bound);
    let manager = tokio::spawn(async move {
        let result = manager.execute().await;
        (result, std::time::Instant::now())
    });
    let address = match address.await {
        Ok(address) => address,
        Err(_) => panic!(
            "the manager never listened: {:?}",
            manager.await.map(|(r, _)| r.map(|_| ()))
        ),
    };
    let port = address.port().to_string();
    let mut workers = Vec::new();
    for worker in 0..count {
        let worker_folder = folder.join(format!("worker{worker}")).display().to_string();
        let mut args = vec![
            "--worker",
            "--manager-host",
            "127.0.0.1",
            "--manager-port",
            &port,
            "--quiet",
            "--output-dir",
            &worker_folder,
        ];
        args.extend_from_slice(worker_args);
        let worker_attack = build(configuration(&args), Some(worker))
            .gaggle_test_faults(faults.get(worker).copied().unwrap_or_default());
        workers.push(tokio::spawn(worker_attack.execute()));
    }
    let mut ends = Vec::new();
    for worker in workers {
        ends.push(worker.await.expect("the worker task ends"));
    }
    (manager.await.expect("the manager task ends"), ends)
}

/// [`load_test`] for every process of a Gaggle.
fn standard(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    load_test(configuration)
}

/// The manager's metrics, when it and every worker exited cleanly.
pub(super) fn clean(
    (manager, workers): (
        Result<GooseMetrics, crate::GooseError>,
        Vec<Result<GooseMetrics, crate::GooseError>>,
    ),
) -> GooseMetrics {
    for worker in workers {
        worker.expect("the worker exits cleanly");
    }
    manager.expect("the manager exits cleanly")
}

/// One manager and two workers. The manager's totals are the server's
/// counts; the users run the scenarios one process would give them;
/// test_start and test_stop run once, on the manager; each worker gets its
/// own data; the reports and every worker's logs land in the manager's run
/// folder; and the metrics reset once, when the ramp ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_manager_and_two_workers_in_one_process() {
    let server = MockServer::start_async().await;
    let mock = |path: &'static str| mock_ok(&server, path);
    let (a, b, pre, start, stop) = (
        mock("/a").await,
        mock("/b").await,
        mock("/pre").await,
        mock("/start").await,
        mock("/stop").await,
    );
    let (data0, data1) = (mock("/data/w0").await, mock("/data/w1").await);
    let folder = scratch("two-workers");
    let host = server.base_url();
    let run = gaggle(
        standard,
        2,
        host.clone(),
        &folder,
        &[
            "--users",
            "6",
            "--increase-rate",
            "4",
            "--run-time",
            "3",
            "--report-file",
            "--request-log",
            "--transaction-log",
            "--scenario-log",
        ],
        &[],
        &[],
    );
    let metrics = clean(
        tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("the Gaggle ends within 30 seconds"),
    );

    // The merged totals are what the server saw.
    let count = |key: &str| {
        metrics
            .requests
            .get(key)
            .map_or(0, |request| request.success_count + request.fail_count)
    };
    let (a_calls, b_calls) = (a.calls_async().await, b.calls_async().await);
    assert!(a_calls > 0 && b_calls > 0, "{} {}", a_calls, b_calls);
    assert_eq!(count("GET /a"), a_calls);
    assert_eq!(count("GET /b"), b_calls);

    // One reset, when the ramp ended: what the users did before it is gone
    // from the metrics on every worker, and nothing after it is.
    assert_eq!(pre.calls_async().await, 6);
    // Only each worker's last user may race the reset with its /pre; without
    // the broadcast reset all six count, and with a worker missing it three.
    assert!(
        count("GET /pre") <= 2,
        "the reset clears the ramp's requests: {}",
        count("GET /pre")
    );

    // test_start and test_stop ran once.
    assert_eq!(start.calls_async().await, 1);
    assert_eq!(stop.calls_async().await, 1);

    // Each worker's users saw its own data: three users each.
    assert_eq!(data0.calls_async().await, 3);
    assert_eq!(data1.calls_async().await, 3);

    // The users run the scenarios one process running all six would give.
    let mut single = load_test(configuration(&[
        "--host",
        "http://127.0.0.1",
        "--users",
        "6",
    ]));
    let order = single.user_scenarios(6).unwrap();
    let mut expected = vec![BTreeSet::new(), BTreeSet::new()];
    for (user, &scenario) in order.iter().enumerate() {
        expected[scenario].insert(user);
    }
    assert_eq!(scenario_users(&metrics), expected);

    // The reports, and each worker's logs, in the manager's folder.
    let manager = folder.join("manager");
    for report in ["report.html", "report.json"] {
        assert!(manager.join(report).is_file(), "{}", report);
    }
    let mut request_lines = 0;
    let workers: Vec<PathBuf> = std::fs::read_dir(manager.join("workers"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(workers.len(), 2, "{workers:?}");
    for worker in &workers {
        for log in ["requests.json", "transactions.json", "scenarios.json"] {
            assert!(worker.join(log).is_file(), "{}", worker.join(log).display());
        }
        request_lines += lines(&worker.join("requests.json"));
    }
    // Every request the users made, in the workers' request logs: test_start
    // and test_stop run on the manager.
    assert_eq!(request_lines, a_calls + b_calls + 6 + 6);

    // The run record names both workers, neither lost.
    let report = std::fs::read_to_string(manager.join("report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    let gaggle = &report["gaggle"];
    assert_eq!(gaggle["degraded"], false, "{gaggle}");
    assert_eq!(gaggle["workers"].as_array().map(Vec::len), Some(2));

    // The Workers table's requests add up to the merged totals, after the
    // reset swapped every worker's totals for its new epoch's.
    let rows: u64 = gaggle["workers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|worker| worker["requests"].as_u64().unwrap_or(0))
        .sum();
    let merged: usize = metrics
        .requests
        .iter()
        .filter(|(name, _)| name.as_str() != "GET /start" && name.as_str() != "GET /stop")
        .map(|(_, request)| request.success_count + request.fail_count)
        .sum();
    assert_eq!(rows as usize, merged, "{}", gaggle);
}

/// More workers than users, with --iterations: the worker holding none is
/// prepared and counts as its users having exited, so the run ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_without_users_ends_with_iterations() {
    let server = answering().await;
    let folder = scratch("no-users");
    // Users that wait for no metrics reset, which never comes under
    // --iterations.
    let run = gaggle(
        quick,
        3,
        server.base_url(),
        &folder,
        &[
            "--users",
            "2",
            "--increase-rate",
            "100",
            "--iterations",
            "1",
            "--report-file",
        ],
        &[],
        &[],
    );
    let metrics = clean(
        tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("the Gaggle ends within 30 seconds"),
    );
    assert_eq!(metrics.total_users, 2);
    // No reset under --iterations: every delta of the start's epoch merged.
    let (gaggle, _) = record(&folder);
    assert_eq!(gaggle["degraded"], false, "{gaggle}");
}

/// A manager that goes away before the run, its task dropped mid wait:
/// its worker loses it, returns an error, and its link stops the dead man's
/// switch's last resort, so this test process lives on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_survives_its_manager_going_before_the_run() {
    let test = async {
        no_token_from_the_environment();
        let server = MockServer::start_async().await;
        let host = server.base_url();
        let (bound, address) = tokio::sync::oneshot::channel();
        let (joins, mut joined) = tokio::sync::mpsc::unbounded_channel();
        // Two workers expected and one started: the manager waits.
        let manager = load_test(configuration(&[
            "--manager",
            "--expect-workers",
            "2",
            "--manager-bind-port",
            "0",
            "--no-telnet",
            "--quiet",
            "--host",
            &host,
            "--users",
            "2",
        ]))
        .report_manager_address(bound)
        .report_joins(joins);
        let manager = tokio::spawn(manager.execute());
        let port = address
            .await
            .expect("the manager listens")
            .port()
            .to_string();
        let worker = load_test(configuration(&[
            "--worker",
            "--manager-host",
            "127.0.0.1",
            "--manager-port",
            &port,
            "--quiet",
        ]));
        // The link marks itself done as `execute()` returns, which stops
        // the switch's last resort: that it does is what this test checks.
        let link_done = worker.link_done_record();
        let worker = tokio::spawn(worker.execute());
        // Once it joined, the manager goes.
        assert_eq!(joined.recv().await, Some(1));
        manager.abort();
        let gone = std::time::Instant::now();
        let result = worker.await.expect("the worker task ends");
        // Within the last resort's real 2 seconds: past them, a worker
        // process would have been ended.
        assert!(
            gone.elapsed() < Duration::from_secs(2),
            "{:?}",
            gone.elapsed()
        );
        match result {
            Err(crate::GooseError::Gaggle { detail }) => {
                assert!(detail.contains("lost the manager"), "{}", detail)
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        let done = link_done
            .lock()
            .unwrap()
            .clone()
            .expect("the worker joined");
        assert!(done.load(std::sync::atomic::Ordering::Relaxed));
    };
    tokio::time::timeout(Duration::from_secs(30), test)
        .await
        .expect("within 30 seconds");
}

/// The run record of the run in `folder`, and its HTML report.
fn record(folder: &Path) -> (serde_json::Value, String) {
    let manager = folder.join("manager");
    let report = std::fs::read_to_string(manager.join("report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    let html = std::fs::read_to_string(manager.join("report.html")).unwrap();
    (report["gaggle"].clone(), html)
}

/// A mock on `server` that answers a GET of `path` with 200 and `ok`.
pub(super) async fn mock_ok<'a>(server: &'a MockServer, path: &str) -> httpmock::Mock<'a> {
    server
        .mock_async(|when, then| {
            when.method(GET).path(path);
            then.status(200).body("ok");
        })
        .await
}

/// A server answering every path the load test asks for.
async fn answering() -> MockServer {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET);
            then.status(200).body("ok");
        })
        .await;
    server
}

/// The one worker in a run record that `degraded` picks, and the others.
fn split_workers(
    gaggle: &serde_json::Value,
    degraded: impl Fn(&serde_json::Value) -> bool,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    gaggle["workers"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .partition(|worker| degraded(worker))
}

/// One worker that missed the metrics reset, left out of the results, and
/// one whose delta the manager refused: the run is degraded, and its record
/// and HTML report name each with its own reason, the refusal with the
/// delta that never merged; the third worker is merged whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_stale_and_refused_workers_degrade_the_run() {
    let server = answering().await;
    let folder = scratch("stale-and-refused");
    let faults = [
        TestFaults::default(),
        TestFaults {
            skip_reset: true,
            ..Default::default()
        },
        TestFaults {
            refused_delta: true,
            ..Default::default()
        },
    ];
    // Users that wait for no reset: the stale worker never gets one.
    let run = gaggle(
        quick,
        3,
        server.base_url(),
        &folder,
        &[
            "--users",
            "6",
            "--increase-rate",
            "20",
            "--run-time",
            "3",
            "--report-file",
        ],
        &[],
        &faults,
    );
    clean(
        tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("the Gaggle ends within 30 seconds"),
    );
    let (gaggle, html) = record(&folder);
    assert_eq!(gaggle["degraded"], true, "{gaggle}");
    let (stale, others) = split_workers(&gaggle, |worker| {
        worker["excluded_reason"]
            .as_str()
            .is_some_and(|why| why.contains("missed a metrics reset"))
    });
    assert_eq!((stale.len(), others.len()), (1, 2), "{gaggle}");
    assert_eq!(stale[0]["state"], "finished", "{gaggle}");
    let (refused, clean_workers) =
        split_workers(&serde_json::json!({ "workers": others }), |worker| {
            worker["refused_deltas"].as_u64() == Some(1)
        });
    assert_eq!((refused.len(), clean_workers.len()), (1, 1), "{gaggle}");
    let reason = refused[0]["refused_reason"].as_str().unwrap_or_default();
    assert!(reason.starts_with("delta 1:"), "{}", gaggle);
    assert_eq!(refused[0]["missing_deltas"], 1, "{gaggle}");
    assert!(refused[0].get("excluded_reason").is_none(), "{}", gaggle);
    let merged = &clean_workers[0];
    for field in ["excluded_reason", "refused_deltas", "missing_deltas"] {
        assert!(merged.get(field).is_none(), "{}: {}", field, gaggle);
    }
    for worker in [&stale[0], &refused[0]] {
        let name = worker["name"].as_str().unwrap();
        assert!(
            html.contains("Degraded") && html.contains(name),
            "the HTML report names {}",
            name
        );
    }
}

/// A worker whose metrics processor never hands over its final metrics, or
/// never answers the metrics reset, fails within the phase's deadline: it
/// exits with an error naming the phase, and the manager marks it lost.
async fn worker_fails_a_phase(faults: TestFaults, phase: &str, expected_last_seq: Option<u64>) {
    let server = answering().await;
    let folder = scratch("failed-phase");
    let started = std::time::Instant::now();
    let faults = [TestFaults::default(), faults];
    let run = gaggle(
        standard,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "4",
            "--increase-rate",
            "10",
            "--run-time",
            "3",
            "--report-file",
        ],
        &[],
        &faults,
    );
    let (manager, workers) = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .expect("the Gaggle ends within 90 seconds");
    manager.expect("the manager exits cleanly");
    // Joining, a 300 ms start, a ramp, 3 seconds of run, and the
    // deadline, under cfg(test): 3 seconds for the final metrics, 2 for a
    // reset.
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
    assert!(workers[0].is_ok(), "{:?}", workers[0].as_ref().map(|_| ()));
    match &workers[1] {
        Err(crate::GooseError::Gaggle { detail }) => assert!(detail.contains(phase), "{}", detail),
        other => panic!("{:?}", other.as_ref().map(|_| ())),
    }
    let (gaggle, _) = record(&folder);
    assert_eq!(gaggle["degraded"], true, "{gaggle}");
    let (lost, _) = split_workers(&gaggle, |worker| worker["state"] == "lost");
    assert_eq!(lost.len(), 1, "{gaggle}");
    // The worker's deadline for its final metrics and the manager's wait
    // for them both start as its users stop: either may end it.
    let reason = lost[0]["lost_reason"].as_str().unwrap_or_default();
    assert!(
        (reason.contains("could not finish") && reason.contains(phase))
            || (phase == "final metrics" && reason.starts_with("no final metrics within")),
        "{}",
        gaggle
    );
    if let Some(seq) = expected_last_seq {
        assert_eq!(lost[0]["last_seq"], seq, "{}", gaggle);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_fails_without_its_reset() {
    worker_fails_a_phase(
        TestFaults {
            silent_reset: true,
            ..Default::default()
        },
        "did not reset",
        None,
    )
    .await;
}

/// The client settings a worker's users get, its load test's client
/// builder asking for a 30 second timeout, gzip and checked certificates,
/// under a manager given `manager_args`.
async fn worker_client_config(manager_args: &[&str]) -> crate::client::GooseClientConfig {
    let server = answering().await;
    let folder = scratch("client-config");
    let (report, reported) = tokio::sync::oneshot::channel();
    let report = std::sync::Mutex::new(Some(report));
    // The worker builds its users' clients with a builder of its own.
    let build = |configuration, place: Option<usize>| {
        let attack = load_test(configuration);
        if place.is_none() {
            return attack;
        }
        let builder = crate::client::GooseClientBuilder::new()
            .without_cookies()
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .accept_invalid_certs(false);
        let attack = attack.set_client_builder_without_cookies(builder).unwrap();
        match report.lock().unwrap().take() {
            Some(report) => attack.report_client_config(report),
            None => attack,
        }
    };
    let mut args = vec!["--users", "1", "--run-time", "1"];
    args.extend_from_slice(manager_args);
    clean(gaggle(build, 1, server.base_url(), &folder, &args, &[], &[]).await);
    reported.await.expect("the worker builds its clients")
}

/// The manager's --timeout, --no-gzip and --accept-invalid-certs win over a
/// worker's client builder, as a command line wins over a builder on one
/// process; without them, the builder's settings stand.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_manager_client_options_win_on_workers() {
    let test = async {
        let config =
            worker_client_config(&["--timeout", "1", "--no-gzip", "--accept-invalid-certs"]).await;
        assert_eq!(config.timeout, Some(Duration::from_secs(1)));
        assert!(!config.gzip);
        assert!(config.accept_invalid_certs);
        let config = worker_client_config(&[]).await;
        assert_eq!(config.timeout, Some(Duration::from_secs(30)));
        assert!(config.gzip);
        assert!(!config.accept_invalid_certs);
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// Each worker's copy of the request log in the run in `folder`.
pub(super) fn worker_logs(folder: &Path, file: &str) -> Vec<Vec<u8>> {
    let workers = folder.join("manager").join("workers");
    let mut logs: Vec<Vec<u8>> = std::fs::read_dir(&workers)
        .unwrap_or_else(|e| panic!("{}: {e}", workers.display()))
        .map(|entry| {
            let path = entry.unwrap().path().join(file);
            std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect();
    logs.sort();
    logs
}

/// A collected log's bytes match its name: one named `.gz` arrives gzipped
/// without --compress, and one named without `.gz` arrives plain with it,
/// as one process writes a log of either name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_collected_logs_match_their_names() {
    use std::io::Read;
    for (file, compress) in [("custom.json.gz", false), ("plain.json", true)] {
        let server = answering().await;
        let folder = scratch("log-names");
        let log = folder.join(file).display().to_string();
        let mut args = vec![
            "--users",
            "2",
            "--increase-rate",
            "10",
            "--run-time",
            "2",
            "--request-log",
            &log,
        ];
        if compress {
            args.push("--compress");
        }
        let run = gaggle(standard, 2, server.base_url(), &folder, &args, &[], &[]);
        clean(
            tokio::time::timeout(Duration::from_secs(30), run)
                .await
                .expect("the Gaggle ends within 30 seconds"),
        );
        let logs = worker_logs(&folder, file);
        assert_eq!(logs.len(), 2, "{file}");
        for bytes in logs {
            let gzipped = bytes.starts_with(&[0x1f, 0x8b]);
            assert_eq!(gzipped, file.ends_with(".gz"), "{file}");
            let text = if gzipped {
                let mut text = String::new();
                flate2::read::MultiGzDecoder::new(&bytes[..])
                    .read_to_string(&mut text)
                    .unwrap_or_else(|e| panic!("{} is not valid gzip: {}", file, e));
                text
            } else {
                String::from_utf8(bytes).unwrap()
            };
            assert!(text.lines().count() > 0, "{} is empty", file);
            assert!(
                text.starts_with('{'),
                "{file}: {}",
                &text[..text.len().min(80)]
            );
        }
    }
}

/// A request log a load test sets as a default in code is not written on a
/// worker whose manager asked for no request log: the manager would never
/// collect it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_writes_no_log_set_in_code() {
    let server = answering().await;
    let folder = scratch("log-in-code");
    let in_code = folder.join("in-code.log").display().to_string();
    let build = |configuration, place: Option<usize>| {
        let attack = load_test(configuration);
        match place {
            None => attack,
            Some(_) => *crate::config::GooseDefaultType::set_default(
                attack,
                crate::config::GooseDefault::RequestLog,
                in_code.as_str(),
            )
            .unwrap(),
        }
    };
    let run = gaggle(
        build,
        2,
        server.base_url(),
        &folder,
        &["--users", "2", "--increase-rate", "10", "--run-time", "2"],
        &[],
        &[],
    );
    clean(
        tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("the Gaggle ends within 30 seconds"),
    );
    assert!(!Path::new(&in_code).exists(), "a worker wrote {}", in_code);
}

/// A manager listening on every address, which needs a token, admits the
/// workers presenting it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_non_loopback_manager_with_a_token_admits_its_workers() {
    let server = answering().await;
    let folder = scratch("non-loopback");
    let run = gaggle(
        standard,
        2,
        server.base_url(),
        &folder,
        &[
            "--manager-bind-host",
            "0.0.0.0",
            "--gaggle-token",
            "tok-non-loopback",
            "--users",
            "2",
            "--increase-rate",
            "10",
            "--run-time",
            "2",
            "--report-file",
        ],
        &["--gaggle-token", "tok-non-loopback"],
        &[],
    );
    let metrics = clean(
        tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("the Gaggle ends within 30 seconds"),
    );
    assert_eq!(metrics.total_users, 2);
}

/// A load test of one scenario, GET /a, whose worker data hook returns
/// `bytes` bytes for each worker.
fn with_worker_data(configuration: GooseConfiguration, bytes: usize) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(Scenario::new("A").register_transaction(get("/a")))
        .set_worker_data(move |_| vec![b'x'; bytes])
}

/// A manager of [`with_worker_data`] handing out `bytes` bytes, and one
/// worker: their results.
async fn worker_data_run(
    bytes: usize,
) -> (
    Result<GooseMetrics, crate::GooseError>,
    Result<GooseMetrics, crate::GooseError>,
) {
    let server = answering().await;
    let folder = scratch("worker-data");
    let build = |configuration, place: Option<usize>| {
        with_worker_data(configuration, if place.is_none() { bytes } else { 0 })
    };
    let (manager, mut workers) = gaggle(
        build,
        1,
        server.base_url(),
        &folder,
        &["--users", "1", "--run-time", "1"],
        &[],
        &[],
    )
    .await;
    (manager, workers.remove(0))
}

/// Worker data of MAX_WORKER_DATA bytes is handed over; one byte more is
/// refused by the manager with its documented error, and the workers are
/// shut down rather than left waiting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_data_is_capped() {
    let test = async {
        let (manager, worker) = worker_data_run(crate::MAX_WORKER_DATA + 1).await;
        match manager {
            Err(crate::GooseError::InvalidOption { detail, .. }) => assert!(
                detail.contains(&format!(
                    "{} bytes for worker 0, more than the {} allowed",
                    crate::MAX_WORKER_DATA + 1,
                    crate::MAX_WORKER_DATA
                )),
                "{}",
                detail
            ),
            other => panic!("{:?}", other.map(|_| ())),
        }
        match worker {
            Err(crate::GooseError::Gaggle { detail }) => {
                assert!(
                    detail.contains("shut down before the run started"),
                    "{}",
                    detail
                )
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        let (manager, worker) = worker_data_run(crate::MAX_WORKER_DATA).await;
        manager.expect("the manager hands over the largest data allowed");
        worker.expect("the worker runs with it");
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A run of one process whose worker data hook returns too much fails, and
/// its test_stop still runs, after test_start did.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn worker_data_past_the_cap_still_runs_test_stop() {
    let server = MockServer::start_async().await;
    let mock = |path: &'static str| mock_ok(&server, path);
    let (start, stop) = (mock("/start").await, mock("/stop").await);
    let host = server.base_url();
    let run = with_worker_data(
        configuration(&[
            "--host",
            &host,
            "--users",
            "1",
            "--run-time",
            "1",
            "--quiet",
            "--no-telnet",
        ]),
        crate::MAX_WORKER_DATA + 1,
    )
    .test_start(get("/start"))
    .test_stop(get("/stop"))
    .execute();
    let result = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("within 30 seconds");
    assert!(
        matches!(&result, Err(crate::GooseError::InvalidOption { detail, .. }) if detail.contains("more than the")),
        "{:?}",
        result.map(|_| ())
    );
    assert_eq!(start.calls_async().await, 1);
    assert_eq!(stop.calls_async().await, 1);
}

/// A lost worker's record keeps the last delta the merge took, not the last
/// one it sent: every delta from 2 on is refused, and its final never comes,
/// its processor taking the command long after the worker's wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_lost_worker_record_keeps_the_last_merged_seq() {
    worker_fails_a_phase(
        TestFaults {
            final_after: Some(Duration::from_secs(60)),
            refuse_from: 2,
            ..Default::default()
        },
        "final metrics",
        Some(1),
    )
    .await;
}

/// A load test of one scenario, GET /a, whose users wait for nothing.
fn quick(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(Scenario::new("A").register_transaction(get("/a")))
}

/// A worker whose load test differs from its manager's, or whose token is
/// wrong, is refused, and fails naming why, never panicking; the manager,
/// its worker never joining, gives up at its connect timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_handshake_refusals() {
    let test = async {
        let server = answering().await;
        let folder = scratch("refusals");
        for (case, manager_extra, worker_extra, named) in [
            ("plan", vec![], vec![], "refused this worker: plan_hash:"),
            (
                "token",
                vec!["--gaggle-token", "tok-right-5e1b"],
                vec!["--gaggle-token", "tok-wrong-9c2d"],
                "refused this worker: token:",
            ),
        ] {
            // The plan case's worker runs another load test.
            let build = |configuration, place: Option<usize>| match (case, place) {
                ("plan", Some(_)) => quick(configuration, place),
                _ => standard(configuration, place),
            };
            let mut manager_args = vec!["--users", "2", "--gaggle-connect-timeout", "3s"];
            manager_args.extend(manager_extra);
            let (manager, workers) = gaggle(
                build,
                1,
                server.base_url(),
                &folder,
                &manager_args,
                &worker_extra,
                &[],
            )
            .await;
            match &workers[0] {
                Err(crate::GooseError::Gaggle { detail }) => {
                    assert!(detail.contains(named), "{}: {}", case, detail);
                    assert!(
                        !detail.contains("tok-right") && !detail.contains("tok-wrong"),
                        "{}: {}",
                        case,
                        detail
                    );
                }
                other => panic!("{}: {:?}", case, other.as_ref().map(|_| ())),
            }
            match manager {
                Err(crate::GooseError::Gaggle { detail }) => {
                    assert!(
                        detail.contains("0 of 1 workers joined"),
                        "{}: {}",
                        case,
                        detail
                    )
                }
                other => panic!("{}: {:?}", case, other.map(|_| ())),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A worker with no manager to reach tries again after a second, and then
/// fails, without a panic, within its connect timeout plus 5 seconds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_without_a_manager() {
    let test = async {
        no_token_from_the_environment();
        // A port bound and not listening, for the whole test: connections
        // to it are refused, and no other test can take it.
        let held = tokio::net::TcpSocket::new_v4().unwrap();
        held.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = held.local_addr().unwrap().port().to_string();
        let started = std::time::Instant::now();
        let worker = quick(
            configuration(&[
                "--worker",
                "--manager-host",
                "127.0.0.1",
                "--manager-port",
                &port,
                "--quiet",
                "--gaggle-connect-timeout",
                "2s",
            ]),
            Some(0),
        );
        let result = worker.execute().await;
        let took = started.elapsed();
        match result {
            Err(crate::GooseError::Gaggle { detail }) => {
                assert!(detail.contains("could not reach the manager"), "{}", detail)
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        // Its first wait, 1 second, passed; the next, 2, would pass the
        // timeout. `backoff_is_one_two_four_eight_then_ten` checks the waits
        // that follow.
        assert!(took >= Duration::from_secs(1), "{:?}", took);
        assert!(took <= Duration::from_secs(2 + 5), "{:?}", took);
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// `--no-metrics` on the manager: the run starts, stops on time, and every
/// process ends cleanly (#156).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_without_metrics_exits_cleanly() {
    let server = MockServer::start_async().await;
    let a = server
        .mock_async(|when, then| {
            when.method(GET).path("/a");
            then.status(200).body("ok");
        })
        .await;
    let folder = scratch("no-metrics");
    let started = std::time::Instant::now();
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "4",
            "--increase-rate",
            "4",
            "--run-time",
            "3",
            "--no-metrics",
        ],
        &[],
        &[],
    );
    clean(
        tokio::time::timeout(Duration::from_secs(60), run)
            .await
            .expect("the Gaggle ends within 60 seconds"),
    );
    // Joining, a 300 ms start, a 1 second ramp and 3 seconds of run.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert!(a.calls_async().await > 0);
}

/// Two workers running a test plan that goes up, down and up again, under
/// a throttle: the manager counts the users the plan starts and the most it
/// runs at once, the merged scenarios hold every user the workers started,
/// and the workers' throttle shares add up to the throttle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_test_plan_and_throttle_across_workers() {
    let server = MockServer::start_async().await;
    let a = server
        .mock_async(|when, then| {
            when.method(GET).path("/a");
            then.status(200).body("ok");
        })
        .await;
    let folder = scratch("plan-throttle");
    let started = std::time::Instant::now();
    // 6 users, 2, then 6 again: 10 started, at most 6 at once.
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--test-plan",
            "6,1s;2,1s;6,1s;6,1s;0,0s",
            "--throttle-requests",
            "6",
        ],
        &[],
        &[],
    );
    let metrics = clean(
        tokio::time::timeout(Duration::from_secs(60), run)
            .await
            .expect("the Gaggle ends within 60 seconds"),
    );
    assert_eq!((metrics.total_users, metrics.maximum_users), (10, 6));
    let users: usize = scenario_users(&metrics).iter().map(BTreeSet::len).sum();
    assert_eq!(users, 10);
    // Unthrottled, 6 users against a server that answers at once make
    // thousands of requests a second. The plan's 4 seconds at 6 a second
    // make about 24: shares adding up to more, as each worker taking the
    // whole throttle would, make about 48.
    let requests = a.calls_async().await;
    assert!(
        requests > 0 && requests <= 36,
        "{} requests in {:?}",
        requests,
        started.elapsed()
    );
}

/// A manager that fails after its workers joined, as `fault` on worker 1,
/// `too_much_data` from the worker data hook or an uncreatable `report`
/// file make it: one that fails before test_start runs neither it nor
/// test_stop, one that fails after runs test_stop once, and no user runs.
async fn manager_fails(
    fault: PrepareFault,
    too_much_data: bool,
    report: Option<&str>,
    test_start_runs: bool,
) {
    let test = async {
        let server = MockServer::start_async().await;
        let mock = |path: &'static str| mock_ok(&server, path);
        let (start, stop, a) = (mock("/start").await, mock("/stop").await, mock("/a").await);
        let folder = scratch("manager-failures");
        let build = |configuration, place: Option<usize>| {
            let bytes = if too_much_data && place.is_none() {
                crate::MAX_WORKER_DATA + 1
            } else {
                0
            };
            with_worker_data(configuration, bytes)
                .test_start(get("/start"))
                .test_stop(get("/stop"))
        };
        let mut manager_args = vec!["--users", "2", "--run-time", "2"];
        // One argument, so the run gets no folder that would hold nothing.
        let report = report.map(|report| format!("--report-file={report}"));
        if let Some(report) = report.as_deref() {
            manager_args.push(report);
        }
        let faults = [
            TestFaults::default(),
            TestFaults {
                prepare: fault,
                ..Default::default()
            },
        ];
        let (manager, workers) = gaggle(
            build,
            2,
            server.base_url(),
            &folder,
            &manager_args,
            &[],
            &faults,
        )
        .await;
        assert!(manager.is_err(), "the manager fails");
        // A worker that prepared and is shut down before the start ends
        // cleanly, having run nothing; the one at fault fails.
        if fault != PrepareFault::None {
            assert!(workers[1].is_err(), "the faulty worker fails");
        }
        let runs = usize::from(test_start_runs);
        assert_eq!(start.calls_async().await, runs, "test_start");
        assert_eq!(stop.calls_async().await, runs, "test_stop");
        assert_eq!(a.calls_async().await, 0, "no user ran");
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_data_hook_failure_runs_test_stop() {
    manager_fails(PrepareFault::None, true, None, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_prepare_error_runs_test_stop() {
    manager_fails(PrepareFault::Refuse, false, None, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_lost_preparing_runs_test_stop() {
    manager_fails(PrepareFault::Vanish, false, None, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_prepare_timeout_runs_test_stop() {
    manager_fails(PrepareFault::Stall, false, None, true).await;
}

/// A manager given `more` past what a worker runs, worker 0's users or the
/// test plan's steps, is refused with `message` before it listens, so no
/// worker can join it and test_start never runs. One that listens is past
/// the check: it would wait for its workers, run test_start, and then have
/// every worker refuse its share.
async fn manager_past_a_worker_bound(more: &[&str], message: &str) {
    let test = async {
        no_token_from_the_environment();
        let server = MockServer::start_async().await;
        let start = mock_ok(&server, "/start").await;
        let host = server.base_url();
        let mut args = vec![
            "--manager",
            "--expect-workers",
            "2",
            "--manager-bind-port",
            "0",
            "--no-telnet",
            "--quiet",
            "--host",
            &host,
        ];
        args.extend_from_slice(more);
        let (bound, address) = tokio::sync::oneshot::channel();
        let manager = tokio::spawn(
            load_test(configuration(&args))
                .report_manager_address(bound)
                .execute(),
        );
        if let Ok(address) = address.await {
            manager.abort();
            panic!("the manager listened on {}, past the check", address);
        }
        match manager.await.expect("the manager task ends") {
            Err(crate::GooseError::InvalidOption { detail, .. }) => {
                assert!(detail.contains(message), "{}", detail)
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(start.calls_async().await, 0, "test_start");
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_manager_refuses_more_users_than_a_worker_runs() {
    manager_past_a_worker_bound(
        &["--users", "2000001"],
        "gives worker 0 1000001 users, more than the 1000000 users a worker runs",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_manager_refuses_more_plan_steps_than_a_worker_runs() {
    let plan = vec!["1,1"; 100_001].join(";");
    manager_past_a_worker_bound(
        &["--test-plan", &plan],
        "100001 test plan steps, more than the 100000 steps a worker runs",
    )
    .await;
}

/// The report file is checked before test_start, so neither it nor
/// test_stop runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_uncreatable_report_file_runs_neither_hook() {
    manager_fails(
        PrepareFault::None,
        false,
        Some("/nonexistent-goose-folder/report.html"),
        false,
    )
    .await;
}

/// The manager of a run that a killswitch cancels part way: it ends within
/// 5 seconds of it, with an error saying it was canceled, which no timeout
/// gives.
fn canceled_within(manager: &Result<GooseMetrics, crate::GooseError>, took: Duration, stage: &str) {
    crate::reset_killswitch();
    match manager {
        Err(crate::GooseError::Gaggle { detail }) => {
            assert!(detail.contains("canceled"), "{}: {}", stage, detail)
        }
        other => panic!("{stage}: {:?}", other.as_ref().map(|_| ())),
    }
    assert!(took <= Duration::from_secs(5), "{}: {:?}", stage, took);
}

/// The killswitch, as Ctrl-C, while the manager waits for its workers to
/// join, and while it waits for one to prepare: the manager shuts its
/// workers down and ends within 2 seconds, test_stop run only once
/// test_start did.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_while_joining_and_preparing() {
    let test = async {
        for stage in ["joining", "preparing"] {
            no_token_from_the_environment();
            crate::reset_killswitch();
            let server = MockServer::start_async().await;
            let mock = |path: &'static str| mock_ok(&server, path);
            let (start, stop) = (mock("/start").await, mock("/stop").await);
            let host = server.base_url();
            let (bound, address) = tokio::sync::oneshot::channel();
            let (joins, mut joined) = tokio::sync::mpsc::unbounded_channel();
            // Joining: two workers expected and one started. Preparing: one
            // worker, which never answers its Prepare.
            let expected = if stage == "joining" { "2" } else { "1" };
            let manager = load_test(configuration(&[
                "--manager",
                "--expect-workers",
                expected,
                "--manager-bind-port",
                "0",
                "--no-telnet",
                "--quiet",
                "--host",
                &host,
                "--users",
                "2",
                "--gaggle-connect-timeout",
                "10s",
            ]))
            .report_manager_address(bound)
            .report_joins(joins);
            let manager = tokio::spawn(async move {
                let result = manager.execute().await;
                (result, std::time::Instant::now())
            });
            let port = address
                .await
                .expect("the manager listens")
                .port()
                .to_string();
            let worker = load_test(configuration(&[
                "--worker",
                "--manager-host",
                "127.0.0.1",
                "--manager-port",
                &port,
                "--quiet",
            ]))
            .gaggle_test_faults(TestFaults {
                prepare: PrepareFault::Stall,
                ..Default::default()
            });
            let worker = tokio::spawn(worker.execute());
            assert_eq!(joined.recv().await, Some(1));
            // The manager sends its Prepare at once.
            tokio::time::sleep(Duration::from_millis(200)).await;
            crate::trigger_killswitch("a test's Ctrl-C");
            let canceled = std::time::Instant::now();
            let (result, ended) = manager.await.expect("the manager task ends");
            canceled_within(&result, ended - canceled, stage);
            assert!(
                worker.await.expect("the worker task ends").is_err(),
                "{}",
                stage
            );
            let ran = usize::from(stage == "preparing");
            assert_eq!(start.calls_async().await, ran, "{stage}: test_start");
            assert_eq!(stop.calls_async().await, ran, "{stage}: test_stop");
        }
    };
    tokio::time::timeout(Duration::from_secs(90), test)
        .await
        .expect("within 90 seconds");
}

/// A run whose one worker sends its request log as `fault` says: the
/// manager's result, when it ended against [`LOG_FAULT_AT`] if the fault
/// set it, its run record, and the run's folder.
async fn faulty_logs_run(
    fault: LogsFault,
) -> (
    Result<GooseMetrics, crate::GooseError>,
    Option<Duration>,
    serde_json::Value,
    Scratch,
) {
    let server = answering().await;
    let folder = scratch("faulty-logs");
    *LOG_FAULT_AT.lock().unwrap() = None;
    let faults = [TestFaults {
        logs: fault,
        ..Default::default()
    }];
    let ((manager, ended), workers) = gaggle_timed(
        quick,
        1,
        server.base_url(),
        &folder,
        &[
            "--users",
            "1",
            "--run-time",
            "1",
            "--report-file",
            "--request-log",
        ],
        &[],
        &faults,
    )
    .await;
    crate::reset_killswitch();
    drop(workers);
    let at = *LOG_FAULT_AT.lock().unwrap();
    let (gaggle, _) = record(&folder);
    let took = at.map(|at| ended.saturating_duration_since(at));
    (manager, took, gaggle, folder)
}

/// The `.part` files under `folder`.
fn parts(folder: &Path) -> Vec<PathBuf> {
    walk(folder)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "part"))
        .collect()
}

/// Every file under `folder`.
fn walk(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// The killswitch while a log arrives, one full chunk at a time: the manager
/// stops waiting within 5 seconds, sooner than the log idle timeout would
/// give it up, deletes what arrived, and records the log as left on its
/// worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_while_collecting_logs() {
    let test = async {
        let (manager, took, gaggle, folder) = faulty_logs_run(LogsFault::Slow).await;
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        let took = took.expect("the worker triggered the killswitch");
        assert!(took <= Duration::from_secs(5), "{:?}", took);
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{gaggle}"
        );
        assert_eq!(parts(&folder), Vec::<PathBuf>::new());
    };
    tokio::time::timeout(Duration::from_secs(90), test)
        .await
        .expect("within 90 seconds");
}

/// A log arriving 10 bytes at a time, never its last chunk, moves too
/// little to keep its transfer alive: it is given up at the idle timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_trickled_log_is_given_up() {
    let test = async {
        let (manager, took, gaggle, folder) = faulty_logs_run(LogsFault::Trickle).await;
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        let took = took.expect("the worker sent a chunk");
        // The idle timeout from the first chunk, and the reports.
        assert!(
            took <= super::run::LOG_IDLE_TIMEOUT + Duration::from_secs(3),
            "{:?}",
            took
        );
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{gaggle}"
        );
        assert_eq!(parts(&folder), Vec::<PathBuf>::new());
    };
    tokio::time::timeout(Duration::from_secs(90), test)
        .await
        .expect("within 90 seconds");
}

/// Two workers whose users wait on a target slower than the manager's wait
/// for final metrics once they stop: every user's last request outlasts
/// that wait, and still no worker is lost, every final merges, and the run
/// is not degraded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_slow_stop_keeps_every_final() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/a");
            then.status(200)
                .body("ok")
                // Past the merge's clock slack too, so the final delta
                // carries seconds past the clock the run ended on.
                .delay(
                    super::run::FINAL_TIMEOUT + Duration::from_secs(super::run::CLOCK_SLACK + 3),
                );
        })
        .await;
    let folder = scratch("slow-stop");
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "4",
            "--increase-rate",
            "100",
            "--run-time",
            "2",
            "--timeout",
            "20",
            "--report-file",
        ],
        &[],
        &[],
    );
    clean(
        tokio::time::timeout(Duration::from_secs(60), run)
            .await
            .expect("the Gaggle ends within 60 seconds"),
    );
    let (gaggle, _) = record(&folder);
    assert_eq!(gaggle["degraded"], false, "{gaggle}");
    for worker in gaggle["workers"].as_array().unwrap() {
        assert_eq!(worker["state"], "finished", "{gaggle}");
        assert!(worker.get("final_missing").is_none(), "{}", gaggle);
    }
}

/// A log that grows past the size its worker announced is cut off: nothing
/// of it is left on the manager, and the run record names it as left on the
/// worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_log_past_its_announced_size_is_cut_off() {
    let test = async {
        let (manager, _, gaggle, folder) = faulty_logs_run(LogsFault::Oversize).await;
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{gaggle}"
        );
        let collected: Vec<PathBuf> = walk(&folder.join("manager").join("workers"));
        assert_eq!(collected, Vec::<PathBuf>::new());
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A log the manager cannot write, as on a full disk, is deleted and
/// recorded as left on its worker, and the reports are whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_log_write_failure_leaves_the_reports_whole() {
    let test = async {
        // Writes work again however the test ends.
        struct FailWrites;
        impl Drop for FailWrites {
            fn drop(&mut self) {
                super::run::FAIL_LOG_WRITES.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        super::run::FAIL_LOG_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
        let restore = FailWrites;
        let (manager, _, gaggle, folder) = faulty_logs_run(LogsFault::None).await;
        drop(restore);
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{gaggle}"
        );
        assert_eq!(parts(&folder), Vec::<PathBuf>::new());
        let (_, html) = record(&folder);
        assert!(
            html.contains("Logs left on their workers"),
            "the HTML report names it"
        );
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// The manager writes its reports before any worker's log arrives, so a
/// disk the logs fill costs only logs. File times of a second's resolution
/// can make this pass with the order reversed;
/// `gaggle_full_disk_keeps_the_first_reports` checks the order too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_reports_are_written_before_the_logs() {
    let test = async {
        let (manager, _, _, folder) = faulty_logs_run(LogsFault::None).await;
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        let modified = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        };
        let report = modified(&folder.join("manager").join("report.json"));
        let logs = walk(&folder.join("manager").join("workers"));
        assert_eq!(logs.len(), 1, "{logs:?}");
        assert!(
            report <= modified(&logs[0]),
            "the report came after the log"
        );
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A run that loses every worker ends early, and its record says so: ended
/// by every worker lost, each worker lost, the run degraded, and test_stop
/// run once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_every_worker_lost_ends_the_run() {
    let server = MockServer::start_async().await;
    let mock = |path: &'static str| mock_ok(&server, path);
    let (_a, stop) = (mock("/a").await, mock("/stop").await);
    let _start = mock("/start").await;
    let folder = scratch("every-worker-lost");
    let build = |configuration, place| {
        quick(configuration, place)
            .test_start(get("/start"))
            .test_stop(get("/stop"))
    };
    let faults = [TestFaults {
        exit_mid_run: true,
        ..Default::default()
    }; 2];
    let started = std::time::Instant::now();
    let run = gaggle(
        build,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "4",
            "--increase-rate",
            "100",
            "--run-time",
            "30",
            "--report-file",
        ],
        &[],
        &faults,
    );
    let (manager, workers) = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("the Gaggle ends within 60 seconds");
    assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
    assert!(workers.iter().all(Result::is_err));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    let report = std::fs::read_to_string(folder.join("manager").join("report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["ended_by"], "workers_lost", "{}", report["gaggle"]);
    let gaggle = &report["gaggle"];
    assert_eq!(gaggle["degraded"], true, "{}", gaggle);
    let (lost, others) = split_workers(gaggle, |worker| worker["state"] == "lost");
    assert_eq!((lost.len(), others.len()), (2, 0), "{}", gaggle);
    assert_eq!(stop.calls_async().await, 1);
}

/// A worker whose final delta is refused, and which then stops as told, no
/// longer holds back which seconds of the graphs are complete: the final
/// graphs cover the whole run, the merge built past the workers' lag.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_stopped_worker_without_its_final_completes_the_graphs() {
    let server = answering().await;
    let folder = scratch("final-refused");
    let faults = [
        TestFaults::default(),
        TestFaults {
            refused_final: true,
            ..Default::default()
        },
    ];
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "2",
            "--increase-rate",
            "100",
            "--run-time",
            "4",
            "--report-file",
            "--report-graph-data",
        ],
        &[],
        &faults,
    );
    clean(
        tokio::time::timeout(Duration::from_secs(60), run)
            .await
            .expect("the Gaggle ends within 60 seconds"),
    );
    let report = std::fs::read_to_string(folder.join("manager").join("report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    let (_, html) = record(&folder);
    assert!(
        html.contains("final metrics never arrived"),
        "the run is degraded"
    );
    let transactions = &report["graphs"]["transactions_per_second"];
    let seconds = transactions["data"].as_array().map_or(0, Vec::len);
    // 4 seconds of run, and the ramp: none of it held back.
    assert!(
        seconds >= 4,
        "{} seconds of graph: {}",
        seconds,
        transactions
    );
}

/// A full disk while the logs arrive costs only logs: the reports written
/// first stay whole when the second write, to name the logs left, fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_full_disk_keeps_the_first_reports() {
    use std::sync::atomic::Ordering::Relaxed;
    // The disk empties again however the test ends.
    struct Faults;
    impl Drop for Faults {
        fn drop(&mut self) {
            super::run::FAIL_LOG_WRITES.store(false, Relaxed);
            super::run::DISK_FULL_AFTER_LOGS.store(false, Relaxed);
            super::run::DISK_FULL.store(false, Relaxed);
        }
    }
    let _faults = Faults;
    super::run::FAIL_LOG_WRITES.store(true, Relaxed);
    super::run::DISK_FULL_AFTER_LOGS.store(true, Relaxed);
    let server = answering().await;
    let folder = scratch("full-disk");
    let report = folder.join("report.html");
    let report_file = format!("--report-file={}", report.display());
    let (manager, _) = gaggle(
        quick,
        1,
        server.base_url(),
        &folder,
        &[
            "--users",
            "1",
            "--run-time",
            "1",
            &report_file,
            "--request-log",
        ],
        &[],
        &[],
    )
    .await;
    assert!(super::run::DISK_FULL.load(Relaxed), "the disk filled");
    // The second write failed, and says so.
    assert!(manager.is_err(), "the manager reports the failed write");
    let html = std::fs::read_to_string(&report).unwrap();
    assert!(html.trim_end().ends_with("</html>"), "{} bytes", html.len());
    assert_eq!(parts(&folder), Vec::<PathBuf>::new());
}

/// A manager that fails once its workers were told to start, but before its
/// own run, as when its dashboard cannot listen, still runs test_stop once
/// and shuts its workers down, which end without waiting out their switch.
#[cfg(feature = "dashboard")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_manager_failing_to_start_its_run_runs_test_stop() {
    let test = async {
        let server = MockServer::start_async().await;
        let mock = |path: &'static str| mock_ok(&server, path);
        let (start, stop, _a) = (mock("/start").await, mock("/stop").await, mock("/a").await);
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port().to_string();
        let folder = scratch("dashboard-taken");
        let build = |configuration, place| {
            quick(configuration, place)
                .test_start(get("/start"))
                .test_stop(get("/stop"))
        };
        let started = std::time::Instant::now();
        let (manager, workers) = gaggle(
            build,
            2,
            server.base_url(),
            &folder,
            &[
                "--users",
                "2",
                "--run-time",
                "30",
                "--dashboard",
                "--dashboard-host",
                "127.0.0.1",
                "--dashboard-port",
                &port,
            ],
            &[],
            &[],
        )
        .await;
        assert!(manager.is_err(), "the dashboard cannot listen");
        drop(workers);
        assert_eq!(start.calls_async().await, 1, "test_start");
        assert_eq!(stop.calls_async().await, 1, "test_stop");
        // Told to shut down: not the heartbeat timeout, nor the run time.
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{:?}",
            started.elapsed()
        );
        drop(taken);
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A Gaggle run canceled by the killswitch, as Ctrl-C or a threshold does,
/// still shows its test_stop the killswitch triggered, as a run of one
/// process does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_canceled_run_shows_test_stop_the_killswitch() {
    static SEEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    SEEN.store(0, std::sync::atomic::Ordering::SeqCst);
    let test_stop = || {
        Transaction::new(Arc::new(|_| {
            Box::pin(async move {
                let seen = if crate::is_killswitch_triggered() {
                    2
                } else {
                    1
                };
                SEEN.store(seen, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        }))
    };
    let server = MockServer::start_async().await;
    let a = mock_ok(&server, "/a").await;
    let folder = scratch("canceled-test-stop");
    let build = |configuration, place| quick(configuration, place).test_stop(test_stop());
    // Once the run has started: its first request reached the server.
    let canceling = async {
        while a.calls_async().await == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        crate::trigger_killswitch("a test's Ctrl-C mid run");
    };
    let run = gaggle(
        build,
        1,
        server.base_url(),
        &folder,
        &["--users", "1", "--run-time", "30"],
        &[],
        &[],
    );
    let ((manager, _), ()) = tokio::time::timeout(
        Duration::from_secs(60),
        futures::future::join(run, canceling),
    )
    .await
    .expect("the Gaggle ends within 60 seconds");
    crate::reset_killswitch();
    assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
    assert_eq!(
        SEEN.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "test_stop saw no cancel"
    );
}

/// A Ctrl-C once the manager sent `Stop`, before it asks for the logs, is
/// kept for the log collection: the log stays on its worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_after_stop_leaves_the_logs() {
    let test = async {
        let server = answering().await;
        let folder = scratch("cancel-on-stop");
        *LOG_FAULT_AT.lock().unwrap() = None;
        let faults = [TestFaults {
            cancel_on_stop: true,
            ..Default::default()
        }];
        let _ = gaggle(
            quick,
            1,
            server.base_url(),
            &folder,
            &[
                "--users",
                "1",
                "--run-time",
                "1",
                "--report-file",
                "--request-log",
            ],
            &[],
            &faults,
        )
        .await;
        crate::reset_killswitch();
        assert!(
            LOG_FAULT_AT.lock().unwrap().is_some(),
            "the worker saw Stop"
        );
        let (gaggle, _) = record(&folder);
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{}",
            gaggle
        );
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A Ctrl-C while the manager waits for its workers' final metrics, one
/// worker never saying its users stopped: the manager ends within 5 seconds
/// of it, where it would wait the request timeout and more, and still runs
/// test_stop once, writes its report and records that worker lost, canceled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_during_the_final_wait() {
    let test = async {
        let server = MockServer::start_async().await;
        let (_a, stop) = (
            mock_ok(&server, "/a").await,
            mock_ok(&server, "/stop").await,
        );
        let folder = scratch("cancel-final-wait");
        *LOG_FAULT_AT.lock().unwrap() = None;
        super::faults::STOPPED_SEEN.store(0, std::sync::atomic::Ordering::SeqCst);
        let faults = [
            TestFaults::default(),
            TestFaults {
                withhold_final: true,
                cancel_on_stop: true,
                ..Default::default()
            },
        ];
        let build = |configuration, place| quick(configuration, place).test_stop(get("/stop"));
        let ((manager, ended), _) = gaggle_timed(
            build,
            2,
            server.base_url(),
            &folder,
            &[
                "--users",
                "2",
                "--increase-rate",
                "10",
                "--run-time",
                "2",
                "--report-file",
            ],
            &[],
            &faults,
        )
        .await;
        crate::reset_killswitch();
        let canceled = LOG_FAULT_AT
            .lock()
            .unwrap()
            .expect("the withholding worker saw Stop");
        let took = ended.saturating_duration_since(canceled);
        assert!(took <= Duration::from_secs(5), "{:?}", took);
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        assert_eq!(stop.calls_async().await, 1, "test_stop");
        let (gaggle, _) = record(&folder);
        let (lost, _) = split_workers(&gaggle, |worker| worker["state"] == "lost");
        assert_eq!(lost.len(), 1, "{gaggle}");
        let reason = lost[0]["lost_reason"].as_str().unwrap_or_default();
        assert!(reason.contains("canceled"), "{}", gaggle);
    };
    tokio::time::timeout(Duration::from_secs(120), test)
        .await
        .expect("within 120 seconds");
}

/// Workers that say, a second into the run, they stopped for a reason the
/// manager does not know, 0 (unspecified) or one past those it knows: each
/// is lost, its reason's value named, and the run ends as every worker is
/// lost, long before its 30 seconds, degraded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_unknown_stop_reasons_lose_the_workers() {
    let server = answering().await;
    let folder = scratch("unknown-stop-reasons");
    let faults = [0, 99].map(|reason| TestFaults {
        stop_reason: Some(reason),
        ..Default::default()
    });
    let started = std::time::Instant::now();
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "2",
            "--increase-rate",
            "10",
            "--run-time",
            "30",
            "--report-file",
        ],
        &[],
        &faults,
    );
    let _ = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .expect("the Gaggle ends within 90 seconds");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    let (gaggle, _) = record(&folder);
    assert_eq!(gaggle["degraded"], true, "{gaggle}");
    let mut reasons: Vec<String> = gaggle["workers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|worker| {
            assert_eq!(worker["state"], "lost", "{}", gaggle);
            worker["lost_reason"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    reasons.sort();
    assert!(reasons[0].contains("an unknown reason, 0"), "{:?}", reasons);
    assert!(
        reasons[1].contains("an unknown reason, 99"),
        "{:?}",
        reasons
    );
}

/// A load test whose user triggers the killswitch, as Ctrl-C, after its
/// first GET /a.
fn canceling(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("A").register_transaction(Transaction::new(Arc::new(|user| {
                Box::pin(async move {
                    let _ = user.get("/a").await;
                    crate::trigger_killswitch("a test's Ctrl-C mid run");
                    Ok(())
                })
            }))),
        )
}

/// After a run canceled mid run, the killswitch reads the same whether the
/// manager collected its workers' logs or not, and as after a canceled run
/// of one process.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_after_a_canceled_run_reads_as_one_process() {
    let test = async {
        let server = answering().await;
        let mut after = Vec::new();
        for logs in [false, true] {
            let folder = scratch("canceled-killswitch");
            let mut args = vec!["--users", "1", "--run-time", "30", "--report-file"];
            if logs {
                args.push("--request-log");
            }
            let _ = gaggle(canceling, 1, server.base_url(), &folder, &args, &[], &[]).await;
            after.push(crate::is_killswitch_triggered());
            crate::reset_killswitch();
        }
        let host = server.base_url();
        let one = canceling(
            configuration(&[
                "--users",
                "1",
                "--run-time",
                "30",
                "--no-telnet",
                "--quiet",
                "--host",
                &host,
            ]),
            None,
        )
        .execute()
        .await;
        assert!(one.is_ok(), "{:?}", one.map(|_| ()));
        let single = crate::is_killswitch_triggered();
        crate::reset_killswitch();
        assert_eq!(
            after,
            [single, single],
            "without and with logs, then one process"
        );
    };
    tokio::time::timeout(Duration::from_secs(120), test)
        .await
        .expect("within 120 seconds");
}

/// A worker whose metrics processor takes its final metrics command 2.5
/// seconds late, past the worker's own bound and inside the manager's 3
/// seconds (25 of 30 seconds outside unit tests): the worker says it failed
/// before the manager gives up on it, so its loss names the real cause.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_late_final_handover_names_the_processor() {
    worker_fails_a_phase(
        TestFaults {
            final_after: Some(Duration::from_millis(2500)),
            ..Default::default()
        },
        "did not hand over",
        None,
    )
    .await;
}

/// A worker one of whose running deltas encodes past the message limit
/// keeps running, the manager recording the gap; one whose final does is
/// lost for that, its size named, never for losing its manager.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_delta_past_the_message_limit() {
    let server = answering().await;
    let folder = scratch("oversized-delta");
    let faults = [
        TestFaults {
            oversize_delta: 2,
            ..Default::default()
        },
        TestFaults {
            oversize_final: true,
            ..Default::default()
        },
    ];
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "2",
            "--increase-rate",
            "10",
            "--run-time",
            "4",
            "--report-file",
        ],
        &[],
        &faults,
    );
    let (manager, workers) = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .expect("the Gaggle ends within 90 seconds");
    manager.expect("the manager exits cleanly");
    assert!(workers[0].is_ok(), "{:?}", workers[0].as_ref().map(|_| ()));
    let (gaggle, _) = record(&folder);
    let (lost, kept) = split_workers(&gaggle, |worker| worker["state"] == "lost");
    assert_eq!((lost.len(), kept.len()), (1, 1), "{gaggle}");
    assert_eq!(kept[0]["state"], "finished", "{gaggle}");
    assert!(kept[0]["missing_deltas"].as_u64() >= Some(1), "{}", gaggle);
    let reason = lost[0]["lost_reason"].as_str().unwrap_or_default();
    assert!(reason.contains("bytes, more than the"), "{}", gaggle);
}

/// A worker that never says its users stopped is lost once the users
/// deadline passes, the request timeout and the final wait after Stop,
/// naming it; the other worker's final metrics merge.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_whose_users_never_stop_is_lost_at_the_deadline() {
    let server = answering().await;
    let folder = scratch("users-never-stop");
    let faults = [
        TestFaults::default(),
        TestFaults {
            withhold_final: true,
            ..Default::default()
        },
    ];
    let run = gaggle(
        quick,
        2,
        server.base_url(),
        &folder,
        &[
            "--users",
            "2",
            "--increase-rate",
            "10",
            "--run-time",
            "2",
            "--timeout",
            "1",
            "--report-file",
        ],
        &[],
        &faults,
    );
    let (manager, _) = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("the Gaggle ends within 60 seconds");
    manager.expect("the manager exits cleanly");
    let (gaggle, _) = record(&folder);
    let (lost, kept) = split_workers(&gaggle, |worker| worker["state"] == "lost");
    assert_eq!((lost.len(), kept.len()), (1, 1), "{gaggle}");
    let reason = lost[0]["lost_reason"].as_str().unwrap_or_default();
    assert!(
        reason.starts_with("its users had not stopped within 4 seconds of Stop"),
        "{}",
        gaggle
    );
    assert_eq!(kept[0]["state"], "finished", "{gaggle}");
    assert!(kept[0].get("final_missing").is_none(), "{}", gaggle);
}

/// A failing threshold check still queued in the manager's metrics processor
/// as the run stops is judged before the cancel is set aside: the worker's
/// late final metrics are kept, never lost as to a Ctrl-C during the wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_queued_threshold_failure_is_not_a_ctrl_c() {
    use std::sync::atomic::Ordering::SeqCst;
    // Disarmed however the test ends, so no later test meets the hold.
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            super::faults::HOLD_THRESHOLD_CHECK.store(false, SeqCst);
            super::faults::MANAGER_STOPPING.store(false, SeqCst);
        }
    }
    let _disarm = Disarm;
    let test = async {
        let server = answering().await;
        let folder = scratch("queued-threshold");
        super::faults::MANAGER_STOPPING.store(false, SeqCst);
        super::faults::HOLD_THRESHOLD_CHECK.store(true, SeqCst);
        super::faults::HELD_TO_STOP.store(false, SeqCst);
        let faults = [TestFaults {
            final_after: Some(Duration::from_secs(1)),
            ..Default::default()
        }];
        let (manager, _) = gaggle(
            quick,
            1,
            server.base_url(),
            &folder,
            &[
                "--users",
                "1",
                "--run-time",
                "5",
                "--threshold",
                "error-rate>99%",
                "--threshold-abort-delay",
                "1",
                "--report-file",
            ],
            &[],
            &faults,
        )
        .await;
        crate::reset_killswitch();
        super::faults::MANAGER_STOPPING.store(false, SeqCst);
        assert!(
            !super::faults::HOLD_THRESHOLD_CHECK.swap(false, SeqCst),
            "no threshold check was held"
        );
        assert!(
            super::faults::HELD_TO_STOP.swap(false, SeqCst),
            "the held check was released before the manager stopped"
        );
        // The held check judged, and failed, the run.
        assert!(
            matches!(manager, Err(crate::GooseError::ThresholdsFailed { .. })),
            "{:?}",
            manager.as_ref().map(|_| ())
        );
        let (gaggle, _) = record(&folder);
        let (lost, kept) = split_workers(&gaggle, |worker| worker["state"] == "lost");
        assert_eq!((lost.len(), kept.len()), (0, 1), "{gaggle}");
        assert_eq!(kept[0]["state"], "finished", "{gaggle}");
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// A Ctrl-C while the stopping manager waits for its processor to work
/// through its queue, behind a slow threshold check that passes, is a
/// Ctrl-C during the final wait: the manager ends within 5 seconds of the
/// drain, where it would wait the request timeout and more for a worker
/// that never says its users stopped, records that worker lost, canceled,
/// and runs test_stop once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_during_the_stop_drain() {
    use std::sync::atomic::Ordering::SeqCst;
    // Disarmed however the test ends, so no later test meets the hold.
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            super::faults::HOLD_THRESHOLD_CHECK.store(false, SeqCst);
            super::faults::CANCEL_IN_HELD_CHECK.store(false, SeqCst);
            super::faults::MANAGER_STOPPING.store(false, SeqCst);
        }
    }
    let _disarm = Disarm;
    let test = async {
        let server = MockServer::start_async().await;
        let (_a, stop) = (
            mock_ok(&server, "/a").await,
            mock_ok(&server, "/stop").await,
        );
        let folder = scratch("cancel-stop-drain");
        *super::faults::HELD_CHECK_ENDED.lock().unwrap() = None;
        super::faults::MANAGER_STOPPING.store(false, SeqCst);
        super::faults::HOLD_THRESHOLD_CHECK.store(true, SeqCst);
        super::faults::CANCEL_IN_HELD_CHECK.store(true, SeqCst);
        super::faults::HELD_TO_STOP.store(false, SeqCst);
        let faults = [TestFaults {
            withhold_final: true,
            ..Default::default()
        }];
        let build = |configuration, place| quick(configuration, place).test_stop(get("/stop"));
        let ((manager, ended), _) = gaggle_timed(
            build,
            1,
            server.base_url(),
            &folder,
            &[
                "--users",
                "1",
                "--run-time",
                "3",
                "--threshold",
                "error-rate<50%",
                "--report-file",
            ],
            &[],
            &faults,
        )
        .await;
        crate::reset_killswitch();
        super::faults::MANAGER_STOPPING.store(false, SeqCst);
        assert!(
            super::faults::HELD_TO_STOP.swap(false, SeqCst),
            "the held check was released before the manager stopped"
        );
        assert!(
            !super::faults::CANCEL_IN_HELD_CHECK.swap(false, SeqCst),
            "the held check sent no Ctrl-C"
        );
        let drained = super::faults::HELD_CHECK_ENDED
            .lock()
            .unwrap()
            .expect("the held check ended");
        let took = ended.saturating_duration_since(drained);
        assert!(took <= Duration::from_secs(5), "{:?}", took);
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        assert_eq!(stop.calls_async().await, 1, "test_stop");
        let (gaggle, _) = record(&folder);
        let (lost, _) = split_workers(&gaggle, |worker| worker["state"] == "lost");
        assert_eq!(lost.len(), 1, "{gaggle}");
        let reason = lost[0]["lost_reason"].as_str().unwrap_or_default();
        assert!(
            reason.starts_with("canceled while the manager waited"),
            "{}",
            gaggle
        );
    };
    tokio::time::timeout(Duration::from_secs(120), test)
        .await
        .expect("within 120 seconds");
}

/// A telnet Controller session with the manager in one process.
struct Telnet(tokio::net::TcpStream);

impl Telnet {
    async fn connect(port: u16) -> Self {
        for _ in 0..100 {
            if let Ok(stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                let mut telnet = Telnet(stream);
                telnet.read().await;
                return telnet;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the manager's telnet Controller never listened");
    }

    async fn read(&mut self) -> String {
        use tokio::io::AsyncReadExt;
        let mut reply = Vec::new();
        let mut buf = [0; 65536];
        while !reply.ends_with(b"goose> ") {
            let read = self.0.read(&mut buf).await.expect("telnet read");
            assert!(read > 0, "the telnet Controller closed");
            reply.extend_from_slice(&buf[..read]);
        }
        String::from_utf8(reply).unwrap()
    }

    async fn send(&mut self, command: &str) -> String {
        use tokio::io::AsyncWriteExt;
        self.0
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .unwrap();
        self.read().await
    }
}

/// The next report from each of `workers` workers, by link id.
async fn applied(
    reports: &mut tokio::sync::mpsc::UnboundedReceiver<super::run::WorkerApplied>,
    workers: usize,
) -> Vec<super::run::WorkerApplied> {
    let mut got = Vec::new();
    while got.len() < workers {
        let report = tokio::time::timeout(Duration::from_secs(10), reports.recv())
            .await
            .expect("each worker reports within 10 seconds")
            .expect("a report");
        assert!(!report.refused, "a worker refused a change: {:?}", report);
        got.push(report);
    }
    got.sort_by_key(|report| report.worker);
    got.dedup_by_key(|report| report.worker);
    assert_eq!(got.len(), workers, "a worker reported twice");
    got
}

/// The ordinals of every user the manager's merged metrics counted.
#[cfg(feature = "dashboard")]
async fn merged_ordinals(telnet: &mut Telnet) -> BTreeSet<usize> {
    let reply = telnet.send("metrics-json").await;
    let json = reply.trim_end_matches("goose> ").trim();
    let metrics: serde_json::Value = serde_json::from_str(json).unwrap();
    metrics["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|scenario| scenario["users"].as_array().cloned().unwrap_or_default())
        .map(|user| user.as_u64().unwrap() as usize)
        .collect()
}

/// A dashboard control POST to the manager, with its token: the reply's
/// JSON.
#[cfg(feature = "dashboard")]
async fn dashboard_post(port: u16, path: &str, body: &str) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/control/{path}"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer gaggle-control-token")
        .body(body.to_string())
        .send()
        .await
        .expect("dashboard POST");
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

/// A manager waiting for a Start, and two workers. Every runtime change
/// reaches both workers as their shares, which add up to the change, and
/// shows in the merged state: telnet `users` up and down, a WebSocket
/// `throttle`, the dashboard's users, a test plan replaced by telnet, a Stop
/// and a Start again, and a dashboard settings change of the throttle and
/// host together while idle, which the next run takes.
#[cfg(feature = "dashboard")]
#[test]
#[serial_test::serial]
fn gaggle_runtime_control_in_one_process() {
    // The test's future is too large for a test thread's stack: it runs on
    // a thread with a larger one.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap()
                .block_on(runtime_control_in_one_process())
        })
        .unwrap()
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[cfg(feature = "dashboard")]
async fn runtime_control_in_one_process() {
    const TELNET: u16 = 5226;
    const WEBSOCKET: u16 = 5227;
    const DASHBOARD: u16 = 5228;
    let test = async {
        no_token_from_the_environment();
        let server = answering().await;
        let second = MockServer::start_async().await;
        let second_a = mock_ok(&second, "/a").await;
        let folder = scratch("runtime-control");
        let host = server.base_url();
        let (telnet_port, websocket_port, dashboard_port) = (
            TELNET.to_string(),
            WEBSOCKET.to_string(),
            DASHBOARD.to_string(),
        );
        let manager_folder = folder.join("manager").display().to_string();
        let args = [
            "--manager",
            "--expect-workers",
            "2",
            "--manager-bind-port",
            "0",
            "--no-autostart",
            "--users",
            "20",
            "--increase-rate",
            "200",
            "--throttle-requests",
            "1000",
            "--telnet-port",
            &telnet_port,
            "--websocket",
            "--websocket-port",
            &websocket_port,
            "--dashboard",
            "--dashboard-port",
            &dashboard_port,
            "--dashboard-control",
            "--dashboard-auth-token",
            "gaggle-control-token",
            "--quiet",
            "--output-dir",
            &manager_folder,
            "--host",
            &host,
        ];
        let (bound, address) = tokio::sync::oneshot::channel();
        let manager = quick(configuration(&args), None).report_manager_address(bound);
        let manager = tokio::spawn(manager.execute());
        let port = address
            .await
            .expect("the manager listens")
            .port()
            .to_string();
        let (report, mut reports) = tokio::sync::mpsc::unbounded_channel();
        let mut workers = Vec::new();
        for worker in 0..2 {
            let worker_folder = folder.join(format!("worker{worker}")).display().to_string();
            let args = [
                "--worker",
                "--manager-host",
                "127.0.0.1",
                "--manager-port",
                &port,
                "--quiet",
                "--output-dir",
                &worker_folder,
            ];
            let attack = quick(configuration(&args), Some(worker)).report_applied(report.clone());
            workers.push(tokio::spawn(attack.execute()));
        }
        let mut telnet = Telnet::connect(TELNET).await;

        // Started by telnet: each worker runs half the users, and half the
        // throttle.
        let reply = telnet.send("start").await;
        assert!(reply.starts_with("load test started"), "{}", reply);
        let start = applied(&mut reports, 2).await;
        assert_eq!(
            start.iter().map(|r| r.users).collect::<Vec<_>>(),
            [Some(10), Some(10)]
        );
        assert_eq!(start.iter().map(|r| r.throttle).sum::<usize>(), 1_000);

        // Users up and down by telnet.
        let reply = telnet.send("users 60").await;
        assert!(reply.starts_with("users configured"), "{}", reply);
        let more = applied(&mut reports, 2).await;
        assert_eq!(more.iter().filter_map(|r| r.users).sum::<usize>(), 60);
        assert_eq!(more.iter().map(|r| r.throttle).sum::<usize>(), 1_000);
        // The merged state counts 60 users, ordinals 0 to 59, no gap.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let ordinals = merged_ordinals(&mut telnet).await;
            if ordinals == (0..60).collect() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "merged {:?}",
                ordinals
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let reply = telnet.send("users 20").await;
        assert!(reply.starts_with("users configured"), "{}", reply);
        let fewer = applied(&mut reports, 2).await;
        assert_eq!(fewer.iter().filter_map(|r| r.users).sum::<usize>(), 20);

        // An uneven split: the throttle follows each worker's users.
        let reply = telnet.send("users 21").await;
        assert!(reply.starts_with("users configured"), "{}", reply);
        assert!(!reply.contains("lost"), "{}", reply);
        let uneven = applied(&mut reports, 2).await;
        let counts: Vec<usize> = uneven.iter().map(|r| r.users.unwrap()).collect();
        let shares = super::control::live_throttle_shares(1_000, &counts);
        assert_eq!(
            uneven.iter().map(|r| r.throttle).collect::<Vec<_>>(),
            shares
        );
        assert_ne!(shares, [500, 500]);
        let mut sorted = shares.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [476, 524]);

        // What a Gaggle refuses, before any worker hears of it.
        let reply = telnet.send("throttle 1").await;
        assert!(
            reply.starts_with("failed to configure throttle, 1 is fewer than the 2 workers"),
            "{}",
            reply
        );
        let reply = telnet.send("throttle 1000001").await;
        assert!(
            reply.contains("throttle must be an integer from 1 to 1000000"),
            "{}",
            reply
        );
        let body = dashboard_post(DASHBOARD, "settings", r#"{"host": "http://127.0.0.1:9"}"#).await;
        assert_eq!(
            (&body["ok"], &body["field"]),
            (&false.into(), &"host".into()),
            "{}",
            body
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            reports.try_recv().is_err(),
            "a refused change reached a worker"
        );

        // A running Gaggle's throttle goes past 100 times each worker's
        // start, 500, uncapped.
        let reply = telnet.send("throttle 100002").await;
        assert!(reply.starts_with("throttle configured"), "{}", reply);
        let raised = applied(&mut reports, 2).await;
        assert_eq!(
            raised.iter().map(|r| r.throttle).collect::<Vec<_>>(),
            super::control::live_throttle_shares(100_002, &counts)
        );
        let reply = telnet.send("throttle 1000").await;
        assert!(reply.starts_with("throttle configured"), "{}", reply);
        let _ = applied(&mut reports, 2).await;

        // The rates follow each worker's users; the run time is the same.
        let reply = telnet.send("increase-rate 21").await;
        assert!(reply.starts_with("increase_rate configured"), "{}", reply);
        let rates = applied(&mut reports, 2).await;
        let rate = |r: &super::run::WorkerApplied| -> f64 {
            r.increase_rate.as_deref().unwrap().parse().unwrap()
        };
        for (report, count) in rates.iter().zip(&counts) {
            assert!((rate(report) - *count as f64).abs() < 1e-3, "{:?}", rates);
        }
        let reply = telnet.send("decrease-rate 21").await;
        assert!(reply.starts_with("decrease_rate configured"), "{}", reply);
        let _ = applied(&mut reports, 2).await;
        let reply = telnet.send("runtime 1h").await;
        assert!(reply.starts_with("run_time configured"), "{}", reply);
        let held = applied(&mut reports, 2).await;
        assert!(held
            .iter()
            .all(|r| r.run_time == held[0].run_time && r.run_time.is_some()));

        // Down to one user: a worker holds none, and keeps 1 request a
        // second of the throttle; then up again on both.
        let reply = telnet.send("users 1").await;
        assert!(reply.starts_with("users configured"), "{}", reply);
        let one = applied(&mut reports, 2).await;
        let mut ones: Vec<usize> = one.iter().map(|r| r.users.unwrap()).collect();
        ones.sort_unstable();
        assert_eq!(ones, [0, 1]);
        assert_eq!(one.iter().map(|r| r.throttle).sum::<usize>(), 1_000);
        // The worker with no users keeps 1 request a second.
        assert!(
            one.iter().any(|r| r.users == Some(0) && r.throttle == 1),
            "{:?}",
            one
        );
        let reply = telnet.send("users 4").await;
        assert!(reply.starts_with("users configured"), "{}", reply);
        let four = applied(&mut reports, 2).await;
        assert!(four.iter().all(|r| r.users == Some(2)), "{:?}", four);
        // The decrease rate follows the users that moved.
        assert!(four.iter().all(
            |r| (r.decrease_rate.as_deref().unwrap().parse::<f64>().unwrap() - 10.5).abs() < 1e-3
        ));

        // The throttle by WebSocket.
        let (mut websocket, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{WEBSOCKET}"))
                .await
                .unwrap();
        use futures::{SinkExt, StreamExt};
        websocket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"request":"throttle 100"}"#.into(),
            ))
            .await
            .unwrap();
        let reply = websocket
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap();
        assert!(reply.contains(r#""success":true"#), "{}", reply);
        let throttled = applied(&mut reports, 2).await;
        assert_eq!(throttled.iter().map(|r| r.throttle).sum::<usize>(), 100);
        // The manager's configuration says so too.
        let config = telnet.send("config-json").await;
        assert!(config.contains(r#""throttle_requests":100"#), "{}", config);

        // Users by the dashboard.
        let body = dashboard_post(DASHBOARD, "users", r#"{"users": 40}"#).await;
        assert_eq!(body["ok"], true, "{body}");
        let users = applied(&mut reports, 2).await;
        assert_eq!(users.iter().filter_map(|r| r.users).sum::<usize>(), 40);

        // A test plan replaced by telnet: each step's users add up.
        let reply = telnet.send("test-plan 50,1s;50,1h").await;
        assert!(reply.starts_with("test-plan configured"), "{}", reply);
        let plans = applied(&mut reports, 2).await;
        let steps: Vec<Vec<(usize, usize)>> =
            plans.iter().map(|r| r.test_plan.clone().unwrap()).collect();
        for (first, second) in steps[0].iter().zip(&steps[1]) {
            assert_eq!(first.0 + second.0, 50);
            assert_eq!(first.1, second.1);
        }
        assert_eq!(
            steps.iter().map(Vec::len).collect::<Vec<_>>(),
            [2, 2],
            "{:?}",
            plans
        );
        // A plan sent with the ramp settings it clears loses no worker.
        let body = dashboard_post(
            DASHBOARD,
            "settings",
            r#"{"test_plan": "50,1s;50,1h", "decrease_rate": null, "run_time_secs": null}"#,
        )
        .await;
        assert_eq!(body["ok"], true, "{}", body);
        assert!(
            !body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("lost"),
            "{}",
            body
        );
        let replaced = applied(&mut reports, 2).await;
        assert!(replaced
            .iter()
            .all(|r| r.test_plan.as_ref().map(Vec::len) == Some(2)));

        // Stopped by telnet once the plan holds, as a ramp down can not be
        // stopped: the manager waits for the next Start.
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let reply = telnet.send("stop").await;
        assert!(reply.starts_with("load test stopped"), "{}", reply);
        let _ = applied(&mut reports, 2).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot: serde_json::Value = reqwest::Client::new()
                .get(format!("http://127.0.0.1:{DASHBOARD}/api/v1/snapshot"))
                .header("Authorization", "Bearer gaggle-control-token")
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap_or_default();
            if snapshot["phase"] == "idle" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "never idle: {}",
                snapshot
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let body = dashboard_post(DASHBOARD, "settings", r#"{"throttle": 50}"#).await;
        assert_eq!(body["ok"], true, "{body}");
        let _ = applied(&mut reports, 2).await;

        // The throttle and host together while idle, then a Start by the
        // dashboard: the workers' next shares carry both.
        let second_host = second.base_url();
        let body = dashboard_post(
            DASHBOARD,
            "settings",
            &format!(r#"{{"throttle": 60, "host": "{second_host}"}}"#),
        )
        .await;
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["settings"]["throttle"], 60, "{body}");
        let _ = applied(&mut reports, 2).await;
        // A Start with settings is checked as settings alone are.
        let body = dashboard_post(DASHBOARD, "start", r#"{"throttle": 1}"#).await;
        assert_eq!(
            (&body["ok"], &body["field"]),
            (&false.into(), &"throttle".into()),
            "{}",
            body
        );
        let body = dashboard_post(DASHBOARD, "start", "{}").await;
        assert_eq!(body["ok"], true, "{body}");
        let again = applied(&mut reports, 2).await;
        assert!(again
            .iter()
            .all(|r| r.host.as_deref() == Some(second_host.as_str())));
        assert_eq!(again.iter().map(|r| r.throttle).sum::<usize>(), 60);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while second_a.calls_async().await == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the second host saw no request"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let _ = telnet.send("shutdown").await;
        manager.await.unwrap().expect("the manager exits cleanly");
        for worker in workers {
            worker.await.unwrap().expect("the worker exits cleanly");
        }
    };
    tokio::time::timeout(Duration::from_secs(60), Box::pin(test))
        .await
        .expect("within 60 seconds");
}

/// A manager and two workers in this process, the manager given `args` and
/// listening for telnet on [`IDLE_TELNET`]: the manager's and workers'
/// tasks, and the workers' reports.
#[allow(clippy::type_complexity)]
async fn telnet_gaggle(
    telnet: u16,
    host: &str,
    folder: &Path,
    args: &[&str],
) -> (
    tokio::task::JoinHandle<Result<GooseMetrics, crate::GooseError>>,
    Vec<tokio::task::JoinHandle<Result<GooseMetrics, crate::GooseError>>>,
    tokio::sync::mpsc::UnboundedReceiver<super::run::WorkerApplied>,
) {
    telnet_gaggle_with(telnet, host, folder, args, &[]).await
}

/// [`telnet_gaggle`], worker `n` given `faults[n]`.
#[allow(clippy::type_complexity)]
async fn telnet_gaggle_with(
    telnet: u16,
    host: &str,
    folder: &Path,
    args: &[&str],
    faults: &[TestFaults],
) -> (
    tokio::task::JoinHandle<Result<GooseMetrics, crate::GooseError>>,
    Vec<tokio::task::JoinHandle<Result<GooseMetrics, crate::GooseError>>>,
    tokio::sync::mpsc::UnboundedReceiver<super::run::WorkerApplied>,
) {
    no_token_from_the_environment();
    let telnet_port = telnet.to_string();
    let mut all = vec![
        "--manager",
        "--expect-workers",
        "2",
        "--manager-bind-port",
        "0",
        "--telnet-port",
        &telnet_port,
        "--quiet",
        "--host",
        host,
    ];
    all.extend_from_slice(args);
    let (bound, address) = tokio::sync::oneshot::channel();
    let manager = quick(configuration(&all), None).report_manager_address(bound);
    let manager = tokio::spawn(manager.execute());
    let port = address
        .await
        .expect("the manager listens")
        .port()
        .to_string();
    let (report, reports) = tokio::sync::mpsc::unbounded_channel();
    let mut workers = Vec::new();
    for worker in 0..2 {
        let worker_folder = folder.join(format!("worker{worker}")).display().to_string();
        let args = [
            "--worker",
            "--manager-host",
            "127.0.0.1",
            "--manager-port",
            &port,
            "--quiet",
            "--output-dir",
            &worker_folder,
        ];
        let attack = quick(configuration(&args), Some(worker))
            .report_applied(report.clone())
            .gaggle_test_faults(faults.get(worker).copied().unwrap_or_default());
        workers.push(tokio::spawn(attack.execute()));
    }
    (manager, workers, reports)
}

/// The telnet port of [`gaggle_start_keeps_an_idle_change_s_answer`].
const IDLE_ACK_TELNET: u16 = 5239;

/// A change made while the manager is idle, which a worker says it applied
/// only once the Start that follows it is preparing: the manager takes
/// that answer as it waits for the workers to prepare, so the change is
/// logged as applied and closed, never left open, warned about and holding
/// later requests back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_start_keeps_an_idle_change_s_answer() {
    use std::sync::atomic::Ordering::SeqCst;
    let test = async {
        let server = answering().await;
        let folder = scratch("idle-ack");
        let slow = TestFaults {
            slow_idle_ack: Some(Duration::from_secs(1)),
            ..TestFaults::default()
        };
        let (manager, workers, _reports) = telnet_gaggle_with(
            IDLE_ACK_TELNET,
            &server.base_url(),
            &folder,
            &["--no-autostart", "--users", "4", "--increase-rate", "100"],
            &[slow],
        )
        .await;
        let mut telnet = Telnet::connect(IDLE_ACK_TELNET).await;
        // Both workers joined: an idle change reaches both.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let reply = telnet.send("users 6").await;
            if reply.contains("accepted by 2 of 2 workers") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{}", reply);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        super::faults::CHANGES_DONE.store(0, SeqCst);
        let reply = telnet.send("users 10").await;
        assert!(reply.contains("accepted by 2 of 2 workers"), "{}", reply);
        // At once: the slow worker says it applied the change while the
        // Start waits for its Prepare answer.
        let reply = telnet.send("start").await;
        assert!(reply.starts_with("load test started"), "{}", reply);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        // Both idle changes: `users 6`, whose slow answer comes after the
        // count was reset and may land before the Start or in its prepare,
        // and `users 10`, whose answer always lands in the prepare.
        while super::faults::CHANGES_DONE.load(SeqCst) < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "the idle change was never logged as applied"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let reply = telnet.send("shutdown").await;
        assert!(reply.starts_with("load test shut down"), "{}", reply);
        manager
            .await
            .expect("the manager task ends")
            .expect("the manager ends cleanly");
        for worker in workers {
            worker.await.expect("the worker task ends").expect("clean");
        }
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// The telnet port of [`gaggle_slow_apply_keeps_its_worker`].
const SLOW_TELNET: u16 = 5238;

/// How long the slow worker of [`gaggle_slow_apply_keeps_its_worker`]
/// takes to apply its users change: past the manager's first warning, at
/// 30 seconds, so the warning is logged before the change is applied.
const SLOW_APPLY: Duration = Duration::from_secs(33);

/// A worker that takes 33 seconds to apply a users change says it received
/// the change at once, so it is not lost: the reply comes within the 15
/// seconds a client waits, saying every worker accepted it, the manager
/// warns at 30 seconds that the change is still being applied, and logs
/// when both workers applied it. A second change sent while the first is
/// still applying is answered as soon as both received it, and every worker
/// applies the two in the order sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_slow_apply_keeps_its_worker() {
    use std::sync::atomic::Ordering::SeqCst;
    let test = async {
        let server = answering().await;
        let folder = scratch("slow-apply");
        let manager_folder = folder.join("manager").display().to_string();
        let slow = TestFaults {
            slow_apply: Some(SLOW_APPLY),
            ..TestFaults::default()
        };
        let (manager, workers, mut reports) = telnet_gaggle_with(
            SLOW_TELNET,
            &server.base_url(),
            &folder,
            &[
                "--users",
                "4",
                "--increase-rate",
                "100",
                "--report-file",
                "--output-dir",
                &manager_folder,
            ],
            &[slow],
        )
        .await;
        let mut telnet = Telnet::connect(SLOW_TELNET).await;
        // Each worker took its share, 2 users, and runs.
        let start = applied(&mut reports, 2).await;
        assert_eq!(
            start.iter().map(|r| r.users).collect::<Vec<_>>(),
            [Some(2), Some(2)]
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        super::faults::DONE_WARNINGS.store(0, SeqCst);
        super::faults::CHANGES_DONE.store(0, SeqCst);

        let sent = std::time::Instant::now();
        let reply = telnet.send("users 10").await;
        let took = sent.elapsed();
        assert!(reply.starts_with("users configured"), "{}", reply);
        assert!(!reply.contains("lost"), "{}", reply);
        assert!(reply.contains("accepted by 2 of 2 workers"), "{}", reply);
        assert!(
            took < crate::controller::CONTROL_TIMEOUT,
            "answered after {:?}",
            took
        );
        // The second change, while the slow worker still applies the first.
        let reply = telnet.send("users 6").await;
        let took = sent.elapsed();
        assert!(reply.starts_with("users configured"), "{}", reply);
        assert!(!reply.contains("lost"), "{}", reply);
        assert!(
            took < crate::controller::CONTROL_TIMEOUT,
            "the second answered {:?} after the first was sent",
            took
        );

        // Every worker applies 5 users, then 3, in the order sent: the fast
        // one at once, the slow one after its 33 seconds.
        let mut seen: BTreeMap<u32, Vec<(Option<usize>, Duration)>> = BTreeMap::new();
        while seen.values().map(Vec::len).sum::<usize>() < 4 {
            let report = tokio::time::timeout(SLOW_APPLY + Duration::from_secs(10), reports.recv())
                .await
                .expect("every worker applies both changes")
                .expect("a report");
            assert!(!report.refused, "{:?}", report);
            seen.entry(report.worker)
                .or_default()
                .push((report.users, sent.elapsed()));
        }
        let mut slowest: Vec<Duration> = Vec::new();
        for (worker, applied) in &seen {
            let users: Vec<Option<usize>> = applied.iter().map(|(users, _)| *users).collect();
            assert_eq!(
                users,
                [Some(5), Some(3)],
                "worker {}: {:?}",
                worker,
                applied
            );
            slowest.push(applied[0].1);
        }
        slowest.sort_unstable();
        assert!(slowest[0] < Duration::from_secs(5), "{:?}", seen);
        assert!(slowest[1] >= SLOW_APPLY, "{:?}", seen);
        assert!(
            super::faults::DONE_WARNINGS.load(SeqCst) >= 1,
            "no warning while the slow worker applied its change"
        );
        // Both changes were logged as applied by every worker.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while super::faults::CHANGES_DONE.load(SeqCst) < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "{} changes logged as applied",
                super::faults::CHANGES_DONE.load(SeqCst)
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let reply = telnet.send("shutdown").await;
        assert!(reply.starts_with("load test shut down"), "{}", reply);
        manager
            .await
            .expect("the manager task ends")
            .expect("the manager ends cleanly");
        for worker in workers {
            worker
                .await
                .expect("the worker task ends")
                .expect("no worker was lost");
        }
        let (gaggle, _) = record(&folder);
        let (lost, _) = split_workers(&gaggle, |worker| worker["state"] == "lost");
        assert!(lost.is_empty(), "{}", gaggle);
    };
    tokio::time::timeout(Duration::from_secs(120), test)
        .await
        .expect("within 120 seconds");
}

/// The telnet port of [`telnet_gaggle`].
const IDLE_TELNET: u16 = 5236;
/// [`IDLE_TELNET`]'s, for a second Gaggle: a Controller outlives its run in
/// the test's runtime.
const PLANS_TELNET: u16 = 5237;

/// A manager waiting for a Start shuts down from Idle, and every worker
/// exits cleanly with it. A Gaggle whose workers run test plans of their
/// own, as one had no users at the start, refuses a users or rate change on
/// its running users ramp, and takes a test plan, split step by step.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_idle_shutdown_and_workers_running_plans() {
    let test = async {
        let server = answering().await;
        let folder = scratch("idle-shutdown");
        let host = server.base_url();
        let (manager, workers, _) = telnet_gaggle(
            IDLE_TELNET,
            &host,
            &folder,
            &["--no-autostart", "--users", "4"],
        )
        .await;
        let mut telnet = Telnet::connect(IDLE_TELNET).await;
        let reply = telnet.send("shutdown").await;
        assert!(reply.starts_with("load test shut down"), "{}", reply);
        manager.await.unwrap().expect("the manager exits cleanly");
        for worker in workers {
            worker.await.unwrap().expect("the worker exits cleanly");
        }
        drop(telnet);

        let (manager, workers, mut reports) = telnet_gaggle(
            PLANS_TELNET,
            &host,
            &folder,
            &["--users", "1", "--throttle-requests", "2"],
        )
        .await;
        let started = applied(&mut reports, 2).await;
        assert!(
            started.iter().all(|r| r.test_plan.is_some()),
            "{:?}",
            started
        );
        let mut telnet = Telnet::connect(PLANS_TELNET).await;
        for command in ["users 3", "increaserate 5"] {
            let reply = telnet.send(command).await;
            assert!(
                reply.contains("a worker of this Gaggle had no users when the run started"),
                "{}: {}",
                command,
                reply
            );
        }
        let reply = telnet.send("test-plan 4,1s;4,1h").await;
        assert!(reply.starts_with("test-plan configured"), "{}", reply);
        let plans = applied(&mut reports, 2).await;
        let steps: Vec<Vec<(usize, usize)>> =
            plans.iter().map(|r| r.test_plan.clone().unwrap()).collect();
        assert_eq!(
            steps.iter().map(Vec::len).collect::<Vec<_>>(),
            [2, 2],
            "{:?}",
            plans
        );
        for (first, second) in steps[0].iter().zip(&steps[1]) {
            assert_eq!(first.0 + second.0, 4, "{:?}", plans);
        }
        let reply = telnet.send("shutdown").await;
        assert!(reply.starts_with("load test shut down"), "{}", reply);
        manager.await.unwrap().expect("the manager exits cleanly");
        for worker in workers {
            worker.await.unwrap().expect("the worker exits cleanly");
        }
    };
    tokio::time::timeout(Duration::from_secs(30), Box::pin(test))
        .await
        .expect("within 30 seconds");
}

/// Worker health in a run of a manager and three workers: every worker
/// runs and then finishes, or is lost; the worker that stops on its own a
/// second in is lost, with an event; a users change and a throttle change
/// from the manager's Controller each make an event saying both workers
/// left acknowledged it; and the worker that says 95% of its CPU is busy
/// for 8 seconds is flagged saturated, once, then recovers, once. The
/// reports carry it all: the Workers and Events sections, and the Check
/// sentence.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_health_in_one_process() {
    const TELNET: u16 = 5236;
    let test = async {
        let server = answering().await;
        let folder = scratch("worker-health");
        let telnet_port = TELNET.to_string();
        let faults = [
            TestFaults::default(),
            TestFaults {
                exit_mid_run: true,
                ..Default::default()
            },
            TestFaults {
                busy_cpu: Some((95, Duration::from_secs(8))),
                ..Default::default()
            },
        ];
        let markdown_path = folder.join("report.md").display().to_string();
        let args = [
            "--users",
            "6",
            "--increase-rate",
            "60",
            "--run-time",
            "14",
            "--throttle-requests",
            "300",
            "--telnet-port",
            &telnet_port,
            "--report-file",
            "--report-file",
            &markdown_path,
        ];
        let run = gaggle(standard, 3, server.base_url(), &folder, &args, &[], &faults);
        let control = async {
            let mut telnet = Telnet::connect(TELNET).await;
            // The worker that exits a second in is lost first.
            tokio::time::sleep(Duration::from_secs(4)).await;
            let users = telnet.send("users 8").await;
            assert!(users.contains("accepted by 2 of 2 workers"), "{}", users);
            let throttle = telnet.send("throttle 200").await;
            assert!(
                throttle.contains("accepted by 2 of 2 workers"),
                "{}",
                throttle
            );
        };
        let ((manager, workers), ()) = tokio::join!(run, control);
        manager.expect("the manager exits cleanly");
        assert!(workers[0].is_ok() && workers[2].is_ok());
        assert!(workers[1].is_err(), "the worker that exits mid run fails");

        let (gaggle, html) = record(&folder);
        // The manager numbers workers as they join, which on a loaded host
        // is not the order they were started in: each is found by what it
        // did, and the two left are told apart by their CPU.
        let all = gaggle["workers"].as_array().unwrap();
        assert_eq!(all.len(), 3, "{:?}", all);
        type Workers<'a> = Vec<&'a serde_json::Value>;
        let (lost, finished): (Workers, Workers) =
            all.iter().partition(|worker| worker["state"] == "lost");
        assert_eq!(lost.len(), 1, "the worker that exits mid run: {:?}", all);
        assert_eq!(lost[0]["flags"][0], "lost", "{}", lost[0]);
        for worker in &finished {
            assert_eq!(worker["state"], "finished", "{}", worker);
            assert!(
                worker["running_secs"].as_f64().unwrap() > 10.0,
                "{}",
                worker
            );
            assert!(worker["requests"].as_u64().unwrap() > 0, "{}", worker);
            assert!(worker["cores"].as_u64().unwrap() > 0, "{}", worker);
        }
        // The busy worker, at 95% of its cores, peaks highest: at 95, its
        // own clock's span, never clamped at 100.
        let peak = |worker: &serde_json::Value| worker["peak_cpu_percent"].as_u64();
        let (busy, real) = match peak(finished[0]) >= peak(finished[1]) {
            true => (finished[0], &finished[1..]),
            false => (finished[1], &finished[..1]),
        };
        let busy_peak = peak(busy).unwrap();
        assert!((94..=96).contains(&busy_peak), "{}", busy);
        println!("busy worker peak CPU {}%", busy_peak);
        assert!(busy["saturated_secs"].as_u64().unwrap() >= 5, "{}", busy);
        assert!(
            busy["flags"]
                .as_array()
                .unwrap()
                .contains(&"saturated".into()),
            "{}",
            busy
        );
        // The real worker is far from saturated.
        assert!(
            real[0]["saturated_secs"].as_u64().unwrap_or(0) < 5,
            "{}",
            real[0]
        );

        let events: Vec<(String, String)> = gaggle["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["kind"].as_str().unwrap().to_string(),
                    event["text"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let of = |kind: &str| -> Vec<&String> {
            events
                .iter()
                .filter(|(k, _)| k == kind)
                .map(|(_, text)| text)
                .collect()
        };
        assert_eq!(of("run_started"), ["3 of 3 workers joined, run started"]);
        assert_eq!(of("worker_lost").len(), 1, "{:?}", events);
        assert_eq!(of("throttle_shares_rescaled").len(), 1, "{:?}", events);
        let users = of("users_changed");
        assert_eq!(users.len(), 1, "{:?}", events);
        assert!(
            users[0].starts_with("users 6 to 8, acknowledged by 2 of 2 in "),
            "{:?}",
            users
        );
        let throttle = of("throttle_changed");
        assert_eq!(throttle.len(), 1, "{:?}", events);
        assert!(
            throttle[0].starts_with("throttle set to 200 req/s, acknowledged by 2 of 2 in "),
            "{:?}",
            throttle
        );
        assert_eq!(of("worker_saturated").len(), 1, "{:?}", events);
        assert_eq!(of("worker_recovered").len(), 1, "{:?}", events);

        // The reports.
        for text in [
            "<h2>Workers</h2>",
            "<h2>Events</h2>",
            "Check: ",
            "saturated",
            "users 6 to 8",
        ] {
            assert!(html.contains(text), "{}", text);
        }
        let markdown = std::fs::read_to_string(folder.join("report.md")).unwrap();
        for text in ["## Workers", "## Events", "Check: "] {
            assert!(markdown.contains(text), "{}: {}", text, markdown);
        }
    };
    tokio::time::timeout(Duration::from_secs(120), test)
        .await
        .expect("within 120 seconds");
}

/// A worker that says 95% of its CPU is busy for 8 seconds, every other
/// delta of which is held back by a random time up to 2 seconds after it
/// reads its CPU time,
/// is flagged saturated, once, then recovers, once, and peaks at 95, not
/// clamped at 100: the manager times its CPU by the worker's own clock, not by
/// when each delta arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_cpu_is_timed_by_the_workers_clock() {
    let test = async {
        let server = answering().await;
        let folder = scratch("worker-cpu-clock");
        let faults = [
            TestFaults::default(),
            TestFaults {
                busy_cpu: Some((95, Duration::from_secs(8))),
                delay_deltas: Some(Duration::from_secs(2)),
                ..Default::default()
            },
        ];
        let args = [
            "--users",
            "4",
            "--increase-rate",
            "40",
            "--run-time",
            "14",
            "--throttle-requests",
            "300",
            "--report-file",
        ];
        let (manager, workers) =
            gaggle(standard, 2, server.base_url(), &folder, &args, &[], &faults).await;
        manager.expect("the manager exits cleanly");
        for worker in workers {
            worker.expect("the worker exits cleanly");
        }

        let (gaggle, _) = record(&folder);
        let all = gaggle["workers"].as_array().unwrap();
        assert_eq!(all.len(), 2, "{:?}", all);
        let peak = |worker: &serde_json::Value| worker["peak_cpu_percent"].as_u64();
        let busy = all.iter().max_by_key(|worker| peak(worker)).unwrap();
        let busy_peak = peak(busy).unwrap();
        assert!((94..=96).contains(&busy_peak), "{}", busy);
        println!("busy worker peak CPU {}%", busy_peak);
        assert!(busy["saturated_secs"].as_u64().unwrap() >= 5, "{}", busy);
        assert!(
            busy["flags"]
                .as_array()
                .unwrap()
                .contains(&"saturated".into()),
            "{}",
            busy
        );
        // The real worker is far from saturated.
        let real = all
            .iter()
            .find(|worker| worker["name"] != busy["name"])
            .unwrap();
        assert!(real["saturated_secs"].as_u64().unwrap_or(0) < 5, "{}", real);
        // A delta held 2 s after a 1 s interval can leave the 3 s gap that
        // flags the busy worker lagging, and its recovery from that is a
        // worker_recovered too: each recovery is told apart by its text.
        let events = gaggle["events"].as_array().unwrap();
        let texts = |kind: &str| -> Vec<&str> {
            events
                .iter()
                .filter(|event| event["kind"] == kind)
                .map(|event| event["text"].as_str().unwrap())
                .collect()
        };
        assert_eq!(texts("worker_saturated").len(), 1, "{:?}", events);
        let (saturation, other): (Vec<&str>, Vec<&str>) = texts("worker_recovered")
            .into_iter()
            .partition(|text| text.ends_with("no longer saturated"));
        assert_eq!(saturation.len(), 1, "{:?}", events);
        assert!(
            other.iter().all(|text| text.ends_with("no longer lagging")),
            "{:?}",
            events
        );
    };
    tokio::time::timeout(Duration::from_secs(120), test)
        .await
        .expect("within 120 seconds");
}

/// The manager's dashboard snapshot, read with `token`, as text: `None`
/// until the dashboard answers with one.
#[cfg(feature = "dashboard")]
async fn dashboard_snapshot(port: u16, token: &str) -> Option<String> {
    let response = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/api/v1/snapshot"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.text().await.ok()
}

/// The first snapshot within `within` that `test` passes, its JSON and its
/// text.
#[cfg(feature = "dashboard")]
async fn dashboard_until(
    port: u16,
    token: &str,
    within: Duration,
    what: &str,
    test: impl Fn(&serde_json::Value) -> bool,
) -> (serde_json::Value, String) {
    let deadline = std::time::Instant::now() + within;
    loop {
        let text = dashboard_snapshot(port, token).await.unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if test(&json) {
            return (json, text);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{} within {:?}: {} (phase {}, {} bytes)",
            what,
            within,
            json["gaggle"],
            json["phase"],
            text.len()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A manager's dashboard shows its workers: three running; a users change
/// through the dashboard's settings route as an event saying every worker
/// acknowledged it, and in each worker's users; and a worker that stops a
/// few seconds in, lost, with an event. The snapshot names workers only:
/// no address, no port of a peer, no token.
#[cfg(feature = "dashboard")]
#[test]
#[serial_test::serial]
fn gaggle_dashboard_shows_the_workers() {
    // On its own thread, waited on for at most 150 seconds: a failure
    // leaves the load test's users running, and their runtime would never
    // finish dropping.
    let (done, ended) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap()
                .block_on(dashboard_shows_the_workers());
            let _ = done.send(());
        })
        .unwrap();
    ended
        .recv_timeout(Duration::from_secs(150))
        .expect("the test ends within 150 seconds, without a failure");
}

#[cfg(feature = "dashboard")]
async fn dashboard_shows_the_workers() {
    const DASHBOARD: u16 = 5248;
    const TOKEN: &str = "gaggle-dashboard-panel-token";
    let test = async {
        no_token_from_the_environment();
        let server = answering().await;
        // The target by name, so no address of the test's own is in the
        // snapshot's hosts.
        let host = server.base_url().replace("127.0.0.1", "localhost");
        let folder = scratch("dashboard-workers");
        let dashboard_port = DASHBOARD.to_string();
        let manager_folder = folder.join("manager").display().to_string();
        let args = [
            "--manager",
            "--expect-workers",
            "3",
            "--manager-bind-port",
            "0",
            "--no-telnet",
            "--users",
            "6",
            "--increase-rate",
            "60",
            "--run-time",
            "20",
            "--throttle-requests",
            "300",
            "--dashboard",
            "--dashboard-port",
            &dashboard_port,
            "--dashboard-control",
            "--dashboard-auth-token",
            TOKEN,
            "--quiet",
            "--output-dir",
            &manager_folder,
            "--host",
            &host,
        ];
        let (bound, address) = tokio::sync::oneshot::channel();
        let manager = standard(configuration(&args), None).report_manager_address(bound);
        let manager = tokio::spawn(manager.execute());
        let manager_port = address.await.expect("the manager listens").port();
        let port = manager_port.to_string();
        let mut workers = Vec::new();
        for worker in 0..3 {
            let worker_folder = folder.join(format!("worker{worker}")).display().to_string();
            let args = [
                "--worker",
                "--manager-host",
                "127.0.0.1",
                "--manager-port",
                &port,
                "--quiet",
                "--output-dir",
                &worker_folder,
            ];
            // The second worker stops on its own 8 seconds in.
            let faults = TestFaults {
                exit_after_secs: (worker == 1).then_some(8),
                ..Default::default()
            };
            let attack = standard(configuration(&args), Some(worker)).gaggle_test_faults(faults);
            workers.push(tokio::spawn(attack.execute()));
        }

        // Three workers running, each in the grid.
        let (running, _) = dashboard_until(
            DASHBOARD,
            TOKEN,
            Duration::from_secs(20),
            "3 running",
            |s| s["gaggle"]["summary"]["running"] == 3,
        )
        .await;
        let gaggle = &running["gaggle"];
        assert_eq!(gaggle["expected_workers"], 3);
        assert_eq!(gaggle["grid"]["names"].as_array().unwrap().len(), 3);
        assert_eq!(gaggle["grid"]["states"], serde_json::json!([2, 2, 2]));
        assert_eq!(gaggle["previews"]["workers"], 3);
        assert_eq!(
            gaggle["previews"]["users"],
            serde_json::json!([{"value": 2, "workers": 3}])
        );

        // A users change through the dashboard: its reply says all three
        // acknowledged it, an event says so, and each worker runs its share.
        // A running load test takes users through the users route; the
        // settings route refuses them then.
        let body = dashboard_post_token(DASHBOARD, TOKEN, "users", r#"{"users": 9}"#).await;
        assert_eq!(body["ok"], true, "{}", body);
        assert_eq!(body["acks"]["acknowledged"], 3, "{}", body);
        assert_eq!(body["acks"]["workers"], 3, "{}", body);
        let (changed, _) = dashboard_until(
            DASHBOARD,
            TOKEN,
            Duration::from_secs(10),
            "users event",
            |s| {
                let gaggle = &s["gaggle"];
                let users = gaggle["grid"]["users"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let event = gaggle["events"]["texts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|text| {
                        text.as_str().is_some_and(|text| {
                            text.starts_with("users 6 to 9, acknowledged by 3 of 3")
                        })
                    });
                event && users.iter().all(|users| users == 3)
            },
        )
        .await;
        let events = &changed["gaggle"]["events"];
        let at = events["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .position(|kind| kind == "users_changed")
            .unwrap();
        assert_eq!(events["labels"][at], "users 9");
        assert_eq!(changed["gaggle"]["summary"]["last_users_change"]["to"], 9);

        // A throttle change through the settings route: an event, and the
        // preview of its split over the three workers' 3 users each.
        let body = dashboard_post_token(DASHBOARD, TOKEN, "settings", r#"{"throttle": 240}"#).await;
        assert_eq!(body["ok"], true, "{}", body);
        assert_eq!(body["acks"]["acknowledged"], 3, "{}", body);
        let (throttled, _) = dashboard_until(
            DASHBOARD,
            TOKEN,
            Duration::from_secs(10),
            "throttle event",
            |s| {
                s["gaggle"]["previews"]["throttle"]
                    == serde_json::json!([{"value": 80, "workers": 3}])
            },
        )
        .await;
        let labels = &throttled["gaggle"]["events"]["labels"];
        assert!(
            labels.as_array().unwrap().contains(&"throttle 240".into()),
            "{}",
            labels
        );

        // The worker that stops is lost, with an event.
        let (lost, text) =
            dashboard_until(DASHBOARD, TOKEN, Duration::from_secs(20), "1 lost", |s| {
                s["gaggle"]["summary"]["lost"] == 1
            })
            .await;
        let gaggle = &lost["gaggle"];
        assert_eq!(gaggle["summary"]["running"], 2);
        assert!(
            gaggle["events"]["kinds"]
                .as_array()
                .unwrap()
                .contains(&"worker_lost".into()),
            "{}",
            gaggle["events"]
        );
        let first = gaggle["workers"]["workers"][0].as_u64().unwrap() as usize;
        assert_eq!(
            gaggle["grid"]["states"][first], 6,
            "the lost worker is the first row"
        );
        assert_eq!(gaggle["workers"]["flags"][0], serde_json::json!(["lost"]));
        assert!(gaggle["summary"]["attention"]
            .as_str()
            .unwrap()
            .starts_with("1 worker needs attention: "));

        // Names only: no address and no token anywhere in the snapshot, and
        // in the gaggle section no host name nor the manager's port.
        assert!(!text.contains("127.0.0.1"), "{}", text);
        assert!(!text.contains(TOKEN), "{}", text);
        let section = gaggle.to_string();
        for address in ["localhost", "[::1]", "0.0.0.0", "http"] {
            assert!(!section.contains(address), "{} in {}", address, section);
        }
        assert!(
            !section.contains(&format!(":{port}")) && !section.contains(&format!("{port},")),
            "the manager's port {} in {}",
            port,
            section
        );

        manager.await.unwrap().expect("the manager exits cleanly");
        for (worker, end) in workers.into_iter().enumerate() {
            let end = end.await.unwrap();
            assert_eq!(
                end.is_err(),
                worker == 1,
                "worker {}: {:?}",
                worker,
                end.map(|_| ())
            );
        }
    };
    tokio::time::timeout(Duration::from_secs(120), Box::pin(test))
        .await
        .expect("within 120 seconds");
}

/// A dashboard control POST with `token`: the reply's JSON.
#[cfg(feature = "dashboard")]
async fn dashboard_post_token(port: u16, token: &str, path: &str, body: &str) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/control/{path}"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(body.to_string())
        .send()
        .await
        .expect("dashboard POST");
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}
