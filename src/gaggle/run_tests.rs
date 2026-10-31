//! A manager and two workers in one process, against a mock server.

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
/// pause past the metrics reset, so every later request is measured.
fn on_start() -> Transaction {
    Transaction::new(Arc::new(|user| {
        Box::pin(async move {
            let _ = user.get("/pre").await;
            let data = String::from_utf8_lossy(user.worker_data()).into_owned();
            let _ = user.get(&format!("/data/{data}")).await;
            tokio::time::sleep(Duration::from_millis(800)).await;
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

/// Run a manager with `manager_args` and two workers in this process, each
/// with its own run folder under `folder`; the manager's result.
async fn gaggle(
    server: &MockServer,
    folder: &Path,
    manager_args: &[&str],
) -> Result<GooseMetrics, crate::GooseError> {
    gaggle_of(2, server, folder, manager_args).await
}

/// [`gaggle`], with `count` workers.
async fn gaggle_of(
    count: usize,
    server: &MockServer,
    folder: &Path,
    manager_args: &[&str],
) -> Result<GooseMetrics, crate::GooseError> {
    let count_text = count.to_string();
    let host = server.base_url();
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
        "--output-dir",
        &manager_folder,
    ];
    args.extend_from_slice(manager_args);
    let (bound, address) = tokio::sync::oneshot::channel();
    let manager = load_test(configuration(&args)).report_manager_address(bound);
    let manager = tokio::spawn(manager.execute());
    let address = match address.await {
        Ok(address) => address,
        Err(_) => panic!("the manager never listened: {:?}", manager.await),
    };
    let port = address.port().to_string();
    let mut workers = Vec::new();
    for worker in 0..count {
        let worker_folder = folder.join(format!("worker{worker}")).display().to_string();
        let worker = load_test(configuration(&[
            "--worker",
            "--manager-host",
            "127.0.0.1",
            "--manager-port",
            &port,
            "--no-telnet",
            "--quiet",
            "--output-dir",
            &worker_folder,
        ]));
        workers.push(tokio::spawn(worker.execute()));
    }
    for worker in workers {
        worker
            .await
            .expect("the worker task ends")
            .expect("the worker exits cleanly");
    }
    manager.await.expect("the manager task ends")
}

/// B1: one manager and two workers. The manager's totals are the server's
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
    let run = gaggle(
        &server,
        &folder,
        &[
            "--users",
            "6",
            "--increase-rate",
            "100",
            "--run-time",
            "3",
            "--report-file",
            "--request-log",
            "--transaction-log",
            "--scenario-log",
        ],
    );
    let metrics = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("the Gaggle ends within 30 seconds")
        .expect("the manager exits cleanly");

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
    let server = MockServer::start_async().await;
    for path in [
        "/a", "/b", "/pre", "/start", "/stop", "/data/w0", "/data/w1", "/data/w2",
    ] {
        server
            .mock_async(|when, then| {
                when.method(GET).path(path);
                then.status(200);
            })
            .await;
    }
    let folder = scratch("no-users");
    let run = gaggle_of(
        3,
        &server,
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
    );
    let metrics = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("the Gaggle ends within 30 seconds")
        .expect("the manager exits cleanly");
    assert_eq!(metrics.total_users, 2);
    let _ = std::fs::remove_dir_all(&folder);
}

/// A manager that goes away before the run, its task dropped mid wait:
/// its worker loses it, returns an error, and its link stops the dead man's
/// switch's last resort, so this test process lives on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn gaggle_worker_survives_its_manager_going_before_the_run() {
    let test = async {
        let server = MockServer::start_async().await;
        let host = server.base_url();
        let (bound, address) = tokio::sync::oneshot::channel();
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
        .report_manager_address(bound);
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
            "--no-telnet",
            "--quiet",
        ]));
        let worker = tokio::spawn(worker.execute());
        tokio::time::sleep(Duration::from_secs(2)).await;
        manager.abort();
        let result = worker.await.expect("the worker task ends");
        match result {
            Err(crate::GooseError::Gaggle { detail }) => {
                assert!(detail.contains("lost the manager"), "{}", detail)
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        // Past the switch's 4 second last resort: had the link not marked
        // itself done, the process would have exited.
        tokio::time::sleep(Duration::from_secs(5)).await;
    };
    tokio::time::timeout(Duration::from_secs(30), test)
        .await
        .expect("within 30 seconds");
}
