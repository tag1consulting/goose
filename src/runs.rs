//! Saved runs: every load test is saved by default to its own directory under
//! the runs directory (`goose-runs` unless `--runs-dir` says otherwise).
//!
//! A run directory is named after the local time the run started,
//! `YYYY-MM-DD-HHMMSS`, with `-2`, `-3` and so on appended when two runs start
//! in the same second. It holds the HTML, JSON and Markdown reports, then
//! `run.json`, written last, which marks the run complete.

use crate::metrics::{load_baseline_file, ReportData, ReportSet};
use crate::{GooseAttack, GooseError};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The runs directory when `--runs-dir` is not given.
pub(crate) const DEFAULT_RUNS_DIR: &str = "goose-runs";

/// The HTML report of a saved run.
pub(crate) const REPORT_HTML: &str = "report.html";
/// The JSON report of a saved run, also read by `--baseline-file <run dir>`.
pub(crate) const REPORT_JSON: &str = "report.json";
/// The Markdown report of a saved run.
pub(crate) const REPORT_MD: &str = "report.md";
/// The summary of a saved run, written last.
pub(crate) const RUN_JSON: &str = "run.json";

/// The reports a saved run holds, in the order they are written.
pub(crate) const REPORT_FILES: [&str; 3] = [REPORT_HTML, REPORT_JSON, REPORT_MD];

/// The `format` of the `run.json` this version writes.
pub(crate) const RUN_FORMAT: u32 = 1;

/// Highest suffix tried when run directories for the same second already exist.
const MAX_RUN_SUFFIX: u32 = 999;

/// The `.gitignore` Goose writes in a runs directory it creates, so saved
/// runs are not committed by accident.
const GITIGNORE: &str = "*\n# Created by Goose. Delete this file to commit saved runs.\n";

/// How a run ended, as recorded in `run.json`.
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

/// One file of a saved run, as listed in `run.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunFile {
    pub name: String,
    pub bytes: u64,
}

/// The contents of `run.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunInfo {
    pub format: u32,
    pub id: String,
    pub goose_version: String,
    /// The load test's executable name, without extension.
    pub test: String,
    /// RFC 3339, UTC.
    pub started: String,
    /// RFC 3339, UTC.
    pub ended: String,
    pub duration_secs: u64,
    pub max_users: u64,
    pub hosts: Vec<String>,
    pub requests: u64,
    pub failed_requests: u64,
    pub ended_by: EndedBy,
    pub canceled_reason: Option<String>,
    pub baseline: Option<String>,
    pub files: Vec<RunFile>,
}

/// The run being saved: chosen when it starts, written when it ends.
#[derive(Clone, Debug)]
pub(crate) struct ActiveRun {
    pub id: String,
    /// The run directory on disk.
    pub path: PathBuf,
    /// The run directory as shown to the user: the runs directory as given,
    /// then the id.
    pub display: String,
    pub started: DateTime<Utc>,
}

/// What became of a saved run, printed after the final metrics.
#[derive(Clone, Debug)]
pub(crate) enum SaveOutcome {
    /// Saved; the reports as shown to the user.
    Saved { id: String, files: Vec<String> },
    /// The run could not be saved.
    Failed { id: String, error: String },
}

impl std::fmt::Display for SaveOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveOutcome::Saved { id, files } => {
                writeln!(f, "Saved run {id}:")?;
                for file in files {
                    writeln!(f, "  {file}")?;
                }
                Ok(())
            }
            SaveOutcome::Failed { id, error } => writeln!(f, "Couldn't save run {id}: {error}."),
        }
    }
}

/// Whether runs are being saved, as the dashboard shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SaveState {
    /// Runs are saved.
    On,
    /// `--no-save` or `--no-metrics`.
    #[default]
    Off,
    /// The runs directory or this run's directory couldn't be created.
    Failed,
}

impl SaveState {
    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SaveState::On => "on",
            SaveState::Off => "off",
            SaveState::Failed => "failed",
        }
    }
}

/// Saving runs, kept on [`GooseAttack`].
#[derive(Debug, Default)]
pub(crate) struct Saving {
    /// The run being saved, from its start to its end.
    pub active: Option<ActiveRun>,
    /// What became of the last saved run, until it is printed.
    pub outcome: Option<SaveOutcome>,
    /// Whether runs are saved.
    pub state: SaveState,
    /// Why the last run couldn't be saved, or why runs aren't saved.
    pub reason: Option<String>,
    /// The id of the last run Goose tried to save.
    pub last_run: Option<String>,
    /// A failure to create the default runs directory is logged once.
    pub failure_logged: bool,
}

/// `dir` and `name` joined with one `/`, as shown to the user: never resolved
/// to an absolute path.
pub(crate) fn display_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// True for a run id: `YYYY-MM-DD-HHMMSS`, optionally followed by `-` and a
/// suffix of one to three digits. Nothing else, so an id is always safe to
/// join to the runs directory.
#[allow(dead_code)]
pub(crate) fn is_valid_run_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    // YYYY-MM-DD-HHMMSS is 17 bytes.
    if bytes.len() < 17 {
        return false;
    }
    let (base, suffix) = bytes.split_at(17);
    let base_ok = base.iter().enumerate().all(|(i, b)| match i {
        4 | 7 | 10 => *b == b'-',
        _ => b.is_ascii_digit(),
    });
    let suffix_ok = match suffix {
        [] => true,
        [b'-', digits @ ..] => {
            (1..=3).contains(&digits.len()) && digits.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    };
    base_ok && suffix_ok
}

/// The id of a run started at `started`, local time.
pub(crate) fn run_id_base(started: DateTime<Local>) -> String {
    started.format("%Y-%m-%d-%H%M%S").to_string()
}

/// Create the runs directory if it does not exist. When Goose creates it, a
/// `.gitignore` is written inside so saved runs are not committed by accident.
pub(crate) fn ensure_runs_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    match std::fs::create_dir(dir) {
        Ok(()) => std::fs::write(dir.join(".gitignore"), GITIGNORE),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => Ok(()),
        Err(e) => Err(e),
    }
}

/// Create a new run directory under `runs_dir` named `base`, or `base-2` up
/// to `base-999` when that already exists. Returns the id and the path.
pub(crate) fn create_run_dir(runs_dir: &Path, base: &str) -> io::Result<(String, PathBuf)> {
    for n in 1..=MAX_RUN_SUFFIX {
        let id = if n == 1 {
            base.to_string()
        } else {
            format!("{base}-{n}")
        };
        let path = runs_dir.join(&id);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok((id, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{base} and every suffix up to -{MAX_RUN_SUFFIX} already exist"),
    ))
}

/// Write `run.json` into the run directory through a temporary file and a
/// rename, so a reader never sees it half written.
pub(crate) fn write_run_json(run_dir: &Path, info: &RunInfo) -> io::Result<()> {
    let tmp = run_dir.join(".run.json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    serde_json::to_writer_pretty(&mut file, info).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, run_dir.join(RUN_JSON))
}

/// RFC 3339 in UTC, to the second, as `run.json` records times.
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

/// The text of an error writing a saved run: the I/O error alone where
/// there is one.
fn error_text(error: &GooseError) -> String {
    match error {
        GooseError::Io(e) => e.to_string(),
        GooseError::Serde(e) => e.to_string(),
        GooseError::InvalidOption { detail, .. } => detail.clone(),
        other => other.to_string(),
    }
}

impl GooseAttack {
    /// At startup: say when `--no-save` overrides `--runs-dir`, and create an
    /// explicitly given runs directory, failing before any load if it can't
    /// be created.
    pub(crate) fn check_runs_dir(&mut self) -> Result<(), GooseError> {
        if self.configuration.no_save && !self.configuration.runs_dir.is_empty() {
            println!(
                "Not saving this run: --no-save is set, so --runs-dir {} is ignored.",
                self.configuration.runs_dir
            );
        }
        if !self.configuration.saves_runs() {
            self.saving.state = SaveState::Off;
            return Ok(());
        }
        self.saving.state = SaveState::On;
        if !self.configuration.runs_dir.is_empty() {
            let dir = self.configuration.runs_dir.clone();
            ensure_runs_dir(Path::new(&dir)).map_err(|e| runs_dir_error(&dir, &dir, e))?;
        }
        Ok(())
    }

    /// When a run starts: create its directory under the runs directory. A
    /// directory that can't be created is an error only when the runs
    /// directory was given explicitly; otherwise the run goes unsaved, with
    /// one warning.
    pub(crate) fn start_saved_run(&mut self) -> Result<(), GooseError> {
        self.saving.active = None;
        if !self.configuration.saves_runs() {
            self.saving.state = SaveState::Off;
            return Ok(());
        }
        let dir = self.configuration.runs_dir_or_default().to_string();
        let explicit = !self.configuration.runs_dir.is_empty();
        let started = Utc::now();
        let base = run_id_base(started.with_timezone(&Local));
        let created = ensure_runs_dir(Path::new(&dir))
            .map_err(|e| (dir.clone(), e))
            .and_then(|()| {
                create_run_dir(Path::new(&dir), &base).map_err(|e| (display_join(&dir, &base), e))
            });
        match created {
            Ok((id, path)) => {
                let display = display_join(&dir, &id);
                info!(
                    "Saving this run to {display} (on long soak tests, use --no-save to keep memory flat)"
                );
                self.saving.state = SaveState::On;
                self.saving.reason = None;
                self.saving.active = Some(ActiveRun {
                    id,
                    path,
                    display,
                    started,
                });
                Ok(())
            }
            Err((failed, e)) => {
                if explicit {
                    return Err(runs_dir_error(&dir, &failed, e));
                }
                if !self.saving.failure_logged {
                    warn!(
                        "Not saving this run: can't create {failed} ({e}). Use --runs-dir to choose another directory, or --no-save to stop this warning."
                    );
                    self.saving.failure_logged = true;
                }
                self.saving.state = SaveState::Failed;
                self.saving.reason = Some(e.to_string());
                Ok(())
            }
        }
    }

    /// Record how the run ended. The first cause recorded wins.
    pub(crate) fn set_ended_by(&mut self, ended_by: EndedBy, reason: Option<String>) {
        if self.run_in_progress && self.ended_by.is_none() {
            self.ended_by = Some((ended_by, reason));
        }
    }

    /// When a run has fully stopped: write the `--report-file` reports and
    /// the saved run, attempting both even if one fails. A saved run that
    /// can't be written is a warning; a `--report-file` error is returned.
    pub(crate) async fn write_end_of_run(&mut self) -> Result<(), GooseError> {
        // Load the baseline once for all report writers.
        let (baseline, baseline_error) = match &self.configuration.baseline_file {
            Some(file) => match load_baseline_file(file) {
                Ok(baseline) => (Some(baseline), None),
                Err(e) => (None, Some(e)),
            },
            None => (None, None),
        };
        // The HTML report is generated once for --report-file and the saved run.
        let mut html = None;
        let report_files = match baseline_error {
            Some(e) => Err(e),
            None => self.write_report_files(baseline.as_ref(), &mut html).await,
        };

        if let Some(run) = self.saving.active.take() {
            let outcome = self
                .write_saved_run(&run, baseline.as_ref(), &mut html)
                .await;
            self.saving.last_run = Some(run.id.clone());
            match &outcome {
                SaveOutcome::Saved { .. } => self.saving.reason = None,
                SaveOutcome::Failed { id, error } => {
                    warn!("Couldn't save run {id}: {error}.");
                    self.saving.reason = Some(error.clone());
                }
            }
            self.saving.outcome = Some(outcome);
        }

        report_files
    }

    /// Write a saved run's reports through the report writers, then
    /// `run.json` last.
    async fn write_saved_run(
        &self,
        run: &ActiveRun,
        baseline: Option<&ReportData<'static>>,
        html: &mut Option<String>,
    ) -> SaveOutcome {
        let failed = |error: String| SaveOutcome::Failed {
            id: run.id.clone(),
            error,
        };
        let reports: Vec<String> = REPORT_FILES
            .iter()
            .map(|name| run.path.join(name).to_string_lossy().into_owned())
            .collect();
        if let Err(e) = self
            .process_reports(&reports, ReportSet::SavedRun, true, baseline, html)
            .await
        {
            return failed(error_text(&e));
        }

        let mut files = Vec::with_capacity(REPORT_FILES.len());
        for name in REPORT_FILES {
            match std::fs::metadata(run.path.join(name)) {
                Ok(metadata) => files.push(RunFile {
                    name: name.to_string(),
                    bytes: metadata.len(),
                }),
                Err(e) => return failed(e.to_string()),
            }
        }
        let mut hosts: Vec<String> = self.metrics.hosts.iter().cloned().collect();
        hosts.sort();
        let (requests, failed_requests) =
            self.metrics
                .requests
                .values()
                .fold((0u64, 0u64), |(total, fails), request| {
                    (
                        total + (request.success_count + request.fail_count) as u64,
                        fails + request.fail_count as u64,
                    )
                });
        let (ended_by, canceled_reason) =
            self.ended_by.clone().unwrap_or((EndedBy::Completed, None));
        let info = RunInfo {
            format: RUN_FORMAT,
            id: run.id.clone(),
            goose_version: env!("CARGO_PKG_VERSION").to_string(),
            test: test_name(),
            started: rfc3339(run.started),
            ended: rfc3339(Utc::now()),
            duration_secs: self.metrics.duration as u64,
            max_users: self.metrics.maximum_users as u64,
            hosts,
            requests,
            failed_requests,
            ended_by,
            canceled_reason,
            baseline: self.configuration.baseline_file.clone(),
            files,
        };
        if let Err(e) = write_run_json(&run.path, &info) {
            return failed(e.to_string());
        }

        SaveOutcome::Saved {
            id: run.id.clone(),
            files: REPORT_FILES
                .iter()
                .map(|name| display_join(&run.display, name))
                .collect(),
        }
    }

    /// Print what became of the saved run, once, after the final metrics.
    pub(crate) fn print_save_outcome(&mut self) {
        if let Some(outcome) = self.saving.outcome.take() {
            print!("{outcome}");
        }
    }
}

/// The error for an explicitly given runs directory that can't be used.
fn runs_dir_error(dir: &str, failed: &str, e: io::Error) -> GooseError {
    GooseError::InvalidOption {
        option: "--runs-dir".to_string(),
        value: dir.to_string(),
        detail: format!("Can't create {failed}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "goose-runs-test-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_id_format() {
        let started = Local.with_ymd_and_hms(2026, 10, 9, 14, 12, 3).unwrap();
        let id = run_id_base(started);
        assert_eq!(id, "2026-10-09-141203");
        assert!(is_valid_run_id(&id));
    }

    #[test]
    fn run_id_validation() {
        for good in [
            "2026-10-09-141203",
            "2026-10-09-141203-2",
            "2026-10-09-141203-99",
            "2026-10-09-141203-999",
        ] {
            assert!(is_valid_run_id(good), "{} should be valid", good);
        }
        for bad in [
            "",
            "..",
            "../2026-10-09-141203",
            "2026-10-09-141203/..",
            "2026-10-09-141203/",
            "2026-10-09-141203%2F..",
            "%2F2026-10-09-141203",
            "/2026-10-09-141203",
            "/etc/passwd",
            "2026-10-09-141203-",
            "2026-10-09-141203-1000",
            "2026-10-09-141203-a",
            "2026-10-09-14120",
            "2026-10-09T141203",
            "2026_10-09-141203",
            "2026-10-09-141203 ",
            "\u{ff12}026-10-09-141203",
            "2026-10-09-141203\0",
        ] {
            assert!(!is_valid_run_id(bad), "{:?} should be invalid", bad);
        }
    }

    #[test]
    fn create_run_dir_retries_with_suffix() {
        let runs = temp_dir("suffix");
        let (first, first_path) = create_run_dir(&runs, "2026-10-09-141203").unwrap();
        let (second, second_path) = create_run_dir(&runs, "2026-10-09-141203").unwrap();
        let (third, _) = create_run_dir(&runs, "2026-10-09-141203").unwrap();
        assert_eq!(first, "2026-10-09-141203");
        assert_eq!(second, "2026-10-09-141203-2");
        assert_eq!(third, "2026-10-09-141203-3");
        assert!(first_path.is_dir() && second_path.is_dir());
        assert!(is_valid_run_id(&second) && is_valid_run_id(&third));
        std::fs::remove_dir_all(&runs).ok();
    }

    #[test]
    fn ensure_runs_dir_writes_gitignore_only_when_creating() {
        let parent = temp_dir("gitignore");
        let runs = parent.join("nested").join("goose-runs");
        ensure_runs_dir(&runs).unwrap();
        let gitignore = std::fs::read_to_string(runs.join(".gitignore")).unwrap();
        let mut lines = gitignore.lines();
        assert_eq!(lines.next(), Some("*"));
        assert_eq!(
            lines.next(),
            Some("# Created by Goose. Delete this file to commit saved runs.")
        );

        // An existing directory is left alone.
        let existing = parent.join("existing");
        std::fs::create_dir(&existing).unwrap();
        ensure_runs_dir(&existing).unwrap();
        assert!(!existing.join(".gitignore").exists());
        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn run_json_round_trip() {
        let run_dir = temp_dir("runjson");
        let info = RunInfo {
            format: RUN_FORMAT,
            id: "2026-10-09-141203".to_string(),
            goose_version: "0.19.0-dev".to_string(),
            test: "loadtest".to_string(),
            started: "2026-10-09T12:12:03Z".to_string(),
            ended: "2026-10-09T12:13:03Z".to_string(),
            duration_secs: 60,
            max_users: 10,
            hosts: vec!["http://localhost".to_string()],
            requests: 1234,
            failed_requests: 5,
            ended_by: EndedBy::Canceled,
            canceled_reason: Some("SIGINT received".to_string()),
            baseline: None,
            files: vec![RunFile {
                name: REPORT_HTML.to_string(),
                bytes: 42,
            }],
        };
        write_run_json(&run_dir, &info).unwrap();
        assert!(!run_dir.join(".run.json.tmp").exists());
        let text = std::fs::read_to_string(run_dir.join(RUN_JSON)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["ended_by"], "canceled");
        assert_eq!(value["canceled_reason"], "SIGINT received");
        assert!(value["baseline"].is_null());
        let back: RunInfo = serde_json::from_str(&text).unwrap();
        assert_eq!(back, info);
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn save_outcome_display() {
        let saved = SaveOutcome::Saved {
            id: "2026-10-09-141203".to_string(),
            files: vec![
                "goose-runs/2026-10-09-141203/report.html".to_string(),
                "goose-runs/2026-10-09-141203/report.json".to_string(),
            ],
        };
        assert_eq!(
            saved.to_string(),
            "Saved run 2026-10-09-141203:\n  goose-runs/2026-10-09-141203/report.html\n  goose-runs/2026-10-09-141203/report.json\n"
        );
        let failed = SaveOutcome::Failed {
            id: "2026-10-09-141203".to_string(),
            error: "Permission denied (os error 13)".to_string(),
        };
        assert_eq!(
            failed.to_string(),
            "Couldn't save run 2026-10-09-141203: Permission denied (os error 13).\n"
        );
        assert_eq!(display_join("goose-runs/", "x"), "goose-runs/x");
        assert_eq!(display_join("goose-runs", "x"), "goose-runs/x");
    }
}
