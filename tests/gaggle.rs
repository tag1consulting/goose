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

/// Send process `pid` `signal`: SIGSTOP stops it where it stands, its
/// sockets left open, and SIGINT is Ctrl-C.
fn signal(pid: u32, signal: nix::sys::signal::Signal) {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), signal).unwrap();
}

/// A Gaggle of the `simple` example under load: a manager given `users`
/// users at that rate for `run_time`, its reports in its folder, and
/// `manager_extra`; and two workers, `first` and `second`, given
/// `worker_extra`, once their requests reach the server.
struct UnderLoad {
    _server: MockServer,
    folder: PathBuf,
    manager: Process,
    first: Process,
    second: Process,
}

fn under_load(
    name: &str,
    users: &str,
    run_time: &str,
    manager_extra: &[&str],
    worker_extra: &[&str],
) -> UnderLoad {
    let server = MockServer::start();
    let index = mock_simple(&server, Duration::ZERO);
    let folder = scratch(name);
    let host = server.base_url();
    let manager_folder = folder.join("manager").display().to_string();
    let simple = example("simple");
    let mut extra = vec![
        "--users",
        users,
        "--increase-rate",
        users,
        "--run-time",
        run_time,
        "--report-file",
        "--output-dir",
        &manager_folder,
    ];
    extra.extend_from_slice(manager_extra);
    let manager = Process::start(
        &simple,
        "manager",
        &folder,
        &manager_args(&host, "2", &extra),
    );
    let port = manager.port();
    let first = Process::start(&simple, "first", &folder, &worker_args(&port, worker_extra));
    let second = Process::start(
        &simple,
        "second",
        &folder,
        &worker_args(&port, worker_extra),
    );
    until_load(|| index.calls(), Duration::from_secs(30));
    UnderLoad {
        _server: server,
        folder,
        manager,
        first,
        second,
    }
}

/// A worker killed, or frozen, mid run.
fn worker_lost_mid_run(frozen: bool) {
    let token = ["--gaggle-token", "tok-worker-lost-3c8e"];
    let UnderLoad {
        _server,
        folder,
        mut manager,
        mut first,
        mut second,
    } = under_load("worker-killed", "40", "8s", &token, &token);
    std::thread::sleep(Duration::from_secs(1));
    let killed = chrono::Utc::now();
    if frozen {
        signal(second.child.id(), nix::sys::signal::Signal::SIGSTOP);
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
        signal(manager.child.id(), nix::sys::signal::Signal::SIGSTOP);
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
        assert!(
            output.contains("Error: one or more thresholds failed (p95<5ms, observed "),
            "{}",
            output
        );
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
/// once deadlocked a worker. A third worker, with the wrong token, is
/// refused, without a panic. No log line of any of them holds either token:
/// a process test, to read the logs; the refusals themselves are tested in
/// the library.
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
        let mut refused = Process::start(
            &simple,
            "refused",
            &folder,
            &worker_args(&port, &["--gaggle-token", "tok-wrong-9c2d", "-vv"]),
        );
        let (status, _) = refused
            .wait(Duration::from_secs(20))
            .expect("the refused worker exits");
        assert!(!status.success());
        let output = refused.output();
        assert!(output.contains("refused this worker: token:"), "{}", output);
        assert!(!output.contains("panicked"), "{}", output);
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
        for output in [
            manager.output(),
            workers[0].output(),
            workers[1].output(),
            refused.output(),
        ] {
            assert!(
                !output.contains("tok-trace") && !output.contains("tok-wrong"),
                "a log line holds a token"
            );
        }
    });
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
        signal(manager.child.id(), nix::sys::signal::Signal::SIGINT);
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
        let UnderLoad {
            _server,
            folder,
            mut manager,
            mut first,
            mut second,
        } = under_load("worker-interrupted", "20", "8s", &[], &[]);
        signal(second.child.id(), nix::sys::signal::Signal::SIGINT);
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

/// A telnet line to the manager's Controller on `port`, and its reply.
fn telnet(port: u16, command: &str) -> String {
    use std::io::{Read, Write};
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "the Controller never listened: {}",
                    e
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let read_reply = |stream: &mut std::net::TcpStream| {
        let mut reply = Vec::new();
        let mut buf = [0; 4096];
        while !reply.ends_with(b"goose> ") {
            let read = stream.read(&mut buf).expect("a reply");
            assert!(read > 0, "the Controller closed");
            reply.extend_from_slice(&buf[..read]);
        }
        String::from_utf8(reply).unwrap()
    };
    read_reply(&mut stream);
    stream
        .write_all(format!("{command}\r\n").as_bytes())
        .unwrap();
    read_reply(&mut stream)
}

/// A worker that stops reading its stream, frozen with SIGSTOP, misses the
/// deadline for answering a change: it is marked lost, and the change's
/// reply names it, within the 15 seconds a Controller waits; the change
/// stands on the other worker. Once thawed, it finds itself shut down and
/// exits.
#[test]
#[serial_test::serial]
fn gaggle_worker_missing_the_answer_deadline_is_lost() {
    within(|| {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("missed-answer");
        let host = server.base_url();
        let simple = example("simple");
        // Heartbeats that never run out first: only the answer deadline
        // finds the frozen worker.
        let manager = Process::start(
            &simple,
            "manager",
            &folder,
            &[
                "--manager",
                "--expect-workers",
                "2",
                "--manager-bind-port",
                "0",
                "--host",
                &host,
                "--telnet-port",
                "5246",
                "--gaggle-heartbeat-timeout",
                "60s",
                "--gaggle-connect-timeout",
                "20s",
                "--users",
                "20",
                "--increase-rate",
                "20",
            ],
        );
        let port = manager.port();
        let slow = [
            "--gaggle-heartbeat-timeout",
            "60s",
            "--gaggle-connect-timeout",
            "20s",
        ];
        let worker = |name| {
            let mut args = vec![
                "--worker",
                "--manager-host",
                "localhost",
                "--manager-port",
                &port,
            ];
            args.extend_from_slice(&slow);
            Process::start(&simple, name, &folder, &args)
        };
        let mut first = worker("first");
        std::thread::sleep(Duration::from_millis(500));
        let mut second = worker("second");
        let mut manager = manager;
        until_load(|| index.calls(), Duration::from_secs(30));
        signal(second.child.id(), nix::sys::signal::Signal::SIGSTOP);
        let sent = Instant::now();
        // A change that starts no user, so the 10 second deadline applies:
        // one that starts users waits as long as a Prepare.
        let reply = telnet(5246, "users 6");
        let took = sent.elapsed();
        assert!(reply.starts_with("users configured"), "{}", reply);
        assert!(
            reply.contains("(second) lost: it did not answer a change within 10 seconds"),
            "{}",
            reply
        );
        assert!(took < Duration::from_secs(15), "answered after {:?}", took);
        signal(second.child.id(), nix::sys::signal::Signal::SIGCONT);
        second
            .wait(Duration::from_secs(20))
            .expect("the thawed worker exits");
        let reply = telnet(5246, "shutdown");
        assert!(reply.starts_with("load test shut down"), "{}", reply);
        let (status, _) = manager
            .wait(Duration::from_secs(60))
            .expect("the manager finishes");
        assert!(status.success(), "{}", manager.output());
        first
            .wait(Duration::from_secs(20))
            .expect("the other worker exits");
        // The change stood on the other worker: its 10 users down to 6.
        assert!(
            first
                .output()
                .lines()
                .any(|line| line.contains("changing users from") && line.ends_with(" to 6")),
            "{}",
            first.output()
        );
    });
}

/// The requests per second `count` grew by over `window`.
fn rate(count: impl Fn() -> usize, window: Duration) -> f64 {
    let before = count();
    let start = Instant::now();
    std::thread::sleep(window);
    (count() - before) as f64 / start.elapsed().as_secs_f64()
}

/// A worker killed under a throttle the users always reach: the worker
/// left takes the whole throttle, so the run still makes as many requests
/// a second; with `rebalance`, it also starts the lost worker's users.
fn throttle_after_loss(rebalance: bool) {
    let server = MockServer::start();
    // Every request counts: under the throttle, the users are still logging
    // in for the whole run.
    let index = server.mock(|when, then| {
        when.path_prefix("/");
        then.status(200).body("ok");
    });
    let folder = scratch(if rebalance {
        "rebalance"
    } else {
        "throttle-loss"
    });
    let host = server.base_url();
    let simple = example("simple");
    // 2000 users log in first, far more than the throttle lets through in
    // the run, so the users always reach it.
    let mut extra = vec![
        "--users",
        "2000",
        "--increase-rate",
        "2000",
        "--run-time",
        "30s",
        "--throttle-requests",
        "40",
    ];
    if rebalance {
        extra.push("--gaggle-rebalance");
    }
    let mut manager = Process::start(
        &simple,
        "manager",
        &folder,
        &manager_args(&host, "2", &extra),
    );
    let port = manager.port();
    let mut first = Process::start(&simple, "first", &folder, &worker_args(&port, &[]));
    let mut second = Process::start(&simple, "second", &folder, &worker_args(&port, &[]));
    until_load(|| index.calls(), Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(2));
    // Before the loss, the two workers together make the throttle's 40.
    let before = rate(|| index.calls(), Duration::from_secs(3));
    assert!(
        (before - 40.0).abs() <= 40.0 * 0.15,
        "{:.1} requests a second before the loss",
        before
    );
    second.kill();
    std::thread::sleep(Duration::from_secs(3));
    let after = rate(|| index.calls(), Duration::from_secs(5));
    assert!(
        (after - 40.0).abs() <= 40.0 * 0.15,
        "{after:.1} requests a second after the loss, not the throttle's 40: {}",
        manager.output()
    );
    let (status, _) = manager
        .wait(Duration::from_secs(60))
        .expect("the manager finishes");
    assert!(status.success(), "{}", manager.output());
    first
        .wait(Duration::from_secs(20))
        .expect("the other worker exits");
    // From wherever its own ramp stood: a slow machine may still ramp.
    let moved = first
        .output()
        .lines()
        .any(|line| line.contains("changing users from") && line.ends_with(" to 2000"));
    assert_eq!(moved, rebalance, "{}", first.output());
}

/// A worker lost under a throttle: the other's share rises to the whole.
#[test]
#[serial_test::serial]
fn gaggle_throttle_follows_a_lost_worker() {
    within(|| throttle_after_loss(false));
}

/// With `--gaggle-rebalance`, the worker left also runs the lost worker's
/// users.
#[test]
#[serial_test::serial]
fn gaggle_rebalance_moves_a_lost_workers_users() {
    within(|| throttle_after_loss(true));
}
