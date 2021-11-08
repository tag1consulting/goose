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

use crate::logger::GooseLogFormat;
use crate::metrics::GooseCoordinatedOmissionMitigation;
use crate::util;
use crate::{GooseAttack, GooseError};

/// Constant defining Goose's default port when running a Gaggle.
const DEFAULT_PORT: &str = "5115";

/// Runtime options available when launching a Goose load test.
///
/// Custom defaults can be programmatically set for most of these options using the
/// `GooseDefaults` structure.
///
/// Help is generated for all of these options by passing a `-h` flag to an application
/// built with the Goose Library. For example, using the following command from within the
/// Goose source tree to run the included `simple` example:
///
/// `cargo run --example simple -- -h`
///
/// Goose will generate the following output from the [`GooseConfiguration`] structure:
///
/// ```text
/// Usage: target/debug/examples/simple [OPTIONS]
///
/// Options available when launching a Goose load test.
///
///
/// Optional arguments:
/// -h, --help                 Displays this help
/// -V, --version              Prints version information
/// -l, --list                 Lists all tasks and exits
///
/// -H, --host HOST            Defines host to load test (ie http://10.21.32.33)
/// -u, --users USERS          Sets concurrent users (default: number of CPUs)
/// -r, --hatch-rate RATE      Sets per-second user hatch rate (default: 1)
/// -t, --run-time TIME        Stops after (30s, 20m, 3h, 1h30m, etc)
/// -G, --goose-log NAME       Enables Goose log file and sets name
/// -g, --log-level            Sets Goose log level (-g, -gg, etc)
/// -v, --verbose              Sets Goose verbosity (-v, -vv, etc)
///
/// Metrics:
/// --running-metrics TIME     How often to optionally print running metrics
/// --no-reset-metrics         Doesn't reset metrics after all users have started
/// --no-metrics               Doesn't track metrics
/// --no-task-metrics          Doesn't track task metrics
/// --no-error-summary         Doesn't display an error summary
/// --report-file NAME         Create an html-formatted report
/// -R, --request-log NAME     Sets request log file name
/// --request-format FORMAT    Sets request log format (csv, json, raw, pretty)
/// --request-body             Include the request body in the request log
/// -T, --task-log NAME        Sets task log file name
/// --task-format FORMAT       Sets task log format (csv, json, raw, pretty)
/// -E, --error-log NAME       Sets error log file name
/// --error-format FORMAT      Sets error log format (csv, json, raw, pretty)
/// -D, --debug-log NAME       Sets debug log file name
/// --debug-format FORMAT      Sets debug log format (csv, json, raw, pretty)
/// --no-debug-body            Do not include the response body in the debug log
/// --status-codes             Tracks additional status code metrics
/// --requests-per-second      Tracks additional requests per second metrics
/// --rps-bucket               Size (in seconds) of the time-based requests per second bucket (default: 10)
///
/// Advanced:
/// --no-telnet                Doesn't enable telnet Controller
/// --telnet-host HOST         Sets telnet Controller host (default: 0.0.0.0)
/// --telnet-port PORT         Sets telnet Controller TCP port (default: 5116)
/// --no-websocket             Doesn't enable WebSocket Controller
/// --websocket-host HOST      Sets WebSocket Controller host (default: 0.0.0.0)
/// --websocket-port PORT      Sets WebSocket Controller TCP port (default: 5117)
/// --no-autostart             Doesn't automatically start load test
/// --no-gzip                  Doesn't set the gzip Accept-Encoding header
/// --timeout VALUE            Sets per-request timeout, in seconds (default: 60)
/// --co-mitigation STRATEGY   Sets coordinated omission mitigation strategy
/// --throttle-requests VALUE  Sets maximum requests per second
/// --sticky-follow            Follows base_url redirect with subsequent requests
///
/// Gaggle:
/// --manager                  Enables distributed load test Manager mode
/// --expect-workers VALUE     Sets number of Workers to expect
/// --no-hash-check            Tells Manager to ignore load test checksum
/// --manager-bind-host HOST   Sets host Manager listens on (default: 0.0.0.0)
/// --manager-bind-port PORT   Sets port Manager listens on (default: 5115)
/// --worker                   Enables distributed load test Worker mode
/// --manager-host HOST        Sets host Worker connects to (default: 127.0.0.1)
/// --manager-port PORT        Sets port Worker connects to (default: 5115)
/// ```
///
/// Goose leverages [`gumdrop`](https://docs.rs/gumdrop/) to derive the above help from
/// the the below structure.
#[derive(Options, Debug, Clone, Serialize, Deserialize)]
pub struct GooseConfiguration {
    /// Displays this help
    #[options(short = "h")]
    pub help: bool,
    /// Prints version information
    #[options(short = "V")]
    pub version: bool,
    /// Lists all tasks and exits
    // Add a blank line after this option
    #[options(short = "l", help = "Lists all tasks and exits\n")]
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
    /// Sets Goose log level (-g, -gg, etc)
    #[options(short = "g", count)]
    pub log_level: u8,
    /// Sets Goose verbosity (-v, -vv, etc)
    #[options(
        count,
        short = "v",
        // Add a blank line and then a 'Metrics:' header after this option
        help = "Sets Goose verbosity (-v, -vv, etc)\n\nMetrics:"
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
    /// Doesn't track task metrics
    #[options(no_short)]
    pub no_task_metrics: bool,
    /// Doesn't display an error summary
    #[options(no_short)]
    pub no_error_summary: bool,
    /// Create an html-formatted report
    #[options(no_short, meta = "NAME")]
    pub report_file: String,
    /// Sets request log file name
    #[options(short = "R", meta = "NAME")]
    pub request_log: String,
    /// Sets request log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub request_format: Option<GooseLogFormat>,
    /// Include the request body in the request log
    #[options(no_short)]
    pub request_body: bool,
    /// Sets task log file name
    #[options(short = "T", meta = "NAME")]
    pub task_log: String,
    /// Sets task log format (csv, json, raw, pretty)
    #[options(no_short, meta = "FORMAT")]
    pub task_format: Option<GooseLogFormat>,
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
    /// Tracks additional status code metrics
    #[options(no_short, help = "Tracks additional status code metrics")]
    pub status_codes: bool,
    /// Tracks additional request per second metrics
    #[options(no_short, help = "Tracks additional request per second metrics")]
    pub requests_per_second: bool,
    // Size (in seconds) of the time-based requests per second bucket (default: 10)
    // Add a blank line and then an Advanced: header after this option
    #[options(
        no_short,
        help = "Size of the time bucket in time-based requests per second calculations\n\nAdvanced:"
    )]
    pub rps_bucket: usize,
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
    #[options(
        no_short,
        help = "Follows base_url redirect with subsequent requests\n\nGaggle:"
    )]
    pub sticky_follow: bool,

    /// Enables distributed load test Manager mode
    #[options(no_short)]
    pub manager: bool,
    /// Sets number of Workers to expect
    #[options(no_short, meta = "VALUE")]
    pub expect_workers: Option<usize>,
    /// Tells Manager to ignore load test checksum
    #[options(no_short)]
    pub no_hash_check: bool,
    /// Sets host Manager listens on (default: 0.0.0.0)
    #[options(no_short, meta = "HOST")]
    pub manager_bind_host: String,
    /// Sets port Manager listens on (default: 5115)
    #[options(no_short, meta = "PORT")]
    pub manager_bind_port: u16,
    /// Enables distributed load test Worker mode
    #[options(no_short)]
    pub worker: bool,
    /// Sets host Worker connects to (default: 127.0.0.1)
    #[options(no_short, meta = "HOST")]
    pub manager_host: String,
    /// Sets port Worker connects to (default: 5115)
    #[options(no_short, meta = "PORT")]
    pub manager_port: u16,
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
    /// An optional default log level.
    pub log_level: Option<u8>,
    /// An optional default for the goose log file name.
    pub goose_log: Option<String>,
    /// An optional default value for verbosity level.
    pub verbose: Option<u8>,
    /// An optional default for printing running metrics.
    pub running_metrics: Option<usize>,
    /// An optional default for not resetting metrics after all users started.
    pub no_reset_metrics: Option<bool>,
    /// An optional default for not tracking metrics.
    pub no_metrics: Option<bool>,
    /// An optional default for not tracking task metrics.
    pub no_task_metrics: Option<bool>,
    /// An optional default for not displaying an error summary.
    pub no_error_summary: Option<bool>,
    /// An optional default for the html-formatted report file name.
    pub report_file: Option<String>,
    /// An optional default for the requests log file name.
    pub request_log: Option<String>,
    /// An optional default for the requests log file format.
    pub request_format: Option<GooseLogFormat>,
    /// An optional default for logging the request body.
    pub request_body: Option<bool>,
    /// An optional default for the tasks log file name.
    pub task_log: Option<String>,
    /// An optional default for the tasks log file format.
    pub task_format: Option<GooseLogFormat>,
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
    /// An optional default to track additional status code metrics.
    pub status_codes: Option<bool>,
    /// An optional default to track requests per second metrics.
    pub requests_per_second: Option<bool>,
    // An optional default size of the requests per second bucket size.
    pub rps_bucket: Option<usize>,
    /// An optional default maximum requests per second.
    pub throttle_requests: Option<usize>,
    /// An optional default to follows base_url redirect with subsequent request.
    pub sticky_follow: Option<bool>,
    /// An optional default to enable Manager mode.
    pub manager: Option<bool>,
    /// An optional default for number of Workers to expect.
    pub expect_workers: Option<usize>,
    /// An optional default for Manager to ignore load test checksum.
    pub no_hash_check: Option<bool>,
    /// An optional default for host telnet Controller listens on.
    pub telnet_host: Option<String>,
    /// An optional default for port telnet Controller listens on.
    pub telnet_port: Option<u16>,
    /// An optional default for host WebSocket Controller listens on.
    pub websocket_host: Option<String>,
    /// An optional default for port WebSocket Controller listens on.
    pub websocket_port: Option<u16>,
    /// An optional default for host Manager listens on.
    pub manager_bind_host: Option<String>,
    /// An optional default for port Manager listens on.
    pub manager_bind_port: Option<u16>,
    /// An optional default to enable Worker mode.
    pub worker: Option<bool>,
    /// An optional default for host Worker connects to.
    pub manager_host: Option<String>,
    /// An optional default for port Worker connects to.
    pub manager_port: Option<u16>,
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
    /// An optional default log level.
    LogLevel,
    /// An optional default for the log file name.
    GooseLog,
    /// An optional default value for verbosity level.
    Verbose,
    /// An optional default for printing running metrics.
    RunningMetrics,
    /// An optional default for not resetting metrics after all users started.
    NoResetMetrics,
    /// An optional default for not tracking metrics.
    NoMetrics,
    /// An optional default for not tracking task metrics.
    NoTaskMetrics,
    /// An optional default for not displaying an error summary.
    NoErrorSummary,
    /// An optional default for the report file name.
    ReportFile,
    /// An optional default for the request log file name.
    RequestLog,
    /// An optional default for the request log file format.
    RequestFormat,
    /// An optional default for logging the request body.
    RequestBody,
    /// An optional default for the task log file name.
    TaskLog,
    /// An optional default for the task log file format.
    TaskFormat,
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
    /// An optional default to track additional status code metrics.
    StatusCodes,
    /// An optional default maximum requests per second.
    ThrottleRequests,
    /// An optional default to follows base_url redirect with subsequent request.
    StickyFollow,
    /// An optional default to enable Manager mode.
    Manager,
    /// An optional default for number of Workers to expect.
    ExpectWorkers,
    /// An optional default for Manager to ignore load test checksum.
    NoHashCheck,
    /// An optional default for host telnet Controller listens on.
    TelnetHost,
    /// An optional default for port telnet Controller listens on.
    TelnetPort,
    /// An optional default for host Websocket Controller listens on.
    WebSocketHost,
    /// An optional default for port WebSocket Controller listens on.
    WebSocketPort,
    /// An optional default for host Manager listens on.
    ManagerBindHost,
    /// An optional default for port Manager listens on.
    ManagerBindPort,
    /// An optional default to enable Worker mode.
    Worker,
    /// An optional default for host Worker connects to.
    ManagerHost,
    /// An optional default for port Worker connects to.
    ManagerPort,
}

/// Most run-time options can be programmatically configured with custom defaults.
///
/// For example, you can optionally configure a default host for the load test. This is
/// used if no per-[`GooseTaskSet`](../struct.GooseTaskSet.html) host is defined, no
/// [`--host`](./enum.GooseDefault.html#variant.Host) CLI option is configured, and if
/// the [`GooseTask`](../struct.GooseTask.html) itself doesn't hard-code the host in
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
///  - [`GooseDefault::Host`]
///  - [`GooseDefault::GooseLog`]
///  - [`GooseDefault::RequestFormat`]
///  - [`GooseDefault::TaskLog`]
///  - [`GooseDefault::ErrorLog`]
///  - [`GooseDefault::DebugLog`]
///  - [`GooseDefault::TelnetHost`]
///  - [`GooseDefault::WebSocketHost`]
///  - [`GooseDefault::ManagerBindHost`]
///  - [`GooseDefault::ManagerHost`]
///
/// The following run-time options can be configured with a custom default using a
/// [`usize`] integer:
///  - [`GooseDefault::Users`]
///  - [`GooseDefault::HatchRate`]
///  - [`GooseDefault::StartupTime`]
///  - [`GooseDefault::RunTime`]
///  - [`GooseDefault::RunningMetrics`]
///  - [`GooseDefault::LogLevel`]
///  - [`GooseDefault::Verbose`]
///  - [`GooseDefault::ThrottleRequests`]
///  - [`GooseDefault::ExpectWorkers`]
///  - [`GooseDefault::TelnetPort`]
///  - [`GooseDefault::WebSocketPort`]
///  - [`GooseDefault::ManagerBindPort`]
///  - [`GooseDefault::ManagerPort`]
///
/// The following run-time flags can be configured with a custom default using a
/// [`bool`] (and otherwise default to [`false`]).
///  - [`GooseDefault::NoResetMetrics`]
///  - [`GooseDefault::NoMetrics`]
///  - [`GooseDefault::NoTaskMetrics`]
///  - [`GooseDefault::RequestBody`]
///  - [`GooseDefault::NoErrorSummary`]
///  - [`GooseDefault::NoDebugBody`]
///  - [`GooseDefault::NoTelnet`]
///  - [`GooseDefault::NoWebSocket`]
///  - [`GooseDefault::NoAutoStart`]
///  - [`GooseDefault::NoGzip`]
///  - [`GooseDefault::StatusCodes`]
///  - [`GooseDefault::StickyFollow`]
///  - [`GooseDefault::Manager`]
///  - [`GooseDefault::NoHashCheck`]
///  - [`GooseDefault::Worker`]
///
/// The following run-time flags can be configured with a custom default using a
/// [`GooseLogFormat`].
///  - [`GooseDefault::RequestLog`]
///  - [`GooseDefault::TaskLog`]
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
    ///         // Display info level logs while the test runs.
    ///         .set_default(GooseDefault::Verbose, 1)?
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
            GooseDefault::HatchRate => self.defaults.hatch_rate = Some(value.to_string()),
            GooseDefault::Timeout => self.defaults.timeout = Some(value.to_string()),
            GooseDefault::Host => self.defaults.host = Some(value.to_string()),
            GooseDefault::GooseLog => self.defaults.goose_log = Some(value.to_string()),
            GooseDefault::ReportFile => self.defaults.report_file = Some(value.to_string()),
            GooseDefault::RequestLog => self.defaults.request_log = Some(value.to_string()),
            GooseDefault::TaskLog => self.defaults.task_log = Some(value.to_string()),
            GooseDefault::ErrorLog => self.defaults.error_log = Some(value.to_string()),
            GooseDefault::DebugLog => self.defaults.debug_log = Some(value.to_string()),
            GooseDefault::TelnetHost => self.defaults.telnet_host = Some(value.to_string()),
            GooseDefault::WebSocketHost => self.defaults.websocket_host = Some(value.to_string()),
            GooseDefault::ManagerBindHost => {
                self.defaults.manager_bind_host = Some(value.to_string())
            }
            GooseDefault::ManagerHost => self.defaults.manager_host = Some(value.to_string()),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Users
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::LogLevel
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::ExpectWorkers
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::ManagerBindPort
            | GooseDefault::ManagerPort => {
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
            | GooseDefault::NoTaskMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::StatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::Manager
            | GooseDefault::NoHashCheck
            | GooseDefault::Worker => {
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
            | GooseDefault::TaskFormat
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
            GooseDefault::RunningMetrics => self.defaults.running_metrics = Some(value),
            GooseDefault::LogLevel => self.defaults.log_level = Some(value as u8),
            GooseDefault::Verbose => self.defaults.verbose = Some(value as u8),
            GooseDefault::ThrottleRequests => self.defaults.throttle_requests = Some(value),
            GooseDefault::ExpectWorkers => self.defaults.expect_workers = Some(value),
            GooseDefault::TelnetPort => self.defaults.telnet_port = Some(value as u16),
            GooseDefault::WebSocketPort => self.defaults.websocket_port = Some(value as u16),
            GooseDefault::ManagerBindPort => self.defaults.manager_bind_port = Some(value as u16),
            GooseDefault::ManagerPort => self.defaults.manager_port = Some(value as u16),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Host
            | GooseDefault::HatchRate
            | GooseDefault::Timeout
            | GooseDefault::GooseLog
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::TaskLog
            | GooseDefault::ErrorLog
            | GooseDefault::DebugLog
            | GooseDefault::TelnetHost
            | GooseDefault::WebSocketHost
            | GooseDefault::ManagerBindHost
            | GooseDefault::ManagerHost => {
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
            | GooseDefault::NoTaskMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::StatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::Manager
            | GooseDefault::NoHashCheck
            | GooseDefault::Worker => {
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
            | GooseDefault::TaskFormat => {
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
            GooseDefault::NoTaskMetrics => self.defaults.no_task_metrics = Some(value),
            GooseDefault::RequestBody => self.defaults.request_body = Some(value),
            GooseDefault::NoErrorSummary => self.defaults.no_error_summary = Some(value),
            GooseDefault::NoDebugBody => self.defaults.no_debug_body = Some(value),
            GooseDefault::NoTelnet => self.defaults.no_telnet = Some(value),
            GooseDefault::NoWebSocket => self.defaults.no_websocket = Some(value),
            GooseDefault::NoAutoStart => self.defaults.no_autostart = Some(value),
            GooseDefault::NoGzip => self.defaults.no_gzip = Some(value),
            GooseDefault::StatusCodes => self.defaults.status_codes = Some(value),
            GooseDefault::StickyFollow => self.defaults.sticky_follow = Some(value),
            GooseDefault::Manager => self.defaults.manager = Some(value),
            GooseDefault::NoHashCheck => self.defaults.no_hash_check = Some(value),
            GooseDefault::Worker => self.defaults.worker = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::Host
            | GooseDefault::GooseLog
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::TaskLog
            | GooseDefault::RunningMetrics
            | GooseDefault::ErrorLog
            | GooseDefault::DebugLog
            | GooseDefault::TelnetHost
            | GooseDefault::WebSocketHost
            | GooseDefault::ManagerBindHost
            | GooseDefault::ManagerHost => {
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
            | GooseDefault::HatchRate
            | GooseDefault::Timeout
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::LogLevel
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::ExpectWorkers
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::ManagerBindPort
            | GooseDefault::ManagerPort => {
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
            | GooseDefault::TaskFormat => {
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
            | GooseDefault::NoTaskMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::StatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::Manager
            | GooseDefault::NoHashCheck
            | GooseDefault::Worker => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected bool value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            GooseDefault::Host
            | GooseDefault::GooseLog
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::TaskLog
            | GooseDefault::RunningMetrics
            | GooseDefault::ErrorLog
            | GooseDefault::DebugLog
            | GooseDefault::TelnetHost
            | GooseDefault::WebSocketHost
            | GooseDefault::ManagerBindHost
            | GooseDefault::ManagerHost => {
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
            | GooseDefault::HatchRate
            | GooseDefault::Timeout
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::LogLevel
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::ExpectWorkers
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::ManagerBindPort
            | GooseDefault::ManagerPort => {
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
            | GooseDefault::TaskFormat => {
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
            GooseDefault::TaskFormat => self.defaults.task_format = Some(value),
            // Otherwise display a helpful and explicit error.
            GooseDefault::NoResetMetrics
            | GooseDefault::NoMetrics
            | GooseDefault::NoTaskMetrics
            | GooseDefault::RequestBody
            | GooseDefault::NoErrorSummary
            | GooseDefault::NoDebugBody
            | GooseDefault::NoTelnet
            | GooseDefault::NoWebSocket
            | GooseDefault::NoAutoStart
            | GooseDefault::NoGzip
            | GooseDefault::StatusCodes
            | GooseDefault::StickyFollow
            | GooseDefault::Manager
            | GooseDefault::NoHashCheck
            | GooseDefault::Worker => {
                return Err(GooseError::InvalidOption {
                    option: format!("GooseDefault::{:?}", key),
                    value: format!("{:?}", value),
                    detail: format!(
                        "set_default(GooseDefault::{:?}, {:?}) expected bool value, received GooseCoordinatedOmissionMitigation",
                        key, value
                    ),
                })
            }
            GooseDefault::Host
            | GooseDefault::GooseLog
            | GooseDefault::ReportFile
            | GooseDefault::RequestLog
            | GooseDefault::TaskLog
            | GooseDefault::RunningMetrics
            | GooseDefault::ErrorLog
            | GooseDefault::DebugLog
            | GooseDefault::TelnetHost
            | GooseDefault::WebSocketHost
            | GooseDefault::ManagerBindHost
            | GooseDefault::ManagerHost => {
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
            | GooseDefault::HatchRate
            | GooseDefault::Timeout
            | GooseDefault::StartupTime
            | GooseDefault::RunTime
            | GooseDefault::LogLevel
            | GooseDefault::Verbose
            | GooseDefault::ThrottleRequests
            | GooseDefault::ExpectWorkers
            | GooseDefault::TelnetPort
            | GooseDefault::WebSocketPort
            | GooseDefault::ManagerBindPort
            | GooseDefault::ManagerPort => {
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
    /// Use [`GooseValue`] to set a [`usize`] value.
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

impl GooseConfiguration {
    /// Implement precedence rules for all [`GooseConfiguration`] values.
    pub(crate) fn configure(&mut self, defaults: &GooseDefaults) {
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
            .unwrap_or_else(|| "".to_string());

        // Initialize the Goose logger.
        self.initialize_goose_logger();

        // Configure loggers.
        self.configure_loggers(defaults);

        // Configure `manager`. (DEVELOPER NOTE: manager and worker must be configured
        // immediately after the loggers.)
        self.manager = self
            .get_value(vec![
                // Use --manager if set.
                GooseValue {
                    value: Some(self.manager),
                    filter: !self.manager,
                    message: "",
                },
                // Otherwise use default.
                GooseValue {
                    value: defaults.manager,
                    filter: defaults.manager.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(false);

        // Configure `worker`. (DEVELOPER NOTE: manager and worker must be configured
        // immediately after the loggers.)
        self.worker = self
            .get_value(vec![
                // Use --worker if set.
                GooseValue {
                    value: Some(self.worker),
                    filter: !self.worker,
                    message: "",
                },
                // Otherwise use default.
                GooseValue {
                    value: defaults.worker,
                    filter: defaults.worker.is_none(),
                    message: "",
                },
            ])
            .unwrap_or(false);

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
                filter: defaults.users.is_none() || self.worker,
                message: "users",
            },
            // Otherwise use detected number of CPUs if not on Worker.
            GooseValue {
                value: Some(num_cpus::get()),
                filter: self.worker,
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
                    filter: defaults.startup_time.is_none() || self.worker,
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
                    filter: defaults.run_time.is_none() || self.worker,
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
                    filter: defaults.hatch_rate.is_none() || self.worker,
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
                    filter: defaults.timeout.is_none() || self.worker,
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
                filter: defaults.running_metrics.is_none() || self.worker,
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
                    filter: defaults.no_reset_metrics.is_none() || self.worker,
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
                    filter: defaults.no_metrics.is_none() || self.worker,
                    message: "no_metrics",
                },
            ])
            .unwrap_or(false);

        // Configure `no_task_metrics`.
        self.no_task_metrics = self
            .get_value(vec![
                // Use --no-task-metrics if set.
                GooseValue {
                    value: Some(self.no_task_metrics),
                    filter: !self.no_task_metrics,
                    message: "no_task_metrics",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.no_task_metrics,
                    filter: defaults.no_task_metrics.is_none() || self.worker,
                    message: "no_task_metrics",
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
                    filter: defaults.no_error_summary.is_none() || self.worker,
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
            // Otherwise use GooseDefault if set and not Manager.
            GooseValue {
                value: defaults.report_file.clone(),
                filter: defaults.report_file.is_none() || self.manager,
                message: "report_file",
            },
        ]) {
            Some(v) => v,
            None => "".to_string(),
        };

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
                    filter: defaults.no_debug_body.is_none() || self.manager,
                    message: "no_debug_body",
                },
            ])
            .unwrap_or(false);

        // Configure `status_codes`.
        self.status_codes = self
            .get_value(vec![
                // Use --status_codes if set.
                GooseValue {
                    value: Some(self.status_codes),
                    filter: !self.status_codes,
                    message: "status_codes",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.status_codes,
                    filter: defaults.status_codes.is_none() || self.worker,
                    message: "status_codes",
                },
            ])
            .unwrap_or(false);

        // Configure `requests_per_second`.
        self.requests_per_second = self
            .get_value(vec![
                // Use --requests-per-second if set.
                GooseValue {
                    value: Some(self.requests_per_second),
                    filter: !self.requests_per_second,
                    message: "requests_per_second",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.requests_per_second,
                    filter: defaults.requests_per_second.is_none() || self.worker,
                    message: "requests_per_second",
                },
            ])
            .unwrap_or(false);

        // Configure `rps_bucket`.
        self.rps_bucket = self
            .get_value(vec![
                // Use --requests-per-second-bucket if set.
                GooseValue {
                    value: Some(self.rps_bucket),
                    filter: self.rps_bucket == 0,
                    message: "rps_bucket",
                },
                // Otherwise use GooseDefault if set.
                GooseValue {
                    value: defaults.rps_bucket,
                    filter: defaults.rps_bucket.is_none() || self.worker,
                    message: "rps_bucket",
                },
            ])
            .unwrap_or(10);

        // Configure `no_telnet`.
        self.no_telnet = self
            .get_value(vec![
                // Use --no-telnet if set.
                GooseValue {
                    value: Some(self.no_telnet),
                    filter: !self.no_telnet,
                    message: "no_telnet",
                },
                // Force-disable telnet Controller if in Gaggle mode.
                GooseValue {
                    value: Some(true),
                    filter: !self.manager && !self.worker,
                    message: "",
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
                // Force-disable WebSocket Controller if in Gaggle mode.
                GooseValue {
                    value: Some(true),
                    filter: !self.manager && !self.worker,
                    message: "",
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
                    filter: defaults.no_autostart.is_none() || self.worker,
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
                    filter: defaults.no_gzip.is_none() || self.worker,
                    message: "no_gzip",
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
                filter: defaults.co_mitigation.is_none() || self.worker,
                message: "co_mitigation",
            },
            // Otherwise default to GooseCoordinaatedOmissionMitigation::Disabled.
            GooseValue {
                value: Some(GooseCoordinatedOmissionMitigation::Disabled),
                filter: self.worker,
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
                // Otherwise use GooseDefault if set and not on Manager.
                GooseValue {
                    value: defaults.throttle_requests,
                    filter: defaults.throttle_requests.is_none() || self.manager,
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
                    filter: defaults.sticky_follow.is_none() || self.worker,
                    message: "sticky_follow",
                },
            ])
            .unwrap_or(false);

        // Configure `expect_workers`.
        self.expect_workers = self.get_value(vec![
            // Use --expect-workers if configured.
            GooseValue {
                value: self.expect_workers,
                filter: self.expect_workers.is_none(),
                message: "expect_workers",
            },
            // Use GooseDefault if not already set and not Worker.
            GooseValue {
                value: defaults.expect_workers,
                filter: !self.expect_workers.is_some() && self.worker,
                message: "expect_workers",
            },
        ]);

        // Configure `no_hash_check`.
        self.no_hash_check = self
            .get_value(vec![
                // Use --no-hash_check if set.
                GooseValue {
                    value: Some(self.no_hash_check),
                    filter: !self.no_hash_check,
                    message: "no_hash_check",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.no_hash_check,
                    filter: defaults.no_hash_check.is_none() || self.worker,
                    message: "no_hash_check",
                },
            ])
            .unwrap_or(false);

        // Set `manager_bind_host` on Manager.
        self.manager_bind_host = self
            .get_value(vec![
                // Use --manager-bind-host if configured.
                GooseValue {
                    value: Some(self.manager_bind_host.to_string()),
                    filter: self.manager_bind_host.is_empty(),
                    message: "manager_bind_host",
                },
                // Otherwise use default if set and on Manager.
                GooseValue {
                    value: defaults.manager_bind_host.clone(),
                    filter: defaults.manager_bind_host.is_none() || !self.manager,
                    message: "manager_bind_host",
                },
                // Otherwise default to 0.0.0.0 if on Manager.
                GooseValue {
                    value: Some("0.0.0.0".to_string()),
                    filter: !self.manager,
                    message: "manager_bind_host",
                },
            ])
            .unwrap_or_else(|| "".to_string());

        // Set `manager_bind_port` on Manager.
        self.manager_bind_port = self
            .get_value(vec![
                // Use --manager-bind-port if configured.
                GooseValue {
                    value: Some(self.manager_bind_port),
                    filter: self.manager_bind_port == 0,
                    message: "manager_bind_port",
                },
                // Otherwise use default if set and on Manager.
                GooseValue {
                    value: defaults.manager_bind_port,
                    filter: defaults.manager_bind_port.is_none() || !self.manager,
                    message: "manager_bind_port",
                },
                // Otherwise default to DEFAULT_PORT if on Manager.
                GooseValue {
                    value: Some(DEFAULT_PORT.to_string().parse().unwrap()),
                    filter: !self.manager,
                    message: "manager_bind_port",
                },
            ])
            .unwrap_or(0);

        // Set `manager_host` on Worker.
        self.manager_host = self
            .get_value(vec![
                // Use --manager-host if configured.
                GooseValue {
                    value: Some(self.manager_host.to_string()),
                    filter: self.manager_host.is_empty(),
                    message: "manager_host",
                },
                // Otherwise use default if set and on Worker.
                GooseValue {
                    value: defaults.manager_host.clone(),
                    filter: defaults.manager_host.is_none() || !self.worker,
                    message: "manager_host",
                },
                // Otherwise default to 127.0.0.1 if on Worker.
                GooseValue {
                    value: Some("127.0.0.1".to_string()),
                    filter: !self.worker,
                    message: "manager_host",
                },
            ])
            .unwrap_or_else(|| "".to_string());

        // Set `manager_port` on Worker.
        self.manager_port = self
            .get_value(vec![
                // Use --manager-port if configured.
                GooseValue {
                    value: Some(self.manager_port),
                    filter: self.manager_port == 0,
                    message: "manager_port",
                },
                // Otherwise use default if set and on Worker.
                GooseValue {
                    value: defaults.manager_port,
                    filter: defaults.manager_port.is_none() || !self.worker,
                    message: "manager_port",
                },
                // Otherwise default to DEFAULT_PORT if on Worker.
                GooseValue {
                    value: Some(DEFAULT_PORT.to_string().parse().unwrap()),
                    filter: !self.worker,
                    message: "manager_port",
                },
            ])
            .unwrap_or(0);
    }

    /// Validate configured [`GooseConfiguration`] values.
    pub(crate) fn validate(&self) -> Result<(), GooseError> {
        // Validate nothing incompatible is enabled with --manager.
        if self.manager {
            // Don't allow --manager and --worker together.
            if self.worker {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager` && `configuration.worker`".to_string(),
                    value: "true".to_string(),
                    detail: "Goose can not run as both Manager and Worker".to_string(),
                });
            } else if !self.debug_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.debug_log`".to_string(),
                    value: self.debug_log.clone(),
                    detail: "`configuration.debug_log` can not be set on the Manager.".to_string(),
                });
            } else if !self.error_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.error_log`".to_string(),
                    value: self.error_log.clone(),
                    detail: "`configuration.error_log` can not be set on the Manager.".to_string(),
                });
            } else if !self.request_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.request_log`".to_string(),
                    value: self.request_log.clone(),
                    detail: "`configuration.request_log` can not be set on the Manager."
                        .to_string(),
                });
            } else if !self.task_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.task_log`".to_string(),
                    value: self.request_log.clone(),
                    detail: "`configuration.task_log` can not be set on the Manager.".to_string(),
                });
            } else if self.no_autostart {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_autostart`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_autostart` can not be set on the Manager."
                        .to_string(),
                });
            } else if !self.report_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.report_file`".to_string(),
                    value: self.report_file.to_string(),
                    detail: "`configuration.report_file` can not be set on the Manager."
                        .to_string(),
                });
            } else if self.no_debug_body {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_debug_body`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_debug_body` can not be set on the Manager."
                        .to_string(),
                });
            // Can not set `throttle_requests` on Manager.
            } else if self.throttle_requests > 0 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.throttle_requests`".to_string(),
                    value: self.throttle_requests.to_string(),
                    detail: "`configuration.throttle_requests` can not be set on the Manager."
                        .to_string(),
                });
            }
            if let Some(expect_workers) = self.expect_workers.as_ref() {
                // Must expect at least 1 Worker when running as Manager.
                if expect_workers == &0 {
                    return Err(GooseError::InvalidOption {
                        option: "`configuration.expect_workers`".to_string(),
                        value: expect_workers.to_string(),
                        detail: "`configuration.expect_workers must be set to at least 1."
                            .to_string(),
                    });
                }

                // Must be at least 1 user per worker.
                if let Some(users) = self.users.as_ref() {
                    if expect_workers > users {
                        return Err(GooseError::InvalidOption {
                            option: "`configuration.expect_workers`".to_string(),
                            value: expect_workers.to_string(),
                            detail: "`configuration.expect_workers can not be set to a value larger than `configuration.users`.".to_string(),
                        });
                    }
                } else {
                    return Err(GooseError::InvalidOption {
                        option: "`configuration.expect_workers`".to_string(),
                        value: expect_workers.to_string(),
                        detail: "`configuration.expect_workers can not be set without setting `configuration.users`.".to_string(),
                    });
                }
            } else {
                return Err(GooseError::InvalidOption {
                    option: "configuration.manager".to_string(),
                    value: true.to_string(),
                    detail: "Manager mode requires --expect-workers be configured".to_string(),
                });
            }
        } else {
            // Don't allow `expect_workers` if not running as Manager.
            if let Some(expect_workers) = self.expect_workers.as_ref() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.expect_workers`".to_string(),
                    value: expect_workers.to_string(),
                    detail: "`configuration.expect_workers` can not be set unless on the Manager."
                        .to_string(),
                });
            }
        }

        // Validate nothing incompatible is enabled with --worker.
        if self.worker {
            // Can't set `users` on Worker.
            if self.users.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "configuration.users".to_string(),
                    value: self.users.as_ref().unwrap().to_string(),
                    detail: "`configuration.users` can not be set together with the `configuration.worker`.".to_string(),
                });
            // Can't set `startup_time` on Worker.
            } else if self.startup_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.startup_time".to_string(),
                    value: self.startup_time.to_string(),
                    detail: "`configuration.startup_time` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `run_time` on Worker.
            } else if self.run_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.run_time".to_string(),
                    value: self.run_time.to_string(),
                    detail: "`configuration.run_time` can not be set in Worker mode.".to_string(),
                });
            // Can't set `hatch_rate` on Worker.
            } else if self.hatch_rate.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.hatch_rate`".to_string(),
                    value: self.hatch_rate.as_ref().unwrap().to_string(),
                    detail: "`configuration.hatch_rate` can not be set in Worker mode.".to_string(),
                });
            // Can't set `timeout` on Worker.
            } else if self.timeout.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.timeout`".to_string(),
                    value: self.timeout.as_ref().unwrap().to_string(),
                    detail: "`configuration.timeout` can not be set in Worker mode.".to_string(),
                });
            // Can't set `running_metrics` on Worker.
            } else if self.running_metrics.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.running_metrics".to_string(),
                    value: self.running_metrics.as_ref().unwrap().to_string(),
                    detail: "`configuration.running_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_reset_metrics` on Worker.
            } else if self.no_reset_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_reset_metrics".to_string(),
                    value: self.no_reset_metrics.to_string(),
                    detail: "`configuration.no_reset_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_metrics` on Worker.
            } else if self.no_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_metrics".to_string(),
                    value: self.no_metrics.to_string(),
                    detail: "`configuration.no_metrics` can not be set in Worker mode.".to_string(),
                });
            // Can't set `no_task_metrics` on Worker.
            } else if self.no_task_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_task_metrics".to_string(),
                    value: self.no_task_metrics.to_string(),
                    detail: "`configuration.no_task_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_error_summary` on Worker.
            } else if self.no_error_summary {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_error_summary".to_string(),
                    value: self.no_error_summary.to_string(),
                    detail: "`configuration.no_error_summary` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `status_codes` on Worker.
            } else if self.status_codes {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.status_codes".to_string(),
                    value: self.status_codes.to_string(),
                    detail: "`configuration.status_codes` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_autostart` on Worker.
            } else if self.no_autostart {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_autostart`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_autostart` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_gzip` on Worker.
            } else if self.no_gzip {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_gzip`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_gzip` can not be set in Worker mode.".to_string(),
                });
            } else if self
                .co_mitigation
                .as_ref()
                .unwrap_or(&GooseCoordinatedOmissionMitigation::Disabled)
                != &GooseCoordinatedOmissionMitigation::Disabled
            {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.co_mitigation`".to_string(),
                    value: format!("{:?}", self.co_mitigation.as_ref().unwrap()),
                    detail: "`configuration.co_mitigation` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `manager_bind_host` on Worker.
            } else if !self.manager_bind_host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager_bind_host`".to_string(),
                    value: self.manager_bind_host.to_string(),
                    detail: "`configuration.manager_bind_host` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `manager_bind_port` on Worker.
            } else if self.manager_bind_port > 0 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager_bind_port`".to_string(),
                    value: self.manager_bind_host.to_string(),
                    detail: "`configuration.manager_bind_port` can not be set in Worker mode."
                        .to_string(),
                });
            // Must set `manager_host` on Worker.
            } else if self.manager_host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager_host`".to_string(),
                    value: self.manager_host.clone(),
                    detail: "`configuration.manager_host` must be set when in Worker mode."
                        .to_string(),
                });
            // Must set `manager_port` on Worker.
            } else if self.manager_port == 0 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager_port`".to_string(),
                    value: self.manager_port.to_string(),
                    detail: "`configuration.manager_port` must be set when in Worker mode."
                        .to_string(),
                });
            // Can not set `sticky_follow` on Worker.
            } else if self.sticky_follow {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.sticky_follow`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.sticky_follow` can not be set in Worker mode."
                        .to_string(),
                });
            // Can not set `no_hash_check` on Worker.
            } else if self.no_hash_check {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_hash_check`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_hash_check` can not be set in Worker mode."
                        .to_string(),
                });
            }
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

        // Validate `no_metrics`.
        if self.no_metrics {
            // Status codes are not collected if metrics are disabled.
            if self.status_codes {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_metrics`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_metrics` can not be set with `configuration.status_codes`.".to_string(),
                });
            // Request log can't be written if metrics are disabled.
            } else if !self.request_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.request_log`".to_string(),
                    value: self.request_log.to_string(),
                    detail: "`configuration.request_log` can not be set with `configuration.no_metrics`.".to_string(),
                });
            // Task log can't be written if metrics are disabled.
            } else if !self.task_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.task_log`".to_string(),
                    value: self.task_log.to_string(),
                    detail:
                        "`configuration.task_log` can not be set with `configuration.no_metrics`."
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
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
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
        let verbose: usize = 0;
        let report_file = "custom-goose-report.html".to_string();
        let request_log = "custom-goose-request.log".to_string();
        let task_log = "custom-goose-task.log".to_string();
        let debug_log = "custom-goose-debug.log".to_string();
        let error_log = "custom-goose-error.log".to_string();
        let throttle_requests: usize = 25;
        let expect_workers: usize = 5;
        let manager_bind_host = "127.0.0.1".to_string();
        let manager_bind_port: usize = 1221;
        let manager_host = "127.0.0.1".to_string();
        let manager_port: usize = 1221;

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
            .set_default(GooseDefault::NoTaskMetrics, true)
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
            .set_default(GooseDefault::TaskLog, task_log.as_str())
            .unwrap()
            .set_default(GooseDefault::TaskFormat, GooseLogFormat::Raw)
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
            .set_default(GooseDefault::StatusCodes, true)
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
            .set_default(GooseDefault::Manager, true)
            .unwrap()
            .set_default(GooseDefault::ExpectWorkers, expect_workers)
            .unwrap()
            .set_default(GooseDefault::NoHashCheck, true)
            .unwrap()
            .set_default(GooseDefault::ManagerBindHost, manager_bind_host.as_str())
            .unwrap()
            .set_default(GooseDefault::ManagerBindPort, manager_bind_port)
            .unwrap()
            .set_default(GooseDefault::Worker, true)
            .unwrap()
            .set_default(GooseDefault::ManagerHost, manager_host.as_str())
            .unwrap()
            .set_default(GooseDefault::ManagerPort, manager_port)
            .unwrap();

        assert!(goose_attack.defaults.host == Some(host));
        assert!(goose_attack.defaults.users == Some(users));
        assert!(goose_attack.defaults.run_time == Some(run_time));
        assert!(goose_attack.defaults.hatch_rate == Some(hatch_rate));
        assert!(goose_attack.defaults.log_level == Some(log_level as u8));
        assert!(goose_attack.defaults.goose_log == Some(goose_log));
        assert!(goose_attack.defaults.request_body == Some(true));
        assert!(goose_attack.defaults.no_debug_body == Some(true));
        assert!(goose_attack.defaults.verbose == Some(verbose as u8));
        assert!(goose_attack.defaults.running_metrics == Some(15));
        assert!(goose_attack.defaults.no_reset_metrics == Some(true));
        assert!(goose_attack.defaults.no_metrics == Some(true));
        assert!(goose_attack.defaults.no_task_metrics == Some(true));
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
        assert!(goose_attack.defaults.status_codes == Some(true));
        assert!(
            goose_attack.defaults.co_mitigation
                == Some(GooseCoordinatedOmissionMitigation::Disabled)
        );
        assert!(goose_attack.defaults.throttle_requests == Some(throttle_requests));
        assert!(goose_attack.defaults.sticky_follow == Some(true));
        assert!(goose_attack.defaults.manager == Some(true));
        assert!(goose_attack.defaults.expect_workers == Some(expect_workers));
        assert!(goose_attack.defaults.no_hash_check == Some(true));
        assert!(goose_attack.defaults.manager_bind_host == Some(manager_bind_host));
        assert!(goose_attack.defaults.manager_bind_port == Some(manager_bind_port as u16));
        assert!(goose_attack.defaults.worker == Some(true));
        assert!(goose_attack.defaults.manager_host == Some(manager_host));
        assert!(goose_attack.defaults.manager_port == Some(manager_port as u16));
    }
}
