//! A run folder of saved runs, for the tests of the saved run commands and
//! tools: `runs`, `report` and `compare` over telnet and WebSocket, and
//! `list_runs`, `get_run_report` and `compare_runs` over MCP.
//!
//! Three short load tests write real runs: `plain` (report.json with graph
//! data and report.html, no report.md, so Markdown is rendered), `zipped`
//! (gzipped reports, report.md.gz among them) and `stored` (a report.md of
//! its own). Copies of
//! `plain`'s JSON report, each moved to an earlier start, bring the folder to
//! 22 runs, so a listing takes two pages. A symlink to `plain` and a hidden
//! copy sit beside them and must never be listed.
#![allow(dead_code)]

use goose::config::GooseConfiguration;
use goose::prelude::*;
use httpmock::{Method::GET, MockServer};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// How many runs the folder holds.
pub const RUNS: usize = 22;

/// The runs a listing pages by.
pub const PAGE: usize = 20;

/// The key the JSON report holds its graph data and heatmap under.
pub const GRAPHS: &str = "graphs";

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

/// A run folder of saved runs.
pub struct SavedRuns {
    /// The folder the runs are in.
    pub parent: PathBuf,
    /// Every run's name, newest first.
    pub names: Vec<String>,
}

impl SavedRuns {
    /// The newest run's name.
    pub fn newest(&self) -> &str {
        &self.names[0]
    }

    /// The oldest run's name.
    pub fn oldest(&self) -> &str {
        self.names.last().unwrap()
    }

    /// The JSON report of run `name`, plain or gzipped, as stored.
    pub fn report_json(&self, name: &str) -> Value {
        let folder = self.parent.join(name);
        let text = match std::fs::read(folder.join("report.json")) {
            Ok(bytes) => String::from_utf8(bytes).unwrap(),
            Err(_) => gunzip(&std::fs::read(folder.join("report.json.gz")).unwrap()),
        };
        serde_json::from_str(&text).unwrap()
    }

    /// [`Self::report_json`] without the graph data a Controller leaves out.
    pub fn report_json_without_graphs(&self, name: &str) -> Value {
        let mut value = self.report_json(name);
        value.as_object_mut().unwrap().remove(GRAPHS);
        value
    }

    /// Write `value` as the JSON report of a new run `name`.
    pub fn save(&self, name: &str, value: &Value) -> PathBuf {
        save(&self.parent, name, value)
    }
}

/// `bytes`, gunzipped, as text.
pub fn gunzip(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .read_to_string(&mut text)
        .unwrap();
    text
}

/// Write `value` as the JSON report in `parent/name`.
pub fn save(parent: &Path, name: &str, value: &Value) -> PathBuf {
    let folder = parent.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("report.json"), value.to_string()).unwrap();
    folder
}

/// A new empty folder of its own for test `name`.
pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "goose-saved-runs-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run one short load test against `server`, its reports written into
/// `parent/name` with `args` added.
async fn run(server: &MockServer, parent: &Path, name: &str, args: &[&str]) {
    let output_dir = parent.join(name).to_string_lossy().into_owned();
    let host = server.base_url();
    let mut all = vec![
        "--host",
        &host,
        "--users",
        "2",
        "--increase-rate",
        "20",
        "--run-time",
        "1",
        "--co-mitigation",
        "disabled",
        "--quiet",
        "--no-telnet",
        "--output-dir",
        &output_dir,
        "--report-file",
    ];
    all.extend_from_slice(args);
    let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("SavedRuns").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .unwrap();
}

/// A run folder of [`RUNS`] saved runs in a new folder for test `name`.
pub async fn build(name: &str) -> SavedRuns {
    build_in(folder(name)).await
}

/// A run folder of [`RUNS`] saved runs in `parent`, emptied first.
pub async fn build_in(parent: PathBuf) -> SavedRuns {
    std::fs::remove_dir_all(&parent).ok();
    std::fs::create_dir_all(&parent).unwrap();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200).body("saved runs");
    });
    let zipped_markdown = parent.join("zipped").join("report.md.gz");
    let zipped_markdown = zipped_markdown.to_string_lossy().into_owned();
    run(
        &server,
        &parent,
        "zipped",
        &["--compress", "--report-file", &zipped_markdown],
    )
    .await;
    let markdown = parent.join("stored").join("report.md");
    let markdown = markdown.to_string_lossy().into_owned();
    run(&server, &parent, "stored", &["--report-file", &markdown]).await;
    run(&server, &parent, "plain", &["--report-graph-data"]).await;
    let plain = save_name(&parent, "plain");
    assert!(plain.get(GRAPHS).is_some(), "plain holds graph data");
    assert!(parent.join("stored").join("report.md").is_file());
    assert!(parent.join("zipped").join("report.json.gz").is_file());
    assert!(parent.join("zipped").join("report.md.gz").is_file());
    assert!(!parent.join("plain").join("report.md").exists());

    // Copies of plain, each started an hour before the last.
    let started = plain["started"].as_str().unwrap().to_string();
    let started = chrono::DateTime::parse_from_rfc3339(&started).unwrap();
    // The three real runs, newest first: by when each started, to the
    // second, then by name, as the listing orders them.
    let mut real: Vec<(i64, String)> = ["plain", "stored", "zipped"]
        .iter()
        .map(|name| {
            let value = SavedRuns {
                parent: parent.clone(),
                names: Vec::new(),
            }
            .report_json(name);
            let started = value["started"].as_str().unwrap().to_string();
            let started = chrono::DateTime::parse_from_rfc3339(&started).unwrap();
            (started.timestamp(), name.to_string())
        })
        .collect();
    real.sort_by(|a, b| b.cmp(a));
    let mut names: Vec<String> = real.into_iter().map(|(_, name)| name).collect();
    for copy in 0..RUNS - 3 {
        let mut value = plain.clone();
        let earlier = started - chrono::Duration::hours(copy as i64 + 1);
        let utc = |time: chrono::DateTime<chrono::FixedOffset>| {
            time.with_timezone(&chrono::Utc).to_rfc3339()
        };
        value["started"] = json!(utc(earlier));
        value["ended"] = json!(utc(earlier + chrono::Duration::seconds(2)));
        value["notes"] = json!(format!("copy {copy} of a real run"));
        let name = format!("copy-{copy:02}");
        save(&parent, &name, &value);
        names.push(name);
    }
    // Never listed: a hidden folder and, where there are symlinks, a link
    // to a real run.
    save(&parent, ".hidden", &plain);
    #[cfg(unix)]
    std::os::unix::fs::symlink(parent.join("plain"), parent.join("linked")).unwrap();
    SavedRuns { parent, names }
}

/// The JSON report of the run `name` in `parent`, plain only.
fn save_name(parent: &Path, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(parent.join(name).join("report.json")).unwrap()).unwrap()
}
