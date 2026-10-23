//! Delete refuses the folder a bare `--goose-log` is still open in after its
//! run ends, so the lines Goose logs while Idle never go to a removed file.
//!
//! The Goose log is the global logger, set once for the process, so this is
//! the only test in its file: each integration test file runs in its own
//! process.

#![cfg(feature = "dashboard")]

use httpmock::{Method::GET, MockServer};
use std::time::{Duration, Instant};

use goose::config::GooseConfiguration;
use goose::prelude::*;

const AUTH_TOKEN: &str = "test-dashboard-token";

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Wait until the snapshot's phase is `phase`.
async fn wait_for_phase(client: &reqwest::Client, base: &str, phase: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snap: Option<serde_json::Value> = match client
            .get(format!("{base}/api/v1/snapshot"))
            .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.json().await.ok(),
            _ => None,
        };
        if snap.as_ref().and_then(|snap| snap["phase"].as_str()) == Some(phase) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never reached {}: {:?}",
            phase,
            snap
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn post(client: &reqwest::Client, base: &str, route: &str) {
    let resp = client
        .post(format!("{base}/api/v1/control/{route}"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{route}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_refuses_the_folder_of_the_open_goose_log() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let parent =
        std::env::temp_dir().join(format!("goose-dashboard-goose-log-{}", std::process::id()));
    let folder = parent.join("run");
    let folder_str = folder.to_string_lossy().into_owned();
    let port = free_port().to_string();
    let host = server.base_url();
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        &host,
        "--users",
        "2",
        "--increase-rate",
        "50",
        "--quiet",
        "--no-telnet",
        "--no-websocket",
        "--co-mitigation",
        "disabled",
        "--dashboard",
        "--dashboard-port",
        &port,
        "--dashboard-control",
        "--dashboard-auth-token",
        AUTH_TOKEN,
        "--no-autostart",
        "--output-dir",
        &folder_str,
        "--goose-log",
    ])
    .unwrap();
    let goose_attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)));
    let load = tokio::spawn(goose_attack.execute());
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    wait_for_phase(&client, &base, "idle").await;
    post(&client, &base, "start").await;
    wait_for_phase(&client, &base, "maintain").await;
    post(&client, &base, "stop").await;
    wait_for_phase(&client, &base, "idle").await;

    let resp = client
        .delete(format!("{base}/api/v1/reports/run"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    let log_kept = folder.join("goose.log").is_file();
    load.abort();
    let _ = load.await;
    std::fs::remove_dir_all(&parent).ok();
    assert_eq!(status, 409, "{text}");
    assert_eq!(text, "Goose is writing its log to this folder.");
    assert!(log_kept, "Delete removed the open Goose log");
}
