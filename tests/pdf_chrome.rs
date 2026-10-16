//! Chrome problems stop a run with a PDF report before any user starts.
//!
//! These tests set `CHROME`, so they live in their own test binary, where the
//! variable can't leak into other tests.
#![cfg(feature = "pdf-reports")]

use httpmock::MockServer;
use serial_test::serial;
use std::path::{Path, PathBuf};

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

fn index_mock(server: &MockServer) -> httpmock::Mock<'_> {
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path(INDEX_PATH);
        then.status(200);
    })
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("goose-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs a short load test that writes `reports`, and returns its result.
async fn run(
    server: &MockServer,
    reports: &[&Path],
    extra: &[&str],
) -> Result<GooseMetrics, GooseError> {
    let mut flags = vec![
        "--users".to_string(),
        "1".to_string(),
        "--increase-rate".to_string(),
        "1".to_string(),
        "--run-time".to_string(),
        "1".to_string(),
    ];
    flags.extend(extra.iter().map(|flag| flag.to_string()));
    for report in reports {
        flags.push("--report-file".to_string());
        flags.push(report.to_string_lossy().into_owned());
    }
    let configuration =
        common::build_configuration(server, flags.iter().map(String::as_str).collect());
    let scenario = scenario!("LoadTest").register_transaction(transaction!(get_index));
    common::build_load_test(configuration, vec![scenario], None, None)
        .execute()
        .await
}

/// Asserts a run with `CHROME` set to `chrome` fails before sending a request.
async fn assert_fails_before_start(chrome: &Path, name: &str) {
    let dir = temp_dir(name);
    let pdf = dir.join("report.pdf");
    let server = MockServer::start();
    let index = index_mock(&server);

    std::env::set_var("CHROME", chrome);
    let result = run(&server, &[&pdf], &[]).await;
    std::env::remove_var("CHROME");

    let error = result.expect_err("the run should fail when CHROME can't run");
    assert!(format!("{:?}", error).contains("CHROME"), "{:?}", error);
    assert_eq!(
        index.calls(),
        0,
        "users started although CHROME is unusable"
    );
    assert!(!pdf.exists(), "an empty PDF report was left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[serial]
async fn chrome_that_does_not_exist_fails_before_the_test() {
    let dir = temp_dir("pdf-chrome-missing");
    assert_fails_before_start(&dir.join("no-such-chrome"), "pdf-chrome-missing-run").await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[serial]
async fn chrome_that_is_a_directory_fails_before_the_test() {
    let dir = temp_dir("pdf-chrome-directory");
    assert_fails_before_start(&dir, "pdf-chrome-directory-run").await;
    std::fs::remove_dir_all(&dir).unwrap();
}
