//! `--compress 9` gzips a bare `--goose-log` too, at level 9: `goose.log.gz`
//! in the run's folder, beside the reports.
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
async fn compress_gzips_a_bare_goose_log() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let dir = std::env::temp_dir().join(format!(
        "goose-gzip-goose-log-folder-{}",
        std::process::id()
    ));
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
        "--report-file",
        "-G",
        "-g",
        "--compress",
        "9",
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .unwrap();
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut text = String::new();
    flate2::read::MultiGzDecoder::new(std::fs::File::open(folder.join("goose.log.gz")).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let gz = std::fs::read(folder.join("goose.log.gz")).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        names,
        vec!["goose.log.gz", "report.html.gz", "report.json.gz"]
    );
    // The lines logged before the run's folder was known are there too.
    assert!(text.contains("global host configured"), "{}", text);
    // Rule 4: Goose prints the folder it writes into.
    assert!(
        text.contains(&format!("writing this run's files to {folder_str}")),
        "{}",
        text
    );
    // Rule 15: the level reaches the Goose log: XFL is 2 at level 9.
    assert_eq!(gz[8], 2, "level 9");
}
