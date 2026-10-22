//! Lines logged after `execute()` returns reach a gzipped Goose log, which
//! stays whole gzip: outside a run each line is a whole gzip member.
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

#[tokio::test]
async fn lines_after_execute_reach_a_gz_goose_log() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let dir =
        std::env::temp_dir().join(format!("goose-gzip-goose-log-after-{}", std::process::id()));
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
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .unwrap();
    // More than the 8 KiB a member buffers.
    for line in 0..1000 {
        log::warn!("logged after execute {line}");
    }
    let mut text = String::new();
    let read = flate2::read::MultiGzDecoder::new(std::fs::File::open(&log).unwrap())
        .read_to_string(&mut text);
    std::fs::remove_dir_all(&dir).ok();
    read.expect("the Goose log is whole gzip");
    assert!(text.contains("global host configured"), "{}", text);
    assert!(text.contains("logged after execute 999"), "{}", text);
}
