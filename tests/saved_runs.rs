//! Saved runs: every run is saved to its own directory under the runs
//! directory. Each test saves into its own temporary `--runs-dir`, never into
//! `goose-runs/` in the repository.
//!
//! Every test here is `#[serial]`: one of them triggers the killswitch, which
//! is global to the process and would cancel a run in a test beside it.

use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration, Instant};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_index_then_cancel(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    goose::trigger_killswitch("error budget spent");
    Ok(())
}

fn setup_mock_server_endpoints(server: &MockServer) -> Mock<'_> {
    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200).body("ok");
    })
}

/// A new empty directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "goose-saved-runs-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The run directories under `runs_dir`, sorted by name.
fn run_dirs(runs_dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(runs_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    dirs
}

/// The names of the files in a run directory, sorted.
fn file_names(run_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(run_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn read_run_json(run_dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(run_dir.join("run.json")).expect("run.json");
    serde_json::from_str(&text).expect("run.json is JSON")
}

/// True for a run id: `YYYY-MM-DD-HHMMSS` with an optional `-N` suffix.
fn looks_like_run_id(id: &str) -> bool {
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}-\d{6}(-\d{1,3})?$").unwrap();
    re.is_match(id)
}

fn scenario(transaction: Transaction) -> Scenario {
    scenario!("LoadTest").register_transaction(transaction)
}

#[tokio::test]
#[serial]
async fn autostart_run_writes_one_run_directory() {
    let server = MockServer::start();
    let mock = setup_mock_server_endpoints(&server);
    // A runs directory that does not exist yet: Goose creates it.
    let parent = temp_dir("autostart");
    let runs = parent.join("goose-runs");
    let runs_str = runs.to_string_lossy().into_owned();

    let configuration = common::build_configuration(&server, vec!["--runs-dir", &runs_str]);
    let goose_metrics = common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;
    assert!(mock.calls() > 0);

    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 1, "exactly one run directory: {dirs:?}");
    let run_dir = &dirs[0];
    let id = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(looks_like_run_id(&id), "run id {}", id);
    assert_eq!(
        file_names(run_dir),
        vec!["report.html", "report.json", "report.md", "run.json"]
    );
    // Goose created the runs directory, so it wrote a .gitignore in it.
    let gitignore = std::fs::read_to_string(runs.join(".gitignore")).unwrap();
    assert!(gitignore.starts_with("*\n"));

    let run = read_run_json(run_dir);
    assert_eq!(run["format"], 1);
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["ended_by"], "completed");
    assert!(run["canceled_reason"].is_null());
    assert!(run["baseline"].is_null());
    assert_eq!(run["goose_version"], env!("CARGO_PKG_VERSION"));
    assert!(run["started"].as_str().unwrap().ends_with('Z'));
    assert!(run["ended"].as_str().unwrap() >= run["started"].as_str().unwrap());
    let requests: usize = goose_metrics
        .requests
        .values()
        .map(|r| r.success_count + r.fail_count)
        .sum();
    assert_eq!(run["requests"], requests as u64);
    assert_eq!(run["failed_requests"], 0);
    let files = run["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    for file in files {
        let name = file["name"].as_str().unwrap();
        let bytes = std::fs::metadata(run_dir.join(name)).unwrap().len();
        assert_eq!(file["bytes"], bytes, "{name} size");
    }

    // report.json carries the test plan, so its Markdown has a plan overview.
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("report.json")).unwrap())
            .unwrap();
    assert!(!report["raw_metrics"]["history"]
        .as_array()
        .unwrap()
        .is_empty());
    let markdown = std::fs::read_to_string(run_dir.join("report.md")).unwrap();
    assert!(markdown.contains("Plan Overview"));

    std::fs::remove_dir_all(&parent).ok();
}

#[tokio::test]
#[serial]
async fn no_save_writes_nothing() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("nosave");
    let runs_str = runs.to_string_lossy().into_owned();

    // --no-save wins over --runs-dir.
    let configuration =
        common::build_configuration(&server, vec!["--runs-dir", &runs_str, "--no-save"]);
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;
    assert!(run_dirs(&runs).is_empty());
    std::fs::remove_dir_all(&runs).ok();
}

#[tokio::test]
#[serial]
async fn no_print_metrics_still_writes_report_file() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("noprint");
    let runs_str = runs.to_string_lossy().into_owned();
    let report = runs.join("report-file.html");
    let report_str = report.to_string_lossy().into_owned();

    let configuration = common::build_configuration(
        &server,
        vec![
            "--runs-dir",
            &runs_str,
            "--report-file",
            &report_str,
            "--no-print-metrics",
        ],
    );
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;

    assert!(
        std::fs::metadata(&report).unwrap().len() > 0,
        "--report-file is written under --no-print-metrics"
    );
    // The saved run's HTML is the same report.
    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 1);
    assert_eq!(
        std::fs::read(&report).unwrap(),
        std::fs::read(dirs[0].join("report.html")).unwrap(),
        "the HTML report is generated once for both"
    );
    std::fs::remove_dir_all(&runs).ok();
}

/// The directory `get_index_then_remove_reports` removes, once.
static REPORTS_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub async fn get_index_then_remove_reports(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    if let Some(dir) = REPORTS_DIR.lock().unwrap().take() {
        std::fs::remove_dir_all(dir).ok();
    }
    Ok(())
}

#[tokio::test]
#[serial]
async fn report_file_error_after_the_run_is_returned_and_the_run_saved() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("report-error");
    let runs_str = runs.to_string_lossy().into_owned();
    // --report-file in a directory the load test removes once it runs, so
    // the report can be created at start but not written at the end.
    let reports = temp_dir("report-error-out");
    let report_str = reports.join("report.html").to_string_lossy().into_owned();
    *REPORTS_DIR.lock().unwrap() = Some(reports.clone());

    let configuration = common::build_configuration(
        &server,
        vec!["--runs-dir", &runs_str, "--report-file", &report_str],
    );
    let result = common::build_load_test(
        configuration,
        vec![scenario(transaction!(get_index_then_remove_reports))],
        None,
        None,
    )
    .execute()
    .await;

    assert!(
        !reports.exists(),
        "the transaction removed the reports directory"
    );
    assert!(result.is_err(), "the --report-file error is returned");
    // The saved run is written all the same.
    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 1);
    assert!(dirs[0].join("run.json").exists());
    std::fs::remove_dir_all(&runs).ok();
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn unwritable_explicit_runs_dir_is_an_error_before_load() {
    use std::os::unix::fs::PermissionsExt;

    let server = MockServer::start();
    let mock = setup_mock_server_endpoints(&server);
    let parent = temp_dir("readonly");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(parent.join("probe"), "").is_ok() {
        eprintln!("skipping: running as root, permissions are not enforced");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&parent).ok();
        return;
    }
    let runs = parent.join("runs");
    let runs_str = runs.to_string_lossy().into_owned();

    let configuration = common::build_configuration(&server, vec!["--runs-dir", &runs_str]);
    let result = common::build_load_test(
        configuration,
        vec![scenario(transaction!(get_index))],
        None,
        None,
    )
    .execute()
    .await;

    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(&parent).ok();

    match result {
        Err(GooseError::InvalidOption { option, .. }) => assert_eq!(option, "--runs-dir"),
        Err(other) => panic!("expected an --runs-dir error, got {}", other),
        Ok(_) => panic!("expected an --runs-dir error, the run completed"),
    }
    assert_eq!(mock.calls(), 0, "no load before the error");
}

#[tokio::test]
#[serial]
async fn baseline_from_run_directory_shows_deltas() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("baseline");
    let runs_str = runs.to_string_lossy().into_owned();

    let configuration = common::build_configuration(&server, vec!["--runs-dir", &runs_str]);
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;
    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 1);
    let baseline = dirs[0].to_string_lossy().into_owned();

    let report = runs.join("compare.md");
    let report_str = report.to_string_lossy().into_owned();
    let configuration = common::build_configuration(
        &server,
        vec![
            "--runs-dir",
            &runs_str,
            "--baseline-file",
            &baseline,
            "--report-file",
            &report_str,
            "--users",
            "2",
        ],
    );
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;

    let markdown = std::fs::read_to_string(&report).unwrap();
    let delta = regex::Regex::new(r"\([+-]\d").unwrap();
    assert!(
        delta.is_match(&markdown),
        "the report shows deltas against the run directory:\n{}",
        markdown
    );
    // The second run records its baseline as given.
    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 2);
    let second = read_run_json(&dirs[1]);
    assert_eq!(second["baseline"], baseline.as_str());
    let saved = std::fs::read_to_string(dirs[1].join("report.md")).unwrap();
    assert!(delta.is_match(&saved), "the saved run has deltas too");
    std::fs::remove_dir_all(&runs).ok();
}

#[tokio::test]
#[serial]
async fn killswitch_reason_reaches_run_json() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("killswitch");
    let runs_str = runs.to_string_lossy().into_owned();

    let configuration =
        common::build_configuration(&server, vec!["--runs-dir", &runs_str, "--run-time", "30"]);
    let started = Instant::now();
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index_then_cancel))],
            None,
            None,
        ),
        None,
    )
    .await;
    // Clear the killswitch so later tests in this binary are not canceled.
    goose::reset_killswitch();
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "canceled early"
    );

    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 1);
    let run = read_run_json(&dirs[0]);
    assert_eq!(run["ended_by"], "canceled");
    assert_eq!(run["canceled_reason"], "error budget spent");
    std::fs::remove_dir_all(&runs).ok();
}

/// Send one telnet Controller command and return the reply.
async fn telnet(stream: &mut TcpStream, command: &str) -> String {
    stream
        .write_all(format!("{command}\r\n").as_bytes())
        .await
        .unwrap();
    let mut reply = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0, "controller closed the connection");
        reply.extend_from_slice(&buf[..n]);
        if reply.ends_with(b"goose> ") {
            return String::from_utf8_lossy(&reply).into_owned();
        }
    }
}

/// Send `command` until the reply contains `expect`.
async fn telnet_until(stream: &mut TcpStream, command: &str, expect: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let reply = telnet(stream, command).await;
        if reply.contains(expect) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}: last reply {}",
            command,
            reply
        );
        sleep(Duration::from_millis(100)).await;
    }
}

/// Wait until `count` run directories have a run.json.
async fn wait_for_complete_runs(runs: &Path, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let complete = run_dirs(runs)
            .iter()
            .filter(|dir| dir.join("run.json").exists())
            .count();
        if complete >= count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {} runs saved",
            complete,
            count
        );
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn no_autostart_saves_each_start_and_not_the_idle_shutdown() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("noautostart");
    let runs_str = runs.to_string_lossy().into_owned();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port().to_string()
    };

    let configuration = common::build_configuration(
        &server,
        vec![
            "--runs-dir",
            &runs_str,
            "--no-autostart",
            "--no-websocket",
            "--telnet-host",
            "127.0.0.1",
            "--telnet-port",
            &port,
            "--run-time",
            "0",
        ],
    );
    let attack = common::build_load_test(
        configuration,
        vec![scenario(transaction!(get_index))],
        None,
        None,
    );
    let goose = tokio::spawn(attack.execute());

    let address = format!("127.0.0.1:{port}");
    let mut stream = {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match TcpStream::connect(&address).await {
                Ok(stream) => break stream,
                Err(e) => {
                    assert!(Instant::now() < deadline, "telnet controller: {}", e);
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    };
    // The initial prompt.
    let mut buf = [0u8; 64];
    let _ = stream.read(&mut buf).await.unwrap();

    for run in 1..=2 {
        telnet_until(&mut stream, "start", "load test started").await;
        sleep(Duration::from_millis(500)).await;
        telnet_until(&mut stream, "stop", "load test stopped").await;
        wait_for_complete_runs(&runs, run).await;
    }
    assert_eq!(run_dirs(&runs).len(), 2);

    // Shut down from Idle: no run happened since, so nothing is saved.
    telnet_until(&mut stream, "shutdown", "load test shut down").await;
    let result = tokio::time::timeout(Duration::from_secs(20), goose)
        .await
        .expect("goose exits after shutdown")
        .unwrap();
    assert!(result.is_ok(), "{:?}", result.err());

    let dirs = run_dirs(&runs);
    assert_eq!(dirs.len(), 2, "the Idle shutdown wrote nothing: {dirs:?}");
    for dir in &dirs {
        assert_eq!(read_run_json(dir)["ended_by"], "stopped");
    }
    std::fs::remove_dir_all(&runs).ok();
}

/// Regression: Ctrl-C (the killswitch) while Idle after a run used to cancel
/// into a plan maintaining 0 users with no end, so Goose never exited.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn killswitch_while_idle_after_a_run_exits() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let runs = temp_dir("idlekill");
    let runs_str = runs.to_string_lossy().into_owned();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port().to_string()
    };
    let configuration = common::build_configuration(
        &server,
        vec![
            "--runs-dir",
            &runs_str,
            "--no-autostart",
            "--no-websocket",
            "--telnet-host",
            "127.0.0.1",
            "--telnet-port",
            &port,
            "--run-time",
            "0",
        ],
    );
    let attack = common::build_load_test(
        configuration,
        vec![scenario(transaction!(get_index))],
        None,
        None,
    );
    let goose = tokio::spawn(attack.execute());
    let address = format!("127.0.0.1:{port}");
    let mut stream = {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match TcpStream::connect(&address).await {
                Ok(stream) => break stream,
                Err(e) => {
                    assert!(Instant::now() < deadline, "telnet controller: {}", e);
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    };
    let mut buf = [0u8; 64];
    let _ = stream.read(&mut buf).await.unwrap();
    telnet_until(&mut stream, "start", "load test started").await;
    sleep(Duration::from_millis(500)).await;
    telnet_until(&mut stream, "stop", "load test stopped").await;
    wait_for_complete_runs(&runs, 1).await;
    // Idle now. Cancel as Ctrl-C does.
    sleep(Duration::from_millis(500)).await;
    goose::trigger_killswitch("SIGINT received");
    let result = tokio::time::timeout(Duration::from_secs(15), goose).await;
    goose::reset_killswitch();
    let result = result.expect("Goose exits").unwrap();
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(run_dirs(&runs).len(), 1, "nothing more saved");
    std::fs::remove_dir_all(&runs).ok();
}
