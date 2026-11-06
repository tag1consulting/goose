//! The saved run answers every Controller gives: `runs`, `report` and
//! `compare` over telnet and WebSocket, and the matching MCP tools.
//!
//! Each answer is read from the run folder exactly as the dashboard's runs
//! panel reads it, so the four front ends can not disagree: the listing is
//! [`SavedRuns::list`], the Markdown report is [`report_markdown`], and the
//! comparison is [`compare_markdown`]. A name is only ever a folder name
//! under [`SavedRuns::parent`]: [`report_folder`] refuses anything that is not
//! a valid name of a real folder holding a regular `report.json`, and every
//! refusal reads the same, naming no path. These functions read files and
//! parse JSON, so a Controller calls them off the runtime, never on the main
//! loop.

use super::{
    compare_markdown, report_folder, report_markdown, utc_text, EndedBy, ListedReport, SavedRuns,
};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

/// The most runs one listing holds.
pub(crate) const PAGE: usize = 20;

/// The largest JSON report, without its graph data, a Controller returns:
/// `report` returns the Markdown report of a larger one.
pub(crate) const MAX_REPORT_JSON: usize = 4 * 1024 * 1024;

/// An answer: the text a telnet or WebSocket client reads, which an MCP
/// tool returns as its text too, and the value an MCP tool returns as its
/// structured content. Each is serialized once from the same value; the
/// value is never the text parsed back.
#[derive(Debug)]
pub(crate) struct Answer {
    pub text: String,
    /// Read only by the MCP Controller.
    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub value: Value,
}

impl Answer {
    /// `text` with `value` as its structured form.
    fn text<T: Serialize>(text: String, value: &T) -> Answer {
        Answer {
            text,
            value: serde_json::to_value(value).unwrap_or(Value::Null),
        }
    }

    /// `value` as JSON text, the way `metrics-json` writes it, and as itself.
    fn json<T: Serialize>(value: &T) -> Answer {
        Answer {
            text: serde_json::to_string(value).unwrap_or_else(|e| e.to_string()),
            value: serde_json::to_value(value).unwrap_or(Value::Null),
        }
    }
}

/// One page of the saved runs, as `runs-json` returns it: the folder listed
/// and each run with the fields `GET /api/v1/reports` gives it.
#[derive(Debug, Serialize)]
pub(crate) struct RunsListing {
    /// The folder listed, as given, never made absolute.
    pub dir: String,
    /// Newest first.
    pub runs: Vec<ListedReport>,
    /// Whether older runs follow this page.
    pub more: bool,
}

/// The Markdown report of a saved run, as `get_run_report` structures it.
#[derive(Debug, Serialize)]
struct RunReport<'a> {
    run: &'a str,
    markdown: &'a str,
}

/// A saved run compared to a baseline run, as `compare_runs` structures it.
#[derive(Debug, Serialize)]
struct RunComparison<'a> {
    run: &'a str,
    baseline: &'a str,
    markdown: &'a str,
}

/// The one refusal for a name that is not a saved run, whatever the reason:
/// the name as sent, quoted and escaped, and at most 64 characters of it.
pub(crate) fn no_saved_run(name: &str) -> String {
    let shown: String = name.chars().take(64).collect();
    let more = if shown.len() < name.len() { "..." } else { "" };
    format!("no saved run named {shown:?}{more}")
}

/// The refusal when the folder holds no saved run at all.
pub(crate) const NO_SAVED_RUNS: &str = "no saved runs";

/// Every saved run, newest first.
fn listed(runs: &SavedRuns) -> Result<Vec<ListedReport>, String> {
    runs.list().map_err(|e| {
        warn!("[controller]: can't list the saved runs: {e}");
        "can't list the saved runs".to_string()
    })
}

/// The newest saved run's name.
fn newest(runs: &SavedRuns) -> Result<String, String> {
    listed(runs)?
        .into_iter()
        .next()
        .map(|report| report.name)
        .ok_or_else(|| NO_SAVED_RUNS.to_string())
}

/// The folder and the JSON report of the saved run `name`.
fn saved_run(runs: &SavedRuns, name: &str) -> Result<(PathBuf, PathBuf), String> {
    report_folder(&runs.parent, name).ok_or_else(|| no_saved_run(name))
}

/// `runs [before]` and `runs-json [before]`: the newest [`PAGE`] saved runs,
/// or the [`PAGE`] after the run `before`, newest first.
pub(crate) fn list(runs: &SavedRuns, before: Option<&str>, json: bool) -> Result<Answer, String> {
    let listed = listed(runs)?;
    let start = match before {
        None => 0,
        Some(name) => {
            listed
                .iter()
                .position(|report| report.name == name)
                .ok_or_else(|| no_saved_run(name))?
                + 1
        }
    };
    let mut page: Vec<ListedReport> = listed.into_iter().skip(start).take(PAGE + 1).collect();
    let more = page.len() > PAGE;
    page.truncate(PAGE);
    let listing = RunsListing {
        dir: runs.parent.to_string_lossy().into_owned(),
        runs: page,
        more,
    };
    Ok(match json {
        true => Answer::json(&listing),
        false => Answer::text(listing_text(&listing, before.is_some()), &listing),
    })
}

/// A listing as text: one line per run, then whether more follow.
fn listing_text(listing: &RunsListing, paged: bool) -> String {
    if listing.runs.is_empty() {
        return match paged {
            true => "no older saved runs".to_string(),
            false => format!("{NO_SAVED_RUNS} in {}", listing.dir),
        };
    }
    let mut lines = vec![format!("saved runs in {}, newest first:", listing.dir)];
    for run in &listing.runs {
        lines.push(run_line(run));
    }
    match (listing.more, listing.runs.last()) {
        (true, Some(last)) => lines.push(format!("more saved runs: runs {}", last.name)),
        _ => lines.push("no more saved runs".to_string()),
    }
    lines.join("\n")
}

/// One run's line: its name, when it started, how long it ran, its most
/// users, its requests and failures, and how it ended.
fn run_line(run: &ListedReport) -> String {
    let started = run
        .record
        .started
        .as_deref()
        .and_then(utc_text)
        .unwrap_or_else(|| "at an unknown time".to_string());
    let ended = if run.running {
        "running"
    } else {
        match run.record.ended_by {
            Some(EndedBy::Completed) => "completed",
            Some(EndedBy::Stopped) => "stopped",
            Some(EndedBy::Canceled) => "canceled",
            Some(EndedBy::UsersExited) => "users exited",
            Some(EndedBy::WorkersLost) => "workers lost",
            None if run.record.in_progress => "saved while running",
            None => "end unknown",
        }
    };
    format!(
        "{}  started {started}  {}s  {} max users  {} requests  {} failed  {ended}",
        run.name, run.duration_secs, run.max_users, run.requests, run.failed_requests
    )
}

/// `report [run]` and `report-json [run]`: the saved run's Markdown report,
/// as `GET /api/v1/reports/{run}/report.md` serves it, or its JSON report
/// without graph data; the newest saved run when no run is named.
pub(crate) fn report(runs: &SavedRuns, run: Option<&str>, json: bool) -> Result<Answer, String> {
    report_capped(runs, run, json, MAX_REPORT_JSON)
}

/// [`report`], with the JSON report refused over `cap` bytes: the seam the
/// tests lower the cap through.
pub(crate) fn report_capped(
    runs: &SavedRuns,
    run: Option<&str>,
    json: bool,
    cap: usize,
) -> Result<Answer, String> {
    let name = match run {
        Some(name) => name.to_string(),
        None => newest(runs)?,
    };
    let (folder, json_path) = saved_run(runs, &name)?;
    if json {
        return report_json(&json_path, &name, cap);
    }
    // A report that can't be rendered is named by its run, never its path.
    let markdown = report_markdown(&folder, &json_path).map_err(|reason| {
        reason.replace(
            &json_path.display().to_string(),
            &format!("the report of run {name}"),
        )
    })?;
    let markdown = String::from_utf8_lossy(&markdown).into_owned();
    let value = RunReport {
        run: &name,
        markdown: &markdown,
    };
    Ok(Answer::text(markdown.clone(), &value))
}

/// The JSON report at `path`, plain or gzipped, without the graph data and
/// heatmap it holds under [`crate::metrics::REPORT_GRAPHS`], refused when
/// what is left is over `cap` bytes.
fn report_json(path: &std::path::Path, name: &str, cap: usize) -> Result<Answer, String> {
    let text = super::read_stored_text(path)
        .map_err(|e| format!("Can't read the report of run {name}: {e}"))?;
    let mut value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("The report of run {name} is not JSON: {e}"))?;
    drop(text);
    if let Value::Object(fields) = &mut value {
        fields.remove(crate::metrics::REPORT_GRAPHS);
    }
    let text = serde_json::to_string(&value).unwrap_or_else(|e| e.to_string());
    if text.len() > cap {
        return Err(format!(
            "the JSON report of run {name} is {} without its graph data, over the {} a Controller returns: use report for its Markdown report",
            size_text(text.len()),
            size_text(cap)
        ));
    }
    Ok(Answer { text, value })
}

/// A size in bytes as words: `5.2 MiB`, `12.0 KiB` or `100 bytes`.
fn size_text(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let size = bytes as f64;
    if size >= KIB * KIB {
        format!("{:.1} MiB", size / (KIB * KIB))
    } else if size >= KIB {
        format!("{:.1} KiB", size / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// `compare [run] baseline`: the saved run compared to the saved run
/// `baseline`, as `GET /api/v1/reports/{run}/compare.md?baseline={baseline}`
/// serves it; the newest saved run when no run is named.
pub(crate) fn compare(
    runs: &SavedRuns,
    run: Option<&str>,
    baseline: &str,
) -> Result<Answer, String> {
    let name = match run {
        Some(name) => name.to_string(),
        None => newest(runs)?,
    };
    let (_, run_json) = saved_run(runs, &name)?;
    let (_, baseline_json) = saved_run(runs, baseline)?;
    let markdown = String::from_utf8_lossy(&compare_markdown(
        &run_json,
        &name,
        &baseline_json,
        baseline,
    )?)
    .into_owned();
    let value = RunComparison {
        run: &name,
        baseline,
        markdown: &markdown,
    };
    Ok(Answer::text(markdown.clone(), &value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{RunningFolder, REPORT_JSON, REPORT_MD};
    use serde_json::json;
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// A real JSON report, as Goose 0.20 writes it.
    const FIXTURE: &str = include_str!("../../tests/fixtures/report-0.20.json");

    /// An empty folder of its own for each test.
    fn parent(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "goose-answers-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The saved runs in `parent`.
    fn saved(parent: &Path) -> SavedRuns {
        SavedRuns {
            parent: parent.to_path_buf(),
            running: RunningFolder::default(),
            cache: Arc::new(Mutex::new(Default::default())),
            readers: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    /// The fixture as the report of a run started `minute` minutes past 10.
    fn fixture(minute: u32, ended_by: &str, in_progress: bool) -> Value {
        let mut value: Value = serde_json::from_str(FIXTURE).unwrap();
        value["started"] = json!(format!("2026-11-06T10:{minute:02}:00Z"));
        value["ended"] = json!(format!("2026-11-06T10:{minute:02}:30Z"));
        value["ended_by"] = if ended_by.is_empty() {
            Value::Null
        } else {
            json!(ended_by)
        };
        value["in_progress"] = json!(in_progress);
        value
    }

    /// Save `value` as the JSON report in `parent/name`.
    fn save(parent: &Path, name: &str, value: &Value) -> PathBuf {
        let folder = parent.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(REPORT_JSON), value.to_string()).unwrap();
        folder
    }

    /// 23 runs, so the listing takes two pages: `run-00` the oldest.
    fn many(parent: &Path) {
        for minute in 0..23 {
            save(
                parent,
                &format!("run-{minute:02}"),
                &fixture(minute, "completed", false),
            );
        }
    }

    /// The newest 20, then the rest after the last of them, with whether
    /// more follow, as text and as JSON; a `before` not listed is refused.
    #[test]
    fn runs_are_listed_newest_first_in_pages() {
        let parent = parent("list");
        many(&parent);
        let runs = saved(&parent);
        let first = list(&runs, None, true).unwrap();
        let names = |value: &Value| -> Vec<String> {
            value["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|run| run["name"].as_str().unwrap().to_string())
                .collect()
        };
        let expected: Vec<String> = (3..23).rev().map(|m| format!("run-{m:02}")).collect();
        assert_eq!(names(&first.value), expected);
        assert_eq!(first.value["more"], json!(true));
        assert_eq!(first.value["dir"], json!(parent.to_string_lossy()));
        // The text of the JSON form is the value, serialized once.
        assert_eq!(
            serde_json::from_str::<Value>(&first.text).unwrap(),
            first.value
        );
        let second = list(&runs, Some("run-03"), true).unwrap();
        assert_eq!(names(&second.value), ["run-02", "run-01", "run-00"]);
        assert_eq!(second.value["more"], json!(false));
        // Each run has the fields the runs panel lists.
        let run = &second.value["runs"][0];
        for field in [
            "name",
            "started",
            "ended",
            "ended_by",
            "in_progress",
            "running",
            "duration_secs",
            "max_users",
            "requests",
            "failed_requests",
            "goose_version",
            "test",
        ] {
            assert!(run.get(field).is_some(), "{}: {}", field, run);
        }
        // As text: a heading, one line per run, then whether more follow.
        let text = list(&runs, None, false).unwrap().text;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 22, "{}", text);
        assert!(lines[0].starts_with("saved runs in "), "{}", text);
        assert_eq!(
            lines[1],
            "run-22  started 2026-11-06 10:22:00 UTC  60s  0 max users  17 requests  0 failed  completed"
        );
        assert_eq!(lines[21], "more saved runs: runs run-03");
        let text = list(&runs, Some("run-03"), false).unwrap().text;
        assert_eq!(text.lines().last(), Some("no more saved runs"));
        assert_eq!(
            list(&runs, Some("run-00"), false).unwrap().text,
            "no older saved runs"
        );
        assert_eq!(
            list(&runs, Some("run-99"), false).unwrap_err(),
            "no saved run named \"run-99\""
        );
        std::fs::remove_dir_all(&parent).ok();
    }

    /// The run in progress is listed as running, a report saved while a run
    /// that is no longer in progress ran says so, and an empty folder lists
    /// no runs.
    #[test]
    fn running_and_empty() {
        let parent = parent("running");
        let runs = saved(&parent);
        assert_eq!(
            list(&runs, None, false).unwrap().text,
            format!("no saved runs in {}", parent.to_string_lossy())
        );
        assert_eq!(report(&runs, None, false).unwrap_err(), "no saved runs");
        assert_eq!(compare(&runs, None, "run-00").unwrap_err(), "no saved runs");
        let folder = save(&parent, "now", &fixture(5, "", true));
        save(&parent, "earlier", &fixture(1, "", true));
        runs.running.set(Some(&folder));
        let text = list(&runs, None, false).unwrap().text;
        assert!(
            text.contains("\nnow  started 2026-11-06 10:05:00 UTC"),
            "{}",
            text
        );
        assert!(text.contains("  running\n"), "{}", text);
        assert!(text.contains("  saved while running\n"), "{}", text);
        let listed = list(&runs, None, true).unwrap().value;
        assert_eq!(listed["runs"][0]["running"], json!(true));
        assert_eq!(listed["runs"][1]["running"], json!(false));
        std::fs::remove_dir_all(&parent).ok();
    }

    /// Anything that is not the name of a real folder holding a regular
    /// `report.json` is refused with one message that names no path.
    #[test]
    fn names_that_are_not_saved_runs_are_refused_alike() {
        let parent = parent("names");
        save(&parent, "real", &fixture(1, "completed", false));
        save(&parent, ".hidden", &fixture(2, "completed", false));
        save(&parent, "a b", &fixture(3, "completed", false));
        std::fs::create_dir_all(parent.join("empty")).unwrap();
        // A folder whose report.json is a folder, not a file.
        std::fs::create_dir_all(parent.join("odd").join(REPORT_JSON)).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(parent.join("real"), parent.join("linked")).unwrap();
            let target = save(&parent, "target", &fixture(4, "completed", false));
            std::fs::create_dir_all(parent.join("filelink")).unwrap();
            std::os::unix::fs::symlink(
                target.join(REPORT_JSON),
                parent.join("filelink").join(REPORT_JSON),
            )
            .unwrap();
        }
        let runs = saved(&parent);
        let path = parent.to_string_lossy().into_owned();
        for name in [
            "/", "..", "../real", "real/..", ".hidden", "a b", "empty", "odd", "linked",
            "filelink", "missing", "/etc", "real\n",
        ] {
            let expected = no_saved_run(name);
            for refused in [
                report(&runs, Some(name), false).unwrap_err(),
                report(&runs, Some(name), true).unwrap_err(),
                compare(&runs, Some(name), "real").unwrap_err(),
                compare(&runs, Some("real"), name).unwrap_err(),
                list(&runs, Some(name), false).unwrap_err(),
            ] {
                assert_eq!(refused, expected, "{:?}", name);
                assert!(!refused.contains(&path), "{}", refused);
            }
        }
        // Only the real folder is listed: never a symlink, a hidden name, or
        // a folder without a regular report.
        let listed = list(&runs, None, true).unwrap().value;
        let names: Vec<&str> = listed["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["name"].as_str().unwrap())
            .collect();
        let expected: &[&str] = if cfg!(unix) {
            &["target", "real"]
        } else {
            &["real"]
        };
        assert_eq!(names, expected);
        assert_eq!(
            no_saved_run(&"x".repeat(100)),
            format!("no saved run named \"{}\"...", "x".repeat(64))
        );
        std::fs::remove_dir_all(&parent).ok();
    }

    /// The Markdown report is the folder's `report.md`, plain or gzipped,
    /// else rendered from its JSON report, as the dashboard serves it; with
    /// no name, the newest run's.
    #[test]
    fn markdown_report_is_the_dashboard_s() {
        let parent = parent("markdown");
        let stored = save(&parent, "stored", &fixture(1, "completed", false));
        std::fs::write(stored.join(REPORT_MD), "# stored report\n").unwrap();
        let zipped = save(&parent, "zipped", &fixture(2, "completed", false));
        let mut gz = flate2::write::GzEncoder::new(
            std::fs::File::create(zipped.join(format!("{REPORT_MD}.gz"))).unwrap(),
            flate2::Compression::default(),
        );
        gz.write_all(b"# zipped report\n").unwrap();
        gz.finish().unwrap();
        let rendered = save(&parent, "rendered", &fixture(3, "completed", false));
        let runs = saved(&parent);
        let answer = report(&runs, Some("stored"), false).unwrap();
        assert_eq!(answer.text, "# stored report\n");
        assert_eq!(
            answer.value,
            json!({ "run": "stored", "markdown": "# stored report\n" })
        );
        assert_eq!(
            report(&runs, Some("zipped"), false).unwrap().text,
            "# zipped report\n"
        );
        let expected = crate::runs::render_report(&rendered.join(REPORT_JSON), "md").unwrap();
        assert_eq!(
            report(&runs, Some("rendered"), false)
                .unwrap()
                .text
                .as_bytes(),
            expected
        );
        // A report that can't be rendered is refused naming its run, not its
        // path.
        save(
            &parent,
            "broken",
            &json!({ "raw_metrics": 1, "goose_version": "x" }),
        );
        let refused = report(&runs, Some("broken"), false).unwrap_err();
        assert!(refused.contains("the report of run broken"), "{}", refused);
        assert!(!refused.contains(&*parent.to_string_lossy()), "{}", refused);
        std::fs::remove_dir_all(parent.join("broken")).unwrap();
        // The newest run is `rendered`.
        assert_eq!(
            report(&runs, None, false).unwrap().text.as_bytes(),
            expected
        );
        std::fs::remove_dir_all(&parent).ok();
    }

    /// The JSON report, plain or gzipped, without exactly the graph data and
    /// heatmap; one over the cap without them is refused, naming its size.
    #[test]
    fn json_report_leaves_out_the_graphs() {
        let parent = parent("json");
        // Graph data under the key `report-json` leaves out.
        let mut full = fixture(1, "completed", false);
        full[crate::metrics::REPORT_GRAPHS] = json!({ "users_per_second": [1, 2, 3] });
        save(&parent, "plain", &full);
        let zipped = parent.join("zipped");
        std::fs::create_dir_all(&zipped).unwrap();
        let mut gz = flate2::write::GzEncoder::new(
            std::fs::File::create(zipped.join(format!("{REPORT_JSON}.gz"))).unwrap(),
            flate2::Compression::default(),
        );
        gz.write_all(full.to_string().as_bytes()).unwrap();
        gz.finish().unwrap();
        let runs = saved(&parent);
        let mut expected = full.clone();
        expected
            .as_object_mut()
            .unwrap()
            .remove(crate::metrics::REPORT_GRAPHS);
        for name in ["plain", "zipped"] {
            let answer = report(&runs, Some(name), true).unwrap();
            assert_eq!(answer.value, expected, "{}", name);
            assert_eq!(
                serde_json::from_str::<Value>(&answer.text).unwrap(),
                expected
            );
            assert!(answer.value.get("raw_metrics").is_some());
            assert!(answer.value.get(crate::metrics::REPORT_GRAPHS).is_none());
        }
        // At the cap it is returned; one byte under, refused.
        let size = report(&runs, Some("plain"), true).unwrap().text.len();
        assert!(report_capped(&runs, Some("plain"), true, size).is_ok());
        let refused = report_capped(&runs, Some("plain"), true, size - 1).unwrap_err();
        assert_eq!(
            refused,
            format!(
                "the JSON report of run plain is {} without its graph data, over the {} a Controller returns: use report for its Markdown report",
                size_text(size),
                size_text(size - 1)
            )
        );
        assert_eq!(MAX_REPORT_JSON, 4 * 1024 * 1024);
        assert_eq!(size_text(MAX_REPORT_JSON), "4.0 MiB");
        assert_eq!(size_text(5 * 1024 * 1024 + 200 * 1024), "5.2 MiB");
        assert_eq!(size_text(100), "100 bytes");
        std::fs::remove_dir_all(&parent).ok();
    }

    /// A stored report is read only through a handle that is the regular
    /// file at its path: a symlink swapped in after the path was checked is
    /// refused, never followed.
    #[cfg(unix)]
    #[test]
    fn a_stored_report_is_never_read_through_a_symlink() {
        let parent = parent("swap");
        let secret = parent.join("secret.txt");
        std::fs::write(&secret, "not a report").unwrap();
        let folder = save(&parent, "run", &fixture(1, "completed", false));
        let markdown = folder.join(REPORT_MD);
        std::fs::write(&markdown, "# report\n").unwrap();
        assert_eq!(crate::runs::read_stored(&markdown).unwrap(), b"# report\n");
        // The path checked, then a symlink swapped in before it is read.
        std::fs::remove_file(&markdown).unwrap();
        std::os::unix::fs::symlink(&secret, &markdown).unwrap();
        assert!(crate::runs::open_regular(&markdown).is_err());
        assert!(crate::runs::read_stored(&markdown).is_err());
        // A symlink to a valid report is refused too, by rendering, the
        // listing and a comparison, so it is the check that refuses it.
        let json = folder.join(REPORT_JSON);
        let real = folder.join("real.json");
        std::fs::rename(&json, &real).unwrap();
        assert!(crate::runs::render_report(&real, "md").is_ok());
        assert!(crate::runs::read_record(&real).is_some());
        std::os::unix::fs::symlink(&real, &json).unwrap();
        assert!(crate::runs::read_stored_text(&json).is_err());
        assert!(crate::runs::render_report(&json, "md").is_err());
        assert!(crate::runs::read_record(&json).is_none());
        assert!(crate::metrics::compare_reports(&json, "a", &real, "b").is_err());
        assert!(crate::metrics::compare_reports(&real, "a", &json, "b").is_err());
        std::fs::remove_dir_all(&parent).ok();
    }

    /// A comparison is the dashboard's, first line included; with one name,
    /// the newest run is compared with it.
    #[test]
    fn comparison_is_the_dashboard_s() {
        let parent = parent("compare");
        let older = save(&parent, "older", &fixture(1, "completed", false));
        let newer = save(&parent, "newer", &fixture(2, "stopped", false));
        let runs = saved(&parent);
        let expected = crate::runs::compare_markdown(
            &newer.join(REPORT_JSON),
            "newer",
            &older.join(REPORT_JSON),
            "older",
        )
        .unwrap();
        let answer = compare(&runs, Some("newer"), "older").unwrap();
        assert_eq!(answer.text.as_bytes(), expected);
        assert!(answer.text.starts_with(
            "Run newer (started 2026-11-06 10:02:00 UTC) compared to baseline older (started 2026-11-06 10:01:00 UTC).\n\n"
        ));
        assert_eq!(answer.value["run"], json!("newer"));
        assert_eq!(answer.value["baseline"], json!("older"));
        assert_eq!(answer.value["markdown"], json!(answer.text));
        assert_eq!(compare(&runs, None, "older").unwrap().text, answer.text);
        std::fs::remove_dir_all(&parent).ok();
    }
}
