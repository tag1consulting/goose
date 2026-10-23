//! A `--dashboard` run whose working directory can't be written to still
//! runs: the folder only the dashboard asks for is skipped with a warning,
//! and Save report is off for the run.
//!
//! This changes the working directory, which is global to the process, so it
//! is the only test in its file: each integration test file runs in its own
//! process.

#![cfg(feature = "dashboard")]

use httpmock::{Method::GET, MockServer};
use std::os::unix::fs::PermissionsExt;

use goose::config::GooseConfiguration;
use goose::prelude::*;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn a_dashboard_run_without_a_writable_folder_still_runs() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let host = server.base_url();
    let work =
        std::env::temp_dir().join(format!("goose-dashboard-no-folder-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(work.join("probe"), "x").is_ok() {
        std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&work).ok();
        eprintln!(
            "skipped: {} can still be written to read only, so this runs as root",
            work.display()
        );
        return;
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(&work).unwrap();
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        &host,
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--run-time",
        "1",
        "--co-mitigation",
        "disabled",
        "--quiet",
        "--no-telnet",
        "--no-websocket",
        "--dashboard",
        "--dashboard-port",
        &port,
    ])
    .unwrap();
    let result = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await;
    std::env::set_current_dir(&previous).unwrap();
    let left: Vec<_> = std::fs::read_dir(&work)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(&work).ok();

    assert!(result.is_ok(), "{:?}", result.err());
    assert!(mock.calls() > 0);
    assert!(left.is_empty(), "{:?}", left);
}
