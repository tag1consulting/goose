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
    // Determine if a debug file has been configured.
    let mut debug_file_path: Option<String> = None;
    if !configuration.debug_file.is_empty() {
        debug_file_path = Some(configuration.debug_file.clone());
    }

    // If debug file is configured, prepare an asynchronous buffered file writer.
    let mut debug_file = None;
    if let Some(file_path) = debug_file_path {
        debug_file = match File::create(&file_path).await {
            Ok(f) => {
                info!("writing errors to debug_file: {}", &file_path);
                Some(BufWriter::new(f))
            }
            Err(e) => {
                panic!("failed to create debug_file ({}): {}", file_path, e);
            }
        }
    }

    // Loop waiting for and writing error logs from GooseUser threads.
    while let Some(message) = log_receiver.recv().await {
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

                match file.write(format!("{}\n", formatted_log).as_ref()).await {
                    Ok(_) => (),
                    Err(e) => {
                        warn!("failed to write  to {}: {}", &configuration.debug_file, e);
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
