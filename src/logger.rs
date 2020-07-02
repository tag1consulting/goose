use serde_json::json;
use tokio::fs::File;
use tokio::io::BufWriter;
use tokio::prelude::*;
use tokio::sync::mpsc;

use crate::goose::GooseDebug;
use crate::GooseConfiguration;

/// Logger thread, opens a log file (if configured) and waits for messages from
/// GooseUser threads.
pub async fn logger_main(
    configuration: GooseConfiguration,
    mut log_receiver: mpsc::UnboundedReceiver<Option<GooseDebug>>,
) {
    // Prepare an asynchronous buffered file writer for stats_log_file (if enabled).
    let mut debug_log_file = None;
    if !configuration.debug_log_file.is_empty() {
        debug_log_file = match File::create(&configuration.debug_log_file).await {
            Ok(f) => {
                info!(
                    "writing errors to debug_log_file: {}",
                    &configuration.debug_log_file
                );
                Some(BufWriter::new(f))
            }
            Err(e) => {
                error!(
                    "failed to create debug_log_file ({}): {}",
                    configuration.debug_log_file, e
                );
                std::process::exit(1);
            }
        }
    }

    // Loop waiting for and writing error logs from GooseUser threads.
    loop {
        // Wait here until a GooseUser thread sends us an error to log, or all GooseUser threads
        // close the error log channel.
        if let Some(message) = log_receiver.recv().await {
            if let Some(goose_debug) = message {
                // All Options are defined above, search for formatted_log.
                if let Some(file) = debug_log_file.as_mut() {
                    let formatted_log = match configuration.debug_log_format.as_str() {
                        // Use serde_json to create JSON.
                        "json" => json!(goose_debug).to_string(),
                        // Raw format is Debug output for GooseRawRequest structure.
                        "raw" => format!("{:?}", goose_debug).to_string(),
                        _ => unreachable!(),
                    };

                    match file.write(format!("{}\n", formatted_log).as_ref()).await {
                        Ok(_) => (),
                        Err(e) => {
                            warn!(
                                "failed to write  to {}: {}",
                                &configuration.debug_log_file, e
                            );
                        }
                    }
                };
            } else {
                // Empty message means it's time to exit.
                break;
            }
        } else {
            // Pipe is closed, cleanup and exit.
            break;
        }
    }

    // Cleanup and flush all logs to disk.
    if let Some(file) = debug_log_file.as_mut() {
        info!("flushing debug_log_file: {}", &configuration.debug_log_file);
        let _ = file.flush().await;
    };
}
