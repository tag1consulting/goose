//! A running throttle's rate changes from a Controller and from the
//! dashboard, and holds each new rate; a run started without one can not
//! turn one on.
//!
//! Requires the `dashboard` crate feature.

#![cfg(feature = "dashboard")]

use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use goose::config::GooseConfiguration;
use goose::prelude::*;

const INDEX_PATH: &str = "/";
const TOKEN: &str = "throttle-live-token";
const TELNET_PORT: u16 = 5136;
const DASHBOARD_PORT: u16 = 5138;
/// How long a test may take: it runs three 7 second windows.
const TEST_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a new rate settles before it is measured.
const SETTLE: Duration = Duration::from_secs(2);
/// How long each rate is measured.
const WINDOW: Duration = Duration::from_secs(5);

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

/// A new temporary folder for the run's folder, out of the checkout.
fn output_dir() -> String {
    std::env::temp_dir()
        .join(format!("goose-throttle-live-{}", std::process::id()))
        .display()
        .to_string()
}

fn configuration(server: &MockServer, throttle: Option<usize>) -> GooseConfiguration {
    let mut flags: Vec<String> = [
        "--users",
        "20",
        "--increase-rate",
        "100",
        "--run-time",
        "50s",
        "--no-websocket",
        "--telnet-port",
        &TELNET_PORT.to_string(),
        "--dashboard",
        "--dashboard-port",
        &DASHBOARD_PORT.to_string(),
        "--dashboard-control",
        "--dashboard-auth-token",
        TOKEN,
        "--co-mitigation",
        "disabled",
        "--quiet",
        "--output-dir",
        &output_dir(),
        "--host",
        &server.base_url(),
    ]
    .iter()
    .map(|flag| flag.to_string())
    .collect();
    if let Some(throttle) = throttle {
        flags.push("--throttle-requests".to_string());
        flags.push(throttle.to_string());
    }
    let refs: Vec<&str> = flags.iter().map(String::as_str).collect();
    GooseConfiguration::parse_args_default(&refs).expect("configuration")
}

fn attack(server: &MockServer, throttle: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration(server, throttle))
        .unwrap()
        .register_scenario(scenario!("Index").register_transaction(transaction!(get_index)))
}

/// A telnet Controller session.
struct Telnet(TcpStream);

impl Telnet {
    async fn connect() -> Self {
        for _ in 0..50 {
            if let Ok(stream) = TcpStream::connect(("127.0.0.1", TELNET_PORT)).await {
                let mut telnet = Telnet(stream);
                telnet.read().await;
                return telnet;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the telnet Controller did not listen");
    }

    async fn read(&mut self) -> String {
        let mut reply = String::new();
        let mut buf = [0; 4096];
        while !reply.ends_with("goose> ") {
            let read = self.0.read(&mut buf).await.expect("telnet read");
            assert!(read > 0, "the telnet Controller closed");
            reply.push_str(std::str::from_utf8(&buf[..read]).unwrap());
        }
        reply
    }

    async fn send(&mut self, command: &str) -> String {
        self.0
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .expect("telnet write");
        self.read().await
    }
}

/// Post `body` to the dashboard's settings endpoint: its status and JSON.
async fn settings(body: &str) -> (u16, serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{DASHBOARD_PORT}/api/v1/control/settings"
        ))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {TOKEN}"))
        .body(body.to_string())
        .send()
        .await
        .expect("settings POST");
    let status = response.status().as_u16();
    (status, response.json().await.expect("settings JSON"))
}

/// The requests per second `mock` served over [`WINDOW`], after [`SETTLE`].
async fn measured_rate(mock: &Mock<'_>) -> f64 {
    tokio::time::sleep(SETTLE).await;
    let before = mock.hits_async().await;
    let start = std::time::Instant::now();
    tokio::time::sleep(WINDOW).await;
    let served = mock.hits_async().await - before;
    served as f64 / start.elapsed().as_secs_f64()
}

fn assert_near(measured: f64, setting: f64) {
    let off = (measured - setting).abs() / setting;
    assert!(
        off <= 0.05,
        "measured {measured:.1} requests per second, {:.1}% off the throttle of {setting}",
        off * 100.0
    );
}

/// Start at 200 requests a second, then 50 by telnet, then 400 through the
/// dashboard's settings: each rate holds within 5% over 5 seconds, once 2
/// seconds settled it. The throttle can not be cleared while running.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
async fn a_running_throttle_follows_the_controller_and_dashboard() {
    let server = MockServer::start_async().await;
    let index = server
        .mock_async(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        })
        .await;
    let attack = attack(&server, Some(200));
    let run = tokio::spawn(attack.execute());
    let test = async {
        let mut telnet = Telnet::connect().await;
        assert_near(measured_rate(&index).await, 200.0);

        let reply = telnet.send("throttle 50").await;
        assert!(reply.starts_with("throttle configured"), "{}", reply);
        assert_near(measured_rate(&index).await, 50.0);

        let (status, body) = settings(r#"{"throttle": 400}"#).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["settings"]["throttle"], 400, "{body}");
        assert_near(measured_rate(&index).await, 400.0);

        // Neither front end turns the throttle off while running.
        let reply = telnet.send("throttle 0").await;
        assert!(reply.starts_with("failed to clear throttle"), "{}", reply);
        let (status, body) = settings(r#"{"throttle": null}"#).await;
        assert_eq!((status, &body["ok"]), (200, &false.into()), "{body}");
        assert_eq!(body["field"], "throttle", "{body}");
        // Nor raises it past 100 times its start.
        let (_, body) = settings(r#"{"throttle": 20001}"#).await;
        assert_eq!(body["error"], "conflicting_settings", "{body}");
        assert_eq!(body["field"], "throttle", "{body}");

        let config = telnet.send("config-json").await;
        assert!(config.contains(r#""throttle_requests":400"#), "{}", config);
        let _ = telnet.send("shutdown").await;
    };
    tokio::time::timeout(TEST_TIMEOUT, test)
        .await
        .expect("the test timed out");
    tokio::time::timeout(TEST_TIMEOUT, run)
        .await
        .expect("the load test did not end")
        .unwrap()
        .unwrap();
}

/// A run started without a throttle can not turn one on: refused, naming
/// the throttle.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial]
async fn an_unthrottled_run_can_not_turn_the_throttle_on() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        })
        .await;
    let run = tokio::spawn(attack(&server, None).execute());
    let test = async {
        let mut telnet = Telnet::connect().await;
        let reply = telnet.send("throttle 100").await;
        assert!(
            reply.starts_with("failed to configure throttle, the load test started without one"),
            "{}",
            reply
        );
        let (status, body) = settings(r#"{"throttle": 100}"#).await;
        assert_eq!((status, &body["ok"]), (200, &false.into()), "{body}");
        assert_eq!(body["error"], "invalid_phase", "{body}");
        assert_eq!(body["field"], "throttle", "{body}");
        let _ = telnet.send("shutdown").await;
    };
    tokio::time::timeout(TEST_TIMEOUT, test)
        .await
        .expect("the test timed out");
    tokio::time::timeout(TEST_TIMEOUT, run)
        .await
        .expect("the load test did not end")
        .unwrap()
        .unwrap();
}
