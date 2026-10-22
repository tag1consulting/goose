//! Gzip: a report or log named `.gz` is written gzipped, `--compress` gzips
//! what Goose writes into the run's folder, and Goose reads a `.json.gz`
//! report wherever it reads a JSON report. Each test writes into its own
//! temporary folder.

use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::io::Read;
use std::path::{Path, PathBuf};

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

pub async fn get_missing(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/missing").await?;
    Ok(())
}

pub async fn get_index_then_cancel(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    goose::trigger_killswitch("compression test");
    Ok(())
}

fn setup_mock_server_endpoints(server: &MockServer) -> (Mock<'_>, Mock<'_>) {
    (
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200).body("ok");
        }),
        server.mock(|when, then| {
            when.method(GET).path("/missing");
            then.status(404);
        }),
    )
}

/// A new empty folder for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "goose-compression-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The bytes of a gzip file, every member, checked as `gzip -t` checks it:
/// an error for a stream that is not gzip, is cut short or fails its CRC.
fn gunzip(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    flate2::read::MultiGzDecoder::new(std::fs::File::open(path)?).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The gzip header's XFL byte: 2 at level 9, 4 at level 1, 0 between.
fn xfl(path: &Path) -> u8 {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..2], [0x1f, 0x8b], "{:?} is not gzip", path);
    bytes[8]
}

fn scenario(transaction: Transaction) -> Scenario {
    scenario!("LoadTest").register_transaction(transaction)
}

async fn run(server: &MockServer, args: Vec<&str>) -> GooseMetrics {
    run_with(server, args, scenario(transaction!(get_index))).await
}

async fn run_with(server: &MockServer, args: Vec<&str>, scenario: Scenario) -> GooseMetrics {
    let configuration = common::build_configuration(server, args);
    common::run_load_test(
        common::build_load_test(configuration, vec![scenario], None, None),
        None,
    )
    .await
}

fn requests(metrics: &GooseMetrics) -> usize {
    metrics
        .requests
        .values()
        .map(|r| r.success_count + r.fail_count)
        .sum()
}

/// Rule 14: a report named `.gz` is the same report, gzipped.
#[tokio::test]
#[serial]
async fn a_gz_report_is_the_same_report_gzipped() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);
    let dir = temp_dir("reports");
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let mut args = Vec::new();
    for name in ["r.json", "r.html", "r.md"] {
        args.push(path(name));
        args.push(path(&format!("{name}.gz")));
    }
    let mut flags = Vec::new();
    for arg in &args {
        flags.push("--report-file");
        flags.push(arg.as_str());
    }
    run(&server, flags).await;
    for name in ["r.json", "r.html", "r.md"] {
        let plain = std::fs::read(dir.join(name)).unwrap();
        let gzipped = dir.join(format!("{name}.gz"));
        assert_eq!(gunzip(&gzipped).unwrap(), plain, "{}", name);
        assert_eq!(xfl(&gzipped), 0, "level 6 by default");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule 14: a log named `.gz` is written gzipped, one line for each request
/// in CSV and in JSON, and every other log opens as gzip.
#[tokio::test]
#[serial]
async fn gz_logs_are_whole_gzip_files() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);
    let dir = temp_dir("logs");
    for (format, header) in [("csv", 1), ("json", 0)] {
        let log = dir.join(format!("requests.{format}.gz"));
        let log_str = log.to_string_lossy().into_owned();
        let metrics = run(
            &server,
            vec![
                "-R",
                &log_str,
                "--request-format",
                format,
                "--no-reset-metrics",
            ],
        )
        .await;
        let text = String::from_utf8(gunzip(&log).unwrap()).unwrap();
        assert_eq!(
            text.lines().count(),
            requests(&metrics) + header,
            "{}",
            format
        );
    }
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let (transactions, scenarios, errors, debug) = (
        path("transactions.log.gz"),
        path("scenarios.json.gz"),
        path("errors.csv.gz"),
        path("debug.json.gz"),
    );
    run_with(
        &server,
        vec![
            "-T",
            &transactions,
            "--transaction-format",
            "raw",
            "-S",
            &scenarios,
            "-E",
            &errors,
            "--error-format",
            "csv",
            "-D",
            &debug,
        ],
        scenario(transaction!(get_missing)),
    )
    .await;
    for log in [&transactions, &scenarios, &errors] {
        let text = String::from_utf8(gunzip(Path::new(log)).unwrap()).unwrap();
        assert!(!text.is_empty(), "{}", log);
    }
    // Nothing called log_debug, so the debug log is an empty gzip file.
    assert!(gunzip(Path::new(&debug)).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule 15: `--compress` gzips every file in the run's folder and leaves a
/// path as it is named; `--compress LEVEL` sets the level, also for a
/// path named `.gz`.
#[tokio::test]
#[serial]
async fn compress_gzips_the_run_folder() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);
    let dir = temp_dir("compress");
    let folder = dir.join("run").to_string_lossy().into_owned();
    let plain = dir.join("plain.html").to_string_lossy().into_owned();
    let named = dir.join("named.json.gz").to_string_lossy().into_owned();
    let named_log = dir.join("errors.csv").to_string_lossy().into_owned();
    let metrics = run(
        &server,
        vec![
            "--output-dir",
            &folder,
            "--report-file",
            "--report-file",
            &plain,
            "--report-file",
            &named,
            "-R",
            "--request-format",
            "csv",
            "-T",
            "-E",
            &named_log,
            "--error-format",
            "csv",
            "--compress",
            "9",
            "--no-reset-metrics",
        ],
    )
    .await;
    let folder = dir.join("run");
    assert_eq!(
        names(&folder),
        vec![
            "report.html.gz",
            "report.json.gz",
            "requests.csv.gz",
            "transactions.json.gz",
        ]
    );
    for name in names(&folder) {
        assert_eq!(xfl(&folder.join(&name)), 2, "{}", name);
        gunzip(&folder.join(&name)).unwrap();
    }
    assert_eq!(xfl(Path::new(&named)), 2);
    let html = std::fs::read_to_string(&plain).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"), "a path keeps its name");
    // A log path keeps its name too, and is written plain.
    let errors = std::fs::read(&named_log).unwrap();
    assert!(errors.starts_with(b"elapsed"), "plain CSV");
    assert!(!dir.join("errors.csv.gz").exists());
    let requests_csv = String::from_utf8(gunzip(&folder.join("requests.csv.gz")).unwrap()).unwrap();
    assert_eq!(requests_csv.lines().count(), requests(&metrics) + 1);

    // A bare --compress is level 6, and level 1 is the fastest.
    for (level, expected) in [(None, 0), (Some("1"), 4)] {
        let folder = dir.join(format!("level-{expected}"));
        let folder_str = folder.to_string_lossy().into_owned();
        let mut args = vec![
            "--output-dir",
            folder_str.as_str(),
            "--report-file",
            "--compress",
        ];
        args.extend(level);
        run(&server, args).await;
        assert_eq!(xfl(&folder.join("report.json.gz")), expected);
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// Levels outside 1 to 9 are refused, naming the flag, from the command line
/// and from code.
#[tokio::test]
#[serial]
async fn compress_levels_outside_one_to_nine_are_refused() {
    for level in ["0", "10"] {
        match GooseConfiguration::parse_args_default(&["--compress", level]) {
            Err(GooseError::InvalidOption { option, .. }) => {
                assert!(option.starts_with("--compress"), "{}", option)
            }
            other => panic!("{}: {:?}", level, other.map(|_| ())),
        }
    }
    let server = MockServer::start();
    let configuration = common::build_configuration(&server, vec![]);
    for level in [0, 10] {
        let result = common::build_load_test(
            configuration.clone(),
            vec![scenario(transaction!(get_index))],
            None,
            None,
        )
        .set_default(GooseDefault::Compress, level)
        .unwrap()
        .execute()
        .await;
        match result {
            Err(GooseError::InvalidOption { option, .. }) => {
                assert_eq!(option, "`configuration.compress`")
            }
            other => panic!("{}: {:?}", level, other.map(|_| ())),
        }
    }
}

/// Rule 14: a `.json.gz` report, and a folder that holds only
/// `report.json.gz`, load as a baseline and with `--render-report`; a
/// truncated or non gzip `.gz` is an error naming the flag, never a panic.
#[tokio::test]
#[serial]
async fn a_gz_report_is_read_wherever_a_json_report_is() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);
    let dir = temp_dir("read");
    let baseline = dir.join("baseline.json.gz").to_string_lossy().into_owned();
    let folder = dir.join("small").to_string_lossy().into_owned();
    run(&server, vec!["--report-file", &baseline]).await;
    run(
        &server,
        vec!["--output-dir", &folder, "--report-file", "--compress"],
    )
    .await;
    assert_eq!(
        names(&dir.join("small")),
        vec!["report.html.gz", "report.json.gz"]
    );

    for (from, name) in [(&baseline, "file"), (&folder, "folder")] {
        let md = dir
            .join(format!("against-{name}.md"))
            .to_string_lossy()
            .into_owned();
        run(&server, vec!["--baseline-file", from, "--report-file", &md]).await;
        let md = std::fs::read_to_string(&md).unwrap();
        assert!(md.contains("(+") || md.contains("(-"), "deltas: {}", md);

        let rendered = dir
            .join(format!("rendered-{name}.md"))
            .to_string_lossy()
            .into_owned();
        let configuration = GooseConfiguration::parse_args_default(&[
            "--render-report",
            &if name == "folder" {
                format!("{folder}/report.json.gz")
            } else {
                baseline.clone()
            },
            "--report-file",
            &rendered,
        ])
        .unwrap();
        GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario(transaction!(get_index)))
            .execute()
            .await
            .unwrap();
        assert!(std::fs::read_to_string(&rendered).unwrap().contains("GET"));
    }

    // Cut short, and not gzip at all.
    let whole = std::fs::read(&baseline).unwrap();
    let truncated = dir.join("truncated.json.gz");
    std::fs::write(&truncated, &whole[..whole.len() / 2]).unwrap();
    let not_gzip = dir.join("not-gzip.json.gz");
    std::fs::write(&not_gzip, b"{\"not\": \"gzip\"}").unwrap();
    for bad in [&truncated, &not_gzip] {
        let bad = bad.to_string_lossy().into_owned();
        let configuration = common::build_configuration(&server, vec!["--baseline-file", &bad]);
        match common::build_load_test(
            configuration,
            vec![scenario(transaction!(get_index))],
            None,
            None,
        )
        .execute()
        .await
        {
            Err(GooseError::InvalidOption { option, .. }) => {
                assert_eq!(option, "--baseline-file")
            }
            other => panic!("{}: {:?}", bad, other.map(|_| ())),
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A run that ends early leaves whole gzip logs, with their trailers.
#[tokio::test]
#[serial]
async fn a_run_stopped_early_leaves_whole_gz_logs() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);
    let dir = temp_dir("early");
    let folder = dir.join("run").to_string_lossy().into_owned();
    goose::reset_killswitch();
    run_with(
        &server,
        vec![
            "--output-dir",
            &folder,
            "-R",
            "-T",
            "--compress",
            "--run-time",
            "30",
        ],
        scenario(transaction!(get_index_then_cancel)),
    )
    .await;
    goose::reset_killswitch();
    let folder = dir.join("run");
    assert_eq!(
        names(&folder),
        vec!["requests.json.gz", "transactions.json.gz"]
    );
    for name in names(&folder) {
        let text = String::from_utf8(gunzip(&folder.join(&name)).unwrap()).unwrap();
        assert!(text.lines().count() >= 1, "{}", name);
    }
    std::fs::remove_dir_all(&dir).ok();
}
