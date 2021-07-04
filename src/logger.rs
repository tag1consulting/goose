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

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::io::BufWriter;

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

#[async_trait]
pub(crate) trait GooseLogger<T> {}

impl GooseConfiguration {
    /// Logger thread, opens a log file (if configured) and waits for messages from
    /// [`GooseUser`](../goose/struct.GooseUser.html) threads.
    pub(crate) async fn logger_main(
        self: GooseConfiguration,
        receiver: flume::Receiver<Option<GooseLog>>,
    ) -> Result<(), GooseError> {
        // Determine which log files have been configured.
        let mut debug_file_path: Option<String> = None;
        if !self.debug_file.is_empty() {
            debug_file_path = Some(self.debug_file.clone());
        }

        // If debug file is configured, prepare an asynchronous buffered file writer.
        let mut debug_file = None;
        if let Some(file_path) = debug_file_path {
            // Allocate a smaller buffer (64K) when not logging response bodies.
            let buffer_capacity = if self.no_debug_body {
                64 * 1024
            // Allocate a bigger buffer (8M) when logging response bodies.
            } else {
                8 * 1024 * 1024
            };
            debug_file = match File::create(&file_path).await {
                Ok(f) => {
                    info!("writing errors to debug_file: {}", &file_path);
                    Some(BufWriter::with_capacity(buffer_capacity, f))
                }
                Err(e) => {
                    panic!("failed to create debug_file ({}): {}", file_path, e);
                }
            }
        }

        // Loop waiting for and writing error logs from GooseUser threads.
        while let Ok(message) = receiver.recv_async().await {
            if let Some(goose_debug) = message {
                // All Options are defined above, search for formatted_log.
                if let Some(file) = debug_file.as_mut() {
                    let formatted_log = match self.debug_format.as_str() {
                        // Use serde_json to create JSON.
                        "json" => json!(goose_debug).to_string(),
                        // Raw format is Debug output for GooseRawRequest structure.
                        "raw" => format!("{:?}", goose_debug).to_string(),
                        _ => unreachable!(),
                    };

                    // Start with a line feed instead of ending with a line feed to more gracefully
                    // handle pages too large to fit in the BufWriter.
                    match file.write(format!("\n{}", formatted_log).as_ref()).await {
                        Ok(_) => (),
                        Err(e) => {
                            warn!("failed to write to {}: {}", &self.debug_file, e);
                        }
                    }
                };
            } else {
                // Empty message means it's time to exit.
                break;
            }
        }

        // Cleanup and flush all logs to disk.
        if let Some(file) = debug_file.as_mut() {
            info!("flushing debug_file: {}", &self.debug_file);
            let _ = file.flush().await;
        };

        Ok(())
    }
}
