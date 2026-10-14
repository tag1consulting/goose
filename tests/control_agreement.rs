//! Telnet, WebSocket and the dashboard control endpoints agree: the same
//! command in the same state gets the same accept or refuse decision and the
//! same message from each of them.

#![cfg(feature = "dashboard")]

use futures::{SinkExt, StreamExt};
use gumdrop::Options;
use httpmock::{Method::GET, MockServer};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use goose::config::GooseConfiguration;
use goose::controller::{ControllerWebSocketRequest, ControllerWebSocketResponse};
use goose::prelude::*;

// Ports no other test uses.
const TELNET_PORT: u16 = 5126;
const WEBSOCKET_PORT: u16 = 5127;
const DASHBOARD_PORT: u16 = 5128;
const TOKEN: &str = "agreement-token";

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

/// A telnet client that reads each reply up to the next prompt.
struct Telnet(TcpStream);

impl Telnet {
    async fn connect() -> Self {
        let mut telnet = Telnet(connect(TELNET_PORT).await);
        assert_eq!(telnet.read_reply().await, "goose> ");
        telnet
    }

    /// Read until the prompt or until the server closes the connection.
    async fn read_reply(&mut self) -> String {
        let mut reply = Vec::new();
        let mut buf = [0u8; 4096];
        while !reply.ends_with(b"goose> ") {
            let n = tokio::time::timeout(Duration::from_secs(20), self.0.read(&mut buf))
                .await
                .expect("telnet reply timed out")
                .expect("telnet read");
            if n == 0 {
                break;
            }
            reply.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(reply).expect("utf-8 reply")
    }

    /// Send `line` and return the reply without its prompt.
    async fn send(&mut self, line: &str) -> String {
        self.0
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .expect("telnet write");
        let reply = self.read_reply().await;
        reply
            .strip_suffix("\ngoose> ")
            .unwrap_or(&reply)
            .to_string()
    }
}

type WebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn websocket_connect() -> WebSocket {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{WEBSOCKET_PORT}")).await {
            Ok((stream, _)) => return stream,
            Err(e) if Instant::now() > deadline => panic!("websocket connect: {}", e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Send `line` and return the reply's success flag and text.
async fn websocket_send(websocket: &mut WebSocket, line: &str) -> (bool, String) {
    let request = serde_json::to_string(&ControllerWebSocketRequest {
        request: line.to_string(),
    })
    .unwrap();
    websocket
        .send(Message::Text(request.into()))
        .await
        .expect("websocket send");
    let message = tokio::time::timeout(Duration::from_secs(20), websocket.next())
        .await
        .expect("websocket reply timed out")
        .expect("websocket closed")
        .expect("websocket read");
    let reply: ControllerWebSocketResponse =
        serde_json::from_str(message.to_text().expect("text reply")).expect("json reply");
    (reply.success, reply.response)
}

async fn connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => return stream,
            Err(e) if Instant::now() > deadline => panic!("connect {}: {}", port, e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// POST a dashboard control command and return its `ok` and `message`.
async fn dashboard_post(client: &reqwest::Client, command: &str, body: &str) -> (bool, String) {
    let reply: serde_json::Value = client
        .post(format!(
            "http://127.0.0.1:{DASHBOARD_PORT}/api/v1/control/{command}"
        ))
        .header("Authorization", format!("Bearer {TOKEN}"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("control POST")
        .json()
        .await
        .expect("control json");
    (
        reply["ok"].as_bool().expect("ok"),
        reply["message"].as_str().expect("message").to_string(),
    )
}

/// Poll the dashboard snapshot until the phase is one of `phases`.
async fn wait_for_phase(client: &reqwest::Client, phases: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let phase = match client
            .get(format!("http://127.0.0.1:{DASHBOARD_PORT}/api/v1/snapshot"))
            .header("Authorization", format!("Bearer {TOKEN}"))
            .send()
            .await
        {
            Ok(response) => response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|snapshot| snapshot["phase"].as_str().map(str::to_string)),
            Err(_) => None,
        };
        if phase.as_deref().is_some_and(|p| phases.contains(&p)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "phase {:?} never reached {:?}",
            phase,
            phases
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The three front ends of one Goose process.
struct FrontEnds {
    telnet: Telnet,
    websocket: WebSocket,
    client: reqwest::Client,
}

/// Which front end sends a command.
#[derive(Clone, Copy, Debug)]
enum Via {
    Telnet,
    WebSocket,
    Dashboard,
}

impl FrontEnds {
    /// Send one command through `via` and return its decision and message.
    /// Telnet has no success flag, so its decision is `None`.
    async fn send(
        &mut self,
        via: Via,
        line: &str,
        command: &str,
        body: &str,
    ) -> (Option<bool>, String) {
        match via {
            Via::Telnet => (None, self.telnet.send(line).await),
            Via::WebSocket => {
                let (ok, message) = websocket_send(&mut self.websocket, line).await;
                (Some(ok), message)
            }
            Via::Dashboard => {
                let (ok, message) = dashboard_post(&self.client, command, body).await;
                (Some(ok), message)
            }
        }
    }

    /// Send the same command through all three front ends in the same state
    /// and require the same decision and message from each.
    async fn agree(&mut self, line: &str, command: &str, body: &str) -> (bool, String) {
        let telnet = self.send(Via::Telnet, line, command, body).await;
        let websocket = self.send(Via::WebSocket, line, command, body).await;
        let dashboard = self.send(Via::Dashboard, line, command, body).await;
        assert_eq!(
            websocket, dashboard,
            "{line}: websocket and dashboard differ"
        );
        assert_eq!(
            telnet.1, dashboard.1,
            "{line}: telnet and dashboard messages differ"
        );
        (dashboard.0.unwrap(), dashboard.1)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_agreement() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let host = server.base_url();
    let telnet_port = TELNET_PORT.to_string();
    let websocket_port = WEBSOCKET_PORT.to_string();
    let dashboard_port = DASHBOARD_PORT.to_string();
    let configuration = GooseConfiguration::parse_args_default(&[
        "--users",
        "2",
        "--increase-rate",
        "50",
        "--host",
        &host,
        "--no-autostart",
        "--telnet-host",
        "127.0.0.1",
        "--telnet-port",
        &telnet_port,
        "--websocket-host",
        "127.0.0.1",
        "--websocket-port",
        &websocket_port,
        "--dashboard",
        "--dashboard-host",
        "127.0.0.1",
        "--dashboard-port",
        &dashboard_port,
        "--dashboard-control",
        "--dashboard-auth-token",
        TOKEN,
        "--co-mitigation",
        "disabled",
        "--quiet",
        "--no-save",
    ])
    .unwrap();
    let goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("Agreement").register_transaction(transaction!(get_index)));
    let load = tokio::spawn(async move { goose_attack.execute().await });

    let mut front = FrontEnds {
        telnet: Telnet::connect().await,
        websocket: websocket_connect().await,
        client: reqwest::Client::new(),
    };
    wait_for_phase(&front.client, &["idle"]).await;

    // Users out of range, then in range, while idle.
    let users_message = "users must be an integer between 1 and 100000";
    for (line, body) in [
        ("users 0", r#"{"users":0}"#),
        ("users 100001", r#"{"users":100001}"#),
    ] {
        let (ok, message) = front.agree(line, "users", body).await;
        assert!(!ok, "{}", line);
        assert_eq!(message, users_message, "{line}");
    }
    assert_eq!(
        front.agree("users 3", "users", r#"{"users":3}"#).await,
        (true, "users configured".to_string())
    );

    // Stop while idle is refused by all three.
    assert_eq!(
        front.agree("stop", "stop", "{}").await,
        (false, "load test not running, failed to stop".to_string())
    );

    // Start, start again, change users, and stop, through each front end in
    // turn: each one sees the same replies.
    let mut runs = Vec::new();
    for via in [Via::Telnet, Via::WebSocket, Via::Dashboard] {
        let mut replies = Vec::new();
        replies.push(front.send(via, "start", "start", "{}").await.1);
        wait_for_phase(&front.client, &["increase", "maintain"]).await;
        replies.push(front.send(via, "start", "start", "{}").await.1);
        replies.push(
            front
                .send(via, "users 4", "users", r#"{"users":4}"#)
                .await
                .1,
        );
        replies.push(front.send(via, "stop", "stop", "{}").await.1);
        wait_for_phase(&front.client, &["idle"]).await;
        runs.push((via, replies));
    }
    for (via, replies) in &runs {
        assert_eq!(
            replies,
            &[
                "load test started",
                "unable to start load test, be sure it is idle and host is configured",
                "users configured",
                "load test stopped",
            ],
            "{:?}",
            via
        );
    }

    // Quit: `quit` disconnects a telnet or WebSocket client and leaves the
    // run alone; the dashboard Quit is refused while running and shuts Goose
    // down from Idle.
    assert!(front.send(Via::Dashboard, "start", "start", "{}").await.0 == Some(true));
    wait_for_phase(&front.client, &["increase", "maintain"]).await;
    assert_eq!(
        dashboard_post(&front.client, "quit", "{}").await,
        (
            false,
            "unable to quit, be sure the load test is idle".to_string()
        )
    );
    assert_eq!(front.telnet.send("quit").await, "goodbye!");
    assert_eq!(
        websocket_send(&mut front.websocket, "quit").await,
        (true, "goodbye!".to_string())
    );
    wait_for_phase(&front.client, &["increase", "maintain"]).await;
    assert_eq!(
        dashboard_post(&front.client, "stop", "{}").await,
        (true, "load test stopped".to_string())
    );
    wait_for_phase(&front.client, &["idle"]).await;
    assert_eq!(
        dashboard_post(&front.client, "quit", "{}").await,
        (true, "goose is shutting down".to_string())
    );
    tokio::time::timeout(Duration::from_secs(20), load)
        .await
        .expect("Goose exits on Quit")
        .expect("join load test")
        .expect("load test");
}
