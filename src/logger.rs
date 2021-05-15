//! Optional debug logger thread.
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

use serde_json::json;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::io::BufWriter;

use crate::goose::GooseDebug;
use crate::GooseConfiguration;

/// Logger thread, opens a log file (if configured) and waits for messages from
/// [`GooseUser`](../goose/struct.GooseUser.html) threads. This function is not intended
/// to be invoked manually.
pub async fn logger_main(
    configuration: GooseConfiguration,
    log_receiver: flume::Receiver<Option<GooseDebug>>,
) {
    // Determine if a debug file has been configured.
    let mut debug_file_path: Option<String> = None;
    if !configuration.debug_file.is_empty() {
        debug_file_path = Some(configuration.debug_file.clone());
    }

    // If debug file is configured, prepare an asynchronous buffered file writer.
    let mut debug_file = None;
    if let Some(file_path) = debug_file_path {
        // Allocate a smaller buffer (64K) when not logging response bodies.
        let buffer_capacity = if configuration.no_debug_body {
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
    while let Ok(message) = log_receiver.recv_async().await {
        if let Some(goose_debug) = message {
            // All Options are defined above, search for formatted_log.
            if let Some(file) = debug_file.as_mut() {
                let formatted_log = match configuration.debug_format.as_str() {
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
                        warn!("failed to write to {}: {}", &configuration.debug_file, e);
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
        info!("flushing debug_file: {}", &configuration.debug_file);
        let _ = file.flush().await;
    };
}
