//! The files a run writes: output flags given a path write that file, given
//! no path they write into the run's folder, and a run writes nothing it was
//! not asked for. Each test writes into its own temporary folder, never into
//! the checkout.

use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::path::{Path, PathBuf};

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

const INDEX_PATH: &str = "/";

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

fn setup_mock_server_endpoints(server: &MockServer) -> Mock<'_> {
    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200).body("ok");
    })
}

/// A new empty folder for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "goose-output-files-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The names of the entries in a folder, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn scenario() -> Scenario {
    scenario!("LoadTest").register_transaction(transaction!(get_index))
}

/// Run one load test with `args` against `server`.
async fn run(server: &MockServer, args: Vec<&str>) -> GooseMetrics {
    let configuration = common::build_configuration(server, args);
    common::run_load_test(
        common::build_load_test(configuration, vec![scenario()], None, None),
        None,
    )
    .await
}

/// The error a configuration gets before any load.
async fn refused(server: &MockServer, args: Vec<&str>) -> (String, String) {
    let configuration = common::build_configuration(server, args);
    match common::build_load_test(configuration, vec![scenario()], None, None)
        .execute()
        .await
    {
        Err(GooseError::InvalidOption { option, detail, .. }) => (option, detail),
        other => panic!("expected an invalid option, got {:?}", other.map(|_| ())),
    }
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Rule 2: an output flag given a path writes exactly that file.
#[tokio::test]
#[serial]
async fn a_path_writes_exactly_that_file() {
    let server = MockServer::start();
    let mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("path");
    let results = dir.join("results.html");
    run(&server, vec!["--report-file", &results.to_string_lossy()]).await;
    assert!(mock.calls() > 0);
    assert_eq!(names(&dir), vec!["results.html"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule 3: a bare `--report-file` writes report.json and report.html, and
/// bare logs their names with their format's extension, into the run's
/// folder.
#[tokio::test]
#[serial]
async fn bare_flags_write_into_the_run_folder() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("bare");
    let folder = dir.join("run");
    run(
        &server,
        vec![
            "--output-dir",
            &folder.to_string_lossy(),
            "--report-file",
            "-R",
            "--request-format",
            "csv",
            "-T",
            "-S",
            "--scenario-format",
            "raw",
            "-E",
            "-D",
            "--debug-format",
            "pretty",
        ],
    )
    .await;
    assert_eq!(names(&dir), vec!["run"]);
    assert_eq!(
        names(&folder),
        vec![
            "debug.log",
            "errors.json",
            "report.html",
            "report.json",
            "requests.csv",
            "scenarios.log",
            "transactions.json",
        ]
    );
    let requests = std::fs::read_to_string(folder.join("requests.csv")).unwrap();
    assert!(requests.lines().count() > 1, "{}", requests);
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule 4: a run never writes into a folder that holds files when it
/// starts; it takes the first of DIR-2, DIR-3 and so on that is missing or
/// empty, counted on DIR.
#[tokio::test]
#[serial]
async fn each_run_gets_its_own_folder() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("suffix");
    let nightly = dir.join("nightly").to_string_lossy().into_owned();
    for _ in 0..4 {
        run(&server, vec!["--output-dir", &nightly, "--report-file"]).await;
    }
    assert_eq!(
        names(&dir),
        vec!["nightly", "nightly-2", "nightly-3", "nightly-4"]
    );
    for folder in names(&dir) {
        assert_eq!(
            names(&dir.join(&folder)),
            vec!["report.html", "report.json"]
        );
    }

    // A folder holding only logs holds files: the report goes elsewhere.
    let logs = dir.join("logs").to_string_lossy().into_owned();
    run(&server, vec!["--output-dir", &logs, "-R"]).await;
    run(&server, vec!["--output-dir", &logs, "--report-file"]).await;
    assert_eq!(names(&dir.join("logs")), vec!["requests.json"]);
    assert_eq!(
        names(&dir.join("logs-2")),
        vec!["report.html", "report.json"]
    );

    // An empty folder is used as it is.
    std::fs::create_dir(dir.join("empty")).unwrap();
    let empty = dir.join("empty").to_string_lossy().into_owned();
    run(&server, vec!["--output-dir", &empty, "--report-file"]).await;
    assert_eq!(
        names(&dir.join("empty")),
        vec!["report.html", "report.json"]
    );
    assert!(!dir.join("empty-2").exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule 4: a folder Goose can't write to is an error before any load,
/// naming the folder.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn an_unwritable_folder_stops_the_run_before_load() {
    use std::os::unix::fs::PermissionsExt;
    let server = MockServer::start();
    let mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("unwritable");
    let locked = dir.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(locked.join("probe"), "").is_ok() {
        eprintln!("skipping: running as root, permissions are not enforced");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        return;
    }
    let folder = locked.join("run").to_string_lossy().into_owned();
    let (option, detail) = refused(&server, vec!["--output-dir", &folder, "--report-file"]).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(option, "--output-dir");
    assert!(detail.contains(&folder), "{}", detail);
    assert_eq!(mock.calls(), 0, "no request was sent");
    assert!(names(&locked).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule 6: notes are stored in the JSON report and shown at the top of the
/// HTML report, escaped, and the Markdown report.
#[tokio::test]
#[serial]
async fn notes_are_in_every_report() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("notes");
    let notes = "before the <cache> change & after";
    let html = dir.join("r.html").to_string_lossy().into_owned();
    let json = dir.join("r.json").to_string_lossy().into_owned();
    let md = dir.join("r.md").to_string_lossy().into_owned();
    run(
        &server,
        vec![
            "--notes",
            notes,
            "--report-file",
            &html,
            "--report-file",
            &json,
            "--report-file",
            &md,
        ],
    )
    .await;
    let report = read_json(Path::new(&json));
    assert_eq!(report["notes"], notes);
    assert_eq!(report["in_progress"], false);
    assert_eq!(report["ended_by"], "completed");
    assert!(report["started"].as_str().unwrap().ends_with('Z'));
    assert!(report["ended"].as_str().unwrap().ends_with('Z'));
    assert_eq!(report["goose_version"], env!("CARGO_PKG_VERSION"));
    assert!(report.get("graphs").is_none(), "no graph data unless asked");
    assert!(report.get("graphs_start").is_none());
    let html = std::fs::read_to_string(&html).unwrap();
    assert!(
        html.contains("before the &lt;cache&gt; change &amp; after"),
        "escaped notes"
    );
    assert!(!html.contains(notes));
    let md = std::fs::read_to_string(&md).unwrap();
    let after_title = md.split("# Goose Attack Report").nth(1).unwrap();
    assert!(after_title.trim_start().starts_with(notes), "{}", md);
    std::fs::remove_dir_all(&dir).ok();
}

/// Rules 7 and 8: graph data is in the JSON report only when asked for, and
/// a report rendered from it has its graphs; one rendered from a JSON report
/// without them says the graphs were not recorded.
#[tokio::test]
#[serial]
async fn render_report_with_and_without_graph_data() {
    let server = MockServer::start();
    let _mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("render");
    let with = dir.join("with.json").to_string_lossy().into_owned();
    let without = dir.join("without.json").to_string_lossy().into_owned();
    let live_html = dir.join("live.html").to_string_lossy().into_owned();
    run(
        &server,
        vec![
            "--report-graph-data",
            "--report-file",
            &with,
            "--report-file",
            &live_html,
            "--notes",
            "rendered",
        ],
    )
    .await;
    run(&server, vec!["--report-file", &without]).await;
    let graphs = &read_json(Path::new(&with))["graphs"];
    assert!(graphs["requests_per_second"]["GET /"]["data"].is_array());
    assert!(graphs["users_per_second"]["data"].is_array());
    assert!(read_json(Path::new(&without)).get("graphs").is_none());

    for (json, name) in [(&with, "with"), (&without, "without")] {
        let html = dir
            .join(format!("{name}.html"))
            .to_string_lossy()
            .into_owned();
        let md = dir
            .join(format!("{name}.md"))
            .to_string_lossy()
            .into_owned();
        let again = dir
            .join(format!("{name}-again.json"))
            .to_string_lossy()
            .into_owned();
        let configuration = GooseConfiguration::parse_args_default(&[
            "--render-report",
            json,
            "--report-file",
            &html,
            "--report-file",
            &md,
            "--report-file",
            &again,
        ])
        .unwrap();
        let metrics = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario())
            .execute()
            .await
            .unwrap();
        assert!(metrics.requests.values().any(|r| r.success_count > 0));
        let html = std::fs::read_to_string(&html).unwrap();
        let md = std::fs::read_to_string(&md).unwrap();
        assert!(md.contains("GET"), "{}", md);
        if name == "with" {
            assert!(html.contains("graph-rps"), "the graphs are drawn");
            assert!(html.contains("rendered"), "the notes");
            assert!(!html.contains("graphs were not recorded"));
            assert!(read_json(Path::new(&again))["graphs"].is_object());
        } else {
            assert!(!html.contains("id=\"graph-rps\""));
            assert!(html.contains("graphs were not recorded"));
        }
    }
    // Rendering ran no load test: one run each, and no more requests.
    assert_eq!(names(&dir).len(), 9);
    std::fs::remove_dir_all(&dir).ok();
}

/// `--render-report` needs a `--report-file` path and a Goose JSON report.
#[tokio::test]
#[serial]
async fn render_report_errors() {
    let dir = temp_dir("render-errors");
    let not_a_report = dir.join("other.json");
    std::fs::write(&not_a_report, r#"{"hello": "world"}"#).unwrap();
    let not_a_report = not_a_report.to_string_lossy().into_owned();
    let out = dir.join("out.html").to_string_lossy().into_owned();
    for (args, option) in [
        (
            vec!["--render-report", &not_a_report],
            "`configuration.render_report`",
        ),
        (
            vec!["--render-report", &not_a_report, "--report-file"],
            "`configuration.render_report`",
        ),
        (
            vec!["--render-report", &not_a_report, "--report-file", &out],
            "--render-report",
        ),
    ] {
        let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
        let result = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario())
            .execute()
            .await;
        match result {
            Err(GooseError::InvalidOption { option: got, .. }) => assert_eq!(got, option),
            other => panic!("{:?}: {:?}", args, other.map(|_| ())),
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Each new flag that would write nowhere is refused, naming the fix.
#[tokio::test]
#[serial]
async fn flags_that_would_write_nowhere_are_refused() {
    let server = MockServer::start();
    let mock = setup_mock_server_endpoints(&server);
    let dir = temp_dir("refused")
        .join("run")
        .to_string_lossy()
        .into_owned();
    for (args, option, fix) in [
        (
            vec!["--notes", "lost"],
            "`configuration.notes`",
            "add --report-file or --dashboard",
        ),
        (
            vec!["--output-dir", dir.as_str()],
            "`configuration.output_dir`",
            "give an output flag without a path",
        ),
        (
            vec!["--output-dir", dir.as_str(), "--report-file", "x.html"],
            "`configuration.output_dir`",
            "give an output flag without a path",
        ),
        (
            vec!["--report-graph-data"],
            "`configuration.report_graph_data`",
            "add --report-file or --dashboard",
        ),
    ] {
        let (got, detail) = refused(&server, args.clone()).await;
        assert_eq!(got, option, "{:?}", args);
        assert!(detail.contains(fix), "{:?}: {}", args, detail);
    }
    assert_eq!(mock.calls(), 0);
}
