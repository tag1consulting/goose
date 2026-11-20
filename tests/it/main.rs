//! Goose's integration tests, in one test binary: each file that was a test
//! binary of its own is a module here, so a build links one binary, not one
//! for each file.
//!
//! Run them with cargo-nextest, which runs each test in a process of its
//! own, many at once:
//!
//! ```text
//! cargo nextest run --all-features
//! ```
//!
//! `cargo test` still runs them, all in this one process. A test that sets
//! something global to the process, such as the logger, the working
//! directory, an environment variable or Goose's killswitch (which a failed
//! threshold triggers too), or signals the process, starts with
//! [`own_process`], which runs it in a process of its own under `cargo
//! test` too. A test that shares something with the other tests in its
//! module only, such as a static, is `#[serial(module)]`, keyed by its
//! module, so it waits only for the tests it waited for as its own binary.
//!
//! `tests/gaggle.rs` stays a binary of its own: its tests start the `simple`
//! example as manager and worker processes, and clean up after them from a
//! process wide list. `tests/metrics_print_unlocked.rs` stays one too: it
//! runs without the test harness, as it checks no thread holds stdout.

#[path = "../common/mod.rs"]
mod common;
#[path = "../mcp_client/mod.rs"]
mod mcp_client;
#[path = "../saved_runs/mod.rs"]
mod saved_runs;
#[path = "../common/tls_server.rs"]
mod tls_server;

/// Set in the process [`own_process`] starts, to the test it runs.
const OWN_PROCESS: &str = "GOOSE_TEST_OWN_PROCESS";

/// Whether the calling test, `test` in the module the caller names with
/// `module_path!()`, is to run its body here: true when it is alone in this
/// process, as under cargo-nextest, which runs each test in a process of its
/// own, or in the process this started for it. Otherwise, under `cargo
/// test`, runs the test in a new process of this binary, fails if it fails,
/// and returns false: the test has run, and the caller returns.
///
/// ```ignore
/// if !crate::own_process(module_path!(), "a_gz_goose_log_holds_every_run") {
///     return;
/// }
/// ```
pub fn own_process(module: &str, test: &str) -> bool {
    // The test's name as the harness knows it, with no crate name.
    let name = match module.split_once("::") {
        Some((_crate, path)) => format!("{path}::{test}"),
        None => test.to_string(),
    };
    if std::env::var("NEXTEST_EXECUTION_MODE").as_deref() == Ok("process-per-test")
        || std::env::var(OWN_PROCESS).as_deref() == Ok(name.as_str())
    {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            name.as_str(),
            "--exact",
            "--include-ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(OWN_PROCESS, &name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Shown with the test's own output, which the harness captures.
    print!("{stdout}");
    eprint!("{stderr}");
    assert!(
        output.status.success(),
        "{} failed in its own process: {}",
        name,
        output.status
    );
    // The test ran: not filtered out.
    assert!(
        stdout.contains("1 passed"),
        "{} did not run in its own process",
        name
    );
    false
}

mod atomic_counters;
mod baseline;
mod builder;
#[cfg(unix)]
mod cancel;
mod client_builder;
mod clone_user_with_session_data;
mod closure;
mod collapse_metric_sends;
mod compression;
#[cfg(feature = "dashboard")]
mod control_agreement;
mod controller;
mod controller_bind;
mod controller_no_autostart;
mod coordinated_omission;
mod coordinated_omission_integration;
mod custom_request;
#[cfg(feature = "dashboard")]
mod dashboard;
#[cfg(feature = "dashboard")]
mod dashboard_goose_log;
#[cfg(all(unix, feature = "dashboard"))]
mod dashboard_no_folder;
mod defaults;
mod discard_body;
mod error;
#[cfg(feature = "gaggle")]
mod gaggle_shadow;
mod goose_log_folder;
mod gzip_goose_log;
mod gzip_goose_log_after_execute;
#[cfg(feature = "dashboard")]
mod gzip_goose_log_dashboard;
mod gzip_goose_log_folder;
mod gzip_goose_log_panic;
mod gzip_goose_log_quiet;
mod http2;
#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
mod http_version;
mod iterations;
mod keep_alive_retry;
mod logs;
#[cfg(feature = "mcp")]
mod mcp_controller;
mod no_nan;
mod no_normal_tasks;
mod one_taskset;
mod output_default_folder;
mod output_files;
#[cfg(feature = "pdf-reports")]
mod pdf_chrome;
#[cfg(feature = "pdf-reports")]
mod pdf_print_html_once;
mod pdf_reports;
mod redirect;
mod request_headers;
mod reset_errors;
mod scenarios;
mod scheduler;
mod sequence;
mod session;
mod setup_teardown;
mod status_code_response_times;
mod test_plan;
mod thresholds;
mod throttle;
#[cfg(feature = "dashboard")]
mod throttle_live;
#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
mod tls_roots;
mod transaction_errors;
mod update_graphs;
mod user_metrics_graph_reset;
mod user_rate;
mod worker_data;
