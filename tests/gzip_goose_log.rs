//! A Goose log named `.gz` ends a gzip member at the end of each
//! `execute()`, so a process that runs several load tests, one of them
//! failing, leaves one whole gzip file holding every run's lines.
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
async fn a_gz_goose_log_holds_every_run() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let dir = std::env::temp_dir().join(format!("goose-gzip-goose-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("goose.log.gz");
    let log_str = log.to_string_lossy().into_owned();
    let host = server.base_url();
    for _ in 0..2 {
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
            "-g",
        ])
        .unwrap();
        GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
            .execute()
            .await
            .unwrap();
        // Between and after the runs, the file is whole gzip.
        let mut text = String::new();
        flate2::read::MultiGzDecoder::new(std::fs::File::open(&log).unwrap())
            .read_to_string(&mut text)
            .expect("the Goose log is whole gzip");
    }
    // A run that fails after it has logged leaves the file whole too.
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        &host,
        "--quiet",
        "-G",
        &log_str,
        "-g",
        "--report-file",
        "report.txt",
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .expect_err("a report named .txt is refused");
    let mut text = String::new();
    flate2::read::MultiGzDecoder::new(std::fs::File::open(&log).unwrap())
        .read_to_string(&mut text)
        .expect("the Goose log is whole gzip after a failed run");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        text.matches("global host configured").count(),
        3,
        "every run's lines: {}",
        text
    );
}
