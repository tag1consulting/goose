//! Functions and structures related to configuring a Goose load test.
//!
//! Goose can be configured at run time by passing in the options and flags defined by
//! the [`GooseConfiguration`] structure.
//!
//! Goose can be configured programmatically with [`GooseDefaultType::set_default`].

use clap::builder::{PossibleValue, PossibleValuesParser, TypedValueParser};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Args, FromArgMatches};
use serde::{Deserialize, Serialize};
use simplelog::*;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use crate::logger::GooseLogFormat;
use crate::metrics::GooseCoordinatedOmissionMitigation;
use crate::test_plan::TestPlan;
use crate::util;
use crate::{GooseAttack, GooseError};

/// The PDF scale factor when neither `--pdf-scale` nor [`GooseDefault::PdfScale`] sets one.
#[cfg(feature = "pdf-reports")]
const DEFAULT_PDF_SCALE: f64 = 0.8;

/// The PDF generation timeout, in seconds, when neither `--pdf-timeout` nor
/// [`GooseDefault::PdfTimeout`] sets one.
#[cfg(feature = "pdf-reports")]
const DEFAULT_PDF_TIMEOUT: u64 = 60;

/// Runtime options available when launching a Goose load test.
///
/// Custom defaults can be programmatically set for most of these options using
/// [`GooseDefault`] with [`GooseDefaultType::set_default`].
///
/// [Help is generated for all of these options](https://book.goose.rs/getting-started/runtime-options.html)
/// by passing `-h` (a summary) or `--help` (the full reference) to an application built
/// with the Goose Library.
///
/// Goose leverages [`clap`](https://docs.rs/clap/) to derive that help from the structure
/// below: the first paragraph of each field's documentation is its `-h` summary, and the
/// whole is its `--help` text. Build a configuration with
/// [`GooseConfiguration::parse_args_default_or_exit`] or
/// [`GooseConfiguration::parse_args_default`]: an option that is not on the command
/// line takes its [`Default`] value, which Goose then resolves against any
/// [`GooseDefault`] before falling back to its built in default.
///
/// Options that take a value and have no `Option` or `Vec` type are marked
/// `required = false` so that clap leaves them at their [`Default`] value when absent.
#[derive(Args, Debug, Clone, Default, Serialize, Deserialize)]
pub struct GooseConfiguration {
    /// Target host, such as http://10.21.32.33
    ///
    /// The scheme, host and optional port each request's path is joined to. It overrides a
    /// host set with Scenario::set_host or GooseDefault::Host, and is required unless one
    /// of those gives every scenario a host. A request with a full URL ignores it.
    ///
    /// Example: -H http://localhost:8080. Set in code with GooseDefault::Host.
    #[arg(
        help_heading = "Load test",
        short = 'H',
        long,
        value_name = "URL",
        required = false
    )]
    pub host: String,
    /// Concurrent users [default: one per CPU]
    ///
    /// How many users to start. Without it, Goose starts one user for each CPU available
    /// to it. At least 1, and at least 2 with --increase-time or --decrease-time. Not
    /// allowed with --test-plan, which sets its own user counts.
    ///
    /// Example: -u 100. Set in code with GooseDefault::Users.
    #[arg(help_heading = "Load test", short = 'u', long, value_name = "N")]
    pub users: Option<usize>,
    /// Users started per second [default: 1]
    ///
    /// How fast to start users. A decimal is allowed: 0.5 starts one user every two
    /// seconds. The slowest rate is one user a day. Not allowed with --increase-time or
    /// --test-plan.
    ///
    /// Example: -r 10. Set in code with GooseDefault::IncreaseRate.
    #[arg(help_heading = "Load test", short = 'r', long, value_name = "RATE")]
    pub increase_rate: Option<String>,
    /// Time to start all users, such as 30s
    ///
    /// Starts the users evenly over this time, in place of --increase-rate. TIME is a whole
    /// number of seconds, or hours, minutes and seconds in that order, such as 1h30m or
    /// 2m30s. Needs at least 2 users. Not allowed with --increase-rate or --test-plan.
    ///
    /// Example: -s 2m. Set in code with GooseDefault::IncreaseTime, in seconds.
    #[arg(
        help_heading = "Load test",
        short = 's',
        long,
        value_name = "TIME",
        required = false
    )]
    pub increase_time: String,
    /// Run this long after all users start
    ///
    /// How long to keep every user running once all have started; starting and stopping
    /// them takes extra time. Without it, the load test runs until it is stopped, or until
    /// --iterations completes. TIME is as for --increase-time. When it ends, users stop as
    /// --decrease-rate or --decrease-time set, otherwise all at once. Not allowed with
    /// --iterations or --test-plan.
    ///
    /// Example: -t 20m. Set in code with GooseDefault::RunTime, in seconds.
    #[arg(
        help_heading = "Load test",
        short = 't',
        long,
        value_name = "TIME",
        required = false
    )]
    pub run_time: String,
    /// Users stopped per second
    ///
    /// How fast to stop users at the end of the load test. A decimal is allowed, as for
    /// --increase-rate. Without it or --decrease-time, every user stops at once. Not
    /// allowed with --decrease-time or --test-plan.
    ///
    /// Example: --decrease-rate 5. Set in code with GooseDefault::DecreaseRate.
    #[arg(help_heading = "Load test", long, value_name = "RATE")]
    pub decrease_rate: Option<String>,
    /// Time to stop all users, such as 30s
    ///
    /// Stops the users evenly over this time at the end of the load test, in place of
    /// --decrease-rate. TIME is as for --increase-time. Needs at least 2 users. Not allowed
    /// with --decrease-rate or --test-plan.
    ///
    /// Example: --decrease-time 1m. Set in code with GooseDefault::DecreaseTime, in seconds.
    #[arg(
        help_heading = "Load test",
        long,
        value_name = "TIME",
        required = false
    )]
    pub decrease_time: String,
    /// Each user runs its scenario N times
    ///
    /// Each user stops after running its scenario N times, and the load test ends once
    /// every user has stopped. 0, the default, means no limit. Not allowed with
    /// --run-time, --test-plan or --no-reset-metrics.
    ///
    /// Example: --iterations 5. Set in code with GooseDefault::Iterations.
    #[arg(help_heading = "Load test", long, value_name = "N", required = false)]
    pub iterations: usize,
    /// Load shape, such as "10,60s;0,30s"
    ///
    /// Steps separated by semicolons, each a number of users and the time to reach it:
    /// "10,60s;0,30s" starts 10 users over 60 seconds, then stops them over 30 seconds. A
    /// step that keeps the number of users holds them for its time. TIME is as for
    /// --increase-time. Not allowed with --users, --increase-rate, --increase-time,
    /// --decrease-rate, --decrease-time, --run-time, --iterations or --no-reset-metrics.
    ///
    /// Example: --test-plan "10,1m;10,5m;0,30s". Set in code with GooseDefault::TestPlan.
    #[arg(help_heading = "Load test", long, value_name = "PLAN")]
    pub(crate) test_plan: Option<TestPlan>,
    /// Only run the named scenarios
    ///
    /// A comma separated list of scenario machine names, as --scenarios-list prints them.
    /// Matching ignores case, and * matches any run of characters. Without it, every
    /// scenario runs.
    ///
    /// Example: --scenarios "checkout,browse*". Set in code with GooseDefault::Scenarios.
    #[arg(
        help_heading = "Load test",
        long,
        value_name = "LIST",
        required = false
    )]
    pub scenarios: Scenarios,
    /// Cap total requests per second
    ///
    /// The most requests per second all users together may make. 0, the default, means no
    /// cap.
    ///
    /// Example: --throttle-requests 100. Set in code with GooseDefault::ThrottleRequests.
    #[arg(help_heading = "Load test", long, value_name = "N", required = false)]
    pub throttle_requests: usize,

    /// Print the Goose library version
    ///
    /// Prints the name and version of the Goose library the load test was built with, then
    /// exits.
    #[arg(help_heading = "Information", short = 'V', long)]
    pub version: bool,
    /// List scenarios and their transactions
    ///
    /// Prints each scenario and its transactions, with their weights, then exits without
    /// load testing.
    #[arg(help_heading = "Information", short = 'l', long)]
    pub list: bool,
    /// List scenario names, then exit
    ///
    /// Prints the machine name of each scenario, which --scenarios matches, and its full
    /// name, then exits without load testing.
    #[arg(help_heading = "Information", long)]
    pub scenarios_list: bool,

    /// More console output (-v, -vv)
    ///
    /// The console shows info messages by default. -v adds debug messages and -vv adds
    /// trace messages. Not allowed with --quiet.
    ///
    /// Example: -vv. Set in code with GooseDefault::Verbose.
    #[arg(
        help_heading = "Console and Goose log",
        short = 'v',
        long,
        action = clap::ArgAction::Count
    )]
    pub verbose: u8,
    /// Less console output, warnings only
    ///
    /// The console shows only warnings and errors, leaving out info messages; -qq is the
    /// same as -q. Not allowed with --verbose.
    ///
    /// Example: -q. Set in code with GooseDefault::Quiet.
    #[arg(
        help_heading = "Console and Goose log",
        short = 'q',
        long,
        action = clap::ArgAction::Count
    )]
    pub quiet: u8,
    /// Write the Goose log to FILE
    ///
    /// Writes Goose's own log messages to FILE, at the detail --log-level sets. Without it,
    /// there is no Goose log.
    ///
    /// Example: -G goose.log. Set in code with GooseDefault::GooseLog.
    #[arg(
        help_heading = "Console and Goose log",
        short = 'G',
        long,
        value_name = "FILE",
        required = false
    )]
    pub goose_log: String,
    /// More Goose log detail (-g, -gg)
    ///
    /// The Goose log that --goose-log writes holds warnings and errors by default. -g adds
    /// info messages, -gg debug messages and -ggg trace messages.
    ///
    /// Example: -G goose.log -gg. Set in code with GooseDefault::LogLevel.
    #[arg(
        help_heading = "Console and Goose log",
        short = 'g',
        long,
        action = clap::ArgAction::Count
    )]
    pub log_level: u8,
    /// Print running metrics every N seconds
    ///
    /// Prints the metrics so far to the console every N seconds while the load test runs.
    /// Without it, metrics are printed only at the end.
    ///
    /// Example: --running-metrics 30. Set in code with GooseDefault::RunningMetrics.
    #[arg(help_heading = "Console and Goose log", long, value_name = "N")]
    pub running_metrics: Option<usize>,
    /// Don't print metrics at the end
    ///
    /// Leaves the metrics out of the console when the load test ends. Reports and the saved
    /// run are still written.
    ///
    /// Set in code with GooseDefault::NoPrintMetrics.
    #[arg(help_heading = "Console and Goose log", long)]
    pub no_print_metrics: bool,
    /// Don't collect the error summary
    ///
    /// Leaves the summary of errors out of the metrics, on the console and in reports. The
    /// error log, --error-log, is still written.
    ///
    /// Set in code with GooseDefault::NoErrorSummary.
    #[arg(help_heading = "Console and Goose log", long)]
    pub no_error_summary: bool,

    /// Write a report; repeatable
    ///
    /// Writes a report of the load test to FILE when it ends. The extension picks the type:
    /// .html or .htm, .md, .json, or .pdf with the pdf-reports feature. Repeat the option
    /// to write several. Not allowed with --no-metrics.
    ///
    /// Example: --report-file report.html --report-file report.json. Set in code with
    /// GooseDefault::ReportFile.
    #[arg(help_heading = "Reports", long, value_name = "FILE")]
    pub report_file: Vec<String>,
    /// Don't save this run to the runs folder
    ///
    /// By default, Goose saves each run's reports to its own folder under --runs-dir, where
    /// --baseline-file can find it later. --report-file still writes its reports.
    /// --no-metrics also turns saving off.
    ///
    /// Set in code with GooseDefault::NoSave.
    #[arg(help_heading = "Reports", long)]
    pub no_save: bool,
    /// Runs folder [default: goose-runs]
    ///
    /// The folder saved runs are written to. If a folder given here cannot be created, the
    /// load test does not start; if the default cannot, Goose only warns.
    ///
    /// Example: --runs-dir /var/tmp/goose-runs. Set in code with GooseDefault::RunsDir.
    #[arg(help_heading = "Reports", long, value_name = "DIR", required = false)]
    pub runs_dir: String,
    /// Compare against an earlier run
    ///
    /// PATH is a JSON report written with --report-file, or the folder of a saved run.
    /// Every report then shows how each metric changed from that run.
    ///
    /// Example: --baseline-file baseline.json. Set in code with GooseDefault::BaselineFile.
    #[arg(help_heading = "Reports", long, value_name = "PATH")]
    pub baseline_file: Option<String>,
    /// Graph only totals in reports
    ///
    /// Report graphs show a line for the total and, by default, one more for each request,
    /// transaction or scenario. This leaves out the extra lines. Needs --report-file or a
    /// saved run.
    ///
    /// Set in code with GooseDefault::NoGranularData.
    #[arg(help_heading = "Reports", long)]
    pub no_granular_report: bool,
    /// Write print-ready HTML for PDF
    ///
    /// Writes the HTML report, laid out for printing, to FILE, to turn into a PDF in any
    /// browser. Needs neither Chrome nor the pdf-reports feature.
    ///
    /// Example: --pdf-print-html report-print.html.
    #[arg(help_heading = "Reports", long, value_name = "FILE", required = false)]
    pub pdf_print_html: String,
    /// PDF scale, 0.1 to 2.0 [default: 0.8]
    ///
    /// How much Chrome scales the pages of a .pdf --report-file. Only with the pdf-reports
    /// feature.
    ///
    /// Example: --pdf-scale 1.2. Set in code with GooseDefault::PdfScale.
    #[cfg(feature = "pdf-reports")]
    #[arg(help_heading = "Reports", long, value_name = "SCALE")]
    pub pdf_scale: Option<f64>,
    /// PDF timeout, 10 to 600 s [default: 60]
    ///
    /// Seconds to wait for Chrome to start and to render a .pdf --report-file. Only with
    /// the pdf-reports feature.
    ///
    /// Example: --pdf-timeout 120. Set in code with GooseDefault::PdfTimeout.
    #[cfg(feature = "pdf-reports")]
    #[arg(help_heading = "Reports", long, value_name = "SECS")]
    pub pdf_timeout: Option<u64>,

    /// Keep metrics from the ramp up
    ///
    /// By default, Goose resets the metrics once every user has started, so they cover
    /// only the full load. This keeps what was gathered while users were starting. Not
    /// allowed with --test-plan or --iterations.
    ///
    /// Set in code with GooseDefault::NoResetMetrics.
    #[arg(help_heading = "Metrics", long)]
    pub no_reset_metrics: bool,
    /// Don't collect any metrics
    ///
    /// Turns off every metric, and with them saving the run. Not allowed with
    /// --report-file, --request-log, --transaction-log, --scenario-log, --error-log or
    /// --co-mitigation.
    ///
    /// Set in code with GooseDefault::NoMetrics.
    #[arg(help_heading = "Metrics", long)]
    pub no_metrics: bool,
    /// Don't collect transaction metrics
    ///
    /// Leaves out the metrics for each transaction, keeping those for requests and
    /// scenarios.
    ///
    /// Set in code with GooseDefault::NoTransactionMetrics.
    #[arg(help_heading = "Metrics", long)]
    pub no_transaction_metrics: bool,
    /// Don't collect scenario metrics
    ///
    /// Leaves out the metrics for each scenario, keeping those for requests and
    /// transactions.
    ///
    /// Set in code with GooseDefault::NoScenarioMetrics.
    #[arg(help_heading = "Metrics", long)]
    pub no_scenario_metrics: bool,
    /// Don't collect status code metrics
    ///
    /// Leaves out the count of each HTTP status code.
    ///
    /// Set in code with GooseDefault::NoStatusCodes.
    #[arg(help_heading = "Metrics", long)]
    pub no_status_codes: bool,
    /// Coordinated omission mitigation
    ///
    /// Experimental. While the server stalls, users send fewer requests, so the stall is
    /// under counted. Mitigation backfills the requests each user would have sent at its
    /// usual cadence. MODE is average, maximum or minimum, the cadence to backfill at, or
    /// disabled, the default. Not allowed with --no-metrics.
    ///
    /// Example: --co-mitigation average. Set in code with
    /// GooseDefault::CoordinatedOmissionMitigation.
    #[arg(
        help_heading = "Metrics",
        long,
        value_name = "MODE",
        value_parser = co_mitigation_parser(),
        ignore_case = true,
        hide_possible_values = true
    )]
    pub co_mitigation: Option<GooseCoordinatedOmissionMitigation>,

    /// Log every request to FILE
    ///
    /// Writes a line for each request, in --request-format. Not allowed with --no-metrics.
    ///
    /// Example: -R requests.log. Set in code with GooseDefault::RequestLog.
    #[arg(
        help_heading = "Logs",
        short = 'R',
        long,
        value_name = "FILE",
        required = false
    )]
    pub request_log: String,
    /// Request log format
    ///
    /// FMT is json, the default, csv, raw or pretty.
    ///
    /// Example: --request-format csv. Set in code with GooseDefault::RequestFormat.
    #[arg(
        help_heading = "Logs",
        long,
        value_name = "FMT",
        value_parser = log_format_parser(),
        ignore_case = true,
        hide_possible_values = true
    )]
    pub request_format: Option<GooseLogFormat>,
    /// Add request bodies to the request log
    ///
    /// Includes the body of each request in --request-log.
    ///
    /// Set in code with GooseDefault::RequestBody.
    #[arg(help_heading = "Logs", long)]
    pub request_body: bool,
    /// Log every transaction to FILE
    ///
    /// Writes a line for each transaction, in --transaction-format. Not allowed with
    /// --no-metrics.
    ///
    /// Example: -T transactions.log. Set in code with GooseDefault::TransactionLog.
    #[arg(
        help_heading = "Logs",
        short = 'T',
        long,
        value_name = "FILE",
        required = false
    )]
    pub transaction_log: String,
    /// Transaction log format
    ///
    /// FMT is json, the default, csv, raw or pretty.
    ///
    /// Example: --transaction-format csv. Set in code with GooseDefault::TransactionFormat.
    #[arg(
        help_heading = "Logs",
        long,
        value_name = "FMT",
        value_parser = log_format_parser(),
        ignore_case = true,
        hide_possible_values = true
    )]
    pub transaction_format: Option<GooseLogFormat>,
    /// Log every scenario to FILE
    ///
    /// Writes a line for each run of a scenario, in --scenario-format. Not allowed with
    /// --no-metrics.
    ///
    /// Example: -S scenarios.log. Set in code with GooseDefault::ScenarioLog.
    #[arg(
        help_heading = "Logs",
        short = 'S',
        long,
        value_name = "FILE",
        required = false
    )]
    pub scenario_log: String,
    /// Scenario log format
    ///
    /// FMT is json, the default, csv, raw or pretty.
    ///
    /// Example: --scenario-format csv. Set in code with GooseDefault::ScenarioFormat.
    #[arg(
        help_heading = "Logs",
        long,
        value_name = "FMT",
        value_parser = log_format_parser(),
        ignore_case = true,
        hide_possible_values = true
    )]
    pub scenario_format: Option<GooseLogFormat>,
    /// Log every error to FILE
    ///
    /// Writes a line for each failed request, in --error-format. Not allowed with
    /// --no-metrics.
    ///
    /// Example: -E errors.log. Set in code with GooseDefault::ErrorLog.
    #[arg(
        help_heading = "Logs",
        short = 'E',
        long,
        value_name = "FILE",
        required = false
    )]
    pub error_log: String,
    /// Error log format
    ///
    /// FMT is json, the default, csv, raw or pretty.
    ///
    /// Example: --error-format csv. Set in code with GooseDefault::ErrorFormat.
    #[arg(
        help_heading = "Logs",
        long,
        value_name = "FMT",
        value_parser = log_format_parser(),
        ignore_case = true,
        hide_possible_values = true
    )]
    pub error_format: Option<GooseLogFormat>,
    /// Write the debug log to FILE
    ///
    /// Writes what the load test logs with GooseUser::log_debug and GooseUser::set_failure,
    /// with response bodies unless --no-debug-body, in --debug-format.
    ///
    /// Example: -D debug.log. Set in code with GooseDefault::DebugLog.
    #[arg(
        help_heading = "Logs",
        short = 'D',
        long,
        value_name = "FILE",
        required = false
    )]
    pub debug_log: String,
    /// Debug log format
    ///
    /// FMT is json, the default, csv, raw or pretty.
    ///
    /// Example: --debug-format pretty. Set in code with GooseDefault::DebugFormat.
    #[arg(
        help_heading = "Logs",
        long,
        value_name = "FMT",
        value_parser = log_format_parser(),
        ignore_case = true,
        hide_possible_values = true
    )]
    pub debug_format: Option<GooseLogFormat>,
    /// Leave response bodies out of debug log
    ///
    /// Writes --debug-log without response bodies, which keeps it small.
    ///
    /// Set in code with GooseDefault::NoDebugBody.
    #[arg(help_heading = "Logs", long)]
    pub no_debug_body: bool,

    /// Serve the live web dashboard
    ///
    /// Serves a page that charts the load test as it runs, on 127.0.0.1 port 5118 unless
    /// --dashboard-host or --dashboard-port say otherwise. Needs Goose built with the
    /// dashboard feature.
    ///
    /// Set in code with GooseDefault::Dashboard.
    #[arg(help_heading = "Dashboard", long)]
    pub dashboard: bool,
    /// Bind host [default: 127.0.0.1]
    ///
    /// The address the dashboard listens on, with --dashboard. An address that is not a
    /// loopback address needs --dashboard-auth-token.
    ///
    /// Example: --dashboard-host 0.0.0.0. Set in code with GooseDefault::DashboardHost.
    #[arg(
        help_heading = "Dashboard",
        long,
        value_name = "HOST",
        required = false
    )]
    pub dashboard_host: String,
    /// Bind port [default: 5118]
    ///
    /// The TCP port the dashboard listens on, with --dashboard.
    ///
    /// Example: --dashboard-port 8088. Set in code with GooseDefault::DashboardPort.
    #[arg(
        help_heading = "Dashboard",
        long,
        value_name = "PORT",
        required = false
    )]
    pub dashboard_port: u16,
    /// Auth token
    ///
    /// The token a dashboard client must send, as an Authorization: Bearer header or a
    /// token query parameter. Needed when --dashboard-host is not a loopback address, and
    /// always with --dashboard-control.
    ///
    /// Set in code with GooseDefault::DashboardAuthToken.
    #[arg(
        help_heading = "Dashboard",
        long,
        value_name = "TOKEN",
        required = false
    )]
    pub dashboard_auth_token: String,
    /// Allow control from the dashboard
    ///
    /// Lets the dashboard start, stop and quit the load test and change its users. Needs
    /// --dashboard and --dashboard-auth-token, even on a loopback address.
    ///
    /// Set in code with GooseDefault::DashboardControl.
    #[arg(help_heading = "Dashboard", long)]
    pub dashboard_control: bool,
    /// Max live clients [default: 32]
    ///
    /// The most dashboard pages that may follow the live stream at once.
    ///
    /// Example: --dashboard-max-clients 4. Set in code with GooseDefault::DashboardMaxClients.
    #[arg(help_heading = "Dashboard", long, value_name = "N", required = false)]
    pub dashboard_max_clients: u32,
    /// Don't track per-second response times
    ///
    /// Leaves out the response time percentiles for each second behind the dashboard's
    /// response time heatmap, which saves memory. Needs --dashboard.
    ///
    /// Set in code with GooseDefault::NoPercentilesOverTime.
    #[arg(help_heading = "Dashboard", long)]
    pub no_percentiles_over_time: bool,

    /// Wait for a Controller to start the test
    ///
    /// Starts idle and waits for a start command from the telnet or WebSocket Controller,
    /// or from the dashboard with --dashboard-control. Needs one of them enabled.
    ///
    /// Set in code with GooseDefault::NoAutoStart.
    #[arg(help_heading = "Controllers", long)]
    pub no_autostart: bool,
    /// Disable the telnet Controller
    ///
    /// By default, Goose accepts Controller commands over telnet, on --telnet-host and
    /// --telnet-port.
    ///
    /// Set in code with GooseDefault::NoTelnet.
    #[arg(help_heading = "Controllers", long)]
    pub no_telnet: bool,
    /// Telnet host [default: 0.0.0.0]
    ///
    /// The address the telnet Controller listens on.
    ///
    /// Example: --telnet-host 127.0.0.1. Set in code with GooseDefault::TelnetHost.
    #[arg(
        help_heading = "Controllers",
        long,
        value_name = "HOST",
        required = false
    )]
    pub telnet_host: String,
    /// Telnet port [default: 5116]
    ///
    /// The TCP port the telnet Controller listens on.
    ///
    /// Example: --telnet-port 6116. Set in code with GooseDefault::TelnetPort.
    #[arg(
        help_heading = "Controllers",
        long,
        value_name = "PORT",
        required = false
    )]
    pub telnet_port: u16,
    /// Disable the WebSocket Controller
    ///
    /// By default, Goose accepts Controller commands over WebSocket, on --websocket-host
    /// and --websocket-port.
    ///
    /// Set in code with GooseDefault::NoWebSocket.
    #[arg(help_heading = "Controllers", long)]
    pub no_websocket: bool,
    /// WebSocket host [default: 0.0.0.0]
    ///
    /// The address the WebSocket Controller listens on.
    ///
    /// Example: --websocket-host 127.0.0.1. Set in code with GooseDefault::WebSocketHost.
    #[arg(
        help_heading = "Controllers",
        long,
        value_name = "HOST",
        required = false
    )]
    pub websocket_host: String,
    /// WebSocket port [default: 5117]
    ///
    /// The TCP port the WebSocket Controller listens on.
    ///
    /// Example: --websocket-port 6117. Set in code with GooseDefault::WebSocketPort.
    #[arg(
        help_heading = "Controllers",
        long,
        value_name = "PORT",
        required = false
    )]
    pub websocket_port: u16,

    /// Request timeout in seconds [default: 60]
    ///
    /// How long each request may take before it fails. Greater than 0, in whole seconds;
    /// a fraction is dropped.
    ///
    /// Example: --timeout 15. Set in code with GooseDefault::Timeout.
    #[arg(help_heading = "HTTP client", long, value_name = "SECS")]
    pub timeout: Option<String>,
    /// Don't send Accept-Encoding: gzip
    ///
    /// By default, requests ask for gzip compressed responses, which Goose decompresses.
    ///
    /// Set in code with GooseDefault::NoGzip.
    #[arg(help_heading = "HTTP client", long)]
    pub no_gzip: bool,
    /// Keep following the host's redirect
    ///
    /// When a request is redirected to another host, the user sends its later requests to
    /// that host too.
    ///
    /// Set in code with GooseDefault::StickyFollow.
    #[arg(help_heading = "HTTP client", long)]
    pub sticky_follow: bool,
    /// Accept invalid TLS certificates
    ///
    /// Skips TLS certificate validation, for test servers with self signed or expired
    /// certificates.
    ///
    /// Set in code with GooseDefault::AcceptInvalidCerts.
    #[arg(help_heading = "HTTP client", long)]
    pub accept_invalid_certs: bool,
}

/// The book page that documents every runtime option.
const RUNTIME_OPTIONS_URL: &str = "https://book.goose.rs/getting-started/runtime-options.html";

/// The help width when neither a terminal nor `COLUMNS` gives one.
const DEFAULT_HELP_WIDTH: usize = 80;

/// The widest Goose wraps help, so the long paragraphs stay readable on a wide terminal.
const MAX_HELP_WIDTH: usize = 120;

/// The width to wrap help to: the terminal's when stdout is one, otherwise `COLUMNS`
/// when it is set, otherwise 80, and never more than 120.
fn help_width() -> usize {
    help_width_from(
        terminal_size::terminal_size_of(std::io::stdout())
            .map(|(terminal_size::Width(width), _)| usize::from(width)),
        std::env::var("COLUMNS").ok(),
    )
}

/// The help width from the terminal's width and the value of `COLUMNS`, if any.
fn help_width_from(terminal: Option<usize>, columns: Option<String>) -> usize {
    terminal
        .filter(|width| *width > 0)
        .or_else(|| {
            columns
                .and_then(|columns| columns.trim().parse().ok())
                .filter(|width| *width > 0)
        })
        .unwrap_or(DEFAULT_HELP_WIDTH)
        .min(MAX_HELP_WIDTH)
}

/// The name of the running load test, as help and errors show it.
fn program_name() -> String {
    std::env::args_os()
        .next()
        .as_deref()
        .map(std::path::Path::new)
        .and_then(std::path::Path::file_stem)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "loadtest".to_string())
}

/// The command line Goose parses, its help wrapped to `width` columns and its usage and
/// examples naming the load test `name`.
fn command(name: &str, width: usize) -> clap::Command {
    let examples = format!(
        "Examples:\n  \
         {name} -H http://localhost:8080 -u 10 -r 2 -t 5m\n  \
         {name} -H http://localhost:8080 -u 10 -r 2 -t 5m --report-file report.html"
    );
    let command = GooseConfiguration::augment_args(
        clap::Command::new(env!("CARGO_PKG_NAME"))
            .bin_name(name)
            .before_help(
                "Use -h for a summary of each option, --help for details, defaults and examples.",
            )
            .after_help(format!(
                "{examples}\n\n\
                 Run with --help for details, defaults and examples.\n\
                 {RUNTIME_OPTIONS_URL}"
            ))
            .after_long_help(format!(
                "{examples}\n\n\
                 Most options can also be set in code with GooseDefault; the command line \
                 wins.\n\
                 {RUNTIME_OPTIONS_URL}"
            ))
            .help_template(
                "{about}\n\n{usage-heading} {usage}\n\n{before-help}{all-args}{after-help}",
            )
            .term_width(width)
            .disable_help_flag(true)
            .disable_version_flag(true)
            // A repeated option keeps its last value.
            .args_override_self(true)
            // An option's value may begin with a hyphen.
            .allow_hyphen_values(true),
    );
    // Set after the derive, which would otherwise take the structure's documentation.
    let command = command
        .about("Goose: a distributed HTTP(S) load testing tool written in Rust")
        .long_about(None);
    // -h and --help lead the Information heading, ahead of -V.
    let order = command
        .get_arguments()
        .find(|arg| arg.get_id() == "version")
        .map(clap::Arg::get_display_order)
        .unwrap_or_default();
    command.arg(
        clap::Arg::new("help")
            .short('h')
            .long("help")
            .action(clap::ArgAction::Help)
            .help("Print help (--help for more)")
            .long_help(
                "Print help: -h prints a summary of each option, --help this full reference.",
            )
            .help_heading("Information")
            .display_order(order),
    )
}

/// The values `--co-mitigation` takes, and the abbreviations of each.
fn co_mitigation_parser() -> impl TypedValueParser<Value = GooseCoordinatedOmissionMitigation> {
    PossibleValuesParser::new([
        PossibleValue::new("average").aliases(["ave", "aver", "avg", "mean"]),
        PossibleValue::new("maximum").aliases(["ma", "max", "maxi"]),
        PossibleValue::new("minimum").aliases(["mi", "min", "mini"]),
        PossibleValue::new("disabled").aliases(["di", "dis", "disable", "none", "no"]),
    ])
    .map(|value| {
        value
            .parse()
            .expect("every possible value names a mitigation")
    })
}

/// The values the log format options take.
fn log_format_parser() -> impl TypedValueParser<Value = GooseLogFormat> {
    PossibleValuesParser::new([
        PossibleValue::new("json").alias("jsn"),
        PossibleValue::new("csv"),
        PossibleValue::new("raw"),
        PossibleValue::new("pretty"),
    ])
    .map(|value| value.parse().expect("every possible value names a format"))
}

impl GooseConfiguration {
    /// Parses the command line of the running program into a configuration.
    ///
    /// Prints help to stdout and exits with status 0 when `-h` or `--help` is passed.
    /// Prints an error and exits with status 2 when the command line is invalid.
    ///
    /// # Example
    /// ```rust,no_run
    /// use goose::config::GooseConfiguration;
    ///
    /// let configuration = GooseConfiguration::parse_args_default_or_exit();
    /// ```
    pub fn parse_args_default_or_exit() -> Self {
        let mut command = command(&program_name(), help_width());
        Self::parse_with(&mut command, std::env::args_os()).unwrap_or_else(|e| {
            if e.kind() == ErrorKind::DisplayHelp {
                // A closed pipe, as `| head` leaves, is not an error.
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(tidy_help(&e.to_string()).as_bytes());
                let _ = stdout.flush();
                std::process::exit(0);
            }
            e.exit()
        })
    }

    /// Parses `args` into a configuration. The program name is omitted from `args`.
    ///
    /// An invalid command line, or `-h` or `--help`, is a [`GooseError::InvalidOption`]
    /// naming the option, its value and why it was refused.
    ///
    /// # Example
    /// ```rust
    /// use goose::config::GooseConfiguration;
    ///
    /// let configuration =
    ///     GooseConfiguration::parse_args_default(&["--users", "4", "-t", "30s"]).unwrap();
    /// assert_eq!(configuration.users, Some(4));
    /// assert_eq!(configuration.run_time, "30s");
    /// ```
    pub fn parse_args_default<S: AsRef<str>>(args: &[S]) -> Result<Self, GooseError> {
        let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        let program = program_name();
        Self::parse_with(
            &mut command(&program, DEFAULT_HELP_WIDTH),
            std::iter::once(program.as_str()).chain(args.iter().copied()),
        )
        .map_err(|e| invalid_option(&e, &args))
    }

    /// Parses `args`, the program name first, leaving every option that is not on
    /// the command line at its [`Default`] value.
    fn parse_with<I, T>(command: &mut clap::Command, args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut matches = command.try_get_matches_from_mut(args)?;
        let mut configuration = Self::default();
        configuration
            .update_from_arg_matches_mut(&mut matches)
            .map_err(|e| e.format(command))?;
        Ok(configuration)
    }
}

/// Help as Goose prints it: clap's, without the spaces clap leaves at the end of the
/// blank lines between paragraphs, and with one closing newline.
fn tidy_help(help: &str) -> String {
    let mut tidy = help
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string();
    tidy.push('\n');
    tidy
}

/// A clap error as a [`GooseError::InvalidOption`]: the option and value clap names,
/// otherwise the arguments, and clap's message without its prefix, usage or tips.
fn invalid_option(error: &clap::Error, args: &[&str]) -> GooseError {
    let context = |kind| match error.get(kind) {
        Some(ContextValue::String(value)) => Some(value.clone()),
        _ => None,
    };
    let detail = match error.kind() {
        ErrorKind::DisplayHelp => "help was requested".to_string(),
        _ => {
            let message = error.to_string();
            let line = message.lines().next().unwrap_or_default();
            let line = line.strip_prefix("error: ").unwrap_or(line);
            // clap lists the valid values on a line of their own.
            match error.get(ContextKind::ValidValue) {
                Some(ContextValue::Strings(values)) if !values.is_empty() => {
                    format!("{line}; possible values: {}", values.join(", "))
                }
                _ => line.to_string(),
            }
        }
    };
    GooseError::InvalidOption {
        option: context(ContextKind::InvalidArg).unwrap_or_else(|| args.join(" ")),
        value: context(ContextKind::InvalidValue).unwrap_or_else(|| args.join(" ")),
        detail,
    }
}

/// Optionally defines a subset of active Scenarios to run during a load test.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Scenarios {
    pub active: Vec<String>,
}
/// Implement [`FromStr`] to convert `"foo,bar"` comma separated string to a vector of strings.
impl FromStr for Scenarios {
    type Err = GooseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Convert string into a vector of string.
        let mut active: Vec<String> = Vec::new();
        // Multiple Scenarios can be defined as a comma separated list.
        let lines = s.split(',');
        for line in lines {
            // Ignore white space an case.
            let scenario = line.trim().to_lowercase();
            // Valid scenario names are alphanumeric, with optional * wildcards.
            if scenario.chars().all(|c| c.is_alphanumeric() || c == '*') {
                active.push(scenario);
            } else {
                // Logger isn't initialized yet, provide helpful debug output.
                eprintln!("ERROR: invalid `configuration.scenarios` value: '{line}'");
                eprintln!("  Expected format: --scenarios \"{{one}},{{two}},{{three}}\"");
                eprintln!("    {{one}}, {{two}}, {{three}}, etc must be alphanumeric (use * for wildcard matching)");
                eprintln!("    To view valid scenario names invoke `--scenarios-list`");
                return Err(GooseError::InvalidOption {
                    option: "`configuration.scenarios".to_string(),
                    value: line.to_string(),
                    detail: "invalid `configuration.scenarios` value.".to_string(),
                });
            }
        }
        // The listed scenarios are only valid if the logic gets this far.
        Ok(Scenarios { active })
    }
}

/// Optional default values for Goose run-time options.
///
/// These custom defaults can be configured using [`GooseDefaultType::set_default()`].
#[derive(Clone, Debug, Default)]
pub(crate) struct GooseDefaults {
    /// An optional default host to run this load test against.
    pub host: Option<String>,
    /// An optional default number of users to simulate.
    pub users: Option<usize>,
    /// An optional default number of clients to start per second.
    pub increase_rate: Option<String>,
    /// An optional default number of seconds for the test to increase users.
    pub increase_time: Option<usize>,
    /// An optional default number of clients to stop per second.
    pub decrease_rate: Option<String>,
    /// An optional default number of seconds for the test to decrease users.
    pub decrease_time: Option<usize>,
    /// An optional default number of seconds for the test to run.
    pub run_time: Option<usize>,
    /// An optional default test plan.
    pub test_plan: Option<TestPlan>,
    /// An optional default test plan.
    pub iterations: Option<usize>,
    /// Optional default scenarios.
    pub scenarios: Option<Scenarios>,
    /// An optional default log level.
    pub log_level: Option<u8>,
    /// An optional default for the goose log file name.
    pub goose_log: Option<String>,
    /// An optional default value for quiet level.
    pub quiet: Option<u8>,
    /// An optional default value for verbosity level.
    pub verbose: Option<u8>,
    /// An optional default for printing running metrics.
    pub running_metrics: Option<usize>,
    /// An optional default for not resetting metrics after all users started.
    pub no_reset_metrics: Option<bool>,
    /// An optional default for not tracking metrics.
    pub no_metrics: Option<bool>,
    /// An optional default for not tracking transaction metrics.
    pub no_transaction_metrics: Option<bool>,
    /// An optional default for not tracking scenario metrics.
    pub no_scenario_metrics: Option<bool>,
    /// An optional default for not displaying metrics at the end of the load test.
    pub no_print_metrics: Option<bool>,
    /// An optional default for not displaying an error summary.
    pub no_error_summary: Option<bool>,
    /// An optional default for the html-formatted report file name.
    pub report_file: Option<Vec<String>>,
    /// An optional default for the baseline JSON report file name.
    #[allow(dead_code)]
    pub baseline_file: Option<String>,
    /// An optional default for the flag that disables granular data in HTML report graphs.
    pub no_granular_report: Option<bool>,
    /// An optional default for not saving the run to the runs directory.
    pub no_save: Option<bool>,
    /// An optional default for the runs directory saved runs are written to.
    pub runs_dir: Option<String>,
    /// An optional default for the requests log file name.
    pub request_log: Option<String>,
    /// An optional default for the requests log file format.
    pub request_format: Option<GooseLogFormat>,
    /// An optional default for logging the request body.
    pub request_body: Option<bool>,
    /// An optional default for the transaction log file name.
    pub transaction_log: Option<String>,
    /// An optional default for the transaction log file format.
    pub transaction_format: Option<GooseLogFormat>,
    /// An optional default for the scenario log file name.
    pub scenario_log: Option<String>,
    /// An optional default for the scenario log file format.
    pub scenario_format: Option<GooseLogFormat>,
    /// An optional default for the error log file name.
    pub error_log: Option<String>,
    /// An optional default for the error log format.
    pub error_format: Option<GooseLogFormat>,
    /// An optional default for the debug log file name.
    pub debug_log: Option<String>,
    /// An optional default for the debug log format.
    pub debug_format: Option<GooseLogFormat>,
    /// An optional default for not logging response body in debug log.
    pub no_debug_body: Option<bool>,
    /// An optional default for not enabling telnet Controller thread.
    pub no_telnet: Option<bool>,
    /// An optional default for not enabling WebSocket Controller thread.
    pub no_websocket: Option<bool>,
    /// An optional default for not auto-starting the load test.
    pub no_autostart: Option<bool>,
    /// An optional default for not setting the gzip Accept-Encoding header.
    pub no_gzip: Option<bool>,
    /// An optional default number of seconds to timeout requests.
    pub timeout: Option<String>,
    /// An optional default for coordinated omission mitigation.
    pub co_mitigation: Option<GooseCoordinatedOmissionMitigation>,
    /// An optional default to not track status code metrics.
    pub no_status_codes: Option<bool>,
    /// An optional default maximum requests per second.
    pub throttle_requests: Option<usize>,
    /// An optional default to follows base_url redirect with subsequent request.
    pub sticky_follow: Option<bool>,
    /// An optional default for host telnet Controller listens on.
    pub telnet_host: Option<String>,
    /// An optional default for port telnet Controller listens on.
    pub telnet_port: Option<u16>,
    /// An optional default for host WebSocket Controller listens on.
    pub websocket_host: Option<String>,
    /// An optional default for port WebSocket Controller listens on.
    pub websocket_port: Option<u16>,
    /// An optional default for enabling the live web dashboard.
    pub dashboard: Option<bool>,
    /// An optional default for enabling dashboard control endpoints.
    pub dashboard_control: Option<bool>,
    /// An optional default for host the dashboard listens on.
    pub dashboard_host: Option<String>,
    /// An optional default for port the dashboard listens on.
    pub dashboard_port: Option<u16>,
    /// An optional default for the dashboard auth token.
    pub dashboard_auth_token: Option<String>,
    /// An optional default for max concurrent dashboard SSE clients.
    pub dashboard_max_clients: Option<u32>,
    /// An optional default for not tracking response times by second.
    pub no_percentiles_over_time: Option<bool>,
    /// An optional default for not validating https certificates.
    pub accept_invalid_certs: Option<bool>,
    /// An optional default for the PDF scale factor.
    #[cfg(feature = "pdf-reports")]
    pub pdf_scale: Option<f64>,
    /// An optional default for PDF generation timeout (seconds).
    #[cfg(feature = "pdf-reports")]
    pub pdf_timeout: Option<u64>,
}

/// Defines all [`GooseConfiguration`] options that can be programmatically configured with
/// a custom default.
///
/// These custom defaults can be configured using [`GooseDefaultType::set_default()`].
#[derive(Debug)]
pub enum GooseDefault {
    /// An optional default host to run this load test against.
    Host,
    /// An optional default number of users to simulate.
    Users,
    /// An optional default number of clients to start per second.
    IncreaseRate,
    /// An optional default number of seconds for the test to increase users.
    IncreaseTime,
    /// An optional default number of clients to stop per second.
    DecreaseRate,
    /// An optional default number of seconds for the test to decrease users.
    DecreaseTime,
    /// An optional default number of seconds for the test to run.
    RunTime,
    /// An optional default test plan.
    TestPlan,
    /// An optional default number of iterations to run scenarios then exit.
    Iterations,
    /// Optional default list of scenarios to run.
    Scenarios,
    /// An optional default log level.
    LogLevel,
    /// An optional default for the log file name.
    GooseLog,
    /// An optional default value for quiet level.
    Quiet,
    /// An optional default value for verbosity level.
    Verbose,
    /// An optional default for printing running metrics.
    RunningMetrics,
    /// An optional default for not resetting metrics after all users started.
    NoResetMetrics,
    /// An optional default for not tracking metrics.
    NoMetrics,
    /// An optional default for not tracking transaction metrics.
    NoTransactionMetrics,
    /// An optional default for not tracking scneario metrics.
    NoScenarioMetrics,
    /// An optional default for not displaying metrics at end of load test.
    NoPrintMetrics,
    /// An optional default for not displaying an error summary.
    NoErrorSummary,
    /// An optional default for the report file name.
    ReportFile,
    /// An optional default for the baseline file name.
    BaselineFile,
    /// An optional default for the flag that disables granular data in HTML report graphs.
    NoGranularData,
    /// An optional default for not saving the run to the runs directory.
    NoSave,
    /// An optional default for the runs directory saved runs are written to.
    RunsDir,
    /// An optional default for the request log file name.
    RequestLog,
    /// An optional default for the request log file format.
    RequestFormat,
    /// An optional default for logging the request body.
    RequestBody,
    /// An optional default for the transaction log file name.
    TransactionLog,
    /// An optional default for the transaction log file format.
    TransactionFormat,
    /// An optional default for the scenario log file name.
    ScenarioLog,
    /// An optional default for the scenario log file format.
    ScenarioFormat,
    /// An optional default for the error log file name.
    ErrorLog,
    /// An optional default for the error log format.
    ErrorFormat,
    /// An optional default for the debug log file name.
    DebugLog,
    /// An optional default for the debug log format.
    DebugFormat,
    /// An optional default for not logging the response body in the debug log.
    NoDebugBody,
    /// An optional default for not enabling telnet Controller thread.
    NoTelnet,
    /// An optional default for not enabling WebSocket Controller thread.
    NoWebSocket,
    /// An optional default for coordinated omission mitigation.
    CoordinatedOmissionMitigation,
    /// An optional default for not automatically starting load test.
    NoAutoStart,
    /// An optional default timeout for all requests, in seconds.
    Timeout,
    /// An optional default for not setting the gzip Accept-Encoding header.
    NoGzip,
    /// An optional default to not track status code metrics.
    NoStatusCodes,
    /// An optional default maximum requests per second.
    ThrottleRequests,
    /// An optional default to follows base_url redirect with subsequent request.
    StickyFollow,
    /// An optional default for host telnet Controller listens on.
    TelnetHost,
    /// An optional default for port telnet Controller listens on.
    TelnetPort,
    /// An optional default for host Websocket Controller listens on.
    WebSocketHost,
    /// An optional default for port WebSocket Controller listens on.
    WebSocketPort,
    /// An optional default for enabling the live web dashboard.
    Dashboard,
    /// An optional default for enabling dashboard control endpoints.
    DashboardControl,
    /// An optional default for host the dashboard listens on.
    DashboardHost,
    /// An optional default for port the dashboard listens on.
    DashboardPort,
    /// An optional default for the dashboard auth token.
    DashboardAuthToken,
    /// An optional default for max concurrent dashboard SSE clients.
    DashboardMaxClients,
    /// An optional default for not tracking response times by second in the dashboard.
    NoPercentilesOverTime,
    /// An optional default for not validating https certificates.
    AcceptInvalidCerts,
    /// An optional default for the PDF scale factor.
    #[cfg(feature = "pdf-reports")]
    PdfScale,
    /// An optional default for PDF generation timeout (seconds).
    #[cfg(feature = "pdf-reports")]
    PdfTimeout,
}

/// Most run-time options can be programmatically configured with custom defaults.
///
/// For example, you can optionally configure a default host for the load test. This is
/// used if no per-[`Scenario`](../struct.Scenario.html) host is defined, no
/// [`--host`](./enum.GooseDefault.html#variant.Host) CLI option is configured, and if
/// the [`Transaction`](../struct.Transaction.html) itself doesn't hard-code the host in
/// the base url of its request. In that case, this host is added to all requests.
///
/// In the following example, the load test is programmatically configured with
/// [`GooseDefaultType::set_default`] to default to running against a local development
/// container. The [`--host`](./enum.GooseDefault.html#variant.Host) run time option
/// can be used at start time to override the host value, and the
/// [`GooseControllerCommand::host`](../controller/enum.GooseControllerCommand.html#variant.Host)
/// Controller command can be used to change the host value of an
/// [`AttackPhase::idle`](../enum.AttackPhase.html#variant.Idle) load test.
///
/// # Example
/// ```rust
/// use goose::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<(), GooseError> {
///     GooseAttack::initialize()?
///         .set_default(GooseDefault::Host, "local.dev")?;
///
///     Ok(())
/// }
/// ```
///
/// The following run-time options can be configured with a custom default using a
/// borrowed string slice ([`&str`]):
///  - [`GooseDefault::DebugLog`]
///  - [`GooseDefault::ErrorLog`]
///  - [`GooseDefault::GooseLog`]
///  - [`GooseDefault::IncreaseRate`]
///  - [`GooseDefault::DecreaseRate`]
///  - [`GooseDefault::Host`]
///  - [`GooseDefault::ReportFile`]
///  - [`GooseDefault::RunsDir`]
///  - [`GooseDefault::RequestLog`]
///  - [`GooseDefault::ScenarioLog`]
///  - [`GooseDefault::Scenarios`]
///  - [`GooseDefault::TelnetHost`]
///  - [`GooseDefault::TestPlan`]
///  - [`GooseDefault::Timeout`]
///  - [`GooseDefault::TransactionLog`]
///  - [`GooseDefault::WebSocketHost`]
///  - [`GooseDefault::DashboardHost`]
///  - [`GooseDefault::DashboardAuthToken`]
///
/// The following run-time options can be configured with a custom default using a
/// [`usize`] integer:
///  - [`GooseDefault::Users`]
///  - [`GooseDefault::IncreaseTime`]
///  - [`GooseDefault::DecreaseTime`]
///  - [`GooseDefault::RunTime`]
///  - [`GooseDefault::Iterations`]
///  - [`GooseDefault::RunningMetrics`]
///  - [`GooseDefault::LogLevel`]
///  - [`GooseDefault::Quiet`]
///  - [`GooseDefault::Verbose`]
///  - [`GooseDefault::ThrottleRequests`]
///  - [`GooseDefault::TelnetPort`]
///  - [`GooseDefault::WebSocketPort`]
///  - [`GooseDefault::DashboardPort`]
///  - [`GooseDefault::DashboardMaxClients`]
///  - [`GooseDefault::PdfTimeout`] (with the `pdf-reports` feature)
///
/// The following run-time options can be configured with a custom default using a
/// [`f64`] float:
///  - [`GooseDefault::PdfScale`] (with the `pdf-reports` feature)
///
/// The following run-time flags can be configured with a custom default using a
/// [`bool`] (and otherwise default to [`false`]).
///  - [`GooseDefault::NoResetMetrics`]
///  - [`GooseDefault::NoPrintMetrics`]
///  - [`GooseDefault::NoMetrics`]
///  - [`GooseDefault::NoTransactionMetrics`]
///  - [`GooseDefault::NoScenarioMetrics`]
///  - [`GooseDefault::RequestBody`]
///  - [`GooseDefault::NoErrorSummary`]
///  - [`GooseDefault::NoDebugBody`]
///  - [`GooseDefault::NoTelnet`]
///  - [`GooseDefault::NoWebSocket`]
///  - [`GooseDefault::NoAutoStart`]
///  - [`GooseDefault::NoGzip`]
///  - [`GooseDefault::NoStatusCodes`]
///  - [`GooseDefault::StickyFollow`]
///  - [`GooseDefault::NoGranularData`]
///  - [`GooseDefault::NoSave`]
///  - [`GooseDefault::Dashboard`]
///  - [`GooseDefault::DashboardControl`]
///  - [`GooseDefault::NoPercentilesOverTime`]
///  - [`GooseDefault::AcceptInvalidCerts`]
///
/// The following run-time flags can be configured with a custom default using a
/// [`GooseLogFormat`].
///  - [`GooseDefault::RequestFormat`]
///  - [`GooseDefault::TransactionFormat`]
///  - [`GooseDefault::ScenarioFormat`]
///  - [`GooseDefault::ErrorFormat`]
///  - [`GooseDefault::DebugFormat`]
///
/// The following run-time flags can be configured with a custom default using a
/// [`GooseCoordinatedOmissionMitigation`].
///  - [`GooseDefault::CoordinatedOmissionMitigation`]
pub trait GooseDefaultType<T> {
    /// Sets a [`GooseDefault`] to the provided value. The required type of each option
    /// is documented in [`GooseDefaultType`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         // Do not reset the metrics after the load test finishes starting.
    ///         .set_default(GooseDefault::NoResetMetrics, true)?
    ///         // Do not display info level logs while the test runs.
    ///         .set_default(GooseDefault::Quiet, 1)?
    ///         // Log all requests made during the test to `./goose-request.log`.
    ///         .set_default(GooseDefault::RequestLog, "goose-request.log")?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn set_default(self, key: GooseDefault, value: T) -> Result<Box<Self>, GooseError>;
}
impl GooseDefaultType<&str> for GooseAttack {
    /// Sets [`GooseDefault`] to a [`&str`] value.
    fn set_default(mut self, key: GooseDefault, value: &str) -> Result<Box<Self>, GooseError> {
        match key {
            // Set valid defaults.
            GooseDefault::DebugLog => self.defaults.debug_log = Some(value.to_string()),
            GooseDefault::ErrorLog => self.defaults.error_log = Some(value.to_string()),
            GooseDefault::GooseLog => self.defaults.goose_log = Some(value.to_string()),
            GooseDefault::IncreaseRate => self.defaults.increase_rate = Some(value.to_string()),
            GooseDefault::DecreaseRate => self.defaults.decrease_rate = Some(value.to_string()),
            GooseDefault::Host => {
                self.defaults.host = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            }
            GooseDefault::ReportFile => self.defaults.report_file = Some(vec![value.to_string()]),
            GooseDefault::RunsDir => self.defaults.runs_dir = Some(value.to_string()),
            GooseDefault::BaselineFile => self.defaults.baseline_file = Some(value.to_string()),
            GooseDefault::RequestLog => self.defaults.request_log = Some(value.to_string()),
            GooseDefault::ScenarioLog => self.defaults.scenario_log = Some(value.to_string()),
            GooseDefault::Scenarios => {
                self.defaults.scenarios = Some(value.parse::<Scenarios>().unwrap())
            }
            GooseDefault::TelnetHost => self.defaults.telnet_host = Some(value.to_string()),
            GooseDefault::TestPlan => self.defaults.test_plan = Some(value.parse::<TestPlan>()?),
            GooseDefault::Timeout => self.defaults.timeout = Some(value.to_string()),
            GooseDefault::TransactionLog => self.defaults.transaction_log = Some(value.to_string()),
            GooseDefault::WebSocketHost => self.defaults.websocket_host = Some(value.to_string()),
            GooseDefault::DashboardHost => self.defaults.dashboard_host = Some(value.to_string()),
            GooseDefault::DashboardAuthToken => {
                self.defaults.dashboard_auth_token = Some(value.to_string())
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::Users
            | GooseDefault::IncreaseTime
            | GooseDefault::DecreaseTime
            | GooseDefault::RunTime
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::DashboardPort
            | GooseDefault::DashboardMaxClients => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected usize value, received &str"
                    ),
                });
            }
            GooseDefault::RunningMetrics
            | GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTransactionMetrics
            | GooseDefault::NoScenarioMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoPrintMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::NoStatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::NoGranularData
            | GooseDefault::NoSave
            | GooseDefault::Dashboard
            | GooseDefault::DashboardControl
            | GooseDefault::NoPercentilesOverTime
            | GooseDefault::AcceptInvalidCerts => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected bool value, received &str"
                    ),
                });
            }
            GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::TransactionFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::RequestFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseLogFormat value, received &str"
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseCoordinatedOmissionMitigation value, received &str"
                    ),
                });
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfTimeout => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected usize value, received &str"
                    ),
                });
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfScale => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected f64 value, received &str"
                    ),
                });
            }
        }
        Ok(Box::new(self))
    }
}
impl GooseDefaultType<usize> for GooseAttack {
    /// Sets [`GooseDefault`] to a [`usize`] value.
    fn set_default(mut self, key: GooseDefault, value: usize) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::Users => self.defaults.users = Some(value),
            GooseDefault::IncreaseTime => self.defaults.increase_time = Some(value),
            GooseDefault::DecreaseTime => self.defaults.decrease_time = Some(value),
            GooseDefault::RunTime => self.defaults.run_time = Some(value),
            GooseDefault::Iterations => self.defaults.iterations = Some(value),
            GooseDefault::RunningMetrics => self.defaults.running_metrics = Some(value),
            GooseDefault::LogLevel => self.defaults.log_level = Some(value as u8),
            GooseDefault::Quiet => self.defaults.quiet = Some(value as u8),
            GooseDefault::Verbose => self.defaults.verbose = Some(value as u8),
            GooseDefault::ThrottleRequests => self.defaults.throttle_requests = Some(value),
            GooseDefault::TelnetPort => self.defaults.telnet_port = Some(value as u16),
            GooseDefault::WebSocketPort => self.defaults.websocket_port = Some(value as u16),
            GooseDefault::DashboardPort => self.defaults.dashboard_port = Some(value as u16),
            GooseDefault::DashboardMaxClients => {
                self.defaults.dashboard_max_clients = Some(value as u32)
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfTimeout => {
                self.defaults.pdf_timeout = Some(value as u64)
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::IncreaseRate
            | GooseDefault::DecreaseRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RunsDir
            | GooseDefault::BaselineFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost
            | GooseDefault::DashboardHost
            | GooseDefault::DashboardAuthToken
            => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected &str value, received usize"
                    ),
                })
            }
            GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTransactionMetrics
            | GooseDefault::NoScenarioMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoPrintMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::NoStatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::NoGranularData
            | GooseDefault::NoSave
            | GooseDefault::Dashboard
            | GooseDefault::DashboardControl
            | GooseDefault::NoPercentilesOverTime
            | GooseDefault::AcceptInvalidCerts => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected bool value, received usize"
                    ),
                })
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseLogFormat value, received usize"
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected GooseCoordinatedOmissionMitigation value, received usize"
                    ),
                })
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfScale => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected f64 value, received usize"
                    ),
                })
            }
        }
        Ok(Box::new(self))
    }
}
impl GooseDefaultType<bool> for GooseAttack {
    /// Sets [`GooseDefault`] to a [`bool`] value.
    fn set_default(mut self, key: GooseDefault, value: bool) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::NoResetMetrics => self.defaults.no_reset_metrics = Some(value),
            GooseDefault::NoMetrics => self.defaults.no_metrics = Some(value),
            GooseDefault::NoTransactionMetrics => {
                self.defaults.no_transaction_metrics = Some(value)
            }
            GooseDefault::NoScenarioMetrics => self.defaults.no_scenario_metrics = Some(value),
            GooseDefault::RequestBody => self.defaults.request_body = Some(value),
            GooseDefault::NoPrintMetrics => self.defaults.no_print_metrics = Some(value),
            GooseDefault::NoErrorSummary => self.defaults.no_error_summary = Some(value),
            GooseDefault::NoDebugBody => self.defaults.no_debug_body = Some(value),
            GooseDefault::NoTelnet => self.defaults.no_telnet = Some(value),
            GooseDefault::NoWebSocket => self.defaults.no_websocket = Some(value),
            GooseDefault::NoAutoStart => self.defaults.no_autostart = Some(value),
            GooseDefault::NoGzip => self.defaults.no_gzip = Some(value),
            GooseDefault::AcceptInvalidCerts => self.defaults.accept_invalid_certs = Some(value),
            GooseDefault::NoStatusCodes => self.defaults.no_status_codes = Some(value),
            GooseDefault::StickyFollow => self.defaults.sticky_follow = Some(value),
            GooseDefault::NoGranularData => self.defaults.no_granular_report = Some(value),
            GooseDefault::NoSave => self.defaults.no_save = Some(value),
            GooseDefault::Dashboard => self.defaults.dashboard = Some(value),
            GooseDefault::DashboardControl => self.defaults.dashboard_control = Some(value),
            GooseDefault::NoPercentilesOverTime => {
                self.defaults.no_percentiles_over_time = Some(value)
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::IncreaseRate
            | GooseDefault::DecreaseRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RunsDir
            | GooseDefault::BaselineFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost
            | GooseDefault::DashboardHost
            | GooseDefault::DashboardAuthToken
            => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected &str value, received bool"
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::IncreaseTime
            | GooseDefault::DecreaseTime
            | GooseDefault::RunTime
            | GooseDefault::RunningMetrics
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::DashboardPort
            | GooseDefault::DashboardMaxClients => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected usize value, received bool"
                    ),
                })
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseLogFormat value, received bool"
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseCoordinatedOmissionMitigation value, received bool"
                    ),
                });
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfTimeout => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected usize value, received bool"
                    ),
                });
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfScale => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected f64 value, received bool"
                    ),
                });
            }
        }
        Ok(Box::new(self))
    }
}
impl GooseDefaultType<GooseCoordinatedOmissionMitigation> for GooseAttack {
    /// Sets [`GooseDefault`] to a [`GooseCoordinatedOmissionMitigation`] value.
    fn set_default(
        mut self,
        key: GooseDefault,
        value: GooseCoordinatedOmissionMitigation,
    ) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::CoordinatedOmissionMitigation => self.defaults.co_mitigation = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTransactionMetrics
            | GooseDefault::NoScenarioMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoPrintMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::NoStatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::NoGranularData
            | GooseDefault::NoSave
            | GooseDefault::Dashboard
            | GooseDefault::DashboardControl
            | GooseDefault::NoPercentilesOverTime
            | GooseDefault::AcceptInvalidCerts
            => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected bool value, received GooseCoordinatedOmissionMitigation"
                    ),
                })
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::IncreaseRate
            | GooseDefault::DecreaseRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RunsDir
            | GooseDefault::BaselineFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost
            | GooseDefault::DashboardHost
            | GooseDefault::DashboardAuthToken
            => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected &str value, received GooseCoordinatedOmissionMitigation"
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::IncreaseTime
            | GooseDefault::DecreaseTime
            | GooseDefault::RunTime
            | GooseDefault::RunningMetrics
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::DashboardPort
            | GooseDefault::DashboardMaxClients => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected usize value, received GooseCoordinatedOmissionMitigation"
                    ),
                })
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected GooseLogFormat value, received GooseCoordinatedOmissionMitigation"
                    ),
                })
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfTimeout => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected usize value, received GooseCoordinatedOmissionMitigation"
                    ),
                });
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfScale => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected f64 value, received GooseCoordinatedOmissionMitigation"
                    ),
                });
            }
        }
        Ok(Box::new(self))
    }
}
impl GooseDefaultType<GooseLogFormat> for GooseAttack {
    /// Sets [`GooseDefault`] to a [`GooseLogFormat`] value.
    fn set_default(
        mut self,
        key: GooseDefault,
        value: GooseLogFormat,
    ) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::RequestFormat => self.defaults.request_format = Some(value),
            GooseDefault::DebugFormat => self.defaults.debug_format = Some(value),
            GooseDefault::ErrorFormat => self.defaults.error_format = Some(value),
            GooseDefault::TransactionFormat => self.defaults.transaction_format = Some(value),
            GooseDefault::ScenarioFormat => self.defaults.scenario_format = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTransactionMetrics
            | GooseDefault::NoScenarioMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoPrintMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::NoStatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::NoGranularData
            | GooseDefault::NoSave
            | GooseDefault::Dashboard
            | GooseDefault::DashboardControl
            | GooseDefault::NoPercentilesOverTime
            | GooseDefault::AcceptInvalidCerts
            => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected bool value, received GooseLogFormat"
                    ),
                })
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::IncreaseRate
            | GooseDefault::DecreaseRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RunsDir
            | GooseDefault::BaselineFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost
            | GooseDefault::DashboardHost
            | GooseDefault::DashboardAuthToken
            => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected &str value, received GooseLogFormat"
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::IncreaseTime
            | GooseDefault::DecreaseTime
            | GooseDefault::RunTime
            | GooseDefault::RunningMetrics
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::DashboardPort
            | GooseDefault::DashboardMaxClients => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected usize value, received GooseLogFormat"
                    ),
                })
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected GooseCoordinatedOmissionMitigation value, received GooseLogFormat"
                    ),
                })
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfTimeout => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected usize value, received GooseLogFormat"
                    ),
                });
            }
            #[cfg(feature = "pdf-reports")]
            GooseDefault::PdfScale => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: format!("{value:?}"),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value:?}) expected f64 value, received GooseLogFormat"
                    ),
                });
            }
        }
        Ok(Box::new(self))
    }
}
#[cfg(feature = "pdf-reports")]
impl GooseDefaultType<f64> for GooseAttack {
    /// Sets [`GooseDefault`] to a [`f64`] value.
    fn set_default(mut self, key: GooseDefault, value: f64) -> Result<Box<Self>, GooseError> {
        match key {
            GooseDefault::PdfScale => self.defaults.pdf_scale = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::IncreaseRate
            | GooseDefault::DecreaseRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RunsDir
            | GooseDefault::BaselineFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost
            | GooseDefault::DashboardHost
            | GooseDefault::DashboardAuthToken => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected &str value, received f64"
                    ),
                });
            }
            GooseDefault::Users
            | GooseDefault::IncreaseTime
            | GooseDefault::DecreaseTime
            | GooseDefault::RunTime
            | GooseDefault::Iterations
            | GooseDefault::RunningMetrics
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::DashboardPort
            | GooseDefault::DashboardMaxClients
            | GooseDefault::PdfTimeout => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected usize value, received f64"
                    ),
                });
            }
            GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTransactionMetrics
            | GooseDefault::NoScenarioMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoPrintMetrics
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::NoStatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::NoGranularData
            | GooseDefault::NoSave
            | GooseDefault::Dashboard
            | GooseDefault::DashboardControl
            | GooseDefault::NoPercentilesOverTime
            | GooseDefault::AcceptInvalidCerts => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected bool value, received f64"
                    ),
                });
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseLogFormat value, received f64"
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{key:?}"),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{key:?}, {value}) expected GooseCoordinatedOmissionMitigation value, received f64"
                    ),
                });
            }
        }
        Ok(Box::new(self))
    }
}

/// Returns true if `host` is a loopback bind address for dashboard auth policy.
///
/// Accepts `localhost` (case-insensitive, no DNS lookup), IPv4 loopback
/// (`127.0.0.0/8`), and IPv6 loopback (`::1`, with optional brackets).
/// Unspecified addresses (`0.0.0.0`, `::`) and unparseable hostnames are
/// treated as non-loopback.
pub(crate) fn is_loopback_bind(host: &str) -> bool {
    let host = host.trim();
    // Special-case common hostname; DNS is not consulted.
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // Strip IPv6 brackets if present: "[::1]" -> "::1"
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        // Unparseable hostnames (e.g. "my-loadgen.local") are NOT loopback.
        Err(_) => false,
    }
}

/// Used internally to configure [`GooseConfiguration`] values based on precedence rules.
#[derive(Debug, Clone)]
pub(crate) struct GooseValue<'a, T> {
    /// The optional value to set.
    pub(crate) value: Option<T>,
    /// Filter using this value if true.
    pub(crate) filter: bool,
    /// An optional INFO level Goose log message.
    pub(crate) message: &'a str,
}

pub(crate) trait GooseConfigure<T> {
    /// Set [`GooseValue`] with supported type.
    fn get_value(&self, values: Vec<GooseValue<T>>) -> Option<T>;
}

impl GooseConfigure<usize> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`usize`] value.
    fn get_value(&self, values: Vec<GooseValue<usize>>) -> Option<usize> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<u64> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`u64`] value.
    fn get_value(&self, values: Vec<GooseValue<u64>>) -> Option<u64> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<u16> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`u16`] value.
    fn get_value(&self, values: Vec<GooseValue<u16>>) -> Option<u16> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<u32> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`u32`] value.
    fn get_value(&self, values: Vec<GooseValue<u32>>) -> Option<u32> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<u8> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`u8`] value.
    fn get_value(&self, values: Vec<GooseValue<u8>>) -> Option<u8> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<f32> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`f32`] value.
    fn get_value(&self, values: Vec<GooseValue<f32>>) -> Option<f32> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<f64> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`f64`] value.
    fn get_value(&self, values: Vec<GooseValue<f64>>) -> Option<f64> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<TestPlan> for GooseConfiguration {
    /// Use [`GooseValue`] to set a vec<(usize, usize)> value.
    fn get_value(&self, values: Vec<GooseValue<TestPlan>>) -> Option<TestPlan> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {:?}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<String> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`String`] value.
    fn get_value(&self, values: Vec<GooseValue<String>>) -> Option<String> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<Vec<String>> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`String`] value.
    fn get_value(&self, values: Vec<GooseValue<Vec<String>>>) -> Option<Vec<String>> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {:?}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<bool> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`bool`] value.
    fn get_value(&self, values: Vec<GooseValue<bool>>) -> Option<bool> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<GooseLogFormat> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`GooseLogFormat`] value.
    fn get_value(&self, values: Vec<GooseValue<GooseLogFormat>>) -> Option<GooseLogFormat> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {:?}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<GooseCoordinatedOmissionMitigation> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`GooseCoordinatedOmissionMitigation`] value.
    fn get_value(
        &self,
        values: Vec<GooseValue<GooseCoordinatedOmissionMitigation>>,
    ) -> Option<GooseCoordinatedOmissionMitigation> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {:?}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}
impl GooseConfigure<Scenarios> for GooseConfiguration {
    /// Use [`GooseValue`] to set a [`Scenarios`] value.
    fn get_value(&self, values: Vec<GooseValue<Scenarios>>) -> Option<Scenarios> {
        for value in values {
            if let Some(v) = value.value {
                if value.filter {
                    continue;
                } else {
                    if !value.message.is_empty() {
                        info!("{} = {:?}", value.message, v)
                    }
                    return Some(v);
                }
            }
        }
        None
    }
}

/// The seconds in time span option `option`, refused when the span is too long
/// for a `usize` in milliseconds or has a digit of another script before a
/// unit, naming which.
pub(crate) fn timespan(option: &str, value: &str) -> Result<usize, GooseError> {
    util::timespan_seconds(value).map_err(|e| GooseError::InvalidOption {
        option: format!("`configuration.{option}`"),
        value: value.to_string(),
        detail: match e {
            util::SpanError::TooLong => format!("`configuration.{option}` is too long."),
            util::SpanError::ForeignDigit => {
                format!("`configuration.{option}` must use the ASCII digits 0 to 9.")
            }
        },
    })
}

impl GooseConfiguration {
    /// Implement precedence rules for all [`GooseConfiguration`] values.
    ///
    /// Fails when a time span is too long.
    pub(crate) fn configure(&mut self, defaults: &GooseDefaults) -> Result<(), GooseError> {
        // Configure `quiet`.
        self.quiet = self
            .get_value(vec![
                // Use --quiet if set.
                GooseValue {
                    value: Some(self.quiet),
                    filter: self.quiet == 0,
                    message: "",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.quiet,
                    filter: defaults.quiet.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(0);

        // Configure `verbose`.
        self.verbose = self
            .get_value(vec![
                // Use --verbose if set.
                GooseValue {
                    value: Some(self.verbose),
                    filter: self.verbose == 0,
                    message: "",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.verbose,
                    filter: defaults.verbose.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(0);

        // Configure `log_level`.
        self.log_level = self
            .get_value(vec![
                // Use --log-level if set.
                GooseValue {
                    value: Some(self.log_level),
                    filter: self.log_level == 0,
                    message: "",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.log_level,
                    filter: defaults.log_level.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(0);

        // Configure `goose_log`.
        self.goose_log = self
            .get_value(vec![
                // Use --log-level if set.
                GooseValue {
                    value: Some(self.goose_log.to_string()),
                    filter: self.goose_log.is_empty(),
                    message: "",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.goose_log.clone(),
                    filter: defaults.goose_log.is_none(),
                    message: "",
                },
            ])
            .unwrap_or_default();

        // Initialize the Goose logger.
        self.initialize_goose_logger();

        // Configure loggers.
        self.configure_loggers(defaults);

        // Configure `test_plan` before `users` so users doesn't get assigned a default when using a test plan.
        self.test_plan = self.get_value(vec![
            // Use --test-plan if set.
            GooseValue {
                value: self.test_plan.clone(),
                filter: self.test_plan.is_none(),
                message: "test_plan",
            },
            // Otherwise use GooseDefault if set and not on Worker.
            GooseValue {
                value: defaults.test_plan.clone(),
                filter: defaults.test_plan.is_none(),
                message: "test_plan",
            },
        ]);

        // Determine how many CPUs are available.
        let default_users = match std::thread::available_parallelism() {
            Ok(ap) => Some(ap.get()),
            Err(e) => {
                // Default to 1 user if unable to detect number of CPUs.
                info!("failed to detect available_parallelism: {e}");
                Some(1)
            }
        };

        // Configure `users`.
        self.users = self.get_value(vec![
            // Use --users if set.
            GooseValue {
                value: self.users,
                filter: false,
                message: "users",
            },
            // Otherwise use GooseDefault if set and not on Worker.
            GooseValue {
                value: defaults.users,
                filter: defaults.users.is_none(),
                message: "users",
            },
            // Otherwise use detected number of CPUs if not on Worker.
            GooseValue {
                value: default_users,
                filter: self.test_plan.is_some(),
                message: "users defaulted to number of CPUs",
            },
        ]);

        // Configure `increase_time`.
        let increase_time = timespan("increase_time", &self.increase_time)?;
        self.increase_time = self
            .get_value(vec![
                // Use --increase-time if set.
                GooseValue {
                    value: Some(increase_time),
                    filter: increase_time == 0,
                    message: "increase_time",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: defaults.increase_time,
                    filter: defaults.increase_time.is_none(),
                    message: "increase_time",
                },
            ])
            .map_or_else(|| "0".to_string(), |v| v.to_string());

        // Configure `decrease_time`.
        let decrease_time = timespan("decrease_time", &self.decrease_time)?;
        self.decrease_time = self
            .get_value(vec![
                // Use --decrease-time if set.
                GooseValue {
                    value: Some(decrease_time),
                    filter: decrease_time == 0,
                    message: "decrease_time",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: defaults.decrease_time,
                    filter: defaults.decrease_time.is_none(),
                    message: "decrease_time",
                },
            ])
            .map_or_else(|| "0".to_string(), |v| v.to_string());

        // Configure `run_time`.
        let run_time = timespan("run_time", &self.run_time)?;
        self.run_time = self
            .get_value(vec![
                // Use --run-time if set.
                GooseValue {
                    value: Some(run_time),
                    filter: run_time == 0,
                    message: "run_time",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: defaults.run_time,
                    filter: defaults.run_time.is_none(),
                    message: "run_time",
                },
            ])
            .map_or_else(|| "0".to_string(), |v| v.to_string());

        // Configure `increase_rate`.
        self.increase_rate = self
            .get_value(vec![
                // Use --increase-rate if set.
                GooseValue {
                    value: Some(util::parse_rate(self.increase_rate.clone())),
                    filter: self.increase_rate.is_none(),
                    message: "increase_rate",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: Some(util::parse_rate(defaults.increase_rate.clone())),
                    filter: defaults.increase_rate.is_none(),
                    message: "increase_rate",
                },
            ])
            .map(|v| v.to_string());

        // Configure `decrease_rate`.
        self.decrease_rate = self
            .get_value(vec![
                // Use --decrease-rate if set.
                GooseValue {
                    value: Some(util::parse_rate(self.decrease_rate.clone())),
                    filter: self.decrease_rate.is_none(),
                    message: "decrease_rate",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: Some(util::parse_rate(defaults.decrease_rate.clone())),
                    filter: defaults.decrease_rate.is_none(),
                    message: "decrease_rate",
                },
            ])
            .map(|v| v.to_string());

        // Configure `timeout`.
        self.timeout = self
            .get_value(vec![
                // Use --timeout if set.
                GooseValue {
                    value: util::get_float_from_string(self.timeout.clone()),
                    filter: self.timeout.is_none(),
                    message: "timeout",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: util::get_float_from_string(defaults.timeout.clone()),
                    filter: defaults.timeout.is_none(),
                    message: "timeout",
                },
            ])
            .map(|v| v.to_string());

        // Configure `running_metrics`.
        self.running_metrics = self.get_value(vec![
            // Use --running-metrics if set.
            GooseValue {
                value: self.running_metrics,
                filter: self.running_metrics.is_none(),
                message: "running_metrics",
            },
            // Otherwise use GooseDefault if set.
            GooseValue {
                value: defaults.running_metrics,
                filter: defaults.running_metrics.is_none(),
                message: "running_metrics",
            },
        ]);

        // Configure `no_reset_metrics`.
        self.no_reset_metrics = self
            .get_value(vec![
                // Use --no-reset-metrics if set.
                GooseValue {
                    value: Some(self.no_reset_metrics),
                    filter: !self.no_reset_metrics,
                    message: "no_reset_metrics",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_reset_metrics,
                    filter: defaults.no_reset_metrics.is_none(),
                    message: "no_reset_metrics",
                },
            ])
            .unwrap_or(false);

        // Configure `no_metrics`.
        self.no_metrics = self
            .get_value(vec![
                // Use --no-metrics if set.
                GooseValue {
                    value: Some(self.no_metrics),
                    filter: !self.no_metrics,
                    message: "no_metrics",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_metrics,
                    filter: defaults.no_metrics.is_none(),
                    message: "no_metrics",
                },
            ])
            .unwrap_or(false);

        // Configure `no_transaction_metrics`.
        self.no_transaction_metrics = self
            .get_value(vec![
                // Use --no-transaction-metrics if set.
                GooseValue {
                    value: Some(self.no_transaction_metrics),
                    filter: !self.no_transaction_metrics,
                    message: "no_transaction_metrics",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_transaction_metrics,
                    filter: defaults.no_transaction_metrics.is_none(),
                    message: "no_transaction_metrics",
                },
            ])
            .unwrap_or(false);

        // Configure `no_scenario_metrics`.
        self.no_scenario_metrics = self
            .get_value(vec![
                // Use --no-scenario-metrics if set.
                GooseValue {
                    value: Some(self.no_scenario_metrics),
                    filter: !self.no_scenario_metrics,
                    message: "no_scenario_metrics",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_scenario_metrics,
                    filter: defaults.no_scenario_metrics.is_none(),
                    message: "no_scenario_metrics",
                },
            ])
            .unwrap_or(false);

        // Configure `no_print_metrics`.
        self.no_print_metrics = self
            .get_value(vec![
                // Use --no-print-metrics if set.
                GooseValue {
                    value: Some(self.no_print_metrics),
                    filter: !self.no_print_metrics,
                    message: "no_print_metrics",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_print_metrics,
                    filter: defaults.no_print_metrics.is_none(),
                    message: "no_print_metrics",
                },
            ])
            .unwrap_or(false);

        // Configure `no_error_summary`.
        self.no_error_summary = self
            .get_value(vec![
                // Use --no-error-summary if set.
                GooseValue {
                    value: Some(self.no_error_summary),
                    filter: !self.no_error_summary,
                    message: "no_error_summary",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_error_summary,
                    filter: defaults.no_error_summary.is_none(),
                    message: "no_error_summary",
                },
            ])
            .unwrap_or(false);

        // Configure `report_file`.
        self.report_file = self
            .get_value(vec![
                // Use --report-file if set.
                GooseValue {
                    value: Some(self.report_file.clone()),
                    filter: self.report_file.is_empty(),
                    message: "report_file",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.report_file.clone(),
                    filter: defaults.report_file.is_none(),
                    message: "report_file",
                },
            ])
            .unwrap_or_default();

        // Configure `no_save`.
        self.no_save = self
            .get_value(vec![
                // Use --no-save if set.
                GooseValue {
                    value: Some(self.no_save),
                    filter: !self.no_save,
                    message: "no_save",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_save,
                    filter: defaults.no_save.is_none(),
                    message: "no_save",
                },
            ])
            .unwrap_or(false);

        // Configure `runs_dir`. Left empty unless given explicitly, so an
        // explicit directory that can't be created is an error while the
        // default one only warns; see `runs_dir_or_default()`.
        self.runs_dir = self
            .get_value(vec![
                // Use --runs-dir if set.
                GooseValue {
                    value: Some(self.runs_dir.to_string()),
                    filter: self.runs_dir.is_empty(),
                    message: "runs_dir",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.runs_dir.clone(),
                    filter: defaults.runs_dir.is_none(),
                    message: "runs_dir",
                },
            ])
            .unwrap_or_default();

        // Configure `baseline_file`.
        self.baseline_file = self.get_value(vec![
            // Use --baseline-file if set.
            GooseValue {
                value: self.baseline_file.clone(),
                filter: self.baseline_file.is_none(),
                message: "baseline_file",
            },
            // Otherwise use GooseDefault if set.
            GooseValue {
                value: defaults.baseline_file.clone(),
                filter: defaults.baseline_file.is_none(),
                message: "baseline_file",
            },
        ]);

        // Configure `no_granular_report`.
        self.no_granular_report = self
            .get_value(vec![
                // Use --no-granular-report if set.
                GooseValue {
                    value: Some(self.no_granular_report),
                    filter: !self.no_granular_report,
                    message: "no_granular_report",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_granular_report,
                    filter: defaults.no_granular_report.is_none(),
                    message: "no_granular_report",
                },
            ])
            .unwrap_or(false);

        // Configure `iterations`.
        self.iterations = self
            .get_value(vec![
                // Use --iterations if set.
                GooseValue {
                    value: Some(self.iterations),
                    filter: false,
                    message: "iterations",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.iterations,
                    filter: defaults.iterations.is_none(),
                    message: "iterations",
                },
            ])
            .unwrap_or(0);

        // Configure `scenarios`.
        self.scenarios = self
            .get_value(vec![
                // Use --scenarios if set.
                GooseValue {
                    value: Some(self.scenarios.clone()),
                    filter: self.scenarios.active.is_empty(),
                    message: "scenarios",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.scenarios.clone(),
                    filter: defaults.scenarios.is_none(),
                    message: "scenarios",
                },
            ])
            .unwrap_or_else(|| Scenarios { active: Vec::new() });

        // Configure `no_debug_body`.
        self.no_debug_body = self
            .get_value(vec![
                // Use --no-debug-body if set.
                GooseValue {
                    value: Some(self.no_debug_body),
                    filter: !self.no_debug_body,
                    message: "no_debug_body",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_debug_body,
                    filter: defaults.no_debug_body.is_none(),
                    message: "no_debug_body",
                },
            ])
            .unwrap_or(false);

        // Configure `no_status_codes`.
        self.no_status_codes = self
            .get_value(vec![
                // Use --no-status-codes if set.
                GooseValue {
                    value: Some(self.no_status_codes),
                    filter: !self.no_status_codes,
                    message: "no_status_codes",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_status_codes,
                    filter: defaults.no_status_codes.is_none(),
                    message: "no_status_codes",
                },
            ])
            .unwrap_or(false);

        // Configure `no_telnet`.
        self.no_telnet = self
            .get_value(vec![
                // Use --no-telnet if set.
                GooseValue {
                    value: Some(self.no_telnet),
                    filter: !self.no_telnet,
                    message: "no_telnet",
                },
                // Use default if --no-telnet hasn't been set.
                GooseValue {
                    value: defaults.no_telnet,
                    filter: defaults.no_telnet.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(false);

        // Configure `no_websocket`.
        self.no_websocket = self
            .get_value(vec![
                // Use --no-websocket if set.
                GooseValue {
                    value: Some(self.no_websocket),
                    filter: !self.no_websocket,
                    message: "no_websocket",
                },
                // Use default if --no-websocket hasn't been set.
                GooseValue {
                    value: defaults.no_websocket,
                    filter: defaults.no_websocket.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(false);

        // Configure `dashboard`.
        self.dashboard = self
            .get_value(vec![
                // Use --dashboard if set.
                GooseValue {
                    value: Some(self.dashboard),
                    filter: !self.dashboard,
                    message: "dashboard",
                },
                // Use GooseDefault if not already set.
                GooseValue {
                    value: defaults.dashboard,
                    filter: defaults.dashboard.is_none(),
                    message: "dashboard",
                },
            ])
            .unwrap_or(false);

        // Configure `dashboard_control`.
        self.dashboard_control = self
            .get_value(vec![
                // Use --dashboard-control if set.
                GooseValue {
                    value: Some(self.dashboard_control),
                    filter: !self.dashboard_control,
                    message: "dashboard_control",
                },
                // Use GooseDefault if not already set.
                GooseValue {
                    value: defaults.dashboard_control,
                    filter: defaults.dashboard_control.is_none(),
                    message: "dashboard_control",
                },
            ])
            .unwrap_or(false);

        // Configure `dashboard_host`.
        self.dashboard_host = self
            .get_value(vec![
                // Use --dashboard-host if set.
                GooseValue {
                    value: Some(self.dashboard_host.to_string()),
                    filter: self.dashboard_host.is_empty(),
                    message: "dashboard_host",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.dashboard_host.clone(),
                    filter: defaults.dashboard_host.is_none(),
                    message: "dashboard_host",
                },
            ])
            .unwrap_or_else(|| {
                // Built-in default when the dashboard is enabled.
                if self.dashboard {
                    "127.0.0.1".to_string()
                } else {
                    String::new()
                }
            });

        // Configure `dashboard_port`.
        self.dashboard_port = self
            .get_value(vec![
                // Use --dashboard-port if set.
                GooseValue {
                    value: Some(self.dashboard_port),
                    filter: self.dashboard_port == 0,
                    message: "dashboard_port",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.dashboard_port,
                    filter: defaults.dashboard_port.is_none(),
                    message: "dashboard_port",
                },
            ])
            .unwrap_or(if self.dashboard { 5118 } else { 0 });

        // Configure `dashboard_auth_token`.
        // Empty `message` intentionally: never log the raw auth token (shared secret).
        self.dashboard_auth_token = self
            .get_value(vec![
                // Use --dashboard-auth-token if set.
                GooseValue {
                    value: Some(self.dashboard_auth_token.to_string()),
                    filter: self.dashboard_auth_token.is_empty(),
                    message: "",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.dashboard_auth_token.clone(),
                    filter: defaults.dashboard_auth_token.is_none(),
                    message: "",
                },
            ])
            .unwrap_or_default();

        // Configure `dashboard_max_clients`.
        // 0 means unset (CLI default); fill 32 when the dashboard is enabled.
        self.dashboard_max_clients = self
            .get_value(vec![
                // Use --dashboard-max-clients if set.
                GooseValue {
                    value: Some(self.dashboard_max_clients),
                    filter: self.dashboard_max_clients == 0,
                    message: "dashboard_max_clients",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.dashboard_max_clients,
                    filter: defaults.dashboard_max_clients.is_none(),
                    message: "dashboard_max_clients",
                },
            ])
            .unwrap_or(if self.dashboard { 32 } else { 0 });

        // Configure `no_percentiles_over_time`.
        self.no_percentiles_over_time = self
            .get_value(vec![
                // Use --no-percentiles-over-time if set.
                GooseValue {
                    value: Some(self.no_percentiles_over_time),
                    filter: !self.no_percentiles_over_time,
                    message: "no_percentiles_over_time",
                },
                // Otherwise use GooseDefault if set, only with the dashboard:
                // a load test can set it once for runs with and without it.
                GooseValue {
                    value: defaults.no_percentiles_over_time,
                    filter: defaults.no_percentiles_over_time.is_none() || !self.dashboard,
                    message: "no_percentiles_over_time",
                },
            ])
            .unwrap_or(false);

        // Configure `no_autostart`.
        self.no_autostart = self
            .get_value(vec![
                // Use --no-autostart if set.
                GooseValue {
                    value: Some(self.no_autostart),
                    filter: !self.no_autostart,
                    message: "no_autostart",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.no_autostart,
                    filter: defaults.no_autostart.is_none(),
                    message: "no_autostart",
                },
            ])
            .unwrap_or(false);

        // Configure `no_gzip`.
        self.no_gzip = self
            .get_value(vec![
                // Use --no-gzip if set.
                GooseValue {
                    value: Some(self.no_gzip),
                    filter: !self.no_gzip,
                    message: "no_gzip",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.no_gzip,
                    filter: defaults.no_gzip.is_none(),
                    message: "no_gzip",
                },
            ])
            .unwrap_or(false);

        // Configure `accept_invalid_certs`
        self.accept_invalid_certs = self
            .get_value(vec![
                // Use --accept-invalid-certs if set.
                GooseValue {
                    value: Some(self.accept_invalid_certs),
                    filter: !self.accept_invalid_certs,
                    message: "accept_invalid_certs",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.accept_invalid_certs,
                    filter: defaults.accept_invalid_certs.is_none(),
                    message: "accept_invalid_certs",
                },
            ])
            .unwrap_or(false);

        self.co_mitigation = self.get_value(vec![
            // Use --co-mitigation if set.
            GooseValue {
                value: self.co_mitigation.clone(),
                filter: self.co_mitigation.is_none(),
                message: "co_mitigation",
            },
            // Otherwise use GooseDefault if set and not Worker.
            GooseValue {
                value: defaults.co_mitigation.clone(),
                filter: defaults.co_mitigation.is_none(),
                message: "co_mitigation",
            },
            // Otherwise default to GooseCoordinaatedOmissionMitigation::Disabled.
            GooseValue {
                value: Some(GooseCoordinatedOmissionMitigation::Disabled),
                filter: false,
                message: "",
            },
        ]);

        // Configure `throttle_requests`.
        self.throttle_requests = self
            .get_value(vec![
                // Use --throttle-requests if set.
                GooseValue {
                    value: Some(self.throttle_requests),
                    filter: self.throttle_requests == 0,
                    message: "throttle_requests",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.throttle_requests,
                    filter: defaults.throttle_requests.is_none(),
                    message: "throttle_requests",
                },
            ])
            .unwrap_or(0);

        // Configure `sticky_follow`.
        self.sticky_follow = self
            .get_value(vec![
                // Use --sticky-follow if set.
                GooseValue {
                    value: Some(self.sticky_follow),
                    filter: !self.sticky_follow,
                    message: "sticky_follow",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.sticky_follow,
                    filter: defaults.sticky_follow.is_none(),
                    message: "sticky_follow",
                },
            ])
            .unwrap_or(false);

        // Configure `pdf_scale` and `pdf_timeout` when PDF reports feature is enabled.
        #[cfg(feature = "pdf-reports")]
        {
            self.pdf_scale = self.get_value(vec![
                // Use --pdf-scale if set.
                GooseValue {
                    value: self.pdf_scale,
                    filter: self.pdf_scale.is_none(),
                    message: "pdf_scale",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.pdf_scale,
                    filter: defaults.pdf_scale.is_none(),
                    message: "pdf_scale",
                },
                // Otherwise use the built in default.
                GooseValue {
                    value: Some(DEFAULT_PDF_SCALE),
                    filter: false,
                    message: "",
                },
            ]);
            self.pdf_timeout = self.get_value(vec![
                // Use --pdf-timeout if set.
                GooseValue {
                    value: self.pdf_timeout,
                    filter: self.pdf_timeout.is_none(),
                    message: "pdf_timeout",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.pdf_timeout,
                    filter: defaults.pdf_timeout.is_none(),
                    message: "pdf_timeout",
                },
                // Otherwise use the built in default.
                GooseValue {
                    value: Some(DEFAULT_PDF_TIMEOUT),
                    filter: false,
                    message: "",
                },
            ]);
        }

        Ok(())
    }

    /// Validate configured [`GooseConfiguration`] values.
    /// Whether each run is saved to the runs directory: on unless `--no-save`
    /// or `--no-metrics` is set.
    pub(crate) fn saves_runs(&self) -> bool {
        !self.no_save && !self.no_metrics
    }

    /// The PDF scale factor, or the built in default before [`Self::configure`] runs.
    #[cfg(feature = "pdf-reports")]
    pub(crate) fn pdf_scale_or_default(&self) -> f64 {
        self.pdf_scale.unwrap_or(DEFAULT_PDF_SCALE)
    }

    /// The PDF generation timeout in seconds, or the built in default before
    /// [`Self::configure`] runs.
    #[cfg(feature = "pdf-reports")]
    pub(crate) fn pdf_timeout_or_default(&self) -> u64 {
        self.pdf_timeout.unwrap_or(DEFAULT_PDF_TIMEOUT)
    }

    /// The runs directory as given, or the default `goose-runs`.
    pub(crate) fn runs_dir_or_default(&self) -> &str {
        if self.runs_dir.is_empty() {
            crate::runs::DEFAULT_RUNS_DIR
        } else {
            &self.runs_dir
        }
    }

    /// Whether per second graph data is recorded: for a saved run, a
    /// `--report-file`, or the dashboard's charts.
    pub(crate) fn records_graph_data(&self) -> bool {
        self.saves_runs() || !self.report_file.is_empty() || self.dashboard
    }

    /// Whether per second graph data is kept for the whole run, as the
    /// reports need. When only the dashboard records it, older data is pruned.
    pub(crate) fn keeps_full_graph(&self) -> bool {
        self.saves_runs() || !self.report_file.is_empty()
    }

    /// Whether response time histograms are kept for each second, for the
    /// dashboard's response time heatmap. Implies [`Self::records_graph_data`].
    pub(crate) fn records_latency_series(&self) -> bool {
        self.dashboard && !self.no_percentiles_over_time && !self.no_metrics
    }

    /// Whether the heatmap also keeps coordinated omission mitigation's
    /// synthetic times: it tracks percentiles and mitigation is not disabled.
    pub(crate) fn records_synthetic_latency(&self) -> bool {
        self.records_latency_series()
            && self
                .co_mitigation
                .as_ref()
                .is_some_and(|m| m != &GooseCoordinatedOmissionMitigation::Disabled)
    }

    pub(crate) fn validate(&self) -> Result<(), GooseError> {
        // Can't set both --verbose and --quiet.
        if self.verbose > 0 && self.quiet > 0 {
            return Err(GooseError::InvalidOption {
                option: "`configuration.verbose`".to_string(),
                value: self.verbose.to_string(),
                detail: "`configuration.verbose` can not be set with `configuration.quiet`."
                    .to_string(),
            });
        }

        // If set, increase rate must be non-zero, and at least one user a day.
        if let Some(increase_rate) = self.increase_rate.as_ref() {
            if increase_rate == "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.increase_rate`".to_string(),
                    value: increase_rate.to_string(),
                    detail: "`configuration.increase_rate` must be non-zero.".to_string(),
                });
            }
            if increase_rate
                .parse::<f32>()
                .is_ok_and(util::below_one_user_a_day)
            {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.increase_rate`".to_string(),
                    value: increase_rate.to_string(),
                    detail: "`configuration.increase_rate` must be a number of at least 0.0000116 (one user a day).".to_string(),
                });
            }
        }

        // If set, decrease rate must be non-zero, and at least one user a day.
        if let Some(decrease_rate) = self.decrease_rate.as_ref() {
            if decrease_rate == "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.decrease_rate`".to_string(),
                    value: decrease_rate.to_string(),
                    detail: "`configuration.decrease_rate` must be non-zero.".to_string(),
                });
            }
            if decrease_rate
                .parse::<f32>()
                .is_ok_and(util::below_one_user_a_day)
            {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.decrease_rate`".to_string(),
                    value: decrease_rate.to_string(),
                    detail: "`configuration.decrease_rate` must be a number of at least 0.0000116 (one user a day).".to_string(),
                });
            }
        }

        // If set, timeout must be greater than zero.
        if let Some(timeout) = self.timeout.as_ref() {
            if crate::util::get_float_from_string(self.timeout.clone())
                .expect("failed to re-convert string to float")
                <= 0.0
            {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.timeout`".to_string(),
                    value: timeout.to_string(),
                    detail: "`configuration.timeout` must be greater than 0.".to_string(),
                });
            }
        }

        // Validate `users`.
        if let Some(users) = self.users.as_ref() {
            if users == &0 {
                return Err(GooseError::InvalidOption {
                    option: "configuration.users".to_string(),
                    value: users.to_string(),
                    detail: "`configuration.users` must be set to at least 1.".to_string(),
                });
            }
        }

        // Validate `increase_time`.
        if self.increase_time != "0" {
            // Increase time can't be set with increase rate.
            if self.increase_rate.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.increase_time`".to_string(),
                    value: self.increase_time.to_string(),
                    detail: "`configuration.increase_time` can not be set with `configuration.increase_rate`.".to_string(),
                });
            }

            // Increase time requires at least 2 users.
            if let Some(users) = self.users.as_ref() {
                if users < &2 {
                    return Err(GooseError::InvalidOption {
                        option: "configuration.users".to_string(),
                        value: users.to_string(),
                        detail: "`configuration.users` must be set to at least 2 when `configuration.increase_time` is set.".to_string(),
                    });
                }
            }
        }

        // Validate `decrease_time`.
        if self.decrease_time != "0" {
            // Decrease time can't be set with decrease rate.
            if self.decrease_rate.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.decrease_time`".to_string(),
                    value: self.decrease_time.to_string(),
                    detail: "`configuration.decrease_time` can not be set with `configuration.decrease_rate`.".to_string(),
                });
            }

            // Decrease time requires at least 2 users.
            if let Some(users) = self.users.as_ref() {
                if users < &2 {
                    return Err(GooseError::InvalidOption {
                        option: "configuration.users".to_string(),
                        value: users.to_string(),
                        detail: "`configuration.users` must be set to at least 2 when `configuration.decrease_time` is set.".to_string(),
                    });
                }
            }
        }

        // Validate `test_plan`.
        if self.test_plan.is_some() {
            // The --users option isn't compatible with --test-plan.
            if let Some(users) = self.users.as_ref() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.users`".to_string(),
                    value: users.to_string(),
                    detail: "`configuration.users` can not be set with `configuration.test_plan`."
                        .to_string(),
                });
            }
            // The --increase-time option isn't compatible with --test-plan.
            if self.increase_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.increase_time`".to_string(),
                    value: self.increase_time.to_string(),
                    detail: "`configuration.increase_time` can not be set with `configuration.test_plan`.".to_string(),
                });
            }
            // The --increase-rate option isn't compatible with --test-plan.
            if let Some(increase_rate) = self.increase_rate.as_ref() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.increase_rate`".to_string(),
                    value: increase_rate.to_string(),
                    detail:
                        "`configuration.increase_rate` can not be set with `configuration.test_plan`."
                            .to_string(),
                });
            }
            // The --decrease-time option isn't compatible with --test-plan.
            if self.decrease_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.decrease_time`".to_string(),
                    value: self.decrease_time.to_string(),
                    detail: "`configuration.decrease_time` can not be set with `configuration.test_plan`.".to_string(),
                });
            }
            // The --decrease-rate option isn't compatible with --test-plan.
            if let Some(decrease_rate) = self.decrease_rate.as_ref() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.decrease_rate`".to_string(),
                    value: decrease_rate.to_string(),
                    detail:
                        "`configuration.decrease_rate` can not be set with `configuration.test_plan`."
                            .to_string(),
                });
            }
            // The --run-time option isn't compatible with --test-plan.
            if self.run_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.run_time`".to_string(),
                    value: self.run_time.to_string(),
                    detail:
                        "`configuration.run_time` can not be set with `configuration.test_plan`."
                            .to_string(),
                });
            }
            // The --no-reset-metrics option isn't compatible with --test-plan.
            if self.no_reset_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_reset_metrics".to_string(),
                    value: self.no_reset_metrics.to_string(),
                    detail: "`configuration.no_reset_metrics` can not be set with `configuration.test_plan` (metrics are not reset)."
                        .to_string(),
                });
            }
        }

        // Validate `iterations`.
        if self.iterations > 0 {
            // The --run-time option isn't compatible with --iterations.
            if self.run_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.run_time`".to_string(),
                    value: self.run_time.to_string(),
                    detail:
                        "`configuration.run_time` can not be set with `configuration.iterations`."
                            .to_string(),
                });
            }
            // The --test-plan option isn't compatible with --iterations.
            if self.test_plan.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.iterations`".to_string(),
                    value: self.iterations.to_string(),
                    detail:
                        "`configuration.iteratoins` can not be set with `configuration.test_plan`."
                            .to_string(),
                });
            }
            // The --no-reset-metrics option isn't compatible with --iterations.
            if self.no_reset_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_reset_metrics".to_string(),
                    value: self.no_reset_metrics.to_string(),
                    detail: "`configuration.no_reset_metrics` can not be set with `configuration.iterations` (metrics are not reset)."
                        .to_string(),
                });
            }
        }

        // Validate `no_metrics`.
        if self.no_metrics {
            // Request log can't be written if metrics are disabled.
            if !self.request_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.request_log`".to_string(),
                    value: self.request_log.to_string(),
                    detail: "`configuration.request_log` can not be set with `configuration.no_metrics`.".to_string(),
                });
            // Transaction log can't be written if metrics are disabled.
            } else if !self.transaction_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.transaction_log`".to_string(),
                    value: self.transaction_log.to_string(),
                    detail:
                        "`configuration.transaction_log` can not be set with `configuration.no_metrics`."
                            .to_string(),
                });
            // Scenario log can't be written if metrics are disabled.
            } else if !self.scenario_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.scenario_log`".to_string(),
                    value: self.scenario_log.to_string(),
                    detail:
                        "`configuration.scenario_log` can not be set with `configuration.no_metrics`."
                            .to_string(),
                });
            // Error log can't be written if metrics are disabled.
            } else if !self.error_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.error_log`".to_string(),
                    value: self.error_log.to_string(),
                    detail:
                        "`configuration.error_log` can not be set with `configuration.no_metrics`."
                            .to_string(),
                });
            // Report file can't be written if metrics are disabled.
            } else if !self.report_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.report_file`".to_string(),
                    value: format!("{:?}", self.report_file),
                    detail:
                        "`configuration.report_file` can not be set with `configuration.no_metrics`."
                            .to_string(),
                });
            // Coordinated Omission Mitigation can't be enabled if metrics are disabled.
            } else if self.co_mitigation.as_ref().unwrap()
                != &GooseCoordinatedOmissionMitigation::Disabled
            {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.co_mitigation`".to_string(),
                    value: format!("{:?}", self.co_mitigation.as_ref().unwrap()),
                    detail: "`configuration.co_mitigation` can not be set with `configuration.no_metrircs`."
                        .to_string(),
                });
            }
        }

        if self.report_file.is_empty() && !self.saves_runs() && self.no_granular_report {
            return Err(GooseError::InvalidOption {
                option: "`configuration.no_granular_report`".to_string(),
                value: true.to_string(),
                detail:
                    "`configuration.no_granular_report` can not be set without `configuration.report_file` when runs are not saved."
                        .to_string(),
            });
        }

        if !self.dashboard && self.no_percentiles_over_time {
            return Err(GooseError::InvalidOption {
                option: "`configuration.no_percentiles_over_time`".to_string(),
                value: true.to_string(),
                detail:
                    "`configuration.no_percentiles_over_time` can not be set without `configuration.dashboard`."
                        .to_string(),
            });
        }

        // Can't disable autostart unless a Controller or dashboard control can start the test.
        if self.no_autostart && self.no_telnet && self.no_websocket && !self.dashboard_control {
            return Err(GooseError::InvalidOption {
                option: "`configuration.no_autostart`".to_string(),
                value: true.to_string(),
                detail: "`configuration.no_autostart` requires at least one Controller or dashboard control be enabled"
                    .to_string(),
            });
        }

        /* @TODO:
        if let Some(co_mitigation) = self.co_mitigation.as_ref() {
            if co_mitigation != &GooseCoordinatedOmissionMitigation::Disabled
                && self.scheduler == GooseScheduler::Random
            {
                // Coordinated Omission Mitigation is not possible together with the random scheduler,
                // as it's impossible to calculate an accurate request cadence.
                return Err(GooseError::InvalidOption {
                    option: key.to_string(),
                    value: format!("{:?}", value),
                    detail: format!(
                        "{} can not be set together with GooseScheduler::Random.",
                        key
                    ),
                });
            }
        }
        */

        if self.throttle_requests > 0 {
            // Be sure throttle_requests is in allowed range.
            if self.throttle_requests == 0 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.throttle_requests`".to_string(),
                    value: self.throttle_requests.to_string(),
                    detail: "`configuration.throttle_requests` must be set to at least 1 request per second.".to_string(),
                });
            } else if self.throttle_requests > 1_000_000 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.throttle_requests`".to_string(),
                    value: self.throttle_requests.to_string(),
                    detail: "`configuration.throttle_requests` can not be set to more than 1,000,000 request per second.".to_string(),
                });
            }
        }

        // Validate baseline file exists and is parseable before running the load test.
        if let Some(ref baseline_file) = self.baseline_file {
            crate::metrics::load_baseline_file(baseline_file)?;
        }

        // Unified PDF configuration validation
        self.validate_pdf_configuration()?;

        // Dashboard bind host / auth token validation.
        self.validate_dashboard_config()?;

        Ok(())
    }

    /// Validate dashboard configuration.
    ///
    /// When the dashboard is enabled and bound to a non-loopback address, an auth
    /// token is required. Loopback binds may omit the token (local trust model)
    /// unless control endpoints are enabled.
    ///
    /// `--dashboard-control` requires `--dashboard` (does not auto-enable) and a
    /// non-empty auth token even on loopback.
    ///
    /// If `--dashboard` is set but the crate was built without the `dashboard`
    /// feature, returns a compile-time feature error.
    pub(crate) fn validate_dashboard_config(&self) -> Result<(), GooseError> {
        // Control requires --dashboard; do not auto-enable the observe dashboard.
        if self.dashboard_control && !self.dashboard {
            return Err(GooseError::InvalidOption {
                option: "`configuration.dashboard_control`".to_string(),
                value: "true".to_string(),
                detail: "`configuration.dashboard_control` (--dashboard-control) requires `configuration.dashboard` (--dashboard); control does not auto-enable the dashboard".to_string(),
            });
        }

        if !self.dashboard {
            return Ok(());
        }

        // Feature requires the HTTP server feature (DTO/builder always compile).
        if !cfg!(feature = "dashboard") {
            return Err(GooseError::FeatureNotEnabled {
                feature: "dashboard".to_string(),
                detail: "dashboard support not compiled in (rebuild with `--features dashboard`)"
                    .to_string(),
            });
        }

        // Control endpoints always require a configured auth token, even on loopback.
        if self.dashboard_control && self.dashboard_auth_token.is_empty() {
            return Err(GooseError::InvalidOption {
                option: "`configuration.dashboard_control`".to_string(),
                value: "true".to_string(),
                detail: "`configuration.dashboard_control` (--dashboard-control) requires a non-empty `configuration.dashboard_auth_token` (--dashboard-auth-token), even on loopback".to_string(),
            });
        }

        // Effective host for validation (mirrors configure defaults).
        let host = if self.dashboard_host.is_empty() {
            "127.0.0.1"
        } else {
            self.dashboard_host.as_str()
        };

        if !is_loopback_bind(host) && self.dashboard_auth_token.is_empty() {
            return Err(GooseError::InvalidOption {
                option: "`configuration.dashboard_host`".to_string(),
                value: host.to_string(),
                detail: format!(
                    "`configuration.dashboard` is enabled with non-loopback host `{host}`; set `configuration.dashboard_auth_token` (--dashboard-auth-token) when binding outside loopback"
                ),
            });
        }

        Ok(())
    }

    /// Simplified PDF configuration validation.
    ///
    /// This ensures proper error handling when PDF functionality is requested but not available.
    /// Note: --pdf-print-html is always available as it only generates HTML+CSS without requiring Chromium.
    fn validate_pdf_configuration(&self) -> Result<(), GooseError> {
        let has_pdf_reports = self
            .report_file
            .iter()
            .any(|path| path.to_lowercase().ends_with(".pdf"));

        // Check if PDF report generation is requested but feature not compiled
        if !cfg!(feature = "pdf-reports") && has_pdf_reports {
            let pdf_file = self
                .report_file
                .iter()
                .find(|path| path.to_lowercase().ends_with(".pdf"))
                .cloned()
                .unwrap_or_else(|| "*.pdf".to_string());

            return Err(GooseError::InvalidOption {
                option: "--report-file".to_string(),
                value: pdf_file,
                detail: "PDF reports require compiling with the 'pdf-reports' feature flag. Use: cargo build --features pdf-reports".to_string(),
            });
        }

        // Scale validation when feature is available
        #[cfg(feature = "pdf-reports")]
        if has_pdf_reports {
            let pdf_scale = self.pdf_scale_or_default();
            if !(0.1..=2.0).contains(&pdf_scale) {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.pdf_scale`".to_string(),
                    value: pdf_scale.to_string(),
                    detail: "`configuration.pdf_scale` must be between 0.1 and 2.0 (inclusive)."
                        .to_string(),
                });
            }
        }

        // PDF generation timeout validation when feature is available
        #[cfg(feature = "pdf-reports")]
        if has_pdf_reports {
            let pdf_timeout = self.pdf_timeout_or_default();
            if !(10..=600).contains(&pdf_timeout) {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.pdf_timeout`".to_string(),
                    value: pdf_timeout.to_string(),
                    detail:
                        "`configuration.pdf_timeout` must be between 10 and 600 seconds (inclusive)."
                            .to_string(),
                });
            }
        }

        // A CHROME that can't run fails here, before anything is downloaded
        // or any user starts.
        #[cfg(feature = "pdf-reports")]
        if has_pdf_reports {
            if let Some(chrome) = crate::report::chrome::chrome_env() {
                crate::report::chrome::check_chrome_env(&chrome)
                    .map_err(crate::report::chrome::chrome_env_error)?;
            }
        }

        Ok(())
    }

    /// Optionally initialize the Goose logger which writes to standard out and/or to
    /// a configurable log file.
    pub(crate) fn initialize_goose_logger(&self) {
        // Configure debug output level.
        let debug_level = match self.verbose {
            0 => match self.quiet {
                0 => LevelFilter::Info,
                _ => LevelFilter::Warn,
            },
            1 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        };

        // Configure Goose log level.
        let log_level = match self.log_level {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        };

        // Open the log file if configured.
        let goose_log: Option<PathBuf> = if !self.goose_log.is_empty() {
            Some(PathBuf::from(&self.goose_log))
        // Otherwise disable the log.
        } else {
            None
        };

        if let Some(log_to_file) = goose_log {
            match CombinedLogger::init(vec![
                SimpleLogger::new(debug_level, Config::default()),
                WriteLogger::new(
                    log_level,
                    Config::default(),
                    std::fs::File::create(&log_to_file).unwrap(),
                ),
            ]) {
                Ok(_) => (),
                Err(e) => {
                    info!("failed to initialize CombinedLogger: {e}");
                }
            }
            info!("Writing to log file: {}", log_to_file.display());
        } else {
            match CombinedLogger::init(vec![SimpleLogger::new(debug_level, Config::default())]) {
                Ok(_) => (),
                Err(e) => {
                    info!("failed to initialize CombinedLogger: {e}");
                }
            }
        }

        info!("Output verbosity level: {debug_level}");
        info!("Logfile verbosity level: {log_level}");
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn set_defaults() {
        let host = "http://example.com/".to_string();
        let users: usize = 10;
        let run_time: usize = 10;
        let increase_rate = "2".to_string();
        let timeout = "45".to_string();
        let log_level: usize = 1;
        let goose_log = "custom-goose.log".to_string();
        let quiet: usize = 0;
        let verbose: usize = 0;
        let report_file = "custom-goose-report.html".to_string();
        let request_log = "custom-goose-request.log".to_string();
        let transaction_log = "custom-goose-transaction.log".to_string();
        let scenario_log = "custom-goose-scenario.log".to_string();
        let debug_log = "custom-goose-debug.log".to_string();
        let error_log = "custom-goose-error.log".to_string();
        let throttle_requests: usize = 25;

        // Not `initialize()`, which would parse the test harness's own arguments.
        let goose_attack = GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::Host, host.as_str())
            .unwrap()
            .set_default(GooseDefault::Users, users)
            .unwrap()
            .set_default(GooseDefault::RunTime, run_time)
            .unwrap()
            .set_default(GooseDefault::IncreaseRate, increase_rate.as_str())
            .unwrap()
            .set_default(GooseDefault::LogLevel, log_level)
            .unwrap()
            .set_default(GooseDefault::GooseLog, goose_log.as_str())
            .unwrap()
            .set_default(GooseDefault::Quiet, quiet)
            .unwrap()
            .set_default(GooseDefault::Verbose, verbose)
            .unwrap()
            .set_default(GooseDefault::Timeout, timeout.as_str())
            .unwrap()
            .set_default(GooseDefault::RunningMetrics, 15)
            .unwrap()
            .set_default(GooseDefault::NoResetMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoTransactionMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoScenarioMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoPrintMetrics, true)
            .unwrap()
            .set_default(GooseDefault::NoErrorSummary, true)
            .unwrap()
            .set_default(GooseDefault::NoTelnet, true)
            .unwrap()
            .set_default(GooseDefault::NoWebSocket, true)
            .unwrap()
            .set_default(GooseDefault::NoAutoStart, true)
            .unwrap()
            .set_default(GooseDefault::NoGzip, true)
            .unwrap()
            .set_default(GooseDefault::ReportFile, report_file.as_str())
            .unwrap()
            .set_default(GooseDefault::RequestLog, request_log.as_str())
            .unwrap()
            .set_default(GooseDefault::RequestFormat, GooseLogFormat::Raw)
            .unwrap()
            .set_default(GooseDefault::RequestBody, true)
            .unwrap()
            .set_default(GooseDefault::TransactionLog, transaction_log.as_str())
            .unwrap()
            .set_default(GooseDefault::TransactionFormat, GooseLogFormat::Raw)
            .unwrap()
            .set_default(GooseDefault::ScenarioLog, scenario_log.as_str())
            .unwrap()
            .set_default(GooseDefault::ScenarioFormat, GooseLogFormat::Raw)
            .unwrap()
            .set_default(GooseDefault::ErrorLog, error_log.as_str())
            .unwrap()
            .set_default(GooseDefault::ErrorFormat, GooseLogFormat::Csv)
            .unwrap()
            .set_default(GooseDefault::DebugLog, debug_log.as_str())
            .unwrap()
            .set_default(GooseDefault::DebugFormat, GooseLogFormat::Csv)
            .unwrap()
            .set_default(GooseDefault::NoDebugBody, true)
            .unwrap()
            .set_default(GooseDefault::NoStatusCodes, true)
            .unwrap()
            .set_default(
                GooseDefault::CoordinatedOmissionMitigation,
                GooseCoordinatedOmissionMitigation::Disabled,
            )
            .unwrap()
            .set_default(GooseDefault::ThrottleRequests, throttle_requests)
            .unwrap()
            .set_default(GooseDefault::StickyFollow, true)
            .unwrap()
            .set_default(GooseDefault::Dashboard, true)
            .unwrap()
            .set_default(GooseDefault::DashboardControl, true)
            .unwrap()
            .set_default(GooseDefault::DashboardHost, "127.0.0.1")
            .unwrap()
            .set_default(GooseDefault::DashboardPort, 5118)
            .unwrap()
            .set_default(GooseDefault::DashboardAuthToken, "secret")
            .unwrap()
            .set_default(GooseDefault::DashboardMaxClients, 64)
            .unwrap();

        assert!(goose_attack.defaults.host == Some(host));
        assert!(goose_attack.defaults.users == Some(users));
        assert!(goose_attack.defaults.run_time == Some(run_time));
        assert!(goose_attack.defaults.increase_rate == Some(increase_rate));
        assert!(goose_attack.defaults.log_level == Some(log_level as u8));
        assert!(goose_attack.defaults.goose_log == Some(goose_log));
        assert!(goose_attack.defaults.request_body == Some(true));
        assert!(goose_attack.defaults.no_debug_body == Some(true));
        assert!(goose_attack.defaults.quiet == Some(quiet as u8));
        assert!(goose_attack.defaults.verbose == Some(verbose as u8));
        assert!(goose_attack.defaults.running_metrics == Some(15));
        assert!(goose_attack.defaults.no_reset_metrics == Some(true));
        assert!(goose_attack.defaults.no_metrics == Some(true));
        assert!(goose_attack.defaults.no_transaction_metrics == Some(true));
        assert!(goose_attack.defaults.no_scenario_metrics == Some(true));
        assert!(goose_attack.defaults.no_print_metrics == Some(true));
        assert!(goose_attack.defaults.no_error_summary == Some(true));
        assert!(goose_attack.defaults.no_telnet == Some(true));
        assert!(goose_attack.defaults.no_websocket == Some(true));
        assert!(goose_attack.defaults.no_autostart == Some(true));
        assert!(goose_attack.defaults.timeout == Some(timeout));
        assert!(goose_attack.defaults.no_gzip == Some(true));
        assert!(goose_attack.defaults.report_file == Some(vec![report_file]));
        assert!(goose_attack.defaults.request_log == Some(request_log));
        assert!(goose_attack.defaults.request_format == Some(GooseLogFormat::Raw));
        assert!(goose_attack.defaults.error_log == Some(error_log));
        assert!(goose_attack.defaults.error_format == Some(GooseLogFormat::Csv));
        assert!(goose_attack.defaults.debug_log == Some(debug_log));
        assert!(goose_attack.defaults.debug_format == Some(GooseLogFormat::Csv));
        assert!(goose_attack.defaults.no_status_codes == Some(true));
        assert!(
            goose_attack.defaults.co_mitigation
                == Some(GooseCoordinatedOmissionMitigation::Disabled)
        );
        assert!(goose_attack.defaults.throttle_requests == Some(throttle_requests));
        assert!(goose_attack.defaults.sticky_follow == Some(true));
        assert!(goose_attack.defaults.dashboard == Some(true));
        assert!(goose_attack.defaults.dashboard_control == Some(true));
        assert!(goose_attack.defaults.dashboard_host == Some("127.0.0.1".to_string()));
        assert!(goose_attack.defaults.dashboard_port == Some(5118));
        assert!(goose_attack.defaults.dashboard_auth_token == Some("secret".to_string()));
        assert!(goose_attack.defaults.dashboard_max_clients == Some(64));
    }

    #[test]
    fn test_is_loopback_bind() {
        // IPv4 loopback (entire 127.0.0.0/8).
        assert!(is_loopback_bind("127.0.0.1"));
        assert!(is_loopback_bind("127.0.0.2"));
        assert!(is_loopback_bind(" 127.0.0.1 "));

        // IPv6 loopback, with and without brackets.
        assert!(is_loopback_bind("::1"));
        assert!(is_loopback_bind("[::1]"));

        // Special-cased hostname (no DNS).
        assert!(is_loopback_bind("localhost"));
        assert!(is_loopback_bind("LOCALHOST"));
        assert!(is_loopback_bind("LocalHost"));

        // Unspecified addresses are not loopback for auth policy.
        assert!(!is_loopback_bind("0.0.0.0"));
        assert!(!is_loopback_bind("::"));
        assert!(!is_loopback_bind("[::]"));

        // LAN / public / hostnames are not loopback.
        assert!(!is_loopback_bind("192.168.1.10"));
        assert!(!is_loopback_bind("loadgen.example.com"));
        assert!(!is_loopback_bind(""));
    }

    #[test]
    #[cfg(feature = "dashboard")]
    fn test_validate_dashboard_config() {
        // Disabled dashboard: always ok.
        let mut config = GooseConfiguration::default();
        assert!(config.validate_dashboard_config().is_ok());

        // Loopback without token: ok.
        config.dashboard = true;
        config.dashboard_host = "127.0.0.1".to_string();
        config.dashboard_auth_token = String::new();
        assert!(config.validate_dashboard_config().is_ok());

        config.dashboard_host = "localhost".to_string();
        assert!(config.validate_dashboard_config().is_ok());

        config.dashboard_host = "::1".to_string();
        assert!(config.validate_dashboard_config().is_ok());

        config.dashboard_host = "[::1]".to_string();
        assert!(config.validate_dashboard_config().is_ok());

        // Empty host treated as default loopback: ok without token.
        config.dashboard_host = String::new();
        assert!(config.validate_dashboard_config().is_ok());

        // Non-loopback without token: hard error.
        config.dashboard_host = "0.0.0.0".to_string();
        assert!(config.validate_dashboard_config().is_err());

        config.dashboard_host = "192.168.1.10".to_string();
        assert!(config.validate_dashboard_config().is_err());

        config.dashboard_host = "loadgen.example.com".to_string();
        assert!(config.validate_dashboard_config().is_err());

        // Non-loopback with token: ok.
        config.dashboard_auth_token = "s3cret".to_string();
        assert!(config.validate_dashboard_config().is_ok());

        // Token configured on loopback is also fine (enforced at runtime later).
        config.dashboard_host = "127.0.0.1".to_string();
        assert!(config.validate_dashboard_config().is_ok());

        // Control without --dashboard: hard error (does not auto-enable).
        let mut config = GooseConfiguration {
            dashboard: false,
            dashboard_control: true,
            dashboard_auth_token: "s3cret".to_string(),
            ..Default::default()
        };
        assert!(config.validate_dashboard_config().is_err());

        // Control with dashboard but empty token on loopback: hard error.
        config.dashboard = true;
        config.dashboard_host = "127.0.0.1".to_string();
        config.dashboard_auth_token = String::new();
        assert!(config.validate_dashboard_config().is_err());

        // Control with dashboard but empty token on localhost: hard error.
        config.dashboard_host = "localhost".to_string();
        assert!(config.validate_dashboard_config().is_err());

        // Control with dashboard and token on loopback: ok.
        config.dashboard_auth_token = "s3cret".to_string();
        assert!(config.validate_dashboard_config().is_ok());

        // Control with dashboard and token on non-loopback: ok.
        config.dashboard_host = "0.0.0.0".to_string();
        assert!(config.validate_dashboard_config().is_ok());
    }

    #[test]
    #[cfg(feature = "dashboard")]
    fn test_configure_dashboard_defaults() {
        // --dashboard only → host 127.0.0.1, port 5118, empty token, max clients 32.
        let mut config = GooseConfiguration {
            dashboard: true,
            ..Default::default()
        };
        config.configure(&GooseDefaults::default()).unwrap();
        assert!(config.dashboard);
        assert!(!config.dashboard_control);
        assert_eq!(config.dashboard_host, "127.0.0.1");
        assert_eq!(config.dashboard_port, 5118);
        assert!(config.dashboard_auth_token.is_empty());
        assert_eq!(config.dashboard_max_clients, 32);
        assert!(config.validate_dashboard_config().is_ok());

        // Dashboard disabled → host stays empty, port stays 0, max clients stays 0.
        let mut config = GooseConfiguration::default();
        config.configure(&GooseDefaults::default()).unwrap();
        assert!(!config.dashboard);
        assert!(!config.dashboard_control);
        assert!(config.dashboard_host.is_empty());
        assert_eq!(config.dashboard_port, 0);
        assert!(config.dashboard_auth_token.is_empty());
        assert_eq!(config.dashboard_max_clients, 0);

        // CLI host/port/max-clients override GooseDefault.
        let mut config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "192.168.1.10".to_string(),
            dashboard_port: 9999,
            dashboard_auth_token: "cli-token".to_string(),
            dashboard_max_clients: 8,
            ..Default::default()
        };
        let defaults = GooseDefaults {
            dashboard_host: Some("10.0.0.1".to_string()),
            dashboard_port: Some(4000),
            dashboard_auth_token: Some("default-token".to_string()),
            dashboard_max_clients: Some(64),
            ..GooseDefaults::default()
        };
        config.configure(&defaults).unwrap();
        assert_eq!(config.dashboard_host, "192.168.1.10");
        assert_eq!(config.dashboard_port, 9999);
        assert_eq!(config.dashboard_auth_token, "cli-token");
        assert_eq!(config.dashboard_max_clients, 8);
        assert!(config.validate_dashboard_config().is_ok());

        // GooseDefault enablement without CLI flag; host/port from defaults.
        let mut config = GooseConfiguration::default();
        let defaults = GooseDefaults {
            dashboard: Some(true),
            dashboard_host: Some("0.0.0.0".to_string()),
            dashboard_port: Some(6000),
            ..GooseDefaults::default()
        };
        config.configure(&defaults).unwrap();
        assert!(config.dashboard);
        assert_eq!(config.dashboard_host, "0.0.0.0");
        assert_eq!(config.dashboard_port, 6000);
        assert_eq!(config.dashboard_max_clients, 32);
        // Non-loopback without token fails validation after configure.
        assert!(config.validate_dashboard_config().is_err());

        // GooseDefault token fills when CLI token is empty.
        let mut config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "0.0.0.0".to_string(),
            ..Default::default()
        };
        let defaults = GooseDefaults {
            dashboard_auth_token: Some("from-default".to_string()),
            dashboard_max_clients: Some(16),
            ..GooseDefaults::default()
        };
        config.configure(&defaults).unwrap();
        assert_eq!(config.dashboard_auth_token, "from-default");
        assert_eq!(config.dashboard_max_clients, 16);
        assert!(config.validate_dashboard_config().is_ok());

        // GooseDefault enables dashboard_control when CLI flag is unset.
        let mut config = GooseConfiguration {
            dashboard: true,
            dashboard_auth_token: "token".to_string(),
            ..Default::default()
        };
        let defaults = GooseDefaults {
            dashboard_control: Some(true),
            ..GooseDefaults::default()
        };
        config.configure(&defaults).unwrap();
        assert!(config.dashboard_control);
        assert!(config.validate_dashboard_config().is_ok());

        // CLI --dashboard-control takes precedence over GooseDefault false.
        let mut config = GooseConfiguration {
            dashboard: true,
            dashboard_control: true,
            dashboard_auth_token: "token".to_string(),
            ..Default::default()
        };
        let defaults = GooseDefaults {
            dashboard_control: Some(false),
            ..GooseDefaults::default()
        };
        config.configure(&defaults).unwrap();
        assert!(config.dashboard_control);
        assert!(config.validate_dashboard_config().is_ok());
    }

    #[test]
    fn graph_data_helpers_truth_table() {
        // (no_save, no_metrics, report_file, dashboard) ->
        //     (saves_runs, records_graph_data, keeps_full_graph)
        let cases = [
            ((false, false, false, false), (true, true, true)),
            ((false, false, false, true), (true, true, true)),
            ((false, false, true, false), (true, true, true)),
            ((false, false, true, true), (true, true, true)),
            ((true, false, false, false), (false, false, false)),
            ((true, false, false, true), (false, true, false)),
            ((true, false, true, false), (false, true, true)),
            ((true, false, true, true), (false, true, true)),
            // --no-metrics never saves a run.
            ((false, true, false, false), (false, false, false)),
            ((false, true, false, true), (false, true, false)),
        ];
        for ((no_save, no_metrics, report_file, dashboard), expected) in cases {
            let configuration = GooseConfiguration {
                no_save,
                no_metrics,
                report_file: if report_file {
                    vec!["report.html".to_string()]
                } else {
                    Vec::new()
                },
                dashboard,
                ..Default::default()
            };
            assert_eq!(
                (
                    configuration.saves_runs(),
                    configuration.records_graph_data(),
                    configuration.keeps_full_graph()
                ),
                expected,
                "no_save={no_save} no_metrics={no_metrics} report_file={report_file} dashboard={dashboard}"
            );
        }
    }

    #[test]
    fn runs_dir_defaults_to_goose_runs() {
        let mut configuration = GooseConfiguration::default();
        assert_eq!(configuration.runs_dir_or_default(), "goose-runs");
        configuration.runs_dir = "elsewhere".to_string();
        assert_eq!(configuration.runs_dir_or_default(), "elsewhere");
    }

    #[test]
    fn no_granular_report_allowed_when_saving_runs() {
        let mut configuration = GooseConfiguration {
            no_granular_report: true,
            ..Default::default()
        };
        assert!(configuration.saves_runs());
        assert!(configuration.validate().is_ok());
        configuration.no_save = true;
        assert!(configuration.validate().is_err());
        configuration.report_file = vec!["report.html".to_string()];
        assert!(configuration.validate().is_ok());
    }

    #[test]
    #[cfg(feature = "dashboard")]
    fn test_no_autostart_allows_dashboard_control() {
        // Helper: timespan fields must be "0" so validate() skips empty-string branches.
        let base = || GooseConfiguration {
            increase_time: "0".to_string(),
            decrease_time: "0".to_string(),
            run_time: "0".to_string(),
            no_autostart: true,
            no_telnet: true,
            no_websocket: true,
            ..Default::default()
        };

        // no_autostart with no Controllers and no dashboard-control: hard error.
        let config = base();
        assert!(config.validate().is_err());

        // no_autostart + dashboard-control (with dashboard + token): OK.
        let mut config = base();
        config.dashboard = true;
        config.dashboard_control = true;
        config.dashboard_auth_token = "s3cret".to_string();
        assert!(config.validate().is_ok());

        // no_autostart + telnet Controller still OK without dashboard-control.
        let mut config = base();
        config.no_telnet = false;
        assert!(config.validate().is_ok());
    }

    /// Without the `dashboard` feature, enabling `--dashboard` must fail validation.
    #[test]
    #[cfg(not(feature = "dashboard"))]
    fn test_dashboard_requires_feature() {
        let mut config = GooseConfiguration {
            dashboard: true,
            dashboard_host: "127.0.0.1".to_string(),
            ..Default::default()
        };
        assert!(config.validate_dashboard_config().is_err());
        // Control without feature also fails when dashboard is on.
        config.dashboard_control = true;
        config.dashboard_auth_token = "s3cret".to_string();
        assert!(config.validate_dashboard_config().is_err());
        // Disabled dashboard remains ok without the feature.
        config.dashboard = false;
        config.dashboard_control = false;
        assert!(config.validate_dashboard_config().is_ok());
    }

    #[test]
    fn no_percentiles_over_time_requires_dashboard() {
        let config = GooseConfiguration {
            no_percentiles_over_time: true,
            ..Default::default()
        };
        match config.validate() {
            Err(GooseError::InvalidOption { option, detail, .. }) => {
                assert_eq!(option, "`configuration.no_percentiles_over_time`");
                assert_eq!(
                    detail,
                    "`configuration.no_percentiles_over_time` can not be set without `configuration.dashboard`."
                );
            }
            other => panic!("expected InvalidOption, got {:?}", other),
        }
    }

    #[test]
    fn no_percentiles_over_time_from_default() {
        let goose_attack = GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::NoPercentilesOverTime, true)
            .unwrap();
        assert_eq!(goose_attack.defaults.no_percentiles_over_time, Some(true));
        let mut config = GooseConfiguration {
            dashboard: true,
            ..Default::default()
        };
        config.configure(&goose_attack.defaults).unwrap();
        assert!(config.no_percentiles_over_time);

        // Without the dashboard the default is ignored, while the flag on
        // the command line is still refused.
        let mut config = GooseConfiguration::default();
        config.configure(&goose_attack.defaults).unwrap();
        assert!(!config.no_percentiles_over_time);
        assert!(config.validate().is_ok());
        let mut config =
            GooseConfiguration::parse_args_default(&["--no-percentiles-over-time"]).unwrap();
        config.configure(&goose_attack.defaults).unwrap();
        assert!(config.validate().is_err());
    }

    /// A test plan that overflows, or that has no users, is an error on the
    /// command line instead of a panic or a load test that never starts.
    #[test]
    fn test_plan_overflow_and_no_users_refused() {
        for plan in [
            "99999999999999999999,1s",
            "10,99999999999999999999h",
            "10,9999999999999999h",
            "10,18446744073709552",
            "18446744073709551615,1s;0,1s;1,1s",
            "0,1s",
            "١,1s",
            "10,١s",
            "0,1s;0,1h",
        ] {
            assert!(
                GooseConfiguration::parse_args_default(&["--test-plan", plan]).is_err(),
                "{}",
                plan
            );
        }
        // What was accepted is still accepted.
        for plan in ["10,1s;0,1s", "1,0", "100001,1h", "10, 1h2m3s ;0,30s"] {
            assert!(
                GooseConfiguration::parse_args_default(&["--test-plan", plan]).is_ok(),
                "{}",
                plan
            );
        }
    }

    /// `set_default(GooseDefault::TestPlan, ..)` returns an error for a plan
    /// it can't use instead of panicking.
    #[test]
    fn set_default_test_plan_refused() {
        for plan in ["foo", "99999999999999999999,1s", "0,1s"] {
            let result = GooseAttack::initialize_with_config(GooseConfiguration::default())
                .unwrap()
                .set_default(GooseDefault::TestPlan, plan);
            assert!(result.is_err(), "{}", plan);
        }
    }

    /// A time span too large on the command line, or as a default, stops the
    /// load test with an error before it starts instead of panicking.
    #[tokio::test]
    #[serial_test::serial]
    async fn time_span_overflow_refused() {
        use crate::goose::{Scenario, Transaction};

        async fn noop(_user: &mut crate::goose::GooseUser) -> crate::goose::TransactionResult {
            // Yield, so a load test that wrongly starts leaves the timer running.
            tokio::task::yield_now().await;
            Ok(())
        }
        // Two users, so the rule that increase and decrease times need at least
        // two users can not refuse a row in place of the overflow check.
        let base = [
            "--users",
            "2",
            "--no-telnet",
            "--no-websocket",
            "--no-save",
            "--quiet",
            "--host",
            "http://127.0.0.1",
        ];
        let scenario =
            || crate::scenario!("Overflow").register_transaction(crate::transaction!(noop));
        // Every row must be refused as too long, not by another rule. A value that
        // wraps instead of failing starts a load test that never ends, so each
        // row runs under a timeout and fails instead of hanging. Failures are
        // collected so the report names every failing row.
        let mut failures = Vec::new();
        let mut check = |row: String, option: &str, result| match result {
            Ok(Err(GooseError::InvalidOption { detail, .. }))
                if detail == format!("`configuration.{}` is too long.", option) => {}
            Ok(other) => failures.push(format!("{}: {:?}", row, other.map(|_| ()))),
            Err(_) => failures.push(format!("{}: started a load test", row)),
        };
        let timeout = std::time::Duration::from_secs(10);
        for (flag, option) in [
            ("--run-time", "run_time"),
            ("--increase-time", "increase_time"),
            ("--decrease-time", "decrease_time"),
        ] {
            for value in [
                "99999999999999999999h",
                "9999999999999999h",
                "18446744073709552",
                "5124095576030431h100s",
            ] {
                let mut args = base.to_vec();
                args.extend([flag, value]);
                let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
                let attack = GooseAttack::initialize_with_config(configuration)
                    .unwrap()
                    .register_scenario(scenario());
                let result = tokio::time::timeout(timeout, attack.execute()).await;
                check(format!("{} {}", flag, value), option, result);
            }
        }
        for (option, key) in [
            ("run_time", GooseDefault::RunTime),
            ("increase_time", GooseDefault::IncreaseTime),
            ("decrease_time", GooseDefault::DecreaseTime),
        ] {
            let configuration = GooseConfiguration::parse_args_default(&base).unwrap();
            let attack = GooseAttack::initialize_with_config(configuration)
                .unwrap()
                .register_scenario(scenario())
                .set_default(key, usize::MAX)
                .unwrap();
            let result = tokio::time::timeout(timeout, attack.execute()).await;
            check(format!("default {}", option), option, result);
        }
        assert!(failures.is_empty(), "{:#?}", failures);
    }

    /// A command line rate slower than one user a day is refused, and one
    /// user a day is accepted.
    #[test]
    fn rate_below_one_user_a_day_refused() {
        for (flag, option) in [
            ("--increase-rate", "increase_rate"),
            ("--decrease-rate", "decrease_rate"),
        ] {
            // Not below one user a day, so accepted as on main.
            for (value, refused) in [
                ("0.0000001", true),
                ("0.00001157", true),
                ("0.0000116", false),
                ("inf", false),
                ("NaN", false),
            ] {
                let mut config =
                    GooseConfiguration::parse_args_default(&["--users", "2", flag, value]).unwrap();
                config.configure(&GooseDefaults::default()).unwrap();
                match config.validate() {
                    Err(GooseError::InvalidOption { detail, .. }) if refused => assert_eq!(
                        detail,
                        format!(
                            "`configuration.{option}` must be a number of at least 0.0000116 (one user a day)."
                        )
                    ),
                    Ok(()) if !refused => {}
                    other => panic!("{} {}: {:?}", flag, value, other),
                }
            }
            // Zero keeps its own refusal.
            let mut config =
                GooseConfiguration::parse_args_default(&["--users", "2", flag, "0"]).unwrap();
            config.configure(&GooseDefaults::default()).unwrap();
            match config.validate() {
                Err(GooseError::InvalidOption { detail, .. }) => assert_eq!(
                    detail,
                    format!("`configuration.{}` must be non-zero.", option)
                ),
                other => panic!("{} 0: {:?}", flag, other),
            }
        }
    }

    /// A command line time span names why it is refused: a digit of another
    /// script before a unit, or a span too long.
    #[test]
    fn time_span_error_names_cause() {
        for (value, detail) in [
            (
                "\u{661}h",
                "`configuration.run_time` must use the ASCII digits 0 to 9.",
            ),
            (
                "99999999999999999999h",
                "`configuration.run_time` is too long.",
            ),
        ] {
            let mut config =
                GooseConfiguration::parse_args_default(&["--run-time", value]).unwrap();
            match config.configure(&GooseDefaults::default()) {
                Err(GooseError::InvalidOption { detail: got, .. }) => {
                    assert_eq!(got, detail, "{value}")
                }
                other => panic!("{}: {:?}", value, other),
            }
        }
    }

    /// Serializes a configuration so two can be compared field by field.
    fn as_json(configuration: &GooseConfiguration) -> serde_json::Value {
        serde_json::to_value(configuration).unwrap()
    }

    /// A command line that uses every option at least once sets every field.
    #[test]
    fn command_line_sets_every_option() {
        let mut args = vec![
            "--version",
            "--list",
            "--host",
            "http://example.com/",
            "--users",
            "3",
            "--increase-rate",
            "2",
            "--increase-time",
            "5s",
            "--decrease-rate",
            "4",
            "--decrease-time",
            "6s",
            "--run-time",
            "7s",
            "--goose-log",
            "goose.log",
            "--log-level",
            "--log-level",
            "--quiet",
            "--verbose",
            "--verbose",
            "--verbose",
            "--running-metrics",
            "15",
            "--no-reset-metrics",
            "--no-metrics",
            "--no-transaction-metrics",
            "--no-scenario-metrics",
            "--no-print-metrics",
            "--no-error-summary",
            "--report-file",
            "a.html",
            "--report-file",
            "b.json",
            "--no-save",
            "--runs-dir",
            "runs",
            "--baseline-file",
            "baseline.json",
            "--no-granular-report",
            "--pdf-print-html",
            "print.html",
            "--request-log",
            "request.log",
            "--request-format",
            "json",
            "--request-body",
            "--transaction-log",
            "transaction.log",
            "--transaction-format",
            "csv",
            "--scenario-log",
            "scenario.log",
            "--scenario-format",
            "raw",
            "--error-log",
            "error.log",
            "--error-format",
            "pretty",
            "--debug-log",
            "debug.log",
            "--debug-format",
            "json",
            "--no-debug-body",
            "--no-status-codes",
            "--test-plan",
            "10,60s;0,30s",
            "--iterations",
            "8",
            "--scenarios",
            "one,two*",
            "--scenarios-list",
            "--no-telnet",
            "--telnet-host",
            "127.0.0.2",
            "--telnet-port",
            "6000",
            "--no-websocket",
            "--websocket-host",
            "127.0.0.3",
            "--websocket-port",
            "6001",
            "--dashboard",
            "--dashboard-control",
            "--dashboard-host",
            "127.0.0.4",
            "--dashboard-port",
            "6002",
            "--dashboard-auth-token",
            "token",
            "--dashboard-max-clients",
            "9",
            "--no-percentiles-over-time",
            "--no-autostart",
            "--no-gzip",
            "--timeout",
            "30",
            "--co-mitigation",
            "average",
            "--throttle-requests",
            "11",
            "--sticky-follow",
            "--accept-invalid-certs",
        ];
        if cfg!(feature = "pdf-reports") {
            args.extend(["--pdf-scale", "1.5", "--pdf-timeout", "120"]);
        }

        let c = GooseConfiguration::parse_args_default(&args).unwrap();
        assert!(c.version);
        assert!(c.list);
        assert_eq!(c.host, "http://example.com/");
        assert_eq!(c.users, Some(3));
        assert_eq!(c.increase_rate.as_deref(), Some("2"));
        assert_eq!(c.increase_time, "5s");
        assert_eq!(c.decrease_rate.as_deref(), Some("4"));
        assert_eq!(c.decrease_time, "6s");
        assert_eq!(c.run_time, "7s");
        assert_eq!(c.goose_log, "goose.log");
        assert_eq!(c.log_level, 2);
        assert_eq!(c.quiet, 1);
        assert_eq!(c.verbose, 3);
        assert_eq!(c.running_metrics, Some(15));
        assert!(c.no_reset_metrics);
        assert!(c.no_metrics);
        assert!(c.no_transaction_metrics);
        assert!(c.no_scenario_metrics);
        assert!(c.no_print_metrics);
        assert!(c.no_error_summary);
        assert_eq!(c.report_file, vec!["a.html", "b.json"]);
        assert!(c.no_save);
        assert_eq!(c.runs_dir, "runs");
        assert_eq!(c.baseline_file.as_deref(), Some("baseline.json"));
        assert!(c.no_granular_report);
        assert_eq!(c.pdf_print_html, "print.html");
        #[cfg(feature = "pdf-reports")]
        {
            assert_eq!(c.pdf_scale, Some(1.5));
            assert_eq!(c.pdf_timeout, Some(120));
        }
        assert_eq!(c.request_log, "request.log");
        assert_eq!(c.request_format, Some(GooseLogFormat::Json));
        assert!(c.request_body);
        assert_eq!(c.transaction_log, "transaction.log");
        assert_eq!(c.transaction_format, Some(GooseLogFormat::Csv));
        assert_eq!(c.scenario_log, "scenario.log");
        assert_eq!(c.scenario_format, Some(GooseLogFormat::Raw));
        assert_eq!(c.error_log, "error.log");
        assert_eq!(c.error_format, Some(GooseLogFormat::Pretty));
        assert_eq!(c.debug_log, "debug.log");
        assert_eq!(c.debug_format, Some(GooseLogFormat::Json));
        assert!(c.no_debug_body);
        assert!(c.no_status_codes);
        assert_eq!(
            c.test_plan.unwrap().steps,
            TestPlan::from_str("10,60s;0,30s").unwrap().steps
        );
        assert_eq!(c.iterations, 8);
        assert_eq!(c.scenarios.active, vec!["one", "two*"]);
        assert!(c.scenarios_list);
        assert!(c.no_telnet);
        assert_eq!(c.telnet_host, "127.0.0.2");
        assert_eq!(c.telnet_port, 6000);
        assert!(c.no_websocket);
        assert_eq!(c.websocket_host, "127.0.0.3");
        assert_eq!(c.websocket_port, 6001);
        assert!(c.dashboard);
        assert!(c.dashboard_control);
        assert_eq!(c.dashboard_host, "127.0.0.4");
        assert_eq!(c.dashboard_port, 6002);
        assert_eq!(c.dashboard_auth_token, "token");
        assert_eq!(c.dashboard_max_clients, 9);
        assert!(c.no_percentiles_over_time);
        assert!(c.no_autostart);
        assert!(c.no_gzip);
        assert_eq!(c.timeout.as_deref(), Some("30"));
        assert_eq!(
            c.co_mitigation,
            Some(GooseCoordinatedOmissionMitigation::Average)
        );
        assert_eq!(c.throttle_requests, 11);
        assert!(c.sticky_follow);
        assert!(c.accept_invalid_certs);
    }

    /// An option left off the command line keeps its `Default` value.
    #[test]
    fn absent_options_take_their_defaults() {
        let c = GooseConfiguration::parse_args_default::<&str>(&[]).unwrap();
        assert_eq!(as_json(&c), as_json(&GooseConfiguration::default()));
        assert!(c.scenarios.active.is_empty());
    }

    /// Generates a test that a short option sets the same field as its long name,
    /// and that the field differs from its default.
    macro_rules! short_option_test {
        ($name:ident, $short:expr, $long:expr) => {
            #[test]
            fn $name() {
                let short = GooseConfiguration::parse_args_default(&$short).unwrap();
                let long = GooseConfiguration::parse_args_default(&$long).unwrap();
                let none = GooseConfiguration::parse_args_default::<&str>(&[]).unwrap();
                assert_eq!(as_json(&short), as_json(&long));
                assert_ne!(as_json(&short), as_json(&none));
            }
        };
    }

    short_option_test!(short_option_upper_v, ["-V"], ["--version"]);
    short_option_test!(short_option_l, ["-l"], ["--list"]);
    short_option_test!(
        short_option_upper_h,
        ["-H", "http://example.com/"],
        ["--host", "http://example.com/"]
    );
    short_option_test!(short_option_u, ["-u", "5"], ["--users", "5"]);
    short_option_test!(short_option_r, ["-r", "3"], ["--increase-rate", "3"]);
    short_option_test!(short_option_s, ["-s", "10s"], ["--increase-time", "10s"]);
    short_option_test!(short_option_t, ["-t", "1m"], ["--run-time", "1m"]);
    short_option_test!(
        short_option_upper_g,
        ["-G", "g.log"],
        ["--goose-log", "g.log"]
    );
    short_option_test!(short_option_g, ["-gg"], ["--log-level", "--log-level"]);
    short_option_test!(short_option_q, ["-qq"], ["--quiet", "--quiet"]);
    short_option_test!(short_option_v, ["-vvv"], ["-v", "--verbose", "-v"]);
    short_option_test!(
        short_option_upper_r,
        ["-R", "r.log"],
        ["--request-log", "r.log"]
    );
    short_option_test!(
        short_option_upper_t,
        ["-T", "t.log"],
        ["--transaction-log", "t.log"]
    );
    short_option_test!(
        short_option_upper_s,
        ["-S", "s.log"],
        ["--scenario-log", "s.log"]
    );
    short_option_test!(
        short_option_upper_e,
        ["-E", "e.log"],
        ["--error-log", "e.log"]
    );
    short_option_test!(
        short_option_upper_d,
        ["-D", "d.log"],
        ["--debug-log", "d.log"]
    );

    /// Short options combine, and take a value attached or after `=` on a long name.
    #[test]
    fn option_spellings() {
        let a =
            GooseConfiguration::parse_args_default(&["-lu7", "--host=http://h/", "-gv"]).unwrap();
        assert!(a.list);
        assert_eq!(a.users, Some(7));
        assert_eq!(a.host, "http://h/");
        assert_eq!(a.log_level, 1);
        assert_eq!(a.verbose, 1);
    }

    /// A repeated option keeps its last value, a repeated flag stays set, and
    /// `--report-file` collects every value.
    #[test]
    fn repeated_options() {
        let c = GooseConfiguration::parse_args_default(&[
            "--users",
            "1",
            "-u",
            "2",
            "--host",
            "http://a/",
            "--host",
            "http://b/",
            "--no-save",
            "--no-save",
            "--report-file",
            "a.html",
            "--report-file",
            "a.html",
        ])
        .unwrap();
        assert_eq!(c.users, Some(2));
        assert_eq!(c.host, "http://b/");
        assert!(c.no_save);
        assert_eq!(c.report_file, vec!["a.html", "a.html"]);
    }

    /// An option's value may begin with a hyphen.
    #[test]
    fn option_value_may_begin_with_hyphen() {
        let c = GooseConfiguration::parse_args_default(&["--goose-log", "-g", "-H", "--users"])
            .unwrap();
        assert_eq!(c.goose_log, "-g");
        assert_eq!(c.host, "--users");
        assert_eq!(c.log_level, 0);
        assert_eq!(c.users, None);
    }

    /// Unknown options, free arguments, missing values and bad values are errors
    /// that name the option, its value and clap's reason, without clap's usage.
    #[test]
    fn invalid_command_lines_are_errors() {
        for (args, expected_option, expected_value, expected_detail) in [
            (
                &["--bogus"][..],
                "--bogus",
                "--bogus",
                "unexpected argument '--bogus' found",
            ),
            (
                &["-u", "2", "extra"],
                "extra",
                "-u 2 extra",
                "unexpected argument 'extra' found",
            ),
            (
                &["--users"],
                "--users <N>",
                "",
                "a value is required for '--users <N>' but none was supplied",
            ),
            (
                &["--users", "many"],
                "--users <N>",
                "many",
                "invalid value 'many' for '--users <N>': invalid digit found in string",
            ),
            (
                &["--no-save=true"],
                "--no-save",
                "true",
                "unexpected value 'true' for '--no-save' found; no more were expected",
            ),
            (
                &["--request-format", "xml"],
                "--request-format <FMT>",
                "xml",
                "invalid value 'xml' for '--request-format <FMT>'; possible values: json, csv, raw, pretty",
            ),
            (
                &["--co-mitigation", "sometimes"],
                "--co-mitigation <MODE>",
                "sometimes",
                "invalid value 'sometimes' for '--co-mitigation <MODE>'; possible values: average, maximum, minimum, disabled",
            ),
            (
                &["--telnet-port", "70000"],
                "--telnet-port <PORT>",
                "70000",
                "invalid value '70000' for '--telnet-port <PORT>': 70000 is not in 0..=65535",
            ),
            (&["-h"], "-h", "-h", "help was requested"),
            (&["--help"], "--help", "--help", "help was requested"),
        ] {
            match GooseConfiguration::parse_args_default(args) {
                Err(GooseError::InvalidOption {
                    option,
                    value,
                    detail,
                }) => {
                    assert_eq!(option, expected_option, "{:?}", args);
                    assert_eq!(value, expected_value, "{:?}", args);
                    assert_eq!(detail, expected_detail, "{:?}", args);
                }
                other => panic!("{:?}: {:?}", args, other),
            }
        }
    }

    /// The format and mitigation options take their values, and the abbreviations
    /// of each, in any case.
    #[test]
    fn format_and_mitigation_values() {
        for (value, format) in [
            ("json", GooseLogFormat::Json),
            ("JSN", GooseLogFormat::Json),
            ("Csv", GooseLogFormat::Csv),
            ("raw", GooseLogFormat::Raw),
            ("PRETTY", GooseLogFormat::Pretty),
        ] {
            for option in [
                "--request-format",
                "--transaction-format",
                "--scenario-format",
                "--error-format",
                "--debug-format",
            ] {
                let c = GooseConfiguration::parse_args_default(&[option, value]).unwrap();
                let parsed = [
                    c.request_format,
                    c.transaction_format,
                    c.scenario_format,
                    c.error_format,
                    c.debug_format,
                ];
                assert_eq!(
                    parsed.iter().flatten().collect::<Vec<_>>(),
                    vec![&format],
                    "{option} {value}"
                );
            }
        }
        for (value, mitigation) in [
            ("average", GooseCoordinatedOmissionMitigation::Average),
            ("Avg", GooseCoordinatedOmissionMitigation::Average),
            ("mean", GooseCoordinatedOmissionMitigation::Average),
            ("MAX", GooseCoordinatedOmissionMitigation::Maximum),
            ("ma", GooseCoordinatedOmissionMitigation::Maximum),
            ("mini", GooseCoordinatedOmissionMitigation::Minimum),
            ("none", GooseCoordinatedOmissionMitigation::Disabled),
            ("No", GooseCoordinatedOmissionMitigation::Disabled),
            ("disable", GooseCoordinatedOmissionMitigation::Disabled),
        ] {
            let c = GooseConfiguration::parse_args_default(&["--co-mitigation", value]).unwrap();
            assert_eq!(c.co_mitigation, Some(mitigation), "{value}");
        }
    }

    /// The help width: the terminal's, else `COLUMNS`, else 80, at most 120.
    #[test]
    fn help_width_precedence() {
        assert_eq!(help_width_from(Some(100), Some("90".to_string())), 100);
        assert_eq!(help_width_from(None, Some("90".to_string())), 90);
        assert_eq!(help_width_from(None, Some(" 70 ".to_string())), 70);
        assert_eq!(help_width_from(None, None), 80);
        assert_eq!(help_width_from(None, Some("wide".to_string())), 80);
        assert_eq!(help_width_from(None, Some("0".to_string())), 80);
        assert_eq!(help_width_from(Some(300), None), 120);
        assert_eq!(help_width_from(None, Some("200".to_string())), 120);
    }

    /// `-h` or `--help` rendered as Goose prints it, for a load test named loadtest.
    fn help_for(flag: &str, width: usize) -> String {
        match GooseConfiguration::parse_with(&mut command("loadtest", width), ["loadtest", flag]) {
            Err(e) if e.kind() == ErrorKind::DisplayHelp => tidy_help(&e.to_string()),
            other => panic!("{}: {:?}", flag, other),
        }
    }

    /// The book's runtime options page includes the `-h` and `--help` output from
    /// these files. They are rendered with the `pdf-reports` feature, so they always
    /// list the PDF options, and CI's `--all-features` run checks them.
    #[cfg(feature = "pdf-reports")]
    #[test]
    fn help_matches_the_book() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/docs/goose-book/src/getting-started");
        let update = std::env::var_os("GOOSE_UPDATE_HELP").is_some();
        let mut stale = Vec::new();
        for (file, flag) in [
            ("runtime-options-short.txt", "-h"),
            ("runtime-options-long.txt", "--help"),
        ] {
            let help = help_for(flag, 80);
            let path = dir.join(file);
            if update {
                std::fs::write(&path, &help).unwrap();
            } else if std::fs::read(&path).ok().as_deref() != Some(help.as_bytes()) {
                stale.push(path.display().to_string());
            }
        }
        assert!(
            stale.is_empty(),
            "the help no longer matches {}; regenerate with:\n  \
             GOOSE_UPDATE_HELP=1 cargo test --all-features --lib config::test::help_matches_the_book",
            stale.join(" and ")
        );
    }

    /// At 80 columns every line of `-h` and `--help` fits, and every option's
    /// summary is 40 characters or less and stays on its option's line.
    #[test]
    fn help_fits_80_columns() {
        let short = help_for("-h", 80);
        let long = help_for("--help", 80);
        for (flag, help) in [("-h", &short), ("--help", &long)] {
            for line in help.lines() {
                assert!(line.chars().count() <= 80, "{}: {:?}", flag, line);
            }
        }
        for arg in command("loadtest", 80).get_arguments() {
            let summary = arg
                .get_help()
                .expect("every option has a summary")
                .to_string();
            assert!(summary.chars().count() <= 40, "{:?}", summary);
            let flags = match (arg.get_short(), arg.get_long()) {
                (Some(short), Some(long)) => format!("-{short}, --{long}"),
                (None, Some(long)) => format!("--{long}"),
                (Some(short), None) => format!("-{short}"),
                (None, None) => panic!("{:?} has no flag", arg.get_id()),
            };
            let line = short
                .lines()
                .find(|line| {
                    line.trim_start()
                        .strip_prefix(&flags)
                        .is_some_and(|rest| rest.starts_with([' ', '.']))
                })
                .unwrap_or_else(|| panic!("no -h line for {}", flags));
            assert!(line.ends_with(&summary), "{:?}", line);
        }
    }

    /// A configuration built with `GooseConfiguration::default()`, as
    /// `GooseAttack::initialize_with_config` takes, gets the built in PDF scale
    /// and timeout, so a PDF report passes validation.
    #[cfg(feature = "pdf-reports")]
    #[test]
    fn default_configuration_gets_pdf_defaults() {
        let mut config = GooseConfiguration {
            report_file: vec!["report.pdf".to_string()],
            ..Default::default()
        };
        config.configure(&GooseDefaults::default()).unwrap();
        assert_eq!(as_json(&config)["pdf_scale"], 0.8);
        assert_eq!(as_json(&config)["pdf_timeout"], 60);
        if let Err(GooseError::InvalidOption { option, .. }) = config.validate_pdf_configuration() {
            assert!(!option.contains("pdf_scale"), "{}", option);
            assert!(!option.contains("pdf_timeout"), "{}", option);
        }
    }

    /// `--pdf-timeout 60` on the command line beats `GooseDefault::PdfTimeout`,
    /// although 60 is also the built in default.
    #[cfg(feature = "pdf-reports")]
    #[test]
    fn explicit_pdf_timeout_beats_goose_default() {
        let goose_attack = GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::PdfTimeout, 120)
            .unwrap();
        let mut config = GooseConfiguration::parse_args_default(&["--pdf-timeout", "60"]).unwrap();
        config.configure(&goose_attack.defaults).unwrap();
        assert_eq!(as_json(&config)["pdf_timeout"], 60);

        // Without it on the command line, the GooseDefault wins.
        let mut config = GooseConfiguration::parse_args_default::<&str>(&[]).unwrap();
        config.configure(&goose_attack.defaults).unwrap();
        assert_eq!(as_json(&config)["pdf_timeout"], 120);
    }

    /// `--pdf-scale` beats `GooseDefault::PdfScale`, which beats the built in
    /// default, and `PdfScale` takes an `f64`.
    #[cfg(feature = "pdf-reports")]
    #[test]
    fn pdf_scale_precedence() {
        let goose_attack = GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::PdfScale, 1.25)
            .unwrap();
        let mut config = GooseConfiguration::parse_args_default(&["--pdf-scale", "0.8"]).unwrap();
        config.configure(&goose_attack.defaults).unwrap();
        assert_eq!(config.pdf_scale, Some(0.8));

        let mut config = GooseConfiguration::default();
        config.configure(&goose_attack.defaults).unwrap();
        assert_eq!(config.pdf_scale, Some(1.25));

        let mut config = GooseConfiguration::default();
        config.configure(&GooseDefaults::default()).unwrap();
        assert_eq!(config.pdf_scale, Some(0.8));

        assert!(
            GooseAttack::initialize_with_config(GooseConfiguration::default())
                .unwrap()
                .set_default(GooseDefault::PdfScale, 1)
                .is_err()
        );
        assert!(
            GooseAttack::initialize_with_config(GooseConfiguration::default())
                .unwrap()
                .set_default(GooseDefault::PdfTimeout, 1.5)
                .is_err()
        );
    }

    /// `--no-debug-body` on its own leaves response bodies out of the debug log.
    #[test]
    fn no_debug_body_alone_is_kept() {
        let mut config = GooseConfiguration::parse_args_default(&["--no-debug-body"]).unwrap();
        config.configure(&GooseDefaults::default()).unwrap();
        assert!(config.no_debug_body);
    }

    /// `--no-granular-report` changes report graphs only, not the debug log.
    #[test]
    fn no_granular_report_keeps_debug_bodies() {
        let mut config = GooseConfiguration::parse_args_default(&["--no-granular-report"]).unwrap();
        config.configure(&GooseDefaults::default()).unwrap();
        assert!(config.no_granular_report);
        assert!(!config.no_debug_body);
    }

    /// `GooseDefault::NoGranularData` sets `no_granular_report`, and the command
    /// line still wins over it.
    #[test]
    fn no_granular_data_default_is_applied() {
        let goose_attack = GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::NoGranularData, true)
            .unwrap();
        let mut config = GooseConfiguration::default();
        config.configure(&goose_attack.defaults).unwrap();
        assert!(config.no_granular_report);
        assert!(!config.no_debug_body);

        // The command line can only turn the flag on, so a default of false
        // shows the command line winning.
        let goose_attack = GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::NoGranularData, false)
            .unwrap();
        let mut config = GooseConfiguration::parse_args_default(&["--no-granular-report"]).unwrap();
        config.configure(&goose_attack.defaults).unwrap();
        assert!(config.no_granular_report);
    }
}
