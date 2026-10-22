//! A gzipped Goose log ends its gzip member when a panic unwinds through
//! `execute()`, so the file stays whole gzip and keeps its last lines.
//!
//! The Goose log is the global logger, set once for the process, so this is
//! the only test in its file: each integration test file runs in its own
//! process.

use httpmock::{Method::GET, MockServer};
use std::io::Read;

use goose::config::GooseConfiguration;
use goose::prelude::*;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

pub async fn panic_at_stop(_user: &mut GooseUser) -> TransactionResult {
    panic!("test_stop panicked");
}

#[tokio::test]
async fn a_gz_goose_log_is_whole_after_a_panic() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let dir =
        std::env::temp_dir().join(format!("goose-gzip-goose-log-panic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("goose.log.gz");
    let log_str = log.to_string_lossy().into_owned();
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        &server.base_url(),
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--run-time",
        "1",
        "--quiet",
        "-G",
        &log_str,
        "-g",
    ])
    .unwrap();
    let attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .test_stop(transaction!(panic_at_stop));
    let joined = tokio::spawn(attack.execute()).await;
    assert!(joined.unwrap_err().is_panic());
    let mut text = String::new();
    let read = flate2::read::MultiGzDecoder::new(std::fs::File::open(&log).unwrap())
        .read_to_string(&mut text);
    std::fs::remove_dir_all(&dir).ok();
    read.expect("the Goose log is whole gzip");
    assert!(
        text.contains("global host configured"),
        "the run's lines: {}",
        text
    );
}
