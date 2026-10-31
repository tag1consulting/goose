//! A Gaggle as separate processes: the `simple` example, built with the
//! `gaggle` feature, run as a manager and as workers against a mock server,
//! with processes killed mid run.
#![cfg(feature = "gaggle")]

use httpmock::{Method::GET, Method::POST, MockServer};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long a heartbeat may go unanswered in these tests.
const HEARTBEAT: u64 = 3;

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

/// Run `test` on a thread of its own, failing if it takes more than two
/// minutes.
fn within<F: FnOnce() + Send + 'static>(test: F) {
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

/// A free TCP port on 127.0.0.1.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A fresh folder for one test.
fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "goose-gaggle-process-{name}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(&folder).unwrap();
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
                return Some((status, Instant::now()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn output(&self) -> String {
        std::fs::read_to_string(&self.output).unwrap_or_default()
    }

    /// Kill the process at once, as SIGKILL does.
    fn kill(&mut self) -> Instant {
        self.child.kill().unwrap();
        let _ = self.child.wait();
        Instant::now()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// The manager's arguments for `port`, the host and `extra`.
fn manager_args<'a>(
    port: &'a str,
    host: &'a str,
    workers: &'a str,
    extra: &[&'a str],
) -> Vec<&'a str> {
    let mut args = vec![
        "--manager",
        "--expect-workers",
        workers,
        "--manager-bind-port",
        port,
        "--host",
        host,
        "--no-telnet",
        "--gaggle-heartbeat-timeout",
        "3s",
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
        "--no-telnet",
        "--gaggle-heartbeat-timeout",
        "3s",
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

/// B2.1: a worker killed mid run is marked lost within the heartbeat
/// timeout plus 2 seconds; the run finishes on the other, and its reports
/// say it is degraded.
#[test]
#[serial_test::serial]
fn gaggle_worker_killed_mid_run() {
    within(|| worker_lost_mid_run(false));
}

/// As B2.1, the worker frozen with SIGSTOP instead: its connection stays
/// open, so only the heartbeat timeout finds it lost.
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
    {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("worker-killed");
        let port = free_port().to_string();
        let host = server.base_url();
        let manager_folder = folder.join("manager").display().to_string();
        let simple = example("simple");
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &port,
                &host,
                "2",
                &[
                    "--users",
                    "40",
                    "--increase-rate",
                    "40",
                    "--run-time",
                    "12s",
                    "--report-file",
                    "--output-dir",
                    &manager_folder,
                    "--gaggle-token",
                    "tok-b21-secret",
                ],
            ),
        );
        let token = ["--gaggle-token", "tok-b21-secret"];
        let mut first = Process::start(&simple, "first", &folder, &worker_args(&port, &token));
        let mut second = Process::start(&simple, "second", &folder, &worker_args(&port, &token));
        until_load(|| index.calls(), Duration::from_secs(30));
        std::thread::sleep(Duration::from_secs(2));
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
        let lost_at =
            chrono::DateTime::parse_from_rfc3339(lost["lost_at"].as_str().unwrap()).unwrap();
        // `lost_at` is to the second.
        let after = lost_at.timestamp() - killed.timestamp();
        assert!(
            // lost_at and killed are each truncated to the second.
            after <= HEARTBEAT as i64 + 3,
            "lost {} s after the kill: {}",
            after,
            gaggle
        );
        let html = std::fs::read_to_string(folder.join("manager/report.html")).unwrap();
        assert!(html.contains("Degraded"), "the HTML report says degraded");
        // The token is in no report.
        for text in [&report.to_string(), &html] {
            assert!(!text.contains("tok-b21-secret"));
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}

/// B2.2: the manager killed mid run: every worker stops its load and exits
/// with an error within the heartbeat timeout plus 5 seconds.
#[test]
#[serial_test::serial]
fn gaggle_manager_killed_mid_run() {
    within(|| manager_lost_mid_run(false));
}

/// As B2.2, the manager frozen with SIGSTOP instead: its connections stay
/// open, so only the heartbeat timeout stops the workers.
#[test]
#[serial_test::serial]
fn gaggle_manager_frozen_mid_run() {
    within(|| manager_lost_mid_run(true));
}

/// The manager killed, or frozen, mid run.
fn manager_lost_mid_run(frozen: bool) {
    {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("manager-killed");
        let port = free_port().to_string();
        let host = server.base_url();
        let simple = example("simple");
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &port,
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
        let mut workers = [
            Process::start(&simple, "first", &folder, &worker_args(&port, &[])),
            Process::start(&simple, "second", &folder, &worker_args(&port, &[])),
        ];
        until_load(|| index.calls(), Duration::from_secs(30));
        std::thread::sleep(Duration::from_secs(3));
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
        // No more load once the workers are gone.
        let after = index.calls();
        std::thread::sleep(Duration::from_secs(3));
        assert_eq!(index.calls(), after, "load outlived the manager");
        let _ = manager.kill();
        let _ = std::fs::remove_dir_all(&folder);
    }
}

/// B2.3: a worker whose load test differs, or whose token is wrong, is
/// refused, and exits with an error naming why.
#[test]
#[serial_test::serial]
fn gaggle_handshake_refusals() {
    within(|| {
        let server = MockServer::start();
        mock_simple(&server, Duration::ZERO);
        let folder = scratch("refusals");
        let host = server.base_url();
        for (case, worker_binary, manager_extra, worker_extra, named) in [
            (
                "plan",
                "closure",
                vec![],
                vec![],
                "refused this worker: plan_hash:",
            ),
            (
                "token",
                "simple",
                vec!["--gaggle-token", "tok-right-5e1b", "-vv"],
                vec!["--gaggle-token", "tok-wrong-9c2d", "-vv"],
                "refused this worker: token:",
            ),
        ] {
            let port = free_port().to_string();
            let mut extra = vec!["--users", "2", "--run-time", "2s"];
            extra.extend(manager_extra);
            let mut manager = Process::start(
                &example("simple"),
                &format!("manager-{case}"),
                &folder,
                &manager_args(&port, &host, "1", &extra),
            );
            let mut worker = Process::start(
                &example(worker_binary),
                &format!("worker-{case}"),
                &folder,
                &worker_args(&port, &worker_extra),
            );
            let (status, _) = worker
                .wait(Duration::from_secs(20))
                .unwrap_or_else(|| panic!("{}: the refused worker exits", case));
            assert!(!status.success(), "{}", case);
            let output = worker.output();
            assert!(
                output.contains("refused this worker"),
                "{}: {}",
                case,
                output
            );
            assert!(output.contains(named), "{}: {}", case, output);
            assert!(!output.contains("panicked"), "{}: {}", case, output);
            let _ = manager.kill();
            // No log line, at any verbosity, holds a token.
            for output in [manager.output(), output] {
                assert!(
                    !output.contains("tok-right") && !output.contains("tok-wrong"),
                    "{}: {}",
                    case,
                    output
                );
            }
        }
        let _ = std::fs::remove_dir_all(&folder);
    });
}

/// B2.4: a worker with no manager to reach exits with an error, without a
/// panic, within the connect timeout plus 5 seconds.
#[test]
#[serial_test::serial]
fn gaggle_worker_without_a_manager() {
    within(|| {
        let folder = scratch("no-manager");
        let port = free_port().to_string();
        let started = Instant::now();
        let mut worker = Process::start(
            &example("simple"),
            "lonely",
            &folder,
            &[
                "--worker",
                "--manager-host",
                "127.0.0.1",
                "--manager-port",
                &port,
                "--no-telnet",
                "--gaggle-connect-timeout",
                "4s",
            ],
        );
        let (status, exited) = worker
            .wait(Duration::from_secs(20))
            .expect("the worker gives up");
        assert!(!status.success());
        assert!(
            exited.duration_since(started) <= Duration::from_secs(4 + 5),
            "{:?}",
            exited.duration_since(started)
        );
        let output = worker.output();
        assert!(output.contains("could not reach the manager"), "{}", output);
        assert!(!output.contains("panicked"), "{}", output);
        let _ = std::fs::remove_dir_all(&folder);
    });
}

/// B2.5: `--no-metrics` on the manager: the run starts, stops on time, and
/// every process exits 0 (#156).
#[test]
#[serial_test::serial]
fn gaggle_without_metrics_exits_cleanly() {
    within(|| {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::ZERO);
        let folder = scratch("no-metrics");
        let port = free_port().to_string();
        let host = server.base_url();
        let simple = example("simple");
        let started = Instant::now();
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &port,
                &host,
                "2",
                &[
                    "--users",
                    "4",
                    "--increase-rate",
                    "4",
                    "--run-time",
                    "4s",
                    "--no-metrics",
                ],
            ),
        );
        let mut workers = [
            Process::start(&simple, "first", &folder, &worker_args(&port, &[])),
            Process::start(&simple, "second", &folder, &worker_args(&port, &[])),
        ];
        let (status, ended) = manager
            .wait(Duration::from_secs(60))
            .expect("the manager ends");
        assert!(status.success(), "{}", manager.output());
        // Joining, a two second start, a 1 second ramp and 4 seconds of run.
        assert!(
            ended.duration_since(started) < Duration::from_secs(30),
            "{:?}",
            ended.duration_since(started)
        );
        for worker in workers.iter_mut() {
            let (status, _) = worker
                .wait(Duration::from_secs(10))
                .expect("the worker exits");
            assert!(status.success(), "{}: {}", worker.name, worker.output());
        }
        assert!(index.calls() > 0);
        let _ = std::fs::remove_dir_all(&folder);
    });
}

/// B2.6: a threshold that fails on the merged metrics stops every worker,
/// and the manager fails with the thresholds error.
#[test]
#[serial_test::serial]
fn gaggle_threshold_stops_every_worker() {
    within(|| {
        let server = MockServer::start();
        let index = mock_simple(&server, Duration::from_millis(30));
        let folder = scratch("threshold");
        let port = free_port().to_string();
        let host = server.base_url();
        let simple = example("simple");
        let started = Instant::now();
        let mut manager = Process::start(
            &simple,
            "manager",
            &folder,
            &manager_args(
                &port,
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
        let after = index.calls();
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(index.calls(), after, "every worker stopped");
        let _ = std::fs::remove_dir_all(&folder);
    });
}
