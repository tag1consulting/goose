//! The Gaggle options: their defaults, and which combinations a manager and a
//! worker refuse.

use crate::config::{is_loopback_bind, GooseConfiguration};
use crate::util;
use crate::GooseError;

/// The address a manager listens on when `--manager-bind-host` is not set.
const DEFAULT_MANAGER_BIND_HOST: &str = "127.0.0.1";

/// The port a manager listens on, and a worker connects to, by default.
pub(crate) const DEFAULT_MANAGER_PORT: u16 = 5115;

/// How long a worker keeps trying to reach its manager, and a manager waits
/// for its workers, when `--gaggle-connect-timeout` is not set.
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 60;

/// How long the manager and a worker wait to hear from each other when
/// `--gaggle-heartbeat-timeout` is not set.
const DEFAULT_HEARTBEAT_TIMEOUT_SECS: u64 = 10;

/// The shortest heartbeat timeout: heartbeats go every 2 seconds.
const MIN_HEARTBEAT_TIMEOUT_SECS: u64 = 3;

/// The environment variable a token is read from when `--gaggle-token` is not
/// given.
pub(crate) const TOKEN_ENV: &str = "GOOSE_GAGGLE_TOKEN";

/// The option that sets each log, by its index in
/// [`LOG_NAMES`](crate::runs::LOG_NAMES).
const LOG_OPTIONS: [&str; crate::runs::LOG_NAMES.len()] = [
    "--request-log",
    "--transaction-log",
    "--scenario-log",
    "--error-log",
    "--debug-log",
];

/// The error a refused Gaggle option combination returns.
fn refused(option: &str, value: impl ToString, detail: impl Into<String>) -> GooseError {
    GooseError::InvalidOption {
        option: option.to_string(),
        value: value.to_string(),
        detail: detail.into(),
    }
}

/// A time span option in seconds, or its default when not set; refused when
/// it does not parse or is 0.
fn timespan(option: &str, value: &Option<String>, default: u64) -> Result<u64, GooseError> {
    match value.as_deref() {
        None => Ok(default),
        Some(text) => match util::timespan_seconds(text) {
            Ok(seconds) if seconds > 0 => Ok(seconds as u64),
            _ => Err(refused(
                option,
                text,
                format!("{option} must be a time of at least one second, such as 30s or 2m."),
            )),
        },
    }
}

/// The options a worker keeps from its own command line; it refuses every
/// other it is given, but the manager's own (see
/// [`GooseConfiguration::worker_refusals`]).
const WORKER_KEEPS: &[&str] = &[
    "--output-dir",
    "--goose-log",
    // Verbosity, and what the worker's own console shows.
    "--verbose",
    "--quiet",
    "--log-level",
    "--no-print-metrics",
    "--no-error-summary",
    // Help and version, and the lists that print and exit.
    "--help",
    "--version",
    "--list",
    "--scenarios-list",
    // The worker's own Gaggle options.
    "--worker",
    "--manager-host",
    "--manager-port",
    "--gaggle-connect-timeout",
    "--gaggle-token",
    "--no-hash-check",
    "--gaggle-heartbeat-timeout",
];

/// The manager's Gaggle options, which `validate_gaggle` refuses on a worker
/// with its own message.
const MANAGER_ONLY: &[&str] = &[
    "--manager",
    "--expect-workers",
    "--manager-bind-host",
    "--manager-bind-port",
    "--gaggle-shadow-merge",
];

impl GooseConfiguration {
    /// Whether this process is a Gaggle manager.
    pub(crate) fn is_manager(&self) -> bool {
        self.manager
    }

    /// Whether this process is a Gaggle worker.
    pub(crate) fn is_worker(&self) -> bool {
        self.worker
    }

    /// The address and port a manager listens on.
    pub(crate) fn manager_bind(&self) -> (String, u16) {
        (
            self.manager_bind_host
                .as_deref()
                .map(str::trim)
                .filter(|host| !host.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| DEFAULT_MANAGER_BIND_HOST.to_string()),
            self.manager_bind_port.unwrap_or(DEFAULT_MANAGER_PORT),
        )
    }

    /// The manager a worker connects to, as `host:port`, with brackets around
    /// an IPv6 address.
    pub(crate) fn manager_address(&self) -> String {
        let host = self.manager_host.clone().unwrap_or_default();
        let port = self.manager_port.unwrap_or(DEFAULT_MANAGER_PORT);
        if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        }
    }

    /// The connect timeout.
    pub(crate) fn gaggle_connect_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            timespan(
                "--gaggle-connect-timeout",
                &self.gaggle_connect_timeout,
                DEFAULT_CONNECT_TIMEOUT_SECS,
            )
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS),
        )
    }

    /// The heartbeat timeout.
    pub(crate) fn gaggle_heartbeat_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            timespan(
                "--gaggle-heartbeat-timeout",
                &self.gaggle_heartbeat_timeout,
                DEFAULT_HEARTBEAT_TIMEOUT_SECS,
            )
            .unwrap_or(DEFAULT_HEARTBEAT_TIMEOUT_SECS),
        )
    }

    /// The shared token, `None` when neither `--gaggle-token` nor
    /// `GOOSE_GAGGLE_TOKEN` sets one, or it is empty.
    pub(crate) fn gaggle_token(&self) -> Option<&str> {
        self.gaggle_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }

    /// Read the token from `GOOSE_GAGGLE_TOKEN` when `--gaggle-token` is not
    /// given, for a manager or a worker.
    pub(crate) fn configure_gaggle(&mut self) {
        if self.gaggle_token.is_none() && (self.manager || self.worker) {
            self.gaggle_token = std::env::var(TOKEN_ENV).ok();
        }
    }

    /// The options a worker refuses, as the command line gave them: every
    /// option that shapes the load, the metrics, the reports or the logs,
    /// which the manager hands out. Checked before the load test's defaults
    /// are merged in, so a default set in code is not refused; the manager's
    /// run spec replaces it.
    pub(crate) fn worker_refusals(&self) -> Result<(), GooseError> {
        if !self.worker {
            return Ok(());
        }
        #[cfg(feature = "pdf-reports")]
        if self.pdf_scale.is_some() || self.pdf_timeout.is_some() {
            return Err(refused(
                "--pdf-scale",
                "set",
                "--pdf-scale and --pdf-timeout can't be used with --worker: the manager writes the reports.",
            ));
        }
        if let Some(option) = self.worker_refused().first() {
            return Err(refused(
                option,
                "set",
                format!(
                    "{option} can't be used with --worker: the manager gives every option that shapes the load, the metrics, the reports and the logs, and a worker runs no Controller or dashboard. A worker keeps --output-dir, --goose-log and verbosity."
                ),
            ));
        }
        Ok(())
    }

    /// Every option the command line set that [`Self::worker_refusals`]
    /// refuses, in the alphabetical order of their field names, as
    /// serde_json's map keeps them: each whose value
    /// is not its default, but those in [`WORKER_KEEPS`] and
    /// [`MANAGER_ONLY`]. An option is a field whose name, in kebab case, is
    /// its long flag, and which is unset by default: `None`, empty, `false`
    /// or `0`.
    fn worker_refused(&self) -> Vec<String> {
        let (Ok(serde_json::Value::Object(set)), Ok(serde_json::Value::Object(default))) = (
            serde_json::to_value(self),
            serde_json::to_value(GooseConfiguration::default()),
        ) else {
            return Vec::new();
        };
        set.iter()
            .filter(|&(field, value)| default.get(field) != Some(value))
            .map(|(field, _)| format!("--{}", field.replace('_', "-")))
            .filter(|option| {
                !WORKER_KEEPS.contains(&option.as_str()) && !MANAGER_ONLY.contains(&option.as_str())
            })
            .collect()
    }

    /// Validate the Gaggle options, once the defaults are merged in.
    pub(crate) fn validate_gaggle(&self) -> Result<(), GooseError> {
        if self.gaggle_shadow_merge && (self.manager || self.worker) {
            return Err(refused(
                "--gaggle-shadow-merge",
                "set",
                "--gaggle-shadow-merge can't be used with --manager or --worker.",
            ));
        }
        if self.manager && self.worker {
            return Err(refused(
                "--manager",
                "set",
                "--manager can't be used with --worker.",
            ));
        }
        timespan(
            "--gaggle-connect-timeout",
            &self.gaggle_connect_timeout,
            DEFAULT_CONNECT_TIMEOUT_SECS,
        )?;
        let heartbeat = timespan(
            "--gaggle-heartbeat-timeout",
            &self.gaggle_heartbeat_timeout,
            DEFAULT_HEARTBEAT_TIMEOUT_SECS,
        )?;
        // Heartbeats go every 2 seconds: a shorter timeout loses a peer
        // between two of them.
        if heartbeat < MIN_HEARTBEAT_TIMEOUT_SECS {
            return Err(refused(
                "--gaggle-heartbeat-timeout",
                heartbeat,
                format!(
                    "--gaggle-heartbeat-timeout must be at least {MIN_HEARTBEAT_TIMEOUT_SECS} seconds: heartbeats go every 2 seconds."
                ),
            ));
        }
        let either = self.manager || self.worker;
        let shared: [(&str, bool); 4] = [
            ("--gaggle-token", self.gaggle_token.is_some() && !either),
            ("--no-hash-check", self.no_hash_check && !either),
            (
                "--gaggle-connect-timeout",
                self.gaggle_connect_timeout.is_some() && !either,
            ),
            (
                "--gaggle-heartbeat-timeout",
                self.gaggle_heartbeat_timeout.is_some() && !either,
            ),
        ];
        // The token travels as `Bearer <token>` in an HTTP/2 header.
        if either
            && self
                .gaggle_token()
                .is_some_and(|token| !token.bytes().all(|byte| byte.is_ascii_graphic()))
        {
            return Err(refused(
                "--gaggle-token",
                "set",
                "--gaggle-token must be printable ASCII, with no spaces.",
            ));
        }
        let manager_only: [(&str, bool); 3] = [
            ("--expect-workers", self.expect_workers.is_some()),
            ("--manager-bind-host", self.manager_bind_host.is_some()),
            ("--manager-bind-port", self.manager_bind_port.is_some()),
        ];
        let worker_only: [(&str, bool); 2] = [
            ("--manager-host", self.manager_host.is_some()),
            ("--manager-port", self.manager_port.is_some()),
        ];
        if let Some((option, _)) = shared.iter().find(|(_, set)| *set) {
            return Err(refused(
                option,
                "set",
                format!("{option} needs --manager or --worker."),
            ));
        }
        if !self.manager {
            if let Some((option, _)) = manager_only.iter().find(|(_, set)| *set) {
                return Err(refused(option, "set", format!("{option} needs --manager.")));
            }
        }
        if !self.worker {
            if let Some((option, _)) = worker_only.iter().find(|(_, set)| *set) {
                return Err(refused(option, "set", format!("{option} needs --worker.")));
            }
        }
        if self.manager {
            match self.expect_workers {
                None => {
                    return Err(refused(
                        "--expect-workers",
                        "unset",
                        "--manager needs --expect-workers, the number of workers to wait for.",
                    ))
                }
                Some(0) => {
                    return Err(refused(
                        "--expect-workers",
                        0,
                        "--expect-workers must be at least 1.",
                    ))
                }
                Some(workers) if workers > crate::gaggle::delta::MAX_WORKERS as usize => {
                    return Err(refused(
                        "--expect-workers",
                        workers,
                        format!(
                            "--expect-workers can be at most {}.",
                            crate::gaggle::delta::MAX_WORKERS
                        ),
                    ))
                }
                Some(_) => {}
            }
            if self.no_autostart {
                return Err(refused(
                    "--no-autostart",
                    "set",
                    "--no-autostart can't be used with --manager yet: a manager starts its workers as soon as they have all connected.",
                ));
            }
            let (host, _) = self.manager_bind();
            if !is_loopback_bind(&host) && self.gaggle_token().is_none() {
                return Err(refused(
                    "--manager-bind-host",
                    &host,
                    format!(
                        "--manager-bind-host {host} is not a loopback address: set --gaggle-token (or {TOKEN_ENV}) so only your workers can join."
                    ),
                ));
            }
            // The manager keeps each worker's logs in one folder, by the
            // name each log flag gives.
            let logs = crate::gaggle::spec::log_specs(self);
            for (at, log) in logs.iter().enumerate() {
                // Equal names, or one the other's part file while it arrives.
                let clash = |a: &str, b: &str| {
                    a.eq_ignore_ascii_case(b) || a.eq_ignore_ascii_case(&format!("{b}.part"))
                };
                if let Some(other) = logs[at + 1..].iter().find(|other| {
                    clash(&other.manager_file, &log.manager_file)
                        || clash(&log.manager_file, &other.manager_file)
                }) {
                    let (first, second) = (
                        LOG_OPTIONS[log.kind as usize],
                        LOG_OPTIONS[other.kind as usize],
                    );
                    return Err(refused(
                        second,
                        &other.manager_file,
                        format!(
                            "{first} and {second} both name their log {:?}: a manager keeps each worker's logs in one folder, so each needs a file name of its own.",
                            log.manager_file
                        ),
                    ));
                }
            }
            // A worker with no users, when there are more workers than
            // users, takes no share of the throttle.
            let workers = self.expect_workers.unwrap_or(1);
            let throttle = self.throttle_requests();
            let busy = crate::test_plan::TestPlan::build(self).map_or(workers, |plan| {
                crate::gaggle::spec::workers_with_users(&plan.steps, workers)
            });
            if throttle > 0 && throttle < busy {
                return Err(refused(
                    "--throttle-requests",
                    throttle,
                    format!(
                        "--throttle-requests {throttle} is fewer than the {busy} workers that run users: each needs at least 1 request per second."
                    ),
                ));
            }
        }
        if self.worker
            && self
                .manager_host
                .as_deref()
                .is_none_or(|host| host.trim().is_empty())
        {
            return Err(refused(
                "--manager-host",
                "unset",
                "--worker needs --manager-host, the manager's DNS name or address.",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseDefaults;

    fn parse(args: &[&str]) -> GooseConfiguration {
        GooseConfiguration::parse_args_default(args).unwrap()
    }

    /// The detail of the error a configuration of `args` is refused with,
    /// as `execute()` checks it.
    fn refusal(args: &[&str]) -> String {
        let mut configuration = parse(args);
        let result = configuration.worker_refusals().and_then(|()| {
            configuration.configure(&GooseDefaults::default())?;
            configuration.validate()
        });
        match result {
            Err(GooseError::InvalidOption { detail, .. }) => detail,
            other => panic!("{:?} was not refused: {:?}", args, other),
        }
    }

    /// A `--test-plan` of `steps` steps of 1 user for 1 second.
    fn plan_of(steps: usize) -> String {
        vec!["1,1"; steps].join(";")
    }

    /// Every refused combination, with its message. Serial, with no
    /// token from the environment, which a manager or worker reads.
    #[test]
    #[serial_test::serial]
    fn gaggle_refused_option_combinations() {
        std::env::remove_var(TOKEN_ENV);
        let manager = ["--manager", "--expect-workers", "2", "--host", "http://h"];
        let worker = ["--worker", "--manager-host", "m"];
        let with = |base: &[&str], more: &[&str]| -> Vec<String> {
            base.iter().chain(more).map(|arg| arg.to_string()).collect()
        };
        let cases: Vec<(Vec<String>, &str)> = vec![
            (
                with(&["--manager"], &[]),
                "--manager needs --expect-workers",
            ),
            (
                with(&["--manager", "--expect-workers", "0"], &[]),
                "--expect-workers must be at least 1",
            ),
            (
                with(
                    &["--manager", "--expect-workers", "2", "--worker"],
                    &["--manager-host", "m"],
                ),
                "--manager can't be used with --worker",
            ),
            (
                with(&manager, &["--no-autostart"]),
                "--no-autostart can't be used with --manager",
            ),
            (
                with(&manager, &["--manager-bind-host", "0.0.0.0"]),
                "is not a loopback address: set --gaggle-token",
            ),
            (
                with(&manager, &["--throttle-requests", "1", "--users", "4"]),
                "is fewer than the 2 workers that run users",
            ),
            (
                with(&manager, &["--gaggle-shadow-merge"]),
                "--gaggle-shadow-merge can't be used with --manager or --worker",
            ),
            (
                with(&manager, &["--gaggle-heartbeat-timeout", "0"]),
                "--gaggle-heartbeat-timeout must be a time of at least one second",
            ),
            (
                with(&manager, &["--gaggle-heartbeat-timeout", "2s"]),
                "--gaggle-heartbeat-timeout must be at least 3 seconds",
            ),
            (
                with(&manager, &["--gaggle-connect-timeout", "0"]),
                "--gaggle-connect-timeout must be a time of at least one second",
            ),
            (
                with(&["--manager", "--expect-workers", "2049"], &[]),
                "--expect-workers can be at most 2048",
            ),
            (
                with(
                    &["--worker", "--manager-host", "m"],
                    &["--gaggle-shadow-merge"],
                ),
                "--gaggle-shadow-merge can't be used with --manager or --worker",
            ),
            (
                with(&["--worker", "--manager-host", " "], &[]),
                "--worker needs --manager-host",
            ),
            (
                with(&["--manager-bind-host", "h"], &[]),
                "--manager-bind-host needs --manager",
            ),
            (
                with(&["--manager-bind-port", "1"], &[]),
                "--manager-bind-port needs --manager",
            ),
            (
                with(&["--manager-port", "1"], &[]),
                "--manager-port needs --worker",
            ),
            (
                with(&["--gaggle-connect-timeout", "5s"], &[]),
                "--gaggle-connect-timeout needs --manager or --worker",
            ),
            (
                with(&["--gaggle-heartbeat-timeout", "5s"], &[]),
                "--gaggle-heartbeat-timeout needs --manager or --worker",
            ),
            (with(&["--worker"], &[]), "--worker needs --manager-host"),
            (
                with(&["--expect-workers", "2"], &[]),
                "--expect-workers needs --manager",
            ),
            (
                with(&["--manager-host", "m"], &[]),
                "--manager-host needs --worker",
            ),
            (
                with(&["--gaggle-token", "t"], &[]),
                "--gaggle-token needs --manager or --worker",
            ),
            (
                with(&["--no-hash-check"], &[]),
                "--no-hash-check needs --manager.",
            ),
            // Only the manager checks the plan hash.
            (
                with(&worker, &["--no-hash-check"]),
                "--no-hash-check needs --manager.",
            ),
            (
                with(
                    &manager,
                    &["--request-log", "a/x.log", "--error-log", "b/X.log"],
                ),
                "--request-log and --error-log both name their log \"x.log\"",
            ),
            (
                // One log's name is the other's part file as it arrives.
                with(
                    &manager,
                    &["--request-log", "x.log", "--error-log", "x.log.part"],
                ),
                "--request-log and --error-log both name their log \"x.log\"",
            ),
            (
                // A path that takes a log's default name.
                with(
                    &manager,
                    &[
                        "--transaction-log",
                        "--debug-log",
                        "elsewhere/transactions.json",
                    ],
                ),
                "--transaction-log and --debug-log both name their log \"transactions.json\"",
            ),
            (
                with(&worker, &["--gaggle-token", "two words"]),
                "--gaggle-token must be printable ASCII",
            ),
            (
                with(&manager, &["--gaggle-token", "t\u{e9}"]),
                "--gaggle-token must be printable ASCII",
            ),
            // What every worker would refuse in its share: worker 0, the
            // largest, past the users a worker runs, or a plan past the
            // steps.
            (
                with(&manager, &["--users", "2000001"]),
                "gives worker 0 1000001 users, more than the 1000000 users a worker runs",
            ),
            (
                with(&manager, &["--test-plan", &plan_of(100_001)]),
                "100001 test plan steps, more than the 100000 steps a worker runs",
            ),
        ];
        for (args, message) in cases {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let detail = refusal(&args);
            assert!(detail.contains(message), "{:?}: {}", args, detail);
        }
        // A worker refuses every option its manager gives.
        let refused: &[&[&str]] = &[
            &["--host", "http://h"],
            &["--users", "3"],
            &["--increase-rate", "3"],
            &["--increase-time", "3"],
            &["--run-time", "3"],
            &["--decrease-rate", "3"],
            &["--decrease-time", "3"],
            &["--iterations", "3"],
            &["--test-plan", "3,1s"],
            &["--scenarios", "a"],
            &["--throttle-requests", "3"],
            &["--running-metrics", "3"],
            &["--report-file", "r.html"],
            &["--compress"],
            &["--notes", "n"],
            &["--report-graph-data"],
            &["--no-reset-metrics"],
            &["--no-metrics"],
            &["--no-transaction-metrics"],
            &["--no-scenario-metrics"],
            &["--no-status-codes"],
            &["--co-mitigation", "average"],
            &["--threshold", "p95<1s"],
            &["--no-threshold-abort"],
            &["--request-log"],
            &["--transaction-log"],
            &["--scenario-log"],
            &["--error-log"],
            &["--debug-log"],
            &["--dashboard"],
            &["--no-autostart"],
            &["--timeout", "3"],
            &["--render-report", "r.json"],
            &["--baseline-file", "b.json"],
            &["--no-granular-report"],
            &["--pdf-print-html", "p.html"],
            &["--threshold-abort-delay", "3"],
            &["--request-format", "csv"],
            &["--request-body"],
            &["--transaction-format", "csv"],
            &["--scenario-format", "csv"],
            &["--error-format", "csv"],
            &["--debug-format", "csv"],
            &["--no-debug-body"],
            &["--dashboard-host", "h"],
            &["--dashboard-port", "1"],
            &["--dashboard-auth-token", "t"],
            &["--dashboard-control"],
            &["--dashboard-no-auth"],
            &["--no-percentiles-over-time"],
            &["--dashboard-max-clients", "2"],
            &["--no-telnet"],
            &["--telnet-host", "h"],
            &["--telnet-port", "1"],
            &["--websocket"],
            &["--no-websocket"],
            &["--websocket-host", "h"],
            &["--websocket-port", "1"],
            &["--no-gzip"],
            &["--sticky-follow"],
            &["--accept-invalid-certs"],
        ];
        for option in refused {
            let args = with(&worker, option);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let detail = refusal(&args);
            assert!(
                detail.contains(&format!("{} can't be used with --worker", option[0])),
                "{:?}: {}",
                args,
                detail
            );
        }
        #[cfg(feature = "pdf-reports")]
        {
            let args = with(&worker, &["--pdf-scale", "0.5"]);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            assert!(refusal(&args)
                .contains("--pdf-scale and --pdf-timeout can't be used with --worker"));
        }
        // A throttle as large as the workers that run users is enough,
        // when more workers than users are expected.
        let mut fewer = parse(&[
            "--manager",
            "--expect-workers",
            "4",
            "--host",
            "http://h",
            "--users",
            "2",
            "--throttle-requests",
            "2",
        ]);
        fewer.configure(&GooseDefaults::default()).unwrap();
        fewer.validate().unwrap();
        // A worker's share at each size bound is taken.
        for more in [
            vec!["--users".to_string(), "2000000".to_string()],
            vec!["--test-plan".to_string(), plan_of(100_000)],
        ] {
            let more: Vec<&str> = more.iter().map(String::as_str).collect();
            let args = with(&manager, &more);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let mut at_bound = parse(&args);
            at_bound.configure(&GooseDefaults::default()).unwrap();
            at_bound.validate().unwrap();
        }
        // What a worker keeps.
        let mut kept = parse(
            &with(&worker, &["--output-dir", "o", "-v"])
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        kept.worker_refusals().unwrap();
        kept.configure(&GooseDefaults::default()).unwrap();
        kept.validate().unwrap();
        // --goose-log too, checked without configuring, which opens it.
        parse(&[
            "--worker",
            "--manager-host",
            "m",
            "--goose-log",
            "unused.log",
        ])
        .worker_refusals()
        .unwrap();
    }

    /// Every option is classified for a worker: one a worker keeps, a
    /// manager's Gaggle option `validate_gaggle` refuses on a worker, or
    /// refused by `worker_refusals`, which sees an option as a configuration
    /// field named as its long flag. Every flag is such a field, and every
    /// field such a flag, so a new option is refused on a worker until it
    /// is listed as kept.
    #[test]
    fn every_option_is_classified_for_a_worker() {
        let fields = match serde_json::to_value(GooseConfiguration::default()) {
            Ok(serde_json::Value::Object(fields)) => fields,
            other => panic!("{:?}", other),
        };
        let flags: Vec<String> = <GooseConfiguration as clap::CommandFactory>::command()
            .get_arguments()
            .map(|arg| {
                format!(
                    "--{}",
                    arg.get_long().expect("every option has a long flag")
                )
            })
            .collect();
        let mut wrong = Vec::new();
        for flag in &flags {
            let field = flag[2..].replace('-', "_");
            // Help and version are clap's own, never fields.
            if !fields.contains_key(&field) && !["--help", "--version"].contains(&flag.as_str()) {
                wrong.push(format!("{flag} is no field"));
            }
            if WORKER_KEEPS.contains(&flag.as_str()) && MANAGER_ONLY.contains(&flag.as_str()) {
                wrong.push(format!("{flag} is both kept and the manager's"));
            }
        }
        for field in fields.keys() {
            if !flags.contains(&format!("--{}", field.replace('_', "-"))) {
                wrong.push(format!("{field} is no flag"));
            }
        }
        for listed in WORKER_KEEPS.iter().chain(MANAGER_ONLY) {
            if !flags.iter().any(|flag| flag == listed) {
                wrong.push(format!("{listed} is listed but no flag"));
            }
        }
        assert!(wrong.is_empty(), "{:?}", wrong);
        // Nothing set, nothing refused.
        assert_eq!(
            GooseConfiguration::default().worker_refused(),
            Vec::<String>::new()
        );
    }

    /// The token never shows in `config` (the `Debug` output) or
    /// `config-json`, nor when it comes from the environment. The process
    /// tests check no log line holds it.
    #[test]
    #[serial_test::serial]
    fn gaggle_token_never_shows() {
        std::env::remove_var(TOKEN_ENV);
        let token = "tok-7f3a-never-shown";
        let mut configuration = parse(&[
            "--manager",
            "--expect-workers",
            "1",
            "--host",
            "http://h",
            "--gaggle-token",
            token,
        ]);
        configuration.configure(&GooseDefaults::default()).unwrap();
        configuration.validate().unwrap();
        assert_eq!(configuration.gaggle_token(), Some(token));
        let shown = configuration.shown();
        let debug = format!("{:?}", shown);
        let json = serde_json::to_string(&shown).unwrap();
        for output in [&debug, &json] {
            assert!(!output.contains(token), "{}", output);
            assert!(output.contains("gaggle_token"), "{}", output);
        }
        // From the environment.
        std::env::set_var(TOKEN_ENV, token);
        let mut configuration = parse(&["--worker", "--manager-host", "m"]);
        configuration.configure(&GooseDefaults::default()).unwrap();
        std::env::remove_var(TOKEN_ENV);
        assert_eq!(configuration.gaggle_token(), Some(token));
        assert!(!format!("{:?}", configuration).contains(token));
        // A run that is neither ignores the environment.
        std::env::set_var(TOKEN_ENV, token);
        let mut configuration = parse(&["--host", "http://h"]);
        configuration.configure(&GooseDefaults::default()).unwrap();
        std::env::remove_var(TOKEN_ENV);
        assert_eq!(configuration.gaggle_token(), None);
        configuration.validate().unwrap();
    }
}
