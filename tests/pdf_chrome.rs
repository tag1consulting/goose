//! Chrome problems stop a run with a PDF report before any user starts, and a
//! PDF that fails at the end of a run costs no other report.
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

/// Asserts a run with `CHROME` set to `chrome` fails before sending a request,
/// and leaves none of its report files behind.
async fn assert_fails_before_start(chrome: &Path, name: &str) {
    let dir = temp_dir(name);
    let pdf = dir.join("report.pdf");
    let json = dir.join("report.json");
    let server = MockServer::start();
    let index = index_mock(&server);

    std::env::set_var("CHROME", chrome);
    let result = run(&server, &[&pdf, &json], &[]).await;
    std::env::remove_var("CHROME");

    let error = result.expect_err("the run should fail when CHROME can't run");
    assert!(format!("{:?}", error).contains("CHROME"), "{:?}", error);
    assert_eq!(
        index.calls(),
        0,
        "users started although CHROME is unusable"
    );
    assert!(!pdf.exists(), "an empty PDF report was left behind");
    assert!(!json.exists(), "an empty JSON report was left behind");
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

/// Writes an executable shell script.
#[cfg(unix)]
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A Chrome that exits as soon as it starts, as one missing a shared library
/// does, fails the check at startup within `--pdf-timeout`, and the error
/// shows what it printed.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn chrome_that_exits_at_launch_fails_before_the_test() {
    let dir = temp_dir("pdf-chrome-exits");
    let chrome = dir.join("chrome");
    script(
        &chrome,
        "echo 'chrome: error while loading shared libraries: libnss3.so' >&2\nexit 127",
    );
    let pdf = dir.join("report.pdf");
    let json = dir.join("report.json");
    let server = MockServer::start();
    let index = index_mock(&server);

    let started = std::time::Instant::now();
    std::env::set_var("CHROME", &chrome);
    let result = run(&server, &[&pdf, &json], &["--pdf-timeout", "10"]).await;
    std::env::remove_var("CHROME");

    let error = format!("{:?}", result.expect_err("the run should fail"));
    assert!(error.contains("libnss3.so"), "{}", error);
    assert!(error.contains(&*chrome.to_string_lossy()), "{}", error);
    assert!(
        started.elapsed().as_secs() < 30,
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(index.calls(), 0, "users started although Chrome can't run");
    assert!(!pdf.exists(), "an empty PDF report was left behind");
    assert!(!json.exists(), "an empty JSON report was left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A PDF report that fails at the end of the run leaves the JSON report
/// complete and no empty PDF behind.
///
/// `CHROME` points at a script that runs the installed browser for the check
/// at startup and deletes itself, so the launch at the end of the run fails.
#[cfg(unix)]
#[tokio::test]
#[serial]
#[ignore = "needs an installed Chrome or Chromium; CI runs it with --include-ignored"]
async fn failed_pdf_keeps_other_reports() {
    let Ok(real_chrome) = headless_chrome::browser::default_executable() else {
        panic!("this test needs an installed Chrome or Chromium to wrap, and found none");
    };
    let dir = temp_dir("pdf-chrome-fails-late");
    let wrapper = dir.join("chrome");
    script(
        &wrapper,
        &format!("rm -f \"$0\"\nexec '{}' \"$@\"", real_chrome.display()),
    );

    let pdf = dir.join("report.pdf");
    let pdf_gz = dir.join("report.pdf.gz");
    let json = dir.join("report.json");
    let server = MockServer::start();
    let index = index_mock(&server);

    std::env::set_var("CHROME", &wrapper);
    let result = run(&server, &[&pdf, &pdf_gz, &json], &[]).await;
    std::env::remove_var("CHROME");

    assert!(!wrapper.exists(), "Chrome was not checked at startup");
    assert!(index.calls() > 0, "the load test did not run");
    let error = format!("{:?}", result.expect_err("the PDF report should fail"));
    assert!(error.contains("does not exist"), "{}", error);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert!(report.get("raw_metrics").is_some(), "{}", report);
    assert!(!pdf.exists(), "the empty PDF report was left behind");
    assert!(
        !pdf_gz.exists(),
        "the empty gzipped PDF report was left behind"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A PDF report named `.pdf.gz` is the run's PDF report, gzipped: Chrome
/// renders the PDF once for every name.
#[tokio::test]
#[serial]
#[ignore = "needs an installed Chrome or Chromium; CI runs it with --include-ignored"]
async fn a_gz_pdf_is_the_same_pdf_gzipped() {
    use std::io::Read;
    let dir = temp_dir("pdf-chrome-gz");
    let pdf = dir.join("report.pdf");
    let gz = dir.join("report.pdf.gz");
    let server = MockServer::start();
    let _index = index_mock(&server);
    run(&server, &[&pdf, &gz], &[])
        .await
        .expect("the run writes both");
    let plain = std::fs::read(&pdf).unwrap();
    assert!(plain.starts_with(b"%PDF"));
    let mut gunzipped = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&gz).unwrap())
        .read_to_end(&mut gunzipped)
        .unwrap();
    assert_eq!(gunzipped, plain);
    std::fs::remove_dir_all(&dir).unwrap();
}
