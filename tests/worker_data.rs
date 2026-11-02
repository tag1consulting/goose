//! The worker data hook in a run of one process, worker 0 of 1, in every
//! build, the Gaggle feature or not: its bytes reach the users, a hook that
//! returns too much fails the run once test_stop has run, and a Controller's
//! restart calls it again.

use httpmock::{Method::GET, MockServer};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use goose::config::GooseConfiguration;
use goose::prelude::*;

/// GET /data/ and the user's worker data.
async fn get_data(user: &mut GooseUser) -> TransactionResult {
    let data = String::from_utf8_lossy(user.worker_data()).into_owned();
    let _goose = user.get(&format!("/data/{data}")).await?;
    Ok(())
}

async fn get_start(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/start").await?;
    Ok(())
}

async fn get_stop(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/stop").await?;
    Ok(())
}

/// One user against `host`, quiet, with `more`.
fn configuration(host: &str, more: &[&str]) -> GooseConfiguration {
    let mut args = vec!["--host", host, "--users", "1", "--quiet"];
    args.extend_from_slice(more);
    GooseConfiguration::parse_args_default(&args).unwrap()
}

/// The load test: GET /data/ with the worker data, test_start GET /start
/// and test_stop GET /stop.
fn load_test(configuration: GooseConfiguration) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("Data").register_transaction(transaction!(get_data)))
        .test_start(transaction!(get_start))
        .test_stop(transaction!(get_stop))
}

/// A mock on `server` answering a GET of `path` with 200.
async fn ok<'a>(server: &'a MockServer, path: &str) -> httpmock::Mock<'a> {
    server
        .mock_async(|when, then| {
            when.method(GET).path(path);
            then.status(200).body("ok");
        })
        .await
}

/// The hook is given worker 0 of 1, every user and offset 0, and its
/// bytes reach the user.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn worker_data_reaches_the_users() {
    let server = MockServer::start_async().await;
    let data = ok(&server, "/data/0-of-1-1-from-0").await;
    let host = server.base_url();
    let run = load_test(configuration(&host, &["--run-time", "2", "--no-telnet"]))
        .set_worker_data(|share| {
            format!(
                "{}-of-{}-{}-from-{}",
                share.index, share.workers, share.users, share.user_offset
            )
            .into_bytes()
        })
        .execute();
    tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("within 30 seconds")
        .unwrap();
    assert!(data.calls_async().await > 0, "no user sent its worker data");
}

/// A hook that returns more than MAX_WORKER_DATA fails the run with its
/// documented error, after test_start ran, and test_stop still runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn worker_data_past_the_cap_still_runs_test_stop() {
    let server = MockServer::start_async().await;
    let (start, stop) = (ok(&server, "/start").await, ok(&server, "/stop").await);
    let host = server.base_url();
    let run = load_test(configuration(&host, &["--run-time", "1", "--no-telnet"]))
        .set_worker_data(|_| vec![b'x'; goose::MAX_WORKER_DATA + 1])
        .execute();
    let result = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("within 30 seconds");
    assert!(
        matches!(&result, Err(GooseError::InvalidOption { detail, .. }) if detail.contains("more than the")),
        "{:?}",
        result.map(|_| ())
    );
    assert_eq!(start.calls_async().await, 1, "test_start");
    assert_eq!(stop.calls_async().await, 1, "test_stop");
}

/// What the telnet Controller sends up to its next prompt.
async fn reply(client: &mut tokio::net::TcpStream) -> String {
    let mut text = Vec::new();
    let mut buffer = [0u8; 1024];
    while !text.ends_with(b"goose> ") {
        let read = tokio::time::timeout(Duration::from_secs(10), client.read(&mut buffer))
            .await
            .expect("the Controller answers")
            .unwrap();
        assert!(read > 0, "the Controller closed: {:?}", text);
        text.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8_lossy(&text).into_owned()
}

/// Send `command` to the telnet Controller: its reply.
async fn send(client: &mut tokio::net::TcpStream, command: &str) -> String {
    client
        .write_all(format!("{command}\r\n").as_bytes())
        .await
        .unwrap();
    reply(client).await
}

/// Each start from a Controller calls the hook again: once for each run,
/// as each run's users get the data it returns then.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn worker_data_hook_runs_again_on_a_controller_restart() {
    let test = async {
        let server = MockServer::start_async().await;
        let data = ok(&server, "/data/x").await;
        let host = server.base_url();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let run = tokio::spawn(
            load_test(configuration(&host, &["--no-autostart", "--no-websocket"]))
                .set_worker_data(move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    b"x".to_vec()
                })
                .execute(),
        );
        let connected = Instant::now();
        let mut client = loop {
            match tokio::net::TcpStream::connect("127.0.0.1:5116").await {
                Ok(client) => break client,
                Err(e) => {
                    assert!(connected.elapsed() < Duration::from_secs(10), "{}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };
        reply(&mut client).await;
        for run in 1..=2 {
            // A stopped run takes a moment to be idle again.
            let asked = Instant::now();
            loop {
                let started = send(&mut client, "start").await;
                if !started.contains("not idle") {
                    break;
                }
                assert!(asked.elapsed() < Duration::from_secs(20), "{}", started);
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            let running = Instant::now();
            while data.calls_async().await < run {
                assert!(running.elapsed() < Duration::from_secs(20), "run {}", run);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            send(&mut client, "stop").await;
        }
        let _ = client.write_all(b"shutdown\r\n").await;
        let _ = tokio::time::timeout(Duration::from_secs(30), run).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "the hook's calls");
    };
    tokio::time::timeout(Duration::from_secs(90), test)
        .await
        .expect("within 90 seconds");
}
