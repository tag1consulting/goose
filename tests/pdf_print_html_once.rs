//! With a PDF report as well as `--pdf-print-html`, the print HTML is
//! written, and logged, once. Its own test binary: the Goose log it reads is
//! set up once per process, by the first load test that runs.
#![cfg(feature = "pdf-reports")]

use httpmock::{Method::GET, MockServer};

use goose::prelude::*;

mod common;

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn test_pdf_print_html_written_once_with_a_pdf() {
    let pdf_file = "test-pdf-print-html-once.pdf";
    let html_file = "test-pdf-print-html-once.html";
    let log_file = "test-pdf-print-html-once.log";

    let server = MockServer::start();
    let index = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });

    let configuration = common::build_configuration(
        &server,
        vec![
            "--report-file",
            pdf_file,
            "--pdf-print-html",
            html_file,
            "--goose-log",
            log_file,
            "-g",
        ],
    );
    let goose_attack = common::build_load_test(
        configuration,
        vec![scenario!("LoadTest").register_transaction(transaction!(get_index))],
        None,
        None,
    );
    common::run_load_test(goose_attack, None).await;
    assert!(index.calls() > 0);

    let log = std::fs::read_to_string(log_file).expect("the Goose log");
    common::cleanup_files(vec![pdf_file, html_file, log_file]);
    let saved = log
        .lines()
        .filter(|line| line.contains("PDF-optimized HTML saved to"))
        .count();
    assert_eq!(saved, 1, "{}", log);
}
