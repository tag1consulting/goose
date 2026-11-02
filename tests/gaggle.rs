//! A Gaggle as separate processes: the `simple` example, built with the
//! `gaggle` feature, run as a manager and as workers against a mock server,
//! with processes killed, frozen or interrupted mid run, and one run from
//! end to end. Everything that needs no process of its own is tested in
//! the library, in `src/gaggle/run_tests.rs`.
//! Unix only: the processes are signalled with `nix`.
#![cfg(all(unix, feature = "gaggle"))]

use httpmock::{Method::GET, Method::POST, MockServer};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long a heartbeat may go unanswered in these tests: twice the 3
/// second minimum, so a slow machine does not lose a peer between two
/// heartbeats, which go every 2 seconds.
const HEARTBEAT: u64 = 6;

/// `--gaggle-heartbeat-timeout` for [`HEARTBEAT`].
static HEARTBEAT_TIMEOUT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!("{HEARTBEAT}s"));

/// An example binary, built by `cargo test` beside this test.
fn example(name: &str) -> PathBuf {
    let deps = std::env::current_exe().unwrap();
    let path = deps
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("examples")
        .join(name);
    assert!(
        path.is_file(),
        "{} is not built: run the tests with `cargo test --features gaggle`, which builds the examples",
        path.display()
    );
    path
}

/// Every process a test started and every scratch folder it made, for
/// [`within`] to clean up however the test ends. The tests are serial.
static STARTED: std::sync::Mutex<(Vec<u32>, Vec<PathBuf>)> =
    std::sync::Mutex::new((Vec::new(), Vec::new()));

/// Kills every process the test started and removes its scratch folders
/// when dropped: as [`within`] returns, or unwinds from a failure or a
/// test that took too long, whose thread still holds them.
struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        let (pids, folders) = match STARTED.lock() {
            Ok(mut started) => std::mem::take(&mut *started),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        for pid in pids {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        for folder in folders {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

/// Run `test` on a thread of its own, failing if it takes more than two
/// minutes, and then kill every process it started and remove its scratch
/// folders, whether it passed or not.
fn within<F: FnOnce() + Send + 'static>(test: F) {
    let _cleanup = Cleanup;
    let (done, finished) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        test();
        let _ = done.send(());
    });
    match finished.recv_timeout(Duration::from_secs(120)) {
        Ok(()) => handle.join().expect("the test panicked"),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(panic) = handle.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("the test took more than 120 seconds")
        }
    }
}

/// A fresh folder for one test, removed when [`within`] ends.
fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "goose-gaggle-process-{name}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    if let Ok(mut started) = STARTED.lock() {
        started.1.push(folder.clone());
    }
    folder
}

/// A process, its output in `folder/<name>.out`.
struct Process {
    name: String,
    child: Child,
    output: PathBuf,
}

impl Process {
    fn start(binary: &Path, name: &str, folder: &Path, args: &[&str]) -> Process {
        let output = folder.join(format!("{name}.out"));
        let file = std::fs::File::create(&output).unwrap();
        let child = Command::new(binary)
            .args(args)
            .env("HOSTNAME", name)
            .env_remove("GOOSE_GAGGLE_TOKEN")
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap();
        if let Ok(mut started) = STARTED.lock() {
            started.0.push(child.id());
        }
        Process {
            name: name.to_string(),
            child,
            output,
        }
    }

    /// Wait for the process to exit, at most `within`.
    fn wait(&mut self, within: Duration) -> Option<(ExitStatus, Instant)> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                forget(self.child.id());
                return Some((status, Instant::now()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn output(&self) -> String {
        std::fs::read_to_string(&self.output).unwrap_or_default()
    }

    /// The port a manager started on port 0 listens on, from the line it
    /// logs once it does, waiting at most 30 seconds.
    fn port(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = self.output();
            let address = output
                .split("manager listening on ")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next());
            if let Some(port) = address.and_then(|address| address.rsplit(':').next()) {
                return port.to_string();
            }
            assert!(
                Instant::now() < deadline,
                "{} never listened: {}",
                self.name,
                output
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Kill the process at once, as SIGKILL does.
    fn kill(&mut self) -> Instant {
        self.child.kill().unwrap();
        let _ = self.child.wait();
        forget(self.child.id());
        Instant::now()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        forget(self.child.id());
    }
}

/// A process reaped: [`Cleanup`] must never signal its pid, which another
/// process may now hold.
fn forget(pid: u32) {
    if let Ok(mut started) = STARTED.lock() {
        started.0.retain(|&started| started != pid);
    }
}

/// The simple example's paths, answered by `server`; the mock counting
/// every GET.
fn mock_simple(server: &MockServer, delay: Duration) -> httpmock::Mock<'_> {
    server.mock(|when, then| {
        when.method(POST).path("/login");
        then.status(200).body("ok");
    });
    server.mock(|when, then| {
        when.method(GET).path_prefix("/");
        then.status(200).body("ok").delay(delay);
    })
}

/// The manager's arguments for the host and `extra`: it listens on a port
/// the system picks, which [`Process::port`] reads back.
fn manager_args<'a>(host: &'a str, workers: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--manager",
        "--expect-workers",
        workers,
        "--manager-bind-port",
        "0",
        "--host",
        host,
        "--no-telnet",
        "--gaggle-heartbeat-timeout",
        HEARTBEAT_TIMEOUT.as_str(),
        "--gaggle-connect-timeout",
        "20s",
    ];
    args.extend_from_slice(extra);
    args
}

/// A worker's arguments for `port` and `extra`.
fn worker_args<'a>(port: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--worker",
        "--manager-host",
        "localhost",
        "--manager-port",
        port,
        "--gaggle-heartbeat-timeout",
        HEARTBEAT_TIMEOUT.as_str(),
        "--gaggle-connect-timeout",
        "20s",
    ];
    args.extend_from_slice(extra);
    args
}

/// Wait until `count()` passes 0, at most `within`.
fn until_load(count: impl Fn() -> usize, within: Duration) {
    let deadline = Instant::now() + within;
    while count() == 0 {
        assert!(Instant::now() < deadline, "no load reached the server");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A worker killed mid run is marked lost within the heartbeat timeout,
/// with slack; the run finishes on the other, and its reports say it is
/// degraded. No report, and no log line, holds the token.
#[test]
#[serial_test::serial]
fn gaggle_worker_killed_mid_run() {
    within(|| worker_lost_mid_run(false));
}

/// As `gaggle_worker_killed_mid_run`, the worker frozen with SIGSTOP
/// instead: its connection stays open, so only the heartbeat timeout finds
/// it lost.
#[test]
#[serial_test::serial]
fn gaggle_worker_frozen_mid_run() {
    within(|| worker_lost_mid_run(true));
}

/// Stop process `pid` where it stands, its sockets left open.
fn freeze(pid: u32) {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGSTOP,
    )
    .unwrap();
}

/// A worker killed, or frozen, mid run.
fn worker_lost_mid_run(frozen: bool) {
    let server = MockServer::start();
    let index = mock_simple(&server, Duration::ZERO);
    let folder = scratch("worker-killed");
    let host = server.base_url();
    let manager_folder = folder.join("manager").display().to_string();
    let simple = example("simple");
    let mut manager = Process::start(
        &simple,
        "manager",
        &folder,
        &manager_args(
            &host,
            "2",
            &[
                "--users",
                "40",
                "--increase-rate",
                "40",
                "--run-time",
                "8s",
                "--report-file",
                "--output-dir",
                &manager_folder,
                "--gaggle-token",
                "tok-worker-lost-3c8e",
            ],
        ),
    );
    let token = ["--gaggle-token", "tok-worker-lost-3c8e"];
    let port = manager.port();
    let mut first = Process::start(&simple, "first", &folder, &worker_args(&port, &token));
    let mut second = Process::start(&simple, "second", &folder, &worker_args(&port, &token));
    until_load(|| index.calls(), Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(1));
    let killed = chrono::Utc::now();
    if frozen {
        freeze(second.child.id());
    } else {
        second.kill();
    }

    let (status, _) = manager
        .wait(Duration::from_secs(60))
        .expect("the manager finishes");
    assert!(status.success(), "{}", manager.output());
    let (status, _) = first
        .wait(Duration::from_secs(10))
        .expect("the other worker exits");
    assert!(status.success(), "{}", first.output());

    let report = std::fs::read_to_string(folder.join("manager/report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    let gaggle = &report["gaggle"];
    assert_eq!(gaggle["degraded"], true, "{}", gaggle);
    let workers = gaggle["workers"].as_array().unwrap();
    let lost: Vec<&serde_json::Value> = workers
        .iter()
        .filter(|worker| worker["state"] == "lost")
        .collect();
    assert_eq!(lost.len(), 1, "one lost worker: {}", gaggle);
    let lost = lost[0];
    assert_eq!(lost["name"], second.name.as_str());
    assert!(
        workers
            .iter()
            .any(|worker| worker["name"] == "first" && worker["state"] == "finished"),
        "{}",
        gaggle
    );
    // A frozen worker's connection stays open: only its silence finds it.
    if frozen {
        let reason = lost["lost_reason"].as_str().unwrap_or_default();
        assert!(reason.starts_with("nothing from it"), "{}", gaggle);
    }
    let lost_at = chrono::DateTime::parse_from_rfc3339(lost["lost_at"].as_str().unwrap()).unwrap();
    // `lost_at` is to the second.
    let after = lost_at.timestamp() - killed.timestamp();
    assert!(
        // The heartbeat timeout, and 5 seconds of slack for a slow
        // machine; lost_at and killed are each truncated to the second.
        after <= HEARTBEAT as i64 + 5,
        "lost {} s after the kill: {}",
        after,
        gaggle
    );
    let html = std::fs::read_to_string(folder.join("manager/report.html")).unwrap();
    assert!(html.contains("Degraded"), "the HTML report says degraded");
    // The token is in no report, nor in any log line.
    for text in [
        report.to_string(),
        html,
        manager.output(),
        first.output(),
        second.output(),
    ] {
        assert!(!text.contains("tok-worker-lost-3c8e"));
    }
}

/// The manager killed mid run: every worker stops its load and exits
/// with an error within the heartbeat timeout plus 5 seconds.
#[test]
#[serial_test::serial]
fn gaggle_manager_killed_mid_run() {
    within(|| manager_lost_mid_run(false));
}

/// As `gaggle_manager_killed_mid_run`, the manager frozen with SIGSTOP
/// instead: its connections stay open, so only the heartbeat timeout stops
/// the workers.
#[test]
#[serial_test::serial]
fn gaggle_manager_frozen_mid_run() {
    within(|| manager_lost_mid_run(true));
}

/// The manager killed, or frozen, mid run.
fn manager_lost_mid_run(frozen: bool) {
    let server = MockServer::start();
    let index = mock_simple(&server, Duration::ZERO);
    let folder = scratch("manager-killed");
    let host = server.base_url();
    let simple = example("simple");
    let mut manager = Process::start(
        &simple,
        "manager",
        &folder,
        &manager_args(
            &host,
            "2",
            &[
                "--users",
                "100",
                "--increase-rate",
                "100",
                "--run-time",
                "60s",
            ],
        ),
    );
    let port = manager.port();
    let mut workers = [
        Process::start(&simple, "first", &folder, &worker_args(&port, &[])),
        Process::start(&simple, "second", &folder, &worker_args(&port, &[])),
    ];
    until_load(|| index.calls(), Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(1));
    let killed = if frozen {
        freeze(manager.child.id());
        Instant::now()
    } else {
        manager.kill()
    };
    let bound = Duration::from_secs(HEARTBEAT + 5);
    for worker in workers.iter_mut() {
        let (status, exited) = worker
            .wait(bound + Duration::from_secs(5))
            .unwrap_or_else(|| panic!("{} did not exit: {}", worker.name, worker.output()));
        assert!(!status.success(), "{} exits with an error", worker.name);
        assert!(
            exited.duration_since(killed) <= bound,
            "{} exited {:?} after the manager was killed",
            worker.name,
            exited.duration_since(killed)
        );
        let output = worker.output();
        assert!(!output.contains("panicked"), "{}", output);
        // The switch stopped it, not its last resort.
        assert!(output.contains("lost the manager"), "{}", output);
        assert!(!output.contains("exiting now"), "{}", output);
        if frozen {
            assert!(output.contains("nothing from it"), "{}", output);
        }
    }
    let _ = manager.kill();
}

/// A threshold that fails on the merged metrics stops every worker,
/// and the manager fails with the thresholds error. A process test: the
/// threshold trips the process wide killswitch, which in one process would
/// stop every worker itself, and other tests besides.
#[test]
#[serial_test::serial]
fn gaggle_threshold_stops_every_worker() {
    within(|| {
        let server = MockServer::start();
        mock_simple(&server, Duration::from_millis(30));
        let folder = scratch("threshold");
        let host = server.base_url();
        let simple = example("simple");
        let started = Instant::now();
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &host,
                "2",
                &[
                    "--users",
                    "20",
                    "--increase-rate",
                    "20",
                    "--run-time",
                    "60s",
                    "--threshold",
                    "p95<5ms",
                ],
            ),
        );
        let port = manager.port();
        let mut workers = [
            Process::start(&simple, "first", &folder, &worker_args(&port, &[])),
            Process::start(&simple, "second", &folder, &worker_args(&port, &[])),
        ];
        let (status, ended) = manager
            .wait(Duration::from_secs(60))
            .expect("the manager ends");
        assert!(!status.success());
        let output = manager.output();
        assert!(output.contains("ThresholdsFailed"), "{}", output);
        assert!(
            ended.duration_since(started) < Duration::from_secs(30),
            "the threshold ends a 60 s run early: {:?}",
            ended.duration_since(started)
        );
        for worker in workers.iter_mut() {
            let (status, _) = worker
                .wait(Duration::from_secs(10))
                .expect("the worker exits");
            assert!(status.success(), "{}: {}", worker.name, worker.output());
        }
    });
}

/// A manager and two workers at -vv, every crate tracing, run a few
/// seconds of load and all exit: printing metrics while other threads log
/// once deadlocked a worker. No log line of any of them holds the token.
#[test]
#[serial_test::serial]
fn gaggle_at_trace_level_completes() {
    within(|| {
        let server = MockServer::start();
        mock_simple(&server, Duration::ZERO);
        let folder = scratch("trace-level");
        let host = server.base_url();
        let simple = example("simple");
        let token = ["--gaggle-token", "tok-trace-7f3a", "-vv"];
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &host,
                "2",
                &[
                    "--users",
                    "10",
                    "--increase-rate",
                    "10",
                    "--run-time",
                    "4s",
                    "--gaggle-token",
                    "tok-trace-7f3a",
                    "-vv",
                ],
            ),
        );
        let port = manager.port();
        let mut workers = [
            Process::start(&simple, "first", &folder, &worker_args(&port, &token)),
            Process::start(&simple, "second", &folder, &worker_args(&port, &token)),
        ];
        let (status, _) = manager
            .wait(Duration::from_secs(60))
            .expect("the manager finishes");
        assert!(status.success(), "{}", manager.output());
        for worker in workers.iter_mut() {
            let (status, _) = worker
                .wait(Duration::from_secs(20))
                .unwrap_or_else(|| panic!("{} hung at -vv", worker.name));
            assert!(status.success(), "{}", worker.name);
        }
        for output in [manager.output(), workers[0].output(), workers[1].output()] {
            assert!(
                !output.contains("tok-trace-7f3a"),
                "a log line holds the token"
            );
        }
    });
}

/// A worker with the wrong token is refused, and no log line of either
/// process, at -vv, holds either token. A process test, to read the logs;
/// the refusals themselves are tested in the library.
#[test]
#[serial_test::serial]
fn gaggle_refused_token_is_never_logged() {
    within(|| {
        let server = MockServer::start();
        mock_simple(&server, Duration::ZERO);
        let folder = scratch("token-logs");
        let host = server.base_url();
        let mut manager = Process::start(
            &example("simple"),
            "manager",
            &folder,
            &manager_args(
                &host,
                "1",
                &[
                    "--users",
                    "2",
                    "--run-time",
                    "2s",
                    "--gaggle-token",
                    "tok-right-5e1b",
                    "-vv",
                ],
            ),
        );
        let port = manager.port();
        let mut worker = Process::start(
            &example("simple"),
            "worker",
            &folder,
            &worker_args(&port, &["--gaggle-token", "tok-wrong-9c2d", "-vv"]),
        );
        let (status, _) = worker
            .wait(Duration::from_secs(20))
            .expect("the refused worker exits");
        assert!(!status.success());
        let output = worker.output();
        assert!(output.contains("refused this worker: token:"), "{}", output);
        assert!(!output.contains("panicked"), "{}", output);
        let _ = manager.kill();
        for output in [manager.output(), output] {
            assert!(
                !output.contains("tok-right") && !output.contains("tok-wrong"),
                "{}",
                output
            );
        }
    });
}

/// Send process `pid` SIGINT, as Ctrl-C does.
fn interrupt(pid: u32) {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
}

/// The run record in `folder`'s manager folder.
fn manager_record(folder: &Path) -> serde_json::Value {
    let report = std::fs::read_to_string(folder.join("manager/report.json")).unwrap();
    serde_json::from_str(&report).unwrap()
}

/// Ctrl-C on the manager mid run: it stops every worker, each of which
/// sends its final metrics and exits 0, and it writes its reports, the run
/// recorded as canceled and not degraded.
#[test]
#[serial_test::serial]
fn gaggle_manager_interrupted() {
    within(|| {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("manager-interrupted");
        let host = server.base_url();
        let manager_folder = folder.join("manager").display().to_string();
        let simple = example("simple");
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &host,
                "2",
                &[
                    "--users",
                    "20",
                    "--increase-rate",
                    "20",
                    "--run-time",
                    "60s",
                    "--report-file",
                    "--output-dir",
                    &manager_folder,
                ],
            ),
        );
        let port = manager.port();
        let mut workers = [
            Process::start(&simple, "first", &folder, &worker_args(&port, &[])),
            Process::start(&simple, "second", &folder, &worker_args(&port, &[])),
        ];
        until_load(|| index.calls(), Duration::from_secs(30));
        interrupt(manager.child.id());
        manager
            .wait(Duration::from_secs(60))
            .unwrap_or_else(|| panic!("the manager ends: {}", manager.output()));
        for worker in workers.iter_mut() {
            let (status, _) = worker
                .wait(Duration::from_secs(30))
                .unwrap_or_else(|| panic!("{} exits: {}", worker.name, worker.output()));
            assert!(status.success(), "{}: {}", worker.name, worker.output());
        }
        let report = manager_record(&folder);
        assert_eq!(report["ended_by"], "canceled", "{}", manager.output());
        let gaggle = &report["gaggle"];
        assert_eq!(gaggle["degraded"], false, "{}", gaggle);
        let workers = gaggle["workers"].as_array().unwrap();
        assert_eq!(workers.len(), 2, "{}", gaggle);
        for worker in workers {
            assert_eq!(worker["state"], "finished", "{}", gaggle);
        }
        assert!(folder.join("manager/report.html").is_file());
    });
}

/// Ctrl-C on a worker mid run: its final metrics reach the manager, which
/// marks it lost, and the run finishes on the other, degraded.
#[test]
#[serial_test::serial]
fn gaggle_worker_interrupted() {
    within(|| {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("worker-interrupted");
        let host = server.base_url();
        let manager_folder = folder.join("manager").display().to_string();
        let simple = example("simple");
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &host,
                "2",
                &[
                    "--users",
                    "20",
                    "--increase-rate",
                    "20",
                    "--run-time",
                    "8s",
                    "--report-file",
                    "--output-dir",
                    &manager_folder,
                ],
            ),
        );
        let port = manager.port();
        let mut first = Process::start(&simple, "first", &folder, &worker_args(&port, &[]));
        let mut second = Process::start(&simple, "second", &folder, &worker_args(&port, &[]));
        until_load(|| index.calls(), Duration::from_secs(30));
        interrupt(second.child.id());
        second
            .wait(Duration::from_secs(30))
            .unwrap_or_else(|| panic!("the interrupted worker exits: {}", second.output()));
        let (status, _) = manager
            .wait(Duration::from_secs(60))
            .expect("the manager finishes");
        assert!(status.success(), "{}", manager.output());
        let (status, _) = first
            .wait(Duration::from_secs(10))
            .expect("the other worker exits");
        assert!(status.success(), "{}", first.output());
        let gaggle = &manager_record(&folder)["gaggle"];
        assert_eq!(gaggle["degraded"], true, "{}", gaggle);
        let workers = gaggle["workers"].as_array().unwrap();
        let lost: Vec<&serde_json::Value> = workers
            .iter()
            .filter(|worker| worker["state"] == "lost")
            .collect();
        assert_eq!(lost.len(), 1, "{}", gaggle);
        assert_eq!(lost[0]["name"], "second", "{}", gaggle);
        // Stopped on the worker, which sends its final metrics first on the
        // same stream: not a stream closed before them.
        let reason = lost[0]["lost_reason"].as_str().unwrap_or_default();
        assert!(
            reason.starts_with("it was stopped on the worker"),
            "{}",
            gaggle
        );
        assert!(lost[0]["last_seq"].as_u64().unwrap_or(0) > 0, "{}", gaggle);
        assert!(lost[0].get("missing_deltas").is_none(), "{}", gaggle);
        assert!(
            workers
                .iter()
                .any(|worker| worker["name"] == "first" && worker["state"] == "finished"),
            "{}",
            gaggle
        );
    });
}

/// One run from the command line to its end: a manager and two workers of
/// the `simple` example, each exiting 0, the manager's reports written, and
/// both workers merged. A worker left out for a metrics reset it missed is
/// tested in the library (`gaggle_stale_and_refused_workers_degrade_the_run`),
/// and a degraded run through this binary by the killed, frozen and
/// interrupted workers here.
#[test]
#[serial_test::serial]
fn gaggle_run_from_the_command_line() {
    within(|| {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("command-line");
        let host = server.base_url();
        let manager_folder = folder.join("manager").display().to_string();
        let simple = example("simple");
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &host,
                "2",
                &[
                    "--users",
                    "4",
                    "--increase-rate",
                    "4",
                    "--run-time",
                    "4s",
                    "--report-file",
                    "--output-dir",
                    &manager_folder,
                ],
            ),
        );
        let port = manager.port();
        let mut workers = [
            Process::start(&simple, "first", &folder, &worker_args(&port, &[])),
            Process::start(&simple, "second", &folder, &worker_args(&port, &[])),
        ];
        let (status, _) = manager
            .wait(Duration::from_secs(60))
            .expect("the manager ends");
        assert!(status.success(), "{}", manager.output());
        for worker in workers.iter_mut() {
            let (status, _) = worker
                .wait(Duration::from_secs(10))
                .expect("the worker exits");
            assert!(status.success(), "{}: {}", worker.name, worker.output());
        }
        assert!(index.calls() > 0);
        assert!(folder.join("manager/report.html").is_file());
        let gaggle = &manager_record(&folder)["gaggle"];
        assert_eq!(gaggle["degraded"], false, "{}", gaggle);
        let workers = gaggle["workers"].as_array().unwrap();
        assert_eq!(workers.len(), 2, "{}", gaggle);
        for worker in workers {
            assert_eq!(worker["state"], "finished", "{}", gaggle);
            assert!(worker.get("excluded_reason").is_none(), "{}", gaggle);
        }
    });
}
