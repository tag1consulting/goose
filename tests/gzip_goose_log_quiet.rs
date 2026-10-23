//! A Goose log named `.gz` is whole gzip even when nothing reaches it, and it
//! is gzipped at the level `GooseDefault::Compress` sets in code.
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
async fn a_quiet_gz_goose_log_is_whole_gzip_at_the_level_set_in_code() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let dir =
        std::env::temp_dir().join(format!("goose-gzip-goose-log-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("goose.log.gz");
    let log_str = log.to_string_lossy().into_owned();
    let host = server.base_url();
    // The default log level, warnings only: a clean run logs nothing.
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
        "-G",
        &log_str,
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .set_default(GooseDefault::Compress, 9)
        .unwrap()
        .execute()
        .await
        .unwrap();
    let bytes = std::fs::read(&log).unwrap();
    let mut text = String::new();
    let whole = flate2::read::MultiGzDecoder::new(bytes.as_slice()).read_to_string(&mut text);
    std::fs::remove_dir_all(&dir).ok();
    whole.expect("the Goose log is whole gzip");
    assert!(text.is_empty(), "{}", text);
    assert_eq!(
        bytes[8], 2,
        "GooseDefault::Compress 9 reaches the Goose log"
    );
}
