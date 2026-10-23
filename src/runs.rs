//! Run folders: where the output flags given without a path, and the dashboard's
//! Save report, write a run's files.
//!
//! A run's folder is `--output-dir`, or `goose-runs/<YYYYMMDD-HHMMSS>` from the
//! local time the run started. Goose never writes a run into a folder that
//! already holds files when the run starts: it takes `-2`, `-3` and so on,
//! always counted on the original name. The folder is created as the run
//! starts, holding a `.goose-run` file while the run lasts so no other run
//! takes it, and removed at the end of the run if the run wrote nothing.

use crate::config::{GooseConfiguration, RUN_FOLDER};
use crate::graph::GraphData;
use crate::logger::{move_goose_log, GooseLogFormat};
#[cfg(feature = "dashboard")]
use crate::metrics::dashboard_snapshot::SnapshotReport;
#[cfg(feature = "dashboard")]
use crate::metrics::MetricsCommand;
use crate::metrics::{load_baseline_file, write_folder_report, GooseMetrics, LoadedReport};
#[cfg(feature = "dashboard")]
use crate::{ControlOutcome, GooseAttackRunState};
use crate::{GooseAttack, GooseError};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
#[cfg(feature = "dashboard")]
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// The folder run folders go in when `--output-dir` is not given.
pub(crate) const DEFAULT_PARENT: &str = "goose-runs";

/// The HTML report in a run's folder.
pub(crate) const REPORT_HTML: &str = "report.html";
/// The JSON report in a run's folder, also read by `--baseline-file <folder>`.
pub(crate) const REPORT_JSON: &str = "report.json";
/// The Markdown report, which the dashboard renders from the JSON report.
#[cfg(feature = "dashboard")]
pub(crate) const REPORT_MD: &str = "report.md";

/// The file that holds a run's folder for its run, so no other process
/// takes it.
pub(crate) const RESERVED: &str = ".goose-run";

/// The Goose log in a run's folder.
pub(crate) const GOOSE_LOG: &str = "goose.log";
/// The names of the other logs in a run's folder, before their format's
/// extension.
pub(crate) const LOG_NAMES: [&str; 5] =
    ["requests", "transactions", "scenarios", "errors", "debug"];

/// How a run ended, as its JSON report records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndedBy {
    /// The test plan (or `--run-time`) ran to its end.
    Completed,
    /// A Stop from the dashboard or a Controller, or a Controller `shutdown`.
    Stopped,
    /// The killswitch: Ctrl-C, or code calling `trigger_killswitch`.
    Canceled,
    /// Every user exited on its own, for example after `--iterations`.
    UsersExited,
}

/// The record of a run that its JSON report holds beside its metrics. Every
/// field has a default, so a JSON report written before them still loads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    /// The run's `--notes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub goose_version: String,
    /// The load test's executable name.
    #[serde(default)]
    pub test: String,
    /// When the run started, RFC 3339, UTC.
    #[serde(default)]
    pub started: Option<String>,
    /// When the run ended, RFC 3339, UTC; `None` while it is in progress.
    #[serde(default)]
    pub ended: Option<String>,
    /// How the run ended; `None` while it is in progress.
    #[serde(default)]
    pub ended_by: Option<EndedBy>,
    /// Why the run was canceled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canceled_reason: Option<String>,
    /// Written by Save report now while the run continued; the report at the
    /// end of the run replaces it.
    #[serde(default)]
    pub in_progress: bool,
    /// When the graphs do not cover the whole run: where they start, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphs_start: Option<GraphsStart>,
}

/// Where a report's graphs start when they do not cover the whole run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GraphsStart {
    /// RFC 3339, UTC.
    pub time: String,
    pub reason: String,
}

impl GraphsStart {
    /// The sentence a report shows.
    pub(crate) fn describe(&self) -> String {
        let time = DateTime::parse_from_rfc3339(&self.time)
            .map(|time| {
                time.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|_| self.time.clone());
        format!(
            "The graphs start at {time}, not at the start of the run: {}",
            self.reason
        )
    }
}

impl RunRecord {
    /// The paragraphs the HTML and Markdown reports open with: the notes, and
    /// whether the report was written while the run continued.
    pub(crate) fn intro(&self) -> Vec<String> {
        let mut intro = Vec::new();
        if let Some(notes) = self.notes.as_deref().filter(|notes| !notes.is_empty()) {
            intro.push(notes.to_string());
        }
        if self.in_progress {
            intro.push(IN_PROGRESS.to_string());
        }
        intro
    }
}

/// What a report written by Save report now says.
pub(crate) const IN_PROGRESS: &str =
    "This report was saved while the run was in progress. The report written when the run ends replaces it.";

/// `name` without a trailing `.gz`, and whether it had one: a report or log
/// named `.gz` is written gzipped, in the format the rest of the name gives.
pub(crate) fn without_gz(name: &str) -> (&str, bool) {
    match name.strip_suffix(".gz") {
        Some(inner) => (inner, true),
        None => (name, false),
    }
}

/// The file name of a log in a run's folder: `name` with its format's
/// extension.
pub(crate) fn log_file_name(name: &str, format: Option<&GooseLogFormat>) -> String {
    let extension = match format {
        Some(GooseLogFormat::Csv) => "csv",
        Some(GooseLogFormat::Raw | GooseLogFormat::Pretty) => "log",
        Some(GooseLogFormat::Json) | None => "json",
    };
    format!("{name}.{extension}")
}

/// The folder run folders go in: the one `--output-dir` sits in, or
/// `goose-runs`.
#[cfg(any(test, feature = "dashboard"))]
pub(crate) fn parent_folder(configuration: &GooseConfiguration) -> PathBuf {
    match configuration.output_dir() {
        Some(dir) => match Path::new(dir).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        },
        None => PathBuf::from(DEFAULT_PARENT),
    }
}

/// A run's folder, chosen when the run starts.
#[derive(Clone, Debug)]
pub(crate) struct RunFolder {
    /// The folder, as given or as Goose named it, never made absolute.
    pub path: PathBuf,
}

impl RunFolder {
    /// The folder of a run started at `started`: `--output-dir`, or
    /// `goose-runs/<YYYYMMDD-HHMMSS>`, or the first of `-2`, `-3` and so on
    /// that Goose can take. The folder is created and holds [`RESERVED`]
    /// when this returns.
    pub(crate) fn choose(
        configuration: &GooseConfiguration,
        started: DateTime<Local>,
    ) -> io::Result<RunFolder> {
        Ok(RunFolder {
            path: reserve(&Self::base(configuration, started))?,
        })
    }

    /// The name a run started at `started` asks for, before any suffix.
    fn base(configuration: &GooseConfiguration, started: DateTime<Local>) -> PathBuf {
        match configuration.output_dir() {
            Some(dir) => PathBuf::from(dir),
            None => Path::new(DEFAULT_PARENT).join(started.format("%Y%m%d-%H%M%S").to_string()),
        }
    }

    /// `name` in this folder.
    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// The folder's name as the dashboard shows it.
    #[cfg(test)]
    pub(crate) fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The error for a folder Goose can't write to.
    pub(crate) fn error(&self, e: io::Error) -> GooseError {
        GooseError::InvalidOption {
            option: "--output-dir".to_string(),
            value: self.path.to_string_lossy().into_owned(),
            detail: format!("Can't write to {}: {e}", self.path.display()),
        }
    }

    /// Fail before any load when the folder can't be written to: the nearest
    /// folder that exists must be a folder Goose can create a file in. The
    /// file is removed at once.
    pub(crate) fn check_writable(&self) -> Result<(), GooseError> {
        let mut dir = self.path.as_path();
        loop {
            match std::fs::metadata(dir) {
                Ok(metadata) if metadata.is_dir() => break,
                Ok(_) => {
                    return Err(self.error(io::Error::other(format!(
                        "{} is not a folder",
                        dir.display()
                    ))))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    dir = match dir.parent() {
                        Some(parent) if !parent.as_os_str().is_empty() => parent,
                        _ => Path::new("."),
                    };
                }
                Err(e) => return Err(self.error(e)),
            }
        }
        let probe = dir.join(format!(".goose-write-check-{}", std::process::id()));
        // A probe left by an earlier process with the same id goes first.
        let _ = std::fs::remove_file(&probe);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .and_then(|_| std::fs::remove_file(&probe))
            .map_err(|e| self.error(e))
    }
}

/// Take `base`, or the first of `base-2`, `base-3` and so on, that is
/// missing or an empty folder: create it, then create [`RESERVED`] in it only
/// if no other process has, so two processes never take the same folder.
fn reserve(base: &Path) -> io::Result<PathBuf> {
    if let Some(parent) = base
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let name = base
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    for n in 1u32.. {
        let candidate = match (n, &name) {
            (1, _) | (_, None) => base.to_path_buf(),
            (n, Some(name)) => base.with_file_name(format!("{name}-{n}")),
        };
        let free = match std::fs::create_dir(&candidate) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => is_empty_folder(&candidate)?,
            Err(e) => return Err(e),
        };
        if free {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(candidate.join(RESERVED))
            {
                Ok(_) => return Ok(candidate),
                // Another process took it first.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        if name.is_none() {
            break;
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{} already holds files", base.display()),
    ))
}

/// True when `path` is a folder with nothing in it, never following a
/// symlink.
fn is_empty_folder(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(std::fs::read_dir(path)?.next().is_none()),
        Ok(_) => Ok(false),
        Err(e) => Err(e),
    }
}

/// What Goose writes for each run, kept on [`GooseAttack`].
#[derive(Debug, Default)]
pub(crate) struct Output {
    /// The run's folder, chosen when the run starts, when it may need one.
    pub folder: Option<RunFolder>,
    /// The folder this process holds [`RESERVED`] in, until its run ends.
    reserved: Option<PathBuf>,
    /// Save report: write `report.json` and `report.html` into the folder.
    pub save_report: bool,
    /// Include graph data in the JSON report.
    pub graph_data: bool,
    /// When the run started.
    pub started: Option<DateTime<Utc>>,
    /// The folder of the last report Save report wrote.
    pub last_report: Option<String>,
    /// Why the last report Save report tried to write failed.
    pub error: Option<String>,
    /// Save report now wrote a report during this run, which the report at
    /// its end replaces.
    pub saved_now: bool,
    /// The report Save report now is writing.
    pub saving_now: Option<tokio::task::JoinHandle<()>>,
}

impl Output {
    /// Give up the run's folder as its run ends: remove [`RESERVED`], then the
    /// folder if that leaves it empty. Only the first call does anything.
    pub(crate) fn release_folder(&mut self) {
        if let Some(folder) = self.reserved.take() {
            let _ = std::fs::remove_file(folder.join(RESERVED));
            let _ = std::fs::remove_dir(&folder);
        }
    }
}

impl Drop for Output {
    /// A run that ends with an error still gives up its folder.
    fn drop(&mut self) {
        self.release_folder();
    }
}

/// The seconds of graph data Goose keeps for the dashboard while no report
/// at the end of the run needs more: twice its chart window.
pub(crate) const DASHBOARD_GRAPH_SECS: usize =
    crate::metrics::dashboard_snapshot::SERIES_WINDOW_SECS as usize * 2;

impl GooseAttack {
    /// Record how the run ended. The first cause recorded wins.
    pub(crate) fn set_ended_by(&mut self, ended_by: EndedBy, reason: Option<String>) {
        if self.run_in_progress && self.ended_by.is_none() {
            self.ended_by = Some((ended_by, reason));
        }
    }

    /// Whether the graph data is kept for the whole run: a report will be
    /// written at its end, to a `--report-file` path or by Save report.
    pub(crate) fn keeps_full_graph(&self) -> bool {
        !self.configuration.report_paths().is_empty() || self.output.save_report
    }

    /// Whether the per second histograms also fold into whole run columns,
    /// for the report's response time heatmap.
    pub(crate) fn records_latency_columns(&self) -> bool {
        self.keeps_full_graph() && self.configuration.records_latency_series()
    }

    /// When a run starts: choose its folder when it may need one, failing
    /// before any load when it can't be written to.
    pub(crate) fn start_run_folder(&mut self) -> Result<(), GooseError> {
        self.output.release_folder();
        self.output.folder = None;
        self.output.last_report = None;
        self.output.error = None;
        self.output.saved_now = false;
        self.output.started = None;
        if !self.configuration.uses_run_folder() {
            return Ok(());
        }
        let folder = RunFolder::choose(&self.configuration, Local::now()).map_err(|e| {
            GooseError::InvalidOption {
                option: "--output-dir".to_string(),
                value: self
                    .configuration
                    .output_dir()
                    .unwrap_or(DEFAULT_PARENT)
                    .to_string(),
                detail: format!("Can't choose the run's folder: {e}"),
            }
        })?;
        self.output.reserved = Some(folder.path.clone());
        folder.check_writable()?;
        // A bare --goose-log moves to each run's folder as the run starts.
        if self.configuration.goose_log() == Some(RUN_FOLDER) {
            let name = match self.configuration.compress {
                Some(_) => format!("{GOOSE_LOG}.gz"),
                None => GOOSE_LOG.to_string(),
            };
            // Move the log first, so the line naming this run's folder lands
            // in that folder's log.
            move_goose_log(&folder.file(&name)).map_err(|e| folder.error(e))?;
        }
        info!("writing this run's files to {}", folder.path.display());
        self.output.folder = Some(folder);
        Ok(())
    }

    /// The record of a report drawn from `graph_data`, whose first second is
    /// where its graphs start.
    pub(crate) fn report_record_for(&self, in_progress: bool, graph_data: &GraphData) -> RunRecord {
        self.report_record(in_progress, graph_data.first_second())
    }

    /// The record of the run in its reports: written while it continues when
    /// `in_progress`, otherwise at its end. `graphs_from` is the first second
    /// of the run the graph data holds.
    pub(crate) fn report_record(&self, in_progress: bool, graphs_from: usize) -> RunRecord {
        let (ended_by, canceled_reason) = match (in_progress, &self.ended_by) {
            (true, _) => (None, None),
            (false, Some((ended_by, reason))) => (Some(*ended_by), reason.clone()),
            (false, None) => (Some(EndedBy::Completed), None),
        };
        RunRecord {
            notes: self.configuration.notes().map(str::to_string),
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            test: test_name(),
            started: self.output.started.map(rfc3339),
            ended: (!in_progress).then(|| rfc3339(Utc::now())),
            ended_by,
            canceled_reason,
            in_progress,
            graphs_start: self.graphs_start(graphs_from),
        }
    }

    /// Where the graphs start when they do not cover the whole run, as the
    /// dashboard keeps only its window while no report needs more.
    fn graphs_start(&self, graphs_from: usize) -> Option<GraphsStart> {
        let started = self.output.started.filter(|_| graphs_from > 0)?;
        let reason = if self.keeps_full_graph() {
            format!(
                "Save report was off for part of the run, and Goose then keeps only the dashboard's last {} minutes of graphs.",
                DASHBOARD_GRAPH_SECS / 60
            )
        } else {
            format!(
                "Save report is off, so Goose keeps only the dashboard's last {} minutes of graphs.",
                DASHBOARD_GRAPH_SECS / 60
            )
        };
        Some(GraphsStart {
            time: rfc3339(started + chrono::Duration::seconds(graphs_from as i64)),
            reason,
        })
    }

    /// When a run has fully stopped: write the `--report-file` reports and, with
    /// Save report on, the folder's, attempting both even if one fails.
    /// Returns the first error.
    pub(crate) async fn write_end_of_run(&mut self) -> Result<(), GooseError> {
        // A report Save report now is still writing finishes first, so this
        // one replaces it.
        if let Some(saving) = self.output.saving_now.take() {
            let _ = saving.await;
        }
        // Load the baseline once for all report writers. A baseline that no
        // longer loads fails the --report-file reports, as it does at
        // startup, and the folder's report is written without it.
        let (baseline, baseline_error) = match &self.configuration.baseline_file {
            Some(file) => match load_baseline_file(file) {
                Ok(baseline) => (Some(baseline), None),
                Err(e) => (None, Some(e)),
            },
            None => (None, None),
        };
        let record = self.report_record_for(false, &self.graph_data);
        let save = self.output.save_report || self.output.saved_now;
        let folder = match (save, self.output.folder.as_ref()) {
            (true, Some(folder)) => Some(folder.path.clone()),
            _ => None,
        };
        // The HTML report is generated once for --report-file and the folder.
        let mut html = None;
        let source = self.report_source(&record);
        let report_files = match baseline_error {
            Some(e) => Err(e),
            None => {
                self.write_report_files(&source, baseline.as_ref(), &mut html)
                    .await
            }
        };
        let written = match folder {
            None => return report_files,
            Some(folder) => write_folder_report(
                &folder,
                &source,
                baseline.as_ref(),
                &mut html,
                self.configuration.compress,
            )
            .map(|files| (folder, files)),
        };
        match written {
            Ok((folder, files)) => {
                for file in files {
                    info!("report file written to: {}", file.display());
                }
                self.output.last_report = Some(folder.to_string_lossy().into_owned());
                self.output.error = None;
                report_files
            }
            Err(e) => {
                error!("couldn't save the report: {e}");
                self.output.error = Some(e.to_string());
                report_files.and(Err(e))
            }
        }
    }
}

impl GooseAttack {
    /// `--render-report`: write the reports `--report-file` paths name from the
    /// JSON report at `path`, without running a load test.
    pub(crate) async fn render_report(&self, path: &str) -> Result<GooseMetrics, GooseError> {
        let loaded =
            LoadedReport::read(Path::new(path)).map_err(|detail| GooseError::InvalidOption {
                option: "--render-report".to_string(),
                value: path.to_string(),
                detail,
            })?;
        let baseline = match &self.configuration.baseline_file {
            Some(file) => Some(load_baseline_file(file)?),
            None => None,
        };
        let empty = GraphData::new();
        let source = loaded.source(&self.configuration, &empty);
        self.create_reports().await?;
        self.write_report_files(&source, baseline.as_ref(), &mut None)
            .await?;
        Ok(loaded.data.raw_metrics.into_owned())
    }

    /// The report this run writes, as the dashboard shows it.
    #[cfg(feature = "dashboard")]
    pub(crate) fn report_snapshot(&self) -> SnapshotReport {
        SnapshotReport {
            save: self.output.save_report,
            graph_data: self.output.graph_data,
            folder: self
                .output
                .folder
                .as_ref()
                .map(|folder| folder.path.to_string_lossy().into_owned()),
            parent: parent_folder(&self.configuration)
                .to_string_lossy()
                .into_owned(),
            paths: self.configuration.report_paths(),
            notes: self.configuration.notes().map(str::to_string),
            last_report: self.output.last_report.clone(),
            error: self.output.error.clone(),
        }
    }
}

/// One report the runs panel lists: a folder's name and what its
/// `report.json` says about its run.
#[cfg(feature = "dashboard")]
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ListedReport {
    /// The folder's name, which the report routes take.
    pub name: String,
    #[serde(flatten)]
    pub record: RunRecord,
    /// Still marked in progress, and the folder of the run in progress now.
    pub running: bool,
    pub duration_secs: u64,
    pub max_users: u64,
    pub requests: u64,
    pub failed_requests: u64,
    /// When the run started, else when the report was written: what the list
    /// is sorted by. Seconds since the epoch.
    #[serde(skip)]
    sort_key: i64,
}

/// What a listing remembers of each `report.json`, by path, so a file is
/// parsed again only when its size or modification time changes.
#[cfg(feature = "dashboard")]
pub(crate) type ListingCache = HashMap<PathBuf, (u64, std::time::SystemTime, ListedReport)>;

/// True for a folder name the report routes take: a letter or digit, then
/// letters, digits, `.`, `_` and `-`. Never `..`, never a path.
#[cfg(feature = "dashboard")]
pub(crate) fn is_report_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The real folder `name` in `parent` and the regular `report.json`, else
/// `report.json.gz`, in it, never following a symlink.
#[cfg(feature = "dashboard")]
pub(crate) fn report_folder(parent: &Path, name: &str) -> Option<(PathBuf, PathBuf)> {
    if !is_report_name(name) {
        return None;
    }
    let folder = parent.join(name);
    if !std::fs::symlink_metadata(&folder).is_ok_and(|m| m.file_type().is_dir()) {
        return None;
    }
    let json = regular_file(&folder, REPORT_JSON)?;
    Some((folder, json))
}

/// The regular file `name`, else `name.gz`, in `folder`, never following a
/// symlink.
#[cfg(feature = "dashboard")]
pub(crate) fn regular_file(folder: &Path, name: &str) -> Option<PathBuf> {
    [name.to_string(), format!("{name}.gz")]
        .iter()
        .map(|name| folder.join(name))
        .find(|path| std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file()))
}

/// The reports in the folders of `parent`, one level down, newest first.
/// `running` is the folder of the run in progress, if any. A folder that
/// does not exist lists nothing; a report that can't be read is left out.
#[cfg(feature = "dashboard")]
pub(crate) fn list_reports(
    parent: &Path,
    running: Option<&Path>,
    cache: &mut ListingCache,
) -> io::Result<Vec<ListedReport>> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            cache.clear();
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    };
    let mut listed = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries.filter_map(Result::ok) {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some((folder, json)) = report_folder(parent, &name) else {
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&json) else {
            continue;
        };
        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        seen.insert(json.clone());
        let cached = cache
            .get(&json)
            .filter(|(len, time, _)| *len == metadata.len() && *time == modified)
            .map(|(_, _, report)| report.clone());
        let mut report = match cached {
            Some(report) => report,
            None => match read_listed(&json, &name, modified) {
                Some(report) => {
                    cache.insert(json.clone(), (metadata.len(), modified, report.clone()));
                    report
                }
                None => continue,
            },
        };
        report.running = report.record.in_progress && running == Some(folder.as_path());
        listed.push(report);
    }
    cache.retain(|path, _| seen.contains(path));
    listed.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(listed)
}

/// The record of the run in the JSON report at `json`, plain or gzipped.
#[cfg(feature = "dashboard")]
pub(crate) fn read_record(json: &Path) -> Option<RunRecord> {
    let text = crate::metrics::read_report_text(json).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether the runs panel would list the report at `json`: Delete touches
/// nothing in a folder the panel does not show.
#[cfg(feature = "dashboard")]
pub(crate) fn is_listed(json: &Path) -> bool {
    read_listed(json, "", std::time::UNIX_EPOCH).is_some()
}

/// What the runs panel shows of the report at `json`, in the folder `name`.
#[cfg(feature = "dashboard")]
fn read_listed(json: &Path, name: &str, modified: std::time::SystemTime) -> Option<ListedReport> {
    #[derive(Deserialize)]
    struct Listed {
        raw_metrics: Metrics,
    }
    #[derive(Deserialize)]
    struct Metrics {
        #[serde(default)]
        duration: u64,
        #[serde(default)]
        maximum_users: u64,
        #[serde(default)]
        requests: HashMap<String, Requests>,
    }
    #[derive(Deserialize)]
    struct Requests {
        #[serde(default)]
        success_count: u64,
        #[serde(default)]
        fail_count: u64,
    }
    let text = crate::metrics::read_report_text(json).ok()?;
    let record: RunRecord = serde_json::from_str(&text).ok()?;
    let metrics = serde_json::from_str::<Listed>(&text).ok()?.raw_metrics;
    let written = DateTime::<Utc>::from(modified).timestamp();
    let sort_key = record
        .started
        .as_deref()
        .and_then(|started| DateTime::parse_from_rfc3339(started).ok())
        .map_or(written, |started| started.timestamp());
    Some(ListedReport {
        name: name.to_string(),
        record,
        running: false,
        duration_secs: metrics.duration,
        max_users: metrics.maximum_users,
        requests: metrics
            .requests
            .values()
            .map(|r| r.success_count + r.fail_count)
            .sum(),
        failed_requests: metrics.requests.values().map(|r| r.fail_count).sum(),
        sort_key,
    })
}

/// The files `DELETE /api/v1/reports/{name}` removes from a run's folder:
/// its reports and the logs Goose names there, plain and gzipped.
#[cfg(feature = "dashboard")]
pub(crate) fn deletable_files() -> Vec<String> {
    let mut names = vec![
        REPORT_JSON.to_string(),
        REPORT_HTML.to_string(),
        REPORT_MD.to_string(),
        GOOSE_LOG.to_string(),
    ];
    for name in LOG_NAMES {
        for extension in ["json", "csv", "log"] {
            names.push(format!("{name}.{extension}"));
        }
    }
    let gzipped: Vec<String> = names.iter().map(|name| format!("{name}.gz")).collect();
    names.extend(gzipped);
    // The JSON report's temporary file, left by a process that ended while
    // writing it.
    let temporary: Vec<String> = [REPORT_JSON.to_string(), format!("{REPORT_JSON}.gz")]
        .iter()
        .map(|name| format!(".{name}.tmp"))
        .collect();
    names.extend(temporary);
    // Left by a process that ended without giving up the folder.
    names.push(RESERVED.to_string());
    names
}

#[cfg(feature = "dashboard")]
impl GooseAttack {
    /// The dashboard's Save report and Include graph data switches, which can
    /// change at any time. Turning Save report on during a run keeps the
    /// graphs from then on, and the dashboard's window before it.
    pub(crate) fn control_report(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        save: Option<bool>,
        graph_data: Option<bool>,
    ) -> ControlOutcome {
        if save == Some(true) && self.configuration.no_metrics {
            return self.control_outcome(
                goose_attack_run_state,
                false,
                Some("invalid_report"),
                "--no-metrics keeps no report to save",
                None,
            );
        }
        if let Some(graph_data) = graph_data {
            self.output.graph_data = graph_data;
        }
        if let Some(save) = save.filter(|save| *save != self.output.save_report) {
            let keeps_before = self.keeps_full_graph();
            self.output.save_report = save;
            let keep = self.keeps_full_graph();
            if self.run_in_progress && keep != keeps_before {
                let now = self.series_elapsed_secs();
                let _ = goose_attack_run_state
                    .metrics_cmd_tx
                    .send(MetricsCommand::KeepGraph { keep, now });
            }
        }
        let message = format!(
            "save report {}, include graph data {}",
            if self.output.save_report { "on" } else { "off" },
            if self.output.graph_data { "on" } else { "off" }
        );
        self.control_outcome(goose_attack_run_state, true, None, message, None)
    }

    /// Save report now: write the run's report into its folder as it stands,
    /// marked in progress, while the run continues. The report at the end of
    /// the run replaces it.
    pub(crate) async fn control_save_report_now(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<ControlOutcome, GooseError> {
        let refuse = |attack: &Self, state: &GooseAttackRunState, error, message: &str| {
            Ok(attack.control_outcome(state, false, Some(error), message.to_string(), None))
        };
        if self.configuration.no_metrics {
            return refuse(
                self,
                goose_attack_run_state,
                "invalid_report",
                "--no-metrics keeps no report to save",
            );
        }
        let Some(folder) = self.output.folder.clone().filter(|_| self.run_in_progress) else {
            return refuse(
                self,
                goose_attack_run_state,
                "invalid_phase",
                "Save report now needs a run in progress",
            );
        };
        if self
            .output
            .saving_now
            .as_ref()
            .is_some_and(|saving| !saving.is_finished())
        {
            return refuse(
                self,
                goose_attack_run_state,
                "busy",
                "a report is being saved; try again when it is written",
            );
        }
        self.update_duration();
        let (respond, report_data) = tokio::sync::oneshot::channel();
        let sent = goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::GetReportData {
                duration: self.metrics.duration,
                total_users: self.metrics.total_users,
                maximum_users: self.metrics.maximum_users,
                history: self.metrics.history.clone(),
                respond,
            });
        let (Ok(()), Ok((metrics, graph_data))) = (sent, report_data.await) else {
            return refuse(
                self,
                goose_attack_run_state,
                "unavailable",
                "the metrics are not available",
            );
        };
        let folder_path = folder.path.clone();
        let record = self.report_record_for(true, &graph_data);
        let configuration = self.configuration.clone();
        let graph_data_in_json = self.output.graph_data;
        let compress = self.configuration.compress;
        self.output.saved_now = true;
        self.output.saving_now = Some(tokio::task::spawn_blocking(move || {
            let baseline = configuration
                .baseline_file
                .as_ref()
                .and_then(|file| load_baseline_file(file).ok());
            let source = crate::metrics::ReportSource {
                configuration: &configuration,
                metrics: &metrics,
                graph_data: &graph_data,
                graphs: true,
                heatmap: None,
                loaded: None,
                record: &record,
                graph_data_in_json,
            };
            match write_folder_report(
                &folder_path,
                &source,
                baseline.as_ref(),
                &mut None,
                compress,
            ) {
                Ok(files) => {
                    for file in files {
                        info!("in progress report written to: {}", file.display());
                    }
                }
                Err(e) => error!("couldn't save the report now: {e}"),
            }
        }));
        let message = format!("saving the report to {}", folder.path.display());
        Ok(self.control_outcome(goose_attack_run_state, true, None, message, None))
    }
}

/// RFC 3339 in UTC, to the second, as the JSON report records times.
pub(crate) fn rfc3339(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The load test's executable name, without extension.
pub(crate) fn test_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "goose-folder-test-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn configuration(output_dir: Option<&Path>) -> GooseConfiguration {
        GooseConfiguration {
            output_dir: output_dir.map(|dir| dir.to_string_lossy().into_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn default_folder_is_named_for_the_start_time() {
        let started = Local.with_ymd_and_hms(2026, 10, 9, 14, 12, 3).unwrap();
        let base = RunFolder::base(&configuration(None), started);
        assert_eq!(base, Path::new("goose-runs/20261009-141203"));
    }

    /// The first of DIR, DIR-2, DIR-3 that is missing or empty, always
    /// counted on DIR.
    #[test]
    fn folder_holding_files_is_never_reused() {
        let parent = temp_dir("suffix");
        let dir = parent.join("nightly");
        let started = Local::now();
        let choose = || RunFolder::choose(&configuration(Some(&dir)), started).unwrap();
        let mut chosen = Vec::new();
        for _ in 0..4 {
            let folder = choose();
            std::fs::write(folder.file("goose.log"), "x").unwrap();
            std::fs::remove_file(folder.file(RESERVED)).unwrap();
            chosen.push(folder.name());
        }
        assert_eq!(chosen, ["nightly", "nightly-2", "nightly-3", "nightly-4"]);

        // An empty folder is used; a file in the way is skipped.
        std::fs::remove_file(parent.join("nightly-3").join("goose.log")).unwrap();
        assert_eq!(choose().name(), "nightly-3");
        std::fs::write(parent.join("nightly-5"), "a file").unwrap();
        std::fs::write(parent.join("nightly-3").join("goose.log"), "x").unwrap();
        assert_eq!(choose().name(), "nightly-6");
        std::fs::remove_dir_all(&parent).ok();
    }

    /// Two runs that choose before either writes never share a folder, and
    /// an empty folder that is already there goes to the first of them.
    #[test]
    fn a_chosen_folder_is_reserved_at_once() {
        let parent = temp_dir("reserve");
        let dir = parent.join("nightly");
        let started = Local::now();
        let first = RunFolder::choose(&configuration(Some(&dir)), started).unwrap();
        let second = RunFolder::choose(&configuration(Some(&dir)), started).unwrap();
        assert_eq!(
            (first.name(), second.name()),
            ("nightly".into(), "nightly-2".into())
        );
        assert!(first.path.is_dir() && second.path.is_dir());

        let empty = parent.join("empty");
        std::fs::create_dir(&empty).unwrap();
        let first = RunFolder::choose(&configuration(Some(&empty)), started).unwrap();
        let second = RunFolder::choose(&configuration(Some(&empty)), started).unwrap();
        assert_eq!(
            (first.name(), second.name()),
            ("empty".into(), "empty-2".into())
        );
        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn writable_check_finds_the_nearest_folder() {
        let parent = temp_dir("writable");
        let folder = RunFolder {
            path: parent.join("a").join("b"),
        };
        folder.check_writable().unwrap();
        // Nothing is left behind, and nothing is created.
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);

        std::fs::write(parent.join("a"), "a file").unwrap();
        let error = folder.check_writable().unwrap_err();
        match error {
            GooseError::InvalidOption { option, value, .. } => {
                assert_eq!(option, "--output-dir");
                assert_eq!(value, folder.path.to_string_lossy());
            }
            other => panic!("{:?}", other),
        }
        std::fs::remove_dir_all(&parent).ok();
    }

    /// Rule 5: when the graphs start after the run did, the record says when
    /// and why; when they cover the run, it says nothing.
    #[test]
    fn graphs_start_when_the_graphs_do_not_cover_the_run() {
        let mut attack = GooseAttack::initialize_with_config(configuration(None)).unwrap();
        attack.output.started = Some(Utc.with_ymd_and_hms(2026, 10, 22, 10, 0, 0).unwrap());
        assert_eq!(attack.report_record(false, 0).graphs_start, None);
        attack.output.save_report = true;
        let start = attack.report_record(false, 300).graphs_start.unwrap();
        assert_eq!(start.time, "2026-10-22T10:05:00Z");
        assert!(start
            .reason
            .contains("Save report was off for part of the run"));
        attack.output.save_report = false;
        let start = attack.report_record(false, 300).graphs_start.unwrap();
        assert!(start.reason.starts_with("Save report is off"));
    }

    /// Rule 5: the report written at the end of a run whose graph data no
    /// longer reaches back to its start records where its graphs start.
    #[tokio::test]
    async fn the_end_of_run_report_says_where_its_graphs_start() {
        use crate::test_plan::{TestPlanHistory, TestPlanStepAction};
        let parent = temp_dir("graphs-start");
        let mut attack =
            GooseAttack::initialize_with_config(configuration(Some(&parent.join("run")))).unwrap();
        let started = Utc.with_ymd_and_hms(2026, 10, 22, 10, 0, 0).unwrap();
        attack.run_in_progress = true;
        attack.output.started = Some(started);
        attack.output.save_report = true;
        attack.output.folder =
            Some(RunFolder::choose(&attack.configuration, Local::now()).unwrap());
        for step in [TestPlanStepAction::Increasing, TestPlanStepAction::Finished] {
            attack.metrics.history.push(TestPlanHistory::step(step, 0));
        }
        for second in 0..=900 {
            attack.graph_data.record_users_per_second(1, second);
        }
        attack.graph_data.prune_to_window(600);
        let origin = attack.graph_data.first_second();
        assert!(origin > 0);
        attack.write_end_of_run().await.unwrap();
        let report: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(parent.join("run").join(REPORT_JSON)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["graphs_start"]["time"],
            rfc3339(started + chrono::Duration::seconds(origin as i64))
        );
        std::fs::remove_dir_all(&parent).ok();
    }

    /// Rule 11: a report written while the run continues is marked in
    /// progress, has no end, and says so at its top.
    #[test]
    fn a_report_saved_now_is_marked_in_progress() {
        let mut attack = GooseAttack::initialize_with_config(configuration(None)).unwrap();
        attack.run_in_progress = true;
        attack.output.started = Some(Utc::now());
        let record = attack.report_record(true, 0);
        assert!(record.in_progress);
        assert_eq!((record.ended.as_deref(), record.ended_by), (None, None));
        assert_eq!(record.intro(), vec![IN_PROGRESS.to_string()]);
        let record = attack.report_record(false, 0);
        assert!(!record.in_progress);
        assert_eq!(record.ended_by, Some(EndedBy::Completed));
        assert!(record.intro().is_empty());
    }

    /// New fields default, so a JSON report from before them loads.
    #[test]
    fn a_record_without_its_fields_loads() {
        let record: RunRecord = serde_json::from_str("{}").unwrap();
        assert_eq!(record, RunRecord::default());
    }

    #[test]
    fn parent_folder_holds_the_run_folder() {
        assert_eq!(parent_folder(&configuration(None)), Path::new("goose-runs"));
        let nested = configuration(Some(Path::new("goose-runs/nightly")));
        assert_eq!(parent_folder(&nested), Path::new("goose-runs"));
        let bare = configuration(Some(Path::new("nightly")));
        assert_eq!(parent_folder(&bare), Path::new("."));
    }

    #[test]
    fn a_gz_name_is_its_inner_format() {
        assert_eq!(without_gz("report.json.gz"), ("report.json", true));
        assert_eq!(without_gz("report.pdf.gz"), ("report.pdf", true));
        assert_eq!(without_gz("report.json"), ("report.json", false));
        // Only one .gz is taken off.
        assert_eq!(without_gz("x.gz.gz"), ("x.gz", true));
    }

    /// A folder's JSON report is its report.json, else its report.json.gz,
    /// and a gzipped report that is cut short is an error, never a panic.
    #[test]
    fn gzipped_reports_are_read_whole_or_not_at_all() {
        use std::io::Write;
        let dir = temp_dir("gz-read");
        let json = dir.join("report.json.gz");
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(br#"{"notes": "zipped"}"#).unwrap();
        let bytes = gzip.finish().unwrap();
        std::fs::write(&json, &bytes).unwrap();
        assert_eq!(crate::metrics::json_report_path(&dir), json);
        let text = crate::metrics::read_report_text(&json).unwrap();
        assert_eq!(text, r#"{"notes": "zipped"}"#);
        std::fs::write(dir.join("report.json"), "{}").unwrap();
        assert_eq!(
            crate::metrics::json_report_path(&dir),
            dir.join("report.json")
        );

        std::fs::write(&json, &bytes[..bytes.len() - 6]).unwrap();
        assert!(crate::metrics::read_report_text(&json).is_err());
        std::fs::write(&json, b"plain text").unwrap();
        assert!(crate::metrics::read_report_text(&json).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(feature = "dashboard")]
    #[test]
    fn delete_takes_plain_and_gzipped_names() {
        let names = deletable_files();
        for name in [
            "report.json",
            "report.json.gz",
            "goose.log.gz",
            "requests.csv.gz",
        ] {
            assert!(names.iter().any(|n| n == name), "{}", name);
        }
        assert!(!names.iter().any(|n| n.contains('/')));
    }

    /// Rule 12: a listing parses a report again when it changes, so the
    /// report that replaces one saved now lists as ended, never as running
    /// or incomplete; a report marked in progress is running only for the
    /// run in progress.
    #[cfg(feature = "dashboard")]
    #[test]
    fn a_changed_report_is_listed_again() {
        let parent = temp_dir("listing");
        let folder = parent.join("run");
        std::fs::create_dir(&folder).unwrap();
        let json = folder.join(REPORT_JSON);
        let report = |in_progress: bool, requests: u64| {
            serde_json::json!({
                "in_progress": in_progress,
                "started": "2026-10-22T10:00:00Z",
                "raw_metrics": {"requests": {"GET /": {"success_count": requests, "fail_count": 0}}},
            })
            .to_string()
        };
        let mut cache = ListingCache::new();
        std::fs::write(&json, report(true, 5)).unwrap();
        let listed = list_reports(&parent, Some(&folder), &mut cache).unwrap();
        assert!(listed[0].record.in_progress && listed[0].running);
        assert_eq!(listed[0].requests, 5);
        let listed = list_reports(&parent, None, &mut cache).unwrap();
        assert!(listed[0].record.in_progress && !listed[0].running);

        std::fs::write(&json, report(false, 12345)).unwrap();
        let listed = list_reports(&parent, Some(&folder), &mut cache).unwrap();
        assert!(!listed[0].record.in_progress && !listed[0].running);
        assert_eq!(listed[0].requests, 12345);
        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn log_names_follow_the_format() {
        assert_eq!(log_file_name("requests", None), "requests.json");
        assert_eq!(
            log_file_name("errors", Some(&GooseLogFormat::Csv)),
            "errors.csv"
        );
        assert_eq!(
            log_file_name("debug", Some(&GooseLogFormat::Pretty)),
            "debug.log"
        );
        assert_eq!(
            log_file_name("scenarios", Some(&GooseLogFormat::Raw)),
            "scenarios.log"
        );
    }
}
