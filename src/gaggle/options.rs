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

/// The environment variable a token is read from when `--gaggle-token` is not
/// given.
pub(crate) const TOKEN_ENV: &str = "GOOSE_GAGGLE_TOKEN";

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
                .clone()
                .filter(|host| !host.is_empty())
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
        let set = |value: &Option<String>| value.is_some();
        let refused_options: [(&str, bool); 53] = [
            ("--host", set(&self.host)),
            ("--users", self.users.is_some()),
            ("--increase-rate", set(&self.increase_rate)),
            ("--increase-time", set(&self.increase_time)),
            ("--run-time", set(&self.run_time)),
            ("--decrease-rate", set(&self.decrease_rate)),
            ("--decrease-time", set(&self.decrease_time)),
            ("--iterations", self.iterations.is_some()),
            ("--test-plan", self.test_plan.is_some()),
            ("--scenarios", self.scenarios.is_some()),
            ("--throttle-requests", self.throttle_requests.is_some()),
            ("--running-metrics", self.running_metrics.is_some()),
            ("--report-file", !self.report_file.is_empty()),
            ("--compress", self.compress.is_some()),
            ("--notes", set(&self.notes)),
            ("--report-graph-data", self.report_graph_data),
            ("--render-report", set(&self.render_report)),
            ("--baseline-file", set(&self.baseline_file)),
            ("--no-granular-report", self.no_granular_report),
            ("--pdf-print-html", set(&self.pdf_print_html)),
            ("--no-reset-metrics", self.no_reset_metrics),
            ("--no-metrics", self.no_metrics),
            ("--no-transaction-metrics", self.no_transaction_metrics),
            ("--no-scenario-metrics", self.no_scenario_metrics),
            ("--no-status-codes", self.no_status_codes),
            ("--co-mitigation", self.co_mitigation.is_some()),
            ("--threshold", !self.threshold.is_empty()),
            ("--no-threshold-abort", self.no_threshold_abort),
            ("--threshold-abort-delay", set(&self.threshold_abort_delay)),
            ("--request-log", set(&self.request_log)),
            ("--request-format", self.request_format.is_some()),
            ("--request-body", self.request_body),
            ("--transaction-log", set(&self.transaction_log)),
            ("--transaction-format", self.transaction_format.is_some()),
            ("--scenario-log", set(&self.scenario_log)),
            ("--scenario-format", self.scenario_format.is_some()),
            ("--error-log", set(&self.error_log)),
            ("--error-format", self.error_format.is_some()),
            ("--debug-log", set(&self.debug_log)),
            ("--debug-format", self.debug_format.is_some()),
            ("--no-debug-body", self.no_debug_body),
            ("--dashboard", self.dashboard),
            ("--dashboard-host", set(&self.dashboard_host)),
            ("--dashboard-port", self.dashboard_port.is_some()),
            ("--dashboard-auth-token", set(&self.dashboard_auth_token)),
            ("--dashboard-control", self.dashboard_control),
            ("--dashboard-no-auth", self.dashboard_no_auth),
            ("--no-percentiles-over-time", self.no_percentiles_over_time),
            ("--no-autostart", self.no_autostart),
            ("--timeout", set(&self.timeout)),
            ("--no-gzip", self.no_gzip),
            ("--sticky-follow", self.sticky_follow),
            ("--accept-invalid-certs", self.accept_invalid_certs),
        ];
        if let Some((option, _)) = refused_options.iter().find(|(_, set)| *set) {
            return Err(refused(
                option,
                "set",
                format!(
                    "{option} can't be used with --worker: the manager gives every option that shapes the load, the metrics, the reports and the logs. A worker keeps --output-dir, --goose-log and verbosity."
                ),
            ));
        }
        #[cfg(feature = "pdf-reports")]
        if self.pdf_scale.is_some() || self.pdf_timeout.is_some() {
            return Err(refused(
                "--pdf-scale",
                "set",
                "--pdf-scale and --pdf-timeout can't be used with --worker: the manager writes the reports.",
            ));
        }
        Ok(())
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
        timespan(
            "--gaggle-heartbeat-timeout",
            &self.gaggle_heartbeat_timeout,
            DEFAULT_HEARTBEAT_TIMEOUT_SECS,
        )?;
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
            let workers = self.expect_workers.unwrap_or(1);
            let throttle = self.throttle_requests();
            if throttle > 0 && throttle < workers {
                return Err(refused(
                    "--throttle-requests",
                    throttle,
                    format!(
                        "--throttle-requests {throttle} is fewer than the {workers} workers: each worker needs at least 1 request per second."
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

    /// B3: every combination A3 refuses, with its message.
    #[test]
    fn gaggle_refused_option_combinations() {
        crate::gaggle::within(30, || {
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
                    with(&manager, &["--throttle-requests", "1"]),
                    "is fewer than the 2 workers",
                ),
                (
                    with(&manager, &["--gaggle-shadow-merge"]),
                    "--gaggle-shadow-merge can't be used with --manager or --worker",
                ),
                (
                    with(&manager, &["--gaggle-heartbeat-timeout", "0"]),
                    "--gaggle-heartbeat-timeout must be a time of at least one second",
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
                    "--no-hash-check needs --manager or --worker",
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
            // What a worker keeps.
            let mut kept = parse(
                &with(
                    &worker,
                    &["--output-dir", "o", "--goose-log", "g.log", "-v"],
                )
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            );
            kept.worker_refusals().unwrap();
            kept.configure(&GooseDefaults::default()).unwrap();
            kept.validate().unwrap();
        });
    }

    /// B3: the token never shows in `config` (the `Debug` output) or
    /// `config-json`, nor when it comes from the environment. The process
    /// tests check no log line holds it.
    #[test]
    #[serial_test::serial]
    fn gaggle_token_never_shows() {
        crate::gaggle::within(30, || {
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
        });
    }
}
