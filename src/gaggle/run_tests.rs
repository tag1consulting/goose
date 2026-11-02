//! A manager and two workers in one process, against a mock server.

use super::run::{LogsFault, PrepareFault, TestFaults, LOG_FAULT_AT};
use crate::config::GooseConfiguration;
use crate::goose::{Scenario, Transaction};
use crate::metrics::GooseMetrics;
use crate::GooseAttack;
use httpmock::{Method::GET, MockServer};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// A transaction that GETs `path`.
fn get(path: &'static str) -> Transaction {
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

fn configuration(args: &[&str]) -> GooseConfiguration {
    GooseConfiguration::parse_args_default(args).unwrap()
}

/// A fresh folder for this test's runs.
fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "goose-gaggle-{name}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// The lines in `file`.
fn lines(file: &Path) -> usize {
    std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("{}: {e}", file.display()))
        .lines()
        .count()
}

/// The users of each scenario.
fn scenario_users(metrics: &GooseMetrics) -> Vec<BTreeSet<usize>> {
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
async fn gaggle(
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
        "--no-telnet",
        "--quiet",
        "--host",
        &host,
    ];
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
fn clean(
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
    let mock = |path: &'static str| {
        let server = &server;
        async move {
            server
                .mock_async(|when, then| {
                    when.method(GET).path(path);
                    then.status(200).body("ok");
                })
                .await
        }
    };
    let (a, b, pre, start, stop) = (
        mock("/a").await,
        mock("/b").await,
        mock("/pre").await,
        mock("/start").await,
        mock("/stop").await,
    );
    let (data0, data1) = (mock("/data/w0").await, mock("/data/w1").await);
    let folder = scratch("b1");
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
    let _ = std::fs::remove_dir_all(&folder);
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
    let quick = |configuration, _| {
        GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(Scenario::new("A").register_transaction(get("/a")))
    };
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
    let _ = std::fs::remove_dir_all(&folder);
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
        // The last resort records that it would have ended the process:
        // that it does not is what this test checks.
        let last_resort = worker.last_resort_record();
        let worker = tokio::spawn(worker.execute());
        // Once it joined, the manager goes.
        assert_eq!(joined.recv().await, Some(1));
        manager.abort();
        let result = worker.await.expect("the worker task ends");
        match result {
            Err(crate::GooseError::Gaggle { detail }) => {
                assert!(detail.contains("lost the manager"), "{}", detail)
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        // Past the switch's last resort: had the link not marked itself
        // done, it would have ended the process.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let forced = last_resort
            .lock()
            .unwrap()
            .clone()
            .expect("the worker joined");
        assert!(!forced.load(std::sync::atomic::Ordering::Relaxed));
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
    let _ = std::fs::remove_dir_all(&folder);
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
    // Joining, a 2 second start, a ramp, 3 seconds of run, and the
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
    // The worker's deadline for its final metrics is the manager's wait
    // for them, which starts first, when it sends Stop: either may end it.
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
    let _ = std::fs::remove_dir_all(&folder);
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
    let _ = std::fs::remove_dir_all(&folder);
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
fn worker_logs(folder: &Path, file: &str) -> Vec<Vec<u8>> {
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
        let _ = std::fs::remove_dir_all(&folder);
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
    let _ = std::fs::remove_dir_all(&folder);
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
    let _ = std::fs::remove_dir_all(&folder);
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
    let _ = std::fs::remove_dir_all(&folder);
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
    let mock = |path: &'static str| {
        let server = &server;
        async move {
            server
                .mock_async(|when, then| {
                    when.method(GET).path(path);
                    then.status(200).body("ok");
                })
                .await
        }
    };
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
/// one it sent: every delta from 2 on is refused, and its final never comes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_lost_worker_record_keeps_the_last_merged_seq() {
    worker_fails_a_phase(
        TestFaults {
            silent_final: true,
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
            let mut manager_args = vec!["--users", "2", "--gaggle-connect-timeout", "1s"];
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
        let _ = std::fs::remove_dir_all(&folder);
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
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
            .to_string();
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
    // Joining, a 2 second start, a 1 second ramp and 3 seconds of run.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert!(a.calls_async().await > 0);
    let _ = std::fs::remove_dir_all(&folder);
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
    let _ = std::fs::remove_dir_all(&folder);
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
        let mock = |path: &'static str| {
            let server = &server;
            async move {
                server
                    .mock_async(|when, then| {
                        when.method(GET).path(path);
                        then.status(200).body("ok");
                    })
                    .await
            }
        };
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
        let _ = std::fs::remove_dir_all(&folder);
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
/// 2 seconds of it, with an error saying it was canceled.
fn canceled_within(manager: &Result<GooseMetrics, crate::GooseError>, took: Duration, stage: &str) {
    crate::reset_killswitch();
    match manager {
        Err(crate::GooseError::Gaggle { detail }) => {
            assert!(detail.contains("canceled"), "{}: {}", stage, detail)
        }
        other => panic!("{stage}: {:?}", other.as_ref().map(|_| ())),
    }
    assert!(took <= Duration::from_secs(2), "{}: {:?}", stage, took);
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
            let mock = |path: &'static str| {
                let server = &server;
                async move {
                    server
                        .mock_async(|when, then| {
                            when.method(GET).path(path);
                            then.status(200).body("ok");
                        })
                        .await
                }
            };
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
/// set it, its run record, and the run's folder, for the caller to remove.
async fn faulty_logs_run(
    fault: LogsFault,
) -> (
    Result<GooseMetrics, crate::GooseError>,
    Option<Duration>,
    serde_json::Value,
    PathBuf,
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
/// stops waiting within 2 seconds, deletes what arrived, and records the
/// log as left on its worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_killswitch_while_collecting_logs() {
    let test = async {
        let (manager, took, gaggle, folder) = faulty_logs_run(LogsFault::Slow).await;
        assert!(manager.is_ok(), "{:?}", manager.map(|_| ()));
        let took = took.expect("the worker triggered the killswitch");
        assert!(took <= Duration::from_secs(2), "{:?}", took);
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{gaggle}"
        );
        assert_eq!(parts(&folder), Vec::<PathBuf>::new());
        let _ = std::fs::remove_dir_all(&folder);
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
        // The 2 second idle timeout from the first chunk, and the reports.
        assert!(took <= Duration::from_secs(5), "{:?}", took);
        assert_eq!(
            gaggle["logs_left"].as_array().map(Vec::len),
            Some(1),
            "{gaggle}"
        );
        assert_eq!(parts(&folder), Vec::<PathBuf>::new());
        let _ = std::fs::remove_dir_all(&folder);
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
                .delay(super::run::FINAL_TIMEOUT + Duration::from_secs(5));
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
            "10",
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
    let _ = std::fs::remove_dir_all(&folder);
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
        let _ = std::fs::remove_dir_all(&folder);
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
        super::run::FAIL_LOG_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
        let (manager, _, gaggle, folder) = faulty_logs_run(LogsFault::None).await;
        super::run::FAIL_LOG_WRITES.store(false, std::sync::atomic::Ordering::Relaxed);
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
        let _ = std::fs::remove_dir_all(&folder);
    };
    tokio::time::timeout(Duration::from_secs(60), test)
        .await
        .expect("within 60 seconds");
}

/// The manager writes its reports before any worker's log arrives, so a
/// disk the logs fill costs only logs.
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
        let _ = std::fs::remove_dir_all(&folder);
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
    let mock = |path: &'static str| {
        let server = &server;
        async move {
            server
                .mock_async(|when, then| {
                    when.method(GET).path(path);
                    then.status(200).body("ok");
                })
                .await
        }
    };
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
    let _ = std::fs::remove_dir_all(&folder);
}
