//! Optional logging threads.
//!
//! Goose can generate multiple logs during a load test, enabled through any combination of
//! the following run-time options:
//!  - `--log-file`:
//!  - `--debug-file`:
//!  - `--requests-file`:
//!  - `--tasks-file`:
//!  - `--errors-file`:
//!
//! The format of most logs can also be configured:
//!  - `--debug-format`:
//!  - `--requests-format`:
//!  - `--tasks-format`:
//!  - `--errors-format`:
//!
//! NOTES:
//!  - `log-file` is different: it's more syslog
//!  - `requests-file` currently happens in the parent stream: do we want to add overhead to
//!    send this to another thread? (overall maybe an async win?)
//!  - `tasks-file` and `errors-file` are not yet created: the goal is to implement them all
//!    with common logic
//!  - the `format` should be configured through an Enum and as much as possible should be
//!    available to all logs (except _maybe_ `log-file`, although json-format logs here could
//!    also be nice)
//!
//! TODO:
//!  - Traits: what do all the loggers share?
//!     o logger_loop()
//!     o message
//!     o "level"?
//!     o "format"?
//!
//!
//!
//! The Goose debug logger is enabled with the `--debug-file` command-line option, or the
//! [`GooseDefault::DebugFile`](../enum.GooseDefault.html#variant.DebugFile) default
//! configuration option. When enabled, this thread is launched and a channel is provided
//! from all [`GooseUser`](../goose/struct.GooseUser.html) threads to send debug information
//! for efficient logging to file. The debug logger thread uses Tokio's asynchronous
//! [`BufWriter`](https://docs.rs/tokio/*/tokio/io/struct.BufWriter.html).
//!
//! ## Writing Debug Logs
//! Logs can be sent to the logger thread by invoking
//! [`log_debug`](../goose/struct.GooseUser.html#method.log_debug)
//! from load test task functions.
//!
//! Calls to
//! [`set_failure`](../goose/struct.GooseUser.html#method.set_failure)
//! automatically invoke
//! [`log_debug`](../goose/struct.GooseUser.html#method.log_debug).
//!
//! Most of the included examples showing how to use the debug logger include a copy of the
//! request made, the response headers returned by the server, and the response body. It can
//! also be used to log arbitrary information, for example if you want to record everything you
//! sent via a POST to a form.
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut task = task!(post_to_form);
//!
//! async fn post_to_form(user: &GooseUser) -> GooseTaskResult {
//!     let path = "/path/to/form";
//!     let params = [
//!      ("field_1", "foo"),
//!      ("field_2", "bar"),
//!      ("op", "Save"),
//!     ];
//!
//!     // Only log the form parameters we will post.
//!     user.log_debug(
//!         &format!("POSTing {:?} on {}", &params, path),
//!         None,
//!         None,
//!         None,
//!     )?;
//!
//!     let request_builder = user.goose_post(path).await?;
//!     let goose = user.goose_send(request_builder.form(&params), None).await?;
//!
//!     // Log the form parameters that were posted together with details about the entire
//!     // request that was sent to the server.
//!     user.log_debug(
//!         &format!("POSTing {:#?} on {}", &params, path),
//!         Some(&goose.request),
//!         None,
//!         None,
//!     )?;
//!
//!     Ok(())
//! }
//! ```
//!
//! The first call to
//! [`log_debug`](../goose/struct.GooseUser.html#method.log_debug)
//! results in a debug log message similar to:
//! ```json
//! {"body":null,"header":null,"request":null,"tag":"POSTing [(\"field_1\", \"foo\"), (\"field_2\", \"bar\"), (\"op\", \"Save\")] on /path/to/form"}
//! ```
//!
//! The second call to
//! [`log_debug`](../goose/struct.GooseUser.html#method.log_debug)
//! results in a debug log message similar to:
//! ```json
//! {"body":null,"header":null,"request":{"elapsed":1,"final_url":"http://local.dev/path/to/form","method":"POST","name":"(Anon) post to form","redirected":false,"response_time":22,"status_code":404,"success":false,"update":false,"url":"http://local.dev/path/to/form","user":0},"tag":"POSTing [(\"field_1\", \"foo\"), (\"field_2\", \"bar\"), (\"op\", \"Save\")] on /path/to/form"}
//! ```
//!
//! For a more complex debug logging example, refer to the
//! [`log_debug`](../goose/struct.GooseUser.html#method.log_debug) documentation.
//!
//! ## Reducing File And Memory Usage
//!
//! The debug logger can result in a very large debug file, as by default it includes the
//! entire body of any pages returned that result in an error. This also requires allocating
//! a bigger [`BufWriter`](https://docs.rs/tokio/*/tokio/io/struct.BufWriter.html), and can
//! generate a lot of disk io.
//!
//! If you don't need to log response bodies, you can disable this functionality (and reduce
//! the amount of RAM required by the
//! [`BufWriter`](https://docs.rs/tokio/*/tokio/io/struct.BufWriter.html) by setting the
//! `--no-debug-body` command-line option, or the
//! [`GooseDefault::NoDebugBody`](../enum.GooseDefault.html#variant.NoDebugBody) default
//! configuration option. The debug logger will still record any custom messages, details
//! about the request (when available), and all server response headers (when available).

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::goose::GooseDebug;
use crate::metrics::{GooseRequestMetric, GooseTaskMetric};
use crate::{GooseConfiguration, GooseError};

/// OR: what about a single Logger thread that can write to all the log files: receiving messages
/// via an enum...?

/// If enabled, the logger thread can accept any of the following types of messages, and will
/// write them to the correct log file.
#[derive(Debug, Deserialize, Serialize)]
pub enum GooseLog {
    Debug(GooseDebug),
    Request(GooseRequestMetric),
    Task(GooseTaskMetric),
}

pub(crate) trait GooseLogger<T> {
    fn format_message(&self, message: T) -> String;
}
impl GooseLogger<GooseDebug> for GooseConfiguration {
    fn format_message(&self, message: GooseDebug) -> String {
        match self.debug_format.as_str() {
            // Use serde_json to create JSON.
            "json" => json!(message).to_string(),
            // Raw format is Debug output for GooseRawRequest structure.
            "raw" => format!("{:?}", message),
            _ => unreachable!(),
        }
    }
}
impl GooseLogger<GooseRequestMetric> for GooseConfiguration {
    fn format_message(&self, message: GooseRequestMetric) -> String {
        match self.debug_format.as_str() {
            // Use serde_json to create JSON.
            "json" => json!(message).to_string(),
            // Raw format is Debug output for GooseRawRequest structure.
            "raw" => format!("{:?}", message),
            _ => unreachable!(),
        }
    }
}
impl GooseLogger<GooseTaskMetric> for GooseConfiguration {
    fn format_message(&self, message: GooseTaskMetric) -> String {
        match self.debug_format.as_str() {
            // Use serde_json to create JSON.
            "json" => json!(message).to_string(),
            // Raw format is Debug output for GooseRawRequest structure.
            "raw" => format!("{:?}", message),
            _ => unreachable!(),
        }
    }
}

impl GooseConfiguration {
    /*
    fn get_logger_path(&self, log_type: &GooseLog) -> Option<&str> {
        match log_type {
            GooseLog::Debug(_) => {
                if !self.debug_file.is_empty() {
                    Some(&self.debug_file)
                } else {
                    None
                }
            }
            GooseLog::Request(_) => {
                if !self.requests_file.is_empty() {
                    Some(&self.requests_file)
                } else {
                    None
                }
            }
            GooseLog::Task(_) => None,
        }
    }
    */

    /*
    pub(crate) fn enable_logger(self: GooseConfiguration, path: Option<&str>) -> String {
        if let Some(file) = path {
            file.to_string()
        } else {
            "".to_string()
        }
    }
    */

    async fn open_log_file(
        &self,
        log_file_path: &str,
        log_file_type: &str,
        buffer_capacity: usize,
    ) -> std::option::Option<tokio::io::BufWriter<tokio::fs::File>> {
        if log_file_path.is_empty() {
            None
        } else {
            match File::create(log_file_path).await {
                Ok(f) => {
                    info!("writing {} to: {}", log_file_type, log_file_path);
                    Some(BufWriter::with_capacity(buffer_capacity, f))
                }
                Err(e) => {
                    panic!(
                        "failed to create {} ({}): {}",
                        log_file_type, log_file_path, e
                    );
                }
            }
        }
    }

    /// Logger thread, opens a log file (if configured) and waits for messages from
    /// [`GooseUser`](../goose/struct.GooseUser.html) threads.
    pub(crate) async fn logger_main(
        self: GooseConfiguration,
        receiver: flume::Receiver<Option<GooseLog>>,
    ) -> Result<(), GooseError> {
        // If the debug_file is enabled, allocate a buffer and open the file.
        let mut debug_file = self
            .open_log_file(
                &self.debug_file,
                "debug file",
                if self.no_debug_body {
                    // Allocate a smaller 64K buffer if not logging response body.
                    64 * 1024
                } else {
                    // Allocate a larger 8M buffer if logging response body.
                    8 * 1024 * 1024
                },
            )
            .await;

        // If the requests_file is enabled, allocate a buffer and open the file.
        let mut requests_file = self
            .open_log_file(&self.requests_file, "requests file", 64 * 1024)
            .await;

        // @TODO: If tasks log is enabled, allocate buffer.
        let mut tasks_file = self
            .open_log_file(&self.requests_file, "tasks file", 64 * 1024)
            .await;

        // Loop waiting for and writing error logs from GooseUser threads.
        while let Ok(received_message) = receiver.recv_async().await {
            if let Some(message) = received_message {
                let formatted_message;
                if let Some(log_file) = match message {
                    GooseLog::Debug(debug_message) => {
                        formatted_message = self.format_message(debug_message).to_string();
                        debug_file.as_mut()
                    }
                    GooseLog::Request(request_message) => {
                        formatted_message = self.format_message(request_message).to_string();
                        requests_file.as_mut()
                    }
                    GooseLog::Task(task_message) => {
                        formatted_message = self.format_message(task_message).to_string();
                        tasks_file.as_mut()
                    }
                } {
                    // Start with a line feed instead of ending with a line feed to more gracefully
                    // handle pages too large to fit in the BufWriter.
                    match log_file
                        .write(format!("\n{}", formatted_message).as_ref())
                        .await
                    {
                        Ok(_) => (),
                        Err(e) => {
                            warn!("failed to write to {}: {}", &self.debug_file, e);
                        }
                    }
                } else {
                    // Do not accept messages for disabled loggers as it's unnecessary overhead.
                    unreachable!();
                }
            } else {
                // Empty message means it's time to exit.
                break;
            }
        }

        // Cleanup and flush all logs to disk.
        if let Some(log_file) = debug_file.as_mut() {
            info!("flushing debug_file: {}", &self.debug_file);
            let _ = log_file.flush().await;
        };
        if let Some(log_file) = requests_file.as_mut() {
            info!("flushing requests_file: {}", &self.requests_file);
            let _ = log_file.flush().await;
        }
        /*
        if let Some(log_file) = tasks_file.as_mut() {
            info!("flushing tasks_file: {}", &self.tasks_file);
            let _ = log_file.flush().await;
        }
        */

        Ok(())
    }
}
