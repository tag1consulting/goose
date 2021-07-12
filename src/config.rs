use gumdrop::Options;
use serde::{Deserialize, Serialize};

use crate::logger::GooseLogFormat;
use crate::metrics::GooseCoordinatedOmissionMitigation;

/// Options available when launching a Goose load test.
#[derive(Options, Debug, Clone, Serialize, Deserialize)]
pub struct GooseConfiguration {
    /// Displays this help
    #[options(short = "h")]
    pub help: bool,
    /// Prints version information
    #[options(short = "V")]
    pub version: bool,
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
    /// Stops after (30s, 20m, 3h, 1h30m, etc)
    #[options(short = "t", meta = "TIME")]
    pub run_time: String,
    /// Enables Goose log file and sets name
    #[options(short = "G", meta = "NAME")]
    pub goose_log: String,
    /// Sets Goose log level (-g, -gg, etc)
    #[options(short = "g", count)]
    pub log_level: u8,
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
    /// Sets request log format (csv, json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub request_format: Option<GooseLogFormat>,
    /// Sets task log file name
    #[options(short = "T", meta = "NAME")]
    pub task_log: String,
    /// Sets task log format (csv, json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub task_format: Option<GooseLogFormat>,
    /// Sets error log file name
    #[options(short = "E", meta = "NAME")]
    pub error_log: String,
    /// Sets error log format (csv, json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub error_format: Option<GooseLogFormat>,
    /// Sets debug log file name
    #[options(short = "D", meta = "NAME")]
    pub debug_log: String,
    /// Sets debug log format (csv, json, raw)
    #[options(no_short, meta = "FORMAT")]
    pub debug_format: Option<GooseLogFormat>,
    /// Do not include the response body in the debug log
    #[options(no_short)]
    pub no_debug_body: bool,
    // Add a blank line and then an Advanced: header after this option
    #[options(no_short, help = "Tracks additional status code metrics\n\nAdvanced:")]
    pub status_codes: bool,

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
    /// Sets coordinated omission mitigation strategy
    #[options(no_short, meta = "STRATEGY")]
    pub co_mitigation: Option<GooseCoordinatedOmissionMitigation>,
    /// Sets maximum requests per second
    #[options(no_short, meta = "VALUE")]
    pub throttle_requests: usize,
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
    pub expect_workers: Option<u16>,
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
