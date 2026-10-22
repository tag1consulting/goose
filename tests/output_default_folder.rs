//! What a run writes in its working directory: nothing without an output
//! flag, the same files as Goose 0.18 for the forms it took, and with a bare
//! `--report-file` a folder under `goose-runs` named for the start time.
//!
//! This changes the working directory, which is global to the process, so it
//! is the only test in its file: each integration test file runs in its own
//! process.

use httpmock::{Method::GET, MockServer};
use std::path::Path;

use goose::config::GooseConfiguration;
use goose::prelude::*;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

/// Every file and folder under `dir`, as paths relative to it, sorted.
fn tree(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            found.push(
                path.strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    found.sort();
    found
}

async fn run(host: &str, args: &[&str]) {
    let mut all = vec![
        "--host",
        host,
        "--users",
        "1",
        "--increase-rate",
        "1",
        "--run-time",
        "1",
        "--co-mitigation",
        "disabled",
        "--quiet",
    ];
    all.extend_from_slice(args);
    let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("LoadTest").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .unwrap();
}

#[tokio::test]
async fn runs_write_only_what_they_are_asked_for() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let host = server.base_url();
    let work = std::env::temp_dir().join(format!(
        "goose-output-default-folder-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&work).unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(&work).unwrap();

    // Rule 1: no flags write nothing and create no folder.
    run(&host, &[]).await;
    let none = tree(Path::new("."));

    // Rule 2: every 0.18 form of the report and log options writes the file
    // it names, and nothing more.
    run(
        &host,
        &[
            "--report-file",
            "report.html",
            "--report-file",
            "report.json",
            "--report-file",
            "report.md",
            "-R",
            "requests.log",
            "-T",
            "transactions.log",
            "-S",
            "scenarios.log",
            "-E",
            "errors.log",
            "-D",
            "debug.log",
        ],
    )
    .await;
    let old_forms = tree(Path::new("."));
    for file in &old_forms {
        std::fs::remove_file(file).unwrap();
    }

    // Rule 3: a bare --report-file writes report.json and report.html into
    // goose-runs/<YYYYMMDD-HHMMSS>.
    run(&host, &["--report-file"]).await;
    let bare = tree(Path::new("."));

    std::env::set_current_dir(&previous).unwrap();
    std::fs::remove_dir_all(&work).ok();

    assert!(mock.calls() > 0);
    assert!(none.is_empty(), "{:?}", none);
    assert_eq!(
        old_forms,
        vec![
            "debug.log",
            "errors.log",
            "report.html",
            "report.json",
            "report.md",
            "requests.log",
            "scenarios.log",
            "transactions.log",
        ]
    );
    assert_eq!(bare.len(), 4, "{:?}", bare);
    assert_eq!(bare[0], "goose-runs");
    let folder = &bare[1];
    let name = folder.strip_prefix("goose-runs/").unwrap();
    assert!(
        regex::Regex::new(r"^\d{8}-\d{6}$").unwrap().is_match(name),
        "{}",
        name
    );
    assert_eq!(
        &bare[2..],
        [
            format!("{folder}/report.html"),
            format!("{folder}/report.json")
        ]
    );
}
