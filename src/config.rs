//! Functions and structures related to configuring a Goose load test.
//!
//! Goose can be configured at run time by passing in the options and flags defined by
//! the [`GooseConfiguration`] structure.
//!
//! Goose can be configured programmatically with [`GooseDefaultType::set_default`].

use gumdrop::Options;
use serde::{Deserialize, Serialize};
use simplelog::*;
use std::path::PathBuf;
use std::str::FromStr;

use crate::logger::GooseLogFormat;
use crate::metrics::GooseCoordinatedOmissionMitigation;
use crate::test_plan::TestPlan;
use crate::util;
use crate::{GooseAttack, GooseError};

/// Runtime options available when launching a Goose load test.
///
/// Custom defaults can be programmatically set for most of these options using the
/// `GooseDefaults` structure.
///
/// [Help is generated for all of these options](https://book.goose.rs/getting-started/runtime-options.html)
/// by passing a `-h` flag to an application built with the Goose Library.
///
/// Goose leverages [`gumdrop`](https://docs.rs/gumdrop/) to derive the above help from
/// the the below structure.
#[derive(Options, Debug, Clone, Default, Serialize, Deserialize)]
#[options(
    help = r#"Goose is a modern, high-performance, distributed HTTP(S) load testing tool,
written in Rust. Visit https://book.goose.rs/ for more information.

The following runtime options are available when launching a Goose load test:"#
)]
pub struct GooseConfiguration {
    /// Displays this help
    #[options(short = "h")]
    pub help: bool,
    /// Prints version information
    #[options(short = "V")]
    pub version: bool,
    /// Lists all transactions and exits
    // Add a blank line after this option
    #[options(short = "l", help = "Lists all transactions and exits\n")]
    pub list: bool,

    /// Defines host to load test (ie http://10.21.32.33)
    #[options(short = "H")]
    pub host: String,
    /// Sets concurrent users (default: number of CPUs)
    #[options(short = "u")]
    pub users: Option<usize>,
    /// Sets per-second user hatch rate (default: 1)
    #[options(short = "r", meta = "RATE")]
    pub hatch_rate: Option<String>,
    /// Starts users for up to (30s, 20m, 3h, 1h30m, etc)
    #[options(short = "s", meta = "TIME")]
    pub startup_time: String,
    /// Stops load test after (30s, 20m, 3h, 1h30m, etc)
    #[options(short = "t", meta = "TIME")]
    pub run_time: String,
    /// Enables Goose log file and sets name
    #[options(short = "G", meta = "NAME")]
    pub goose_log: String,
    /// Increases Goose log level (-g, -gg, etc)
    #[options(short = "g", count)]
    pub log_level: u8,
    /// Decreases Goose verbosity (-q, -qq, etc)
    #[options(count, short = "q", help = "Decreases Goose verbosity (-q, -qq, etc)")]
    pub quiet: u8,
    /// Increases Goose verbosity (-v, -vv, etc)
    #[options(
        count,
        short = "v",
        // Add a blank line and then a 'Metrics:' header after this option
        help = "Increases Goose verbosity (-v, -vv, etc)\n\nMetrics:"
    )]
    pub verbose: u8,

    /// How often to optionally print running metrics
    #[options(no_short, meta = "TIME")]
    pub running_metrics: Option<usize>,
    /// Doesn't reset metrics after all users have started
    #[options(no_short)]
    pub no_reset_metrics: bool,
    /// Doesn't track metrics
    #[options(no_short)]
    pub no_metrics: bool,
    /// Doesn't track transaction metrics
    #[options(no_short)]
    pub no_transaction_metrics: bool,
    /// Doesn't track scenario metrics
    #[options(no_short)]
    pub no_scenario_metrics: bool,
    /// Doesn't display metrics at end of load test
    #[options(no_short)]
    pub no_print_metrics: bool,
    /// Doesn't display an error summary
    #[options(no_short)]
    pub no_error_summary: bool,
    /// Create an html-formatted report
    #[options(no_short, meta = "NAME")]
    pub report_file: String,
    /// Disable granular graphs in report file
    #[options(no_short)]
    pub no_granular_report: bool,
    /// Sets request log file name
    #[options(short = "R", meta = "NAME")]
    pub request_log: String,
    /// Sets request log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub request_format: Option<GooseLogFormat>,
    /// Include the request body in the request log
    #[options(no_short)]
    pub request_body: bool,
    /// Sets transaction log file name
    #[options(short = "T", meta = "NAME")]
    pub transaction_log: String,
    /// Sets log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub transaction_format: Option<GooseLogFormat>,
    /// Sets scenario log file name
    #[options(short = "S", meta = "NAME")]
    pub scenario_log: String,
    /// Sets log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub scenario_format: Option<GooseLogFormat>,
    /// Sets error log file name
    #[options(short = "E", meta = "NAME")]
    pub error_log: String,
    /// Sets error log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub error_format: Option<GooseLogFormat>,
    /// Sets debug log file name
    #[options(short = "D", meta = "NAME")]
    pub debug_log: String,
    /// Sets debug log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub debug_format: Option<GooseLogFormat>,
    /// Do not include the response body in the debug log
    #[options(no_short)]
    pub no_debug_body: bool,
    /// Do not track status code metrics
    // Add a blank line and then an Advanced: header after this option
    #[options(no_short, help = "Do not track status code metrics\n\nAdvanced:")]
    pub no_status_codes: bool,

    /// Defines a more complex test plan ("10,60s;0,30s")
    #[options(no_short, meta = "\"TESTPLAN\"")]
    pub(crate) test_plan: Option<TestPlan>,
    /// Sets how many times to run scenarios then exit
    #[options(no_short)]
    pub iterations: usize,
    /// Limits load test to only specified scenarios
    #[options(no_short, meta = "\"SCENARIO\"")]
    pub scenarios: Scenarios,
    /// Lists all scenarios and exits
    #[options(no_short)]
    pub scenarios_list: bool,
    /// Doesn't enable telnet Controller
    #[options(no_short)]
    pub no_telnet: bool,
    /// Sets telnet Controller host (default: 0.0.0.0)
    #[options(no_short, meta = "HOST")]
    pub telnet_host: String,
    /// Sets telnet Controller TCP port (default: 5116)
    #[options(no_short, meta = "PORT")]
    pub telnet_port: u16,
    /// Doesn't enable WebSocket Controller
    #[options(no_short)]
    pub no_websocket: bool,
    /// Sets WebSocket Controller host (default: 0.0.0.0)
    #[options(no_short, meta = "HOST")]
    pub websocket_host: String,
    /// Sets WebSocket Controller TCP port (default: 5117)
    #[options(no_short, meta = "PORT")]
    pub websocket_port: u16,
    /// Doesn't automatically start load test
    #[options(no_short)]
    pub no_autostart: bool,
    /// Doesn't set the gzip Accept-Encoding header
    #[options(no_short)]
    pub no_gzip: bool,
    /// Sets per-request timeout, in seconds (default: 60)
    #[options(no_short, meta = "VALUE")]
    pub timeout: Option<String>,
    /// Sets coordinated omission mitigation strategy
    #[options(no_short, meta = "STRATEGY")]
    pub co_mitigation: Option<GooseCoordinatedOmissionMitigation>,
    /// Sets maximum requests per second
    #[options(no_short, meta = "VALUE")]
    pub throttle_requests: usize,
    /// Follows base_url redirect with subsequent requests
    #[options(no_short)]
    pub sticky_follow: bool,
    /// Disables validation of https certificates
    #[options(no_short)]
    pub accept_invalid_certs: bool,
}

/// Optionally defines a subset of active Scenarios to run during a load test.
#[derive(Options, Default, Debug, Clone, Serialize, Deserialize)]
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
            // Valid scenario names are alphanumeric only.
            if scenario.chars().all(char::is_alphanumeric) {
                active.push(scenario);
            } else {
                // Logger isn't initialized yet, provide helpful debug output.
                eprintln!("ERROR: invalid `configuration.scenarios` value: '{}'", line);
                eprintln!("  Expected format: --scenarios \"{{one}},{{two}},{{three}}\"");
                eprintln!("    {{one}}, {{two}}, {{three}}, etc must be alphanumeric");
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
    pub hatch_rate: Option<String>,
    /// An optional default number of seconds for the test to start.
    pub startup_time: Option<usize>,
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
    pub report_file: Option<String>,
    /// An optional default for the flag that disables granular data in HTML report graphs.
    pub no_granular_report: Option<bool>,
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
    /// An optional default for not validating https certificates.
    pub accept_invalid_certs: Option<bool>,
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
    HatchRate,
    /// An optional default number of seconds for the test to start up.
    StartupTime,
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
    /// An optional default for the flag that disables granular data in HTML report graphs.
    NoGranularData,
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
    /// An optional default for not validating https certificates.
    AcceptInvalidCerts,
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
///  - [`GooseDefault::HatchRate`]
///  - [`GooseDefault::Host`]
///  - [`GooseDefault::ReportFile`]
///  - [`GooseDefault::RequestLog`]
///  - [`GooseDefault::ScenarioLog`]
///  - [`GooseDefault::Scenarios`]
///  - [`GooseDefault::TelnetHost`]
///  - [`GooseDefault::TestPlan`]
///  - [`GooseDefault::Timeout`]
///  - [`GooseDefault::TransactionLog`]
///  - [`GooseDefault::WebSocketHost`]
///
/// The following run-time options can be configured with a custom default using a
/// [`usize`] integer:
///  - [`GooseDefault::Users`]
///  - [`GooseDefault::StartupTime`]
///  - [`GooseDefault::RunTime`]
///  - [`GooseDefault::Iterations`]
///  - [`GooseDefault::RunningMetrics`]
///  - [`GooseDefault::LogLevel`]
///  - [`GooseDefault::Quiet`]
///  - [`GooseDefault::Verbose`]
///  - [`GooseDefault::ThrottleRequests`]
///  - [`GooseDefault::TelnetPort`]
///  - [`GooseDefault::WebSocketPort`]
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
            GooseDefault::HatchRate => self.defaults.hatch_rate = Some(value.to_string()),
            GooseDefault::Host => {
                self.defaults.host = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            }
            GooseDefault::ReportFile => self.defaults.report_file = Some(value.to_string()),
            GooseDefault::RequestLog => self.defaults.request_log = Some(value.to_string()),
            GooseDefault::ScenarioLog => self.defaults.scenario_log = Some(value.to_string()),
            GooseDefault::Scenarios => {
                self.defaults.scenarios = Some(value.parse::<Scenarios>().unwrap())
            }
            GooseDefault::TelnetHost => self.defaults.telnet_host = Some(value.to_string()),
            GooseDefault::TestPlan => {
                self.defaults.test_plan = Some(value.parse::<TestPlan>().unwrap())
            }
            GooseDefault::Timeout => self.defaults.timeout = Some(value.to_string()),
            GooseDefault::TransactionLog => self.defaults.transaction_log = Some(value.to_string()),
            GooseDefault::WebSocketHost => self.defaults.websocket_host = Some(value.to_string()),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Users
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected usize value, received &str",
                        key, value
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
            | GooseDefault::AcceptInvalidCerts => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected bool value, received &str",
                        key, value
                    ),
                });
            }
            GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::TransactionFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::RequestFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected GooseLogFormat value, received &str",
                        key, value
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected GooseCoordinatedOmissionMitigation value, received &str",
                        key, value
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
            GooseDefault::StartupTime => self.defaults.startup_time = Some(value),
            GooseDefault::RunTime => self.defaults.run_time = Some(value),
            GooseDefault::Iterations => self.defaults.iterations = Some(value),
            GooseDefault::RunningMetrics => self.defaults.running_metrics = Some(value),
            GooseDefault::LogLevel => self.defaults.log_level = Some(value as u8),
            GooseDefault::Quiet => self.defaults.quiet = Some(value as u8),
            GooseDefault::Verbose => self.defaults.verbose = Some(value as u8),
            GooseDefault::ThrottleRequests => self.defaults.throttle_requests = Some(value),
            GooseDefault::TelnetPort => self.defaults.telnet_port = Some(value as u16),
            GooseDefault::WebSocketPort => self.defaults.websocket_port = Some(value as u16),
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::HatchRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected &str value, received usize",
                        key, value
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
            | GooseDefault::AcceptInvalidCerts => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected bool value, received usize",
                        key, value
                    ),
                })
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected GooseLogFormat value, received usize",
                        key, value
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected GooseCoordinatedOmissionMitigation value, received usize",
                        key, value
                    ),
                });
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
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::HatchRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected &str value, received bool",
                        key, value
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::RunningMetrics
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected usize value, received bool",
                        key, value
                    ),
                })
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected GooseLogFormat value, received bool",
                        key, value
                    ),
                });
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: value.to_string(),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {}) expected GooseCoordinatedOmissionMitigation value, received bool",
                        key, value
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
            | GooseDefault::AcceptInvalidCerts  => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected bool value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::HatchRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected &str value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::RunningMetrics
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected usize value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            GooseDefault::RequestFormat
            | GooseDefault::DebugFormat
            | GooseDefault::ErrorFormat
            | GooseDefault::ScenarioFormat
            | GooseDefault::TransactionFormat => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected GooseLogFormat value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
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
            | GooseDefault::AcceptInvalidCerts => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected bool value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            // Otherwise display a helpful and explicit error.
            GooseDefault::DebugLog
            | GooseDefault::ErrorLog
            | GooseDefault::GooseLog
            | GooseDefault::HatchRate
            | GooseDefault::Host
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::ScenarioLog
            | GooseDefault::Scenarios
            | GooseDefault::TelnetHost
            | GooseDefault::TestPlan
            | GooseDefault::Timeout
            | GooseDefault::TransactionLog
            | GooseDefault::WebSocketHost => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected &str value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            GooseDefault::Users
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::RunningMetrics
            | GooseDefault::Iterations
            | GooseDefault::LogLevel
            | GooseDefault::Quiet
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected usize value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            GooseDefault::CoordinatedOmissionMitigation => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected GooseCoordinatedOmissionMitigation value, received GooseLogFormat",
                        key, value
                    ),
                })
            }
        }
        Ok(Box::new(self))
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

impl GooseConfiguration {
    /// Implement precedence rules for all [`GooseConfiguration`] values.
    pub(crate) fn configure(&mut self, defaults: &GooseDefaults) {
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
                info!("failed to detect available_parallelism: {}", e);
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

        // Configure `startup_time`.
        self.startup_time = self
            .get_value(vec![
                // Use --startup-time if set.
                GooseValue {
                    value: Some(util::parse_timespan(&self.startup_time)),
                    filter: util::parse_timespan(&self.startup_time) == 0,
                    message: "startup_time",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: defaults.startup_time,
                    filter: defaults.startup_time.is_none(),
                    message: "startup_time",
                },
            ])
            .map_or_else(|| "0".to_string(), |v| v.to_string());

        // Configure `run_time`.
        self.run_time = self
            .get_value(vec![
                // Use --run-time if set.
                GooseValue {
                    value: Some(util::parse_timespan(&self.run_time)),
                    filter: util::parse_timespan(&self.run_time) == 0,
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

        // Configure `hatch_rate`.
        self.hatch_rate = self
            .get_value(vec![
                // Use --hatch-rate if set.
                GooseValue {
                    value: Some(util::get_hatch_rate(self.hatch_rate.clone())),
                    filter: self.hatch_rate.is_none(),
                    message: "hatch_rate",
                },
                // Otherwise use GooseDefault if set and not on Worker.
                GooseValue {
                    value: Some(util::get_hatch_rate(defaults.hatch_rate.clone())),
                    filter: defaults.hatch_rate.is_none(),
                    message: "hatch_rate",
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
        self.report_file = match self.get_value(vec![
            // Use --report-file if set.
            GooseValue {
                value: Some(self.report_file.to_string()),
                filter: self.report_file.is_empty(),
                message: "report_file",
            },
            // Otherwise use GooseDefault if set.
            GooseValue {
                value: defaults.report_file.clone(),
                filter: defaults.report_file.is_none(),
                message: "report_file",
            },
        ]) {
            Some(v) => v,
            None => "".to_string(),
        };

        // Configure `no_granular_report`.
        self.no_debug_body = self
            .get_value(vec![
                // Use --no-granular-report if set.
                GooseValue {
                    value: Some(self.no_granular_report),
                    filter: !self.no_granular_report,
                    message: "no_granular_report",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_debug_body,
                    filter: defaults.no_debug_body.is_none(),
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
    }

    /// Validate configured [`GooseConfiguration`] values.
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

        // If set, hatch rate must be non-zero.
        if let Some(hatch_rate) = self.hatch_rate.as_ref() {
            if hatch_rate == "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.hatch_rate`".to_string(),
                    value: hatch_rate.to_string(),
                    detail: "`configuration.hatch_rate` must be set to at least 1.".to_string(),
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

        // Validate `startup_time`.
        if self.startup_time != "0" {
            // Startup time can't be set with hatch rate.
            if self.hatch_rate.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.startup_time`".to_string(),
                    value: self.startup_time.to_string(),
                    detail: "`configuration.startup_time` can not be set with `configuration.hatch_rate`.".to_string(),
                });
            }

            // Startup time requires at least 2 users.
            if let Some(users) = self.users.as_ref() {
                if users < &2 {
                    return Err(GooseError::InvalidOption {
                        option: "configuration.users".to_string(),
                        value: users.to_string(),
                        detail: "`configuration.users` must be set to at least 2 when `configuration.startup_time` is set.".to_string(),
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
            // The --startup-time option isn't compatible with --test-plan.
            if self.startup_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.startup_time`".to_string(),
                    value: self.startup_time.to_string(),
                    detail: "`configuration.startup_time` can not be set with `configuration.test_plan`.".to_string(),
                });
            }
            // The --hatch-rate option isn't compatible with --test-plan.
            if let Some(hatch_rate) = self.hatch_rate.as_ref() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.hatch_rate`".to_string(),
                    value: hatch_rate.to_string(),
                    detail:
                        "`configuration.hatch_rate` can not be set with `configuration.test_plan`."
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
                    value: self.report_file.to_string(),
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

        if self.report_file.is_empty() && self.no_granular_report {
            return Err(GooseError::InvalidOption {
                option: "`configuration.no_granular_report`".to_string(),
                value: true.to_string(),
                detail:
                    "`configuration.no_granular_report` can not be set without `configuration.report_file`."
                        .to_string(),
            });
        }

        // Can't disable autostart if there's no Controller enabled.
        if self.no_autostart && self.no_telnet && self.no_websocket {
            return Err(GooseError::InvalidOption {
                option: "`configuration.no_autostart`".to_string(),
                value: true.to_string(),
                detail: "`configuration.no_autostart` requires at least one Controller be enabled"
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
                    info!("failed to initialize CombinedLogger: {}", e);
                }
            }
            info!("Writing to log file: {}", log_to_file.display());
        } else {
            match CombinedLogger::init(vec![SimpleLogger::new(debug_level, Config::default())]) {
                Ok(_) => (),
                Err(e) => {
                    info!("failed to initialize CombinedLogger: {}", e);
                }
            }
        }

        info!("Output verbosity level: {}", debug_level);
        info!("Logfile verbosity level: {}", log_level);
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
        let hatch_rate = "2".to_string();
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

        let goose_attack = GooseAttack::initialize()
            .unwrap()
            .set_default(GooseDefault::Host, host.as_str())
            .unwrap()
            .set_default(GooseDefault::Users, users)
            .unwrap()
            .set_default(GooseDefault::RunTime, run_time)
            .unwrap()
            .set_default(GooseDefault::HatchRate, hatch_rate.as_str())
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
            .unwrap();

        assert!(goose_attack.defaults.host == Some(host));
        assert!(goose_attack.defaults.users == Some(users));
        assert!(goose_attack.defaults.run_time == Some(run_time));
        assert!(goose_attack.defaults.hatch_rate == Some(hatch_rate));
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
        assert!(goose_attack.defaults.report_file == Some(report_file));
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
    }
}
