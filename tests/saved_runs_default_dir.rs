//! A default runs directory that can't be written only warns: the run
//! completes, unsaved, with its metrics.
//!
//! This changes the working directory, which is global to the process, so it
//! is the only test in its file: each integration test file runs in its own
//! process.

#![cfg(unix)]

use gumdrop::Options;
use httpmock::{Method::GET, MockServer};
use std::os::unix::fs::PermissionsExt;

use goose::config::GooseConfiguration;
use goose::prelude::*;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn unwritable_default_runs_dir_warns_and_the_run_completes() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });

    // A working directory holding a read only `goose-runs`.
    let work = std::env::temp_dir().join(format!(
        "goose-saved-runs-default-dir-{}",
        std::process::id()
    ));
    let runs = work.join("goose-runs");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::set_permissions(&runs, std::fs::Permissions::from_mode(0o555)).unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(&work).unwrap();

    // Saving is on (no --no-save) and the runs directory is the default.
    let host = server.base_url();
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
    ])
    .unwrap();
    let result = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await;

    // Restore the working directory and clean up before asserting.
    std::env::set_current_dir(&previous).unwrap();
    let entries = std::fs::read_dir(&runs).unwrap().count();
    std::fs::set_permissions(&runs, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(&work).ok();

    let metrics = result.expect("the run completes although it can't be saved");
    assert!(mock.calls() > 0);
    let requests: usize = metrics
        .requests
        .values()
        .map(|r| r.success_count + r.fail_count)
        .sum();
    assert!(requests > 0, "the run has metrics");
    assert_eq!(entries, 0, "nothing was saved");
}
