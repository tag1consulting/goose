//! PDF report generation functionality
//!
//! This module provides PDF report generation by converting existing HTML reports
//! to PDF format using headless Chrome. It leverages the HTML report generation
//! from the print module and converts it to PDF with configurable options.
//!
//! **Note**: This entire module is only available when compiled with the `pdf-reports` feature flag.

use crate::logger::ScopedLogLevel;
use crate::GooseError;
use headless_chrome::{Browser, LaunchOptions};
use log::{debug, LevelFilter};
use std::{
    ffi::OsStr,
    fs,
    path::Path,
    time::{Duration, Instant},
};

/// Centralized error message module for consistent PDF error handling
mod pdf_errors {
    use crate::GooseError;
    use std::time::Duration;

    /// Creates a consistent error for Chrome launch failures
    pub fn chrome_launch_error(inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Chrome launch failed: {inner_error}"),
            detail: chrome_launch_detail(&inner_error),
        }
    }

    /// Explains a Chrome launch failure and how to point Goose at a browser.
    ///
    /// Goose does not download Chrome. `headless_chrome` uses the `CHROME`
    /// environment variable when it names an existing file, and otherwise
    /// searches `PATH` and the standard install locations.
    fn chrome_launch_detail(inner_error: &str) -> String {
        const HOW_TO: &str = "PDF generation requires Chrome or Chromium to be installed. Install one, or set the CHROME environment variable to the full path of an existing Chrome or Chromium binary.";
        // `Command::spawn` reports ENOEXEC as "Exec format error (os error 8)"
        // when the binary was found but was built for another architecture.
        if inner_error.contains("Exec format error") {
            format!("The Chrome binary was found but could not run: it does not match this machine's architecture ({}). {HOW_TO}", std::env::consts::ARCH)
        } else {
            HOW_TO.to_string()
        }
    }

    /// Creates a consistent error for Chrome configuration failures
    pub fn chrome_config_error(inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Chrome configuration failed: {inner_error}"),
            detail: "PDF generation requires Chrome/Chromium. Try installing Chrome or running with --pdf-scale to adjust memory usage.".to_string(),
        }
    }

    /// Creates a consistent error for Chrome operation failures
    pub fn chrome_operation_error(operation: &str, inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("{operation} failed: {inner_error}"),
            detail: format!("Chrome {operation} operation failed. This may indicate Chrome instability or insufficient system resources."),
        }
    }

    /// Creates a consistent error for timeout scenarios
    pub fn timeout_error(duration: Duration) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: "timeout".to_string(),
            detail: format!("Chrome operation timed out after {duration:?}. This may indicate Chrome is unresponsive or the system is under heavy load."),
        }
    }

    /// Creates a consistent error for file write failures
    pub fn file_write_error(inner_error: String, output_path: &std::path::Path) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to write PDF file: {inner_error}"),
            detail: format!("Unable to write PDF to {}", output_path.display()),
        }
    }

    /// Creates a consistent error for content measurement failures
    pub fn content_measurement_error(inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to measure content height: {inner_error}"),
            detail: "Unable to calculate content dimensions for PDF".to_string(),
        }
    }

    /// Creates a consistent error for PDF generation failures with scale information
    pub fn pdf_generation_error(
        inner_error: String,
        scale: f64,
        width: f64,
        height: f64,
    ) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("PDF generation failed: {inner_error}"),
            detail: format!("Chrome's print-to-PDF operation failed. Scale: {scale}, Paper size: {width:.1}\" x {height:.1}\". Try adjusting --pdf-scale or reducing report complexity."),
        }
    }
}

/// Manages Chrome browser process lifecycle with automatic cleanup.
///
/// This struct provides RAII-based resource management for Chrome processes,
/// ensuring proper cleanup even in error scenarios.
struct ChromeSession {
    browser: Browser,
    process_id: Option<u32>,
    start_time: Instant,
}

impl ChromeSession {
    /// Creates a new Chrome session with proper resource tracking.
    ///
    /// # Arguments
    /// * `launch_options` - Chrome launch configuration
    ///
    /// # Returns
    /// * `Result<ChromeSession, GooseError>` - The Chrome session or an error
    fn new(launch_options: LaunchOptions) -> Result<Self, GooseError> {
        let start_time = Instant::now();

        debug!("Launching Chrome browser for PDF generation");
        let browser = Browser::new(launch_options)
            .map_err(|e| pdf_errors::chrome_launch_error(e.to_string()))?;

        // Attempt to get the process ID for tracking
        let process_id = browser.get_process_id();

        if let Some(pid) = process_id {
            debug!("Chrome browser launched with process ID: {pid}");
        } else {
            debug!("Chrome browser launched (process ID not available)");
        }

        Ok(ChromeSession {
            browser,
            process_id,
            start_time,
        })
    }

    /// Gets a reference to the browser instance.
    fn browser(&self) -> &Browser {
        &self.browser
    }

    /// Checks if the Chrome operation has exceeded the timeout.
    fn check_timeout(&self, timeout: Duration) -> Result<(), GooseError> {
        if self.start_time.elapsed() > timeout {
            return Err(pdf_errors::timeout_error(timeout));
        }
        Ok(())
    }
}

impl Drop for ChromeSession {
    fn drop(&mut self) {
        let elapsed = self.start_time.elapsed();

        if let Some(pid) = self.process_id {
            debug!("Cleaning up Chrome process {pid} (ran for {elapsed:?})");
        } else {
            debug!("Cleaning up Chrome browser session (ran for {elapsed:?})");
        }

        // The Browser's Drop trait handles the actual process termination
        // when self.browser is dropped
    }
}

/// Get Chrome launch arguments based on verbosity level
fn get_chrome_launch_args(verbose: bool) -> Vec<&'static OsStr> {
    let mut args = vec![
        OsStr::new("--no-sandbox"),
        OsStr::new("--disable-dev-shm-usage"),
        OsStr::new("--headless"),
    ];

    if !verbose {
        // Normal operation: Maximum logging suppression
        args.extend([
            // Core logging suppression
            OsStr::new("--disable-logging"),
            OsStr::new("--log-level=3"), // Only fatal errors
            OsStr::new("--silent"),
            // Component-specific suppression
            OsStr::new("--disable-dev-tools-console"),
            OsStr::new("--disable-extensions"),
            OsStr::new("--disable-plugins"),
            OsStr::new("--disable-default-apps"),
            OsStr::new("--disable-sync"),
            // Background process suppression
            OsStr::new("--disable-background-timer-throttling"),
            OsStr::new("--disable-renderer-backgrounding"),
            OsStr::new("--disable-backgrounding-occluded-windows"),
        ]);
    } else {
        // Verbose mode: Enable Chrome logging for debugging
        args.extend([
            OsStr::new("--enable-logging"),
            OsStr::new("--log-level=0"), // All logs
            OsStr::new("--v=1"),         // Chrome verbose logging
        ]);
    }

    args
}

/// Generate a PDF report from HTML content using headless Chrome
///
/// This function uses Chrome's native argument system to control logging behavior
/// based on the verbose parameter. In normal operation, Chrome logging is suppressed
/// for clean output. In verbose mode, Chrome debugging information is displayed.
///
/// The function implements RAII-based resource management to ensure Chrome processes
/// are properly cleaned up even in error scenarios, with timeout protection to prevent
/// hanging operations.
pub(crate) fn generate_pdf_from_html(
    html_content: &str,
    output_path: &Path,
    scale: f64,
    verbose: bool,
    timeout_seconds: u64,
) -> Result<(), GooseError> {
    // Get appropriate Chrome arguments based on verbosity
    let chrome_args = get_chrome_launch_args(verbose);

    // Create a thread-safe scoped logging guard that temporarily adjusts log level for headless Chrome operations
    // This suppresses Rust logging output during PDF generation while maintaining thread safety
    let _log_guard = if !verbose {
        Some(ScopedLogLevel::new(LevelFilter::Error)?)
    } else {
        None
    };

    // Launch Chrome with appropriate arguments and resource tracking
    let launch_options = LaunchOptions::default_builder()
        .headless(true)
        .args(chrome_args)
        .build()
        .map_err(|e| pdf_errors::chrome_config_error(e.to_string()))?;

    // Create Chrome session with automatic resource management
    let chrome_session = ChromeSession::new(launch_options)?;

    // Set timeout for Chrome operations (configurable)
    let timeout = Duration::from_secs(timeout_seconds);

    // Create a new tab
    chrome_session.check_timeout(timeout)?;
    let tab = chrome_session
        .browser()
        .new_tab()
        .map_err(|e| pdf_errors::chrome_operation_error("tab creation", e.to_string()))?;

    // Create a data URL from the HTML content
    let encoded_html = urlencoding::encode(html_content);
    let data_url = format!("data:text/html;charset=utf-8,{encoded_html}");

    // Navigate to the data URL
    chrome_session.check_timeout(timeout)?;
    tab.navigate_to(&data_url)
        .map_err(|e| pdf_errors::chrome_operation_error("HTML loading", e.to_string()))?;

    // Wait for the page to load
    chrome_session.check_timeout(timeout)?;
    tab.wait_until_navigated()
        .map_err(|e| pdf_errors::chrome_operation_error("page loading", e.to_string()))?;

    // Optimized content height calculation using document.documentElement.scrollHeight
    // This is significantly more performant than iterating through all DOM elements
    // while still accurately capturing the full content height including overflow content
    let content_height_script = r#"
        (function() {
            // Primary method: Use scrollHeight which efficiently captures full content height
            // This includes content that overflows the viewport and handles most edge cases
            const scrollHeight = document.documentElement.scrollHeight / 96; // Convert to inches
            
            // Fallback: Check document body height in case scrollHeight is unreliable
            // Some edge cases with CSS positioning might make scrollHeight insufficient
            const bodyHeight = Math.max(
                document.body.scrollHeight,
                document.body.offsetHeight
            ) / 96; // Convert to inches
            
            // Use the maximum of both methods with a small buffer for safety
            const calculatedHeight = Math.max(scrollHeight, bodyHeight);
            const bufferedHeight = calculatedHeight + 0.05; // ~5px buffer in inches
            
            return Math.max(bufferedHeight, 1.0); // Minimum 1 inch for degenerate cases
        })();
    "#;

    chrome_session.check_timeout(timeout)?;
    let content_height_inches = tab
        .evaluate(content_height_script, true)
        .map_err(|e| pdf_errors::content_measurement_error(e.to_string()))?
        .value
        .unwrap_or_default()
        .as_f64()
        .unwrap_or(11.0);

    // Calculate adjusted page dimensions based on scale factor
    // When scaling up content, we need to increase page size proportionally
    // to ensure the scaled content fits within the page boundaries
    let base_width = 8.5;
    let adjusted_width = base_width * scale;
    let adjusted_height = content_height_inches * scale;

    chrome_session.check_timeout(timeout)?;
    let pdf_data = tab
        .print_to_pdf(Some(headless_chrome::types::PrintToPdfOptions {
            landscape: Some(false),
            display_header_footer: Some(false),
            print_background: Some(true),
            scale: Some(scale), // Use the passed scale parameter
            paper_width: Some(adjusted_width),
            paper_height: Some(adjusted_height),
            margin_top: Some(0.1),    // Hardcoded sensible default
            margin_bottom: Some(0.1), // Hardcoded sensible default
            margin_left: Some(0.1),   // Hardcoded sensible default
            margin_right: Some(0.1),  // Hardcoded sensible default
            page_ranges: None,
            ignore_invalid_page_ranges: Some(false),
            header_template: None,
            footer_template: None,
            prefer_css_page_size: Some(false),
            transfer_mode: None,
            generate_document_outline: Some(false),
            generate_tagged_pdf: Some(false),
        }))
        .map_err(|e| {
            pdf_errors::pdf_generation_error(e.to_string(), scale, adjusted_width, adjusted_height)
        })?;

    // Write PDF to file
    chrome_session.check_timeout(timeout)?;
    fs::write(output_path, pdf_data)
        .map_err(|e| pdf_errors::file_write_error(e.to_string(), output_path))?;

    // The ScopedLogLevel guard (_log_guard) automatically restores the original
    // log level when it goes out of scope here, ensuring no permanent global state changes.
    // Thread safety is guaranteed by the mutex synchronization in ScopedLogLevel.
    // The ChromeSession automatically cleans up Chrome processes via Drop trait.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pdf_errors::chrome_launch_error;
    use crate::GooseError;

    fn detail(inner_error: &str) -> String {
        match chrome_launch_error(inner_error.to_string()) {
            GooseError::InvalidOption { detail, .. } => detail,
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn launch_error_names_chrome_variable_and_promises_no_download() {
        let detail = detail("Could not auto detect a chrome executable");
        assert!(detail.contains("CHROME"), "{}", detail);
        assert!(!detail.to_lowercase().contains("download"), "{}", detail);
        assert!(!detail.contains("architecture"), "{}", detail);
    }

    #[test]
    fn launch_error_reports_architecture_mismatch() {
        let detail = detail("Exec format error (os error 8)");
        assert!(detail.contains("architecture"), "{}", detail);
        assert!(detail.contains(std::env::consts::ARCH), "{}", detail);
        assert!(detail.contains("CHROME"), "{}", detail);
        assert!(!detail.to_lowercase().contains("download"), "{}", detail);
    }
}
