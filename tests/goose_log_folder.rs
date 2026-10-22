//! A bare `--goose-log` writes a plain `goose.log` into the run's folder, and
//! Goose prints the folder it writes into.
//!
//! The Goose log is the global logger, set once for the process, so this is
//! the only test in its file: each integration test file runs in its own
//! process.

use httpmock::{Method::GET, MockServer};

use goose::config::GooseConfiguration;
use goose::prelude::*;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn a_bare_goose_log_is_goose_log_in_the_run_folder() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let dir = std::env::temp_dir().join(format!("goose-goose-log-folder-{}", std::process::id()));
    let folder = dir.join("run");
    let folder_str = folder.to_string_lossy().into_owned();
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
        "--quiet",
        "--output-dir",
        &folder_str,
        "-G",
        "-g",
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .unwrap();
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let text = std::fs::read_to_string(folder.join("goose.log")).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(names, vec!["goose.log"]);
    assert!(text.contains("global host configured"), "{}", text);
    assert!(
        text.contains(&format!("writing this run's files to {folder_str}")),
        "{}",
        text
    );
}
