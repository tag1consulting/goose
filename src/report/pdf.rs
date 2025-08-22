//! PDF report generation functionality
//!
//! This module provides PDF report generation by converting existing HTML reports
//! to PDF format using headless Chrome. It leverages the HTML report generation
//! and converts it to PDF with configurable options.

#[cfg(feature = "pdf-reports")]
use crate::GooseError;

#[cfg(feature = "pdf-reports")]
use headless_chrome::{Browser, LaunchOptions};

#[cfg(feature = "pdf-reports")]
use std::{
    ffi::OsStr,
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[cfg(feature = "pdf-reports")]
use crate::logger::ScopedLogLevel;

#[cfg(feature = "pdf-reports")]
use log::{debug, LevelFilter};

/// Manages Chrome browser process lifecycle with automatic cleanup.
///
/// This struct provides RAII-based resource management for Chrome processes,
/// ensuring proper cleanup even in error scenarios.
#[cfg(feature = "pdf-reports")]
struct ChromeSession {
    browser: Browser,
    process_id: Option<u32>,
    start_time: Instant,
}

#[cfg(feature = "pdf-reports")]
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
        let browser = Browser::new(launch_options).map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Chrome launch failed: {e}"),
            detail: "PDF generation requires Chrome/Chromium to be installed and accessible. The headless_chrome crate will attempt to download Chrome automatically on first use.".to_string(),
        })?;

        // Attempt to get the process ID for tracking
        let process_id = browser.get_process_id();

        if let Some(pid) = process_id {
            debug!("Chrome browser launched with process ID: {}", pid);
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
            return Err(GooseError::InvalidOption {
                option: "--pdf".to_string(),
                value: "timeout".to_string(),
                detail: format!(
                    "Chrome operation timed out after {:?}. This may indicate Chrome is unresponsive or the system is under heavy load.",
                    timeout
                ),
            });
        }
        Ok(())
    }
}

#[cfg(feature = "pdf-reports")]
impl Drop for ChromeSession {
    fn drop(&mut self) {
        let elapsed = self.start_time.elapsed();

        if let Some(pid) = self.process_id {
            debug!("Cleaning up Chrome process {} (ran for {:?})", pid, elapsed);
        } else {
            debug!("Cleaning up Chrome browser session (ran for {:?})", elapsed);
        }

        // The Browser's Drop trait handles the actual process termination
        // when self.browser is dropped
    }
}

/// Get Chrome launch arguments based on verbosity level
#[cfg(feature = "pdf-reports")]
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
#[cfg(feature = "pdf-reports")]
pub(crate) fn generate_pdf_from_html(
    html_content: &str,
    output_path: &Path,
    scale: f64,
    verbose: bool,
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
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Chrome configuration failed: {e}"),
            detail: "PDF generation requires Chrome/Chromium. Try installing Chrome or running with --pdf-scale to adjust memory usage.".to_string(),
        })?;

    // Create Chrome session with automatic resource management
    let chrome_session = ChromeSession::new(launch_options)?;

    // Set timeout for Chrome operations (60 seconds)
    let timeout = Duration::from_secs(60);

    // Create a new tab
    chrome_session.check_timeout(timeout)?;
    let tab = chrome_session.browser().new_tab().map_err(|e| GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: format!("Browser tab creation failed: {e}"),
        detail: "Chrome started successfully but failed to create a new tab. This may indicate insufficient memory or Chrome instability.".to_string(),
    })?;

    // Create a data URL from the HTML content
    let encoded_html = urlencoding::encode(html_content);
    let data_url = format!("data:text/html;charset=utf-8,{encoded_html}");

    // Navigate to the data URL
    chrome_session.check_timeout(timeout)?;
    tab.navigate_to(&data_url)
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to load HTML: {e}"),
            detail: "Unable to load HTML content in browser".to_string(),
        })?;

    // Wait for the page to load
    chrome_session.check_timeout(timeout)?;
    tab.wait_until_navigated()
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to wait for page load: {e}"),
            detail: "Page failed to load completely".to_string(),
        })?;

    // Always use unlimited page approach with hardcoded sensible defaults
    let content_height_script = r#"
        (function() {
            const elements = document.querySelectorAll('*');
            let maxBottom = 0;
            
            for (let element of elements) {
                const rect = element.getBoundingClientRect();
                const bottom = rect.bottom + window.scrollY;
                if (bottom > maxBottom) {
                    maxBottom = bottom;
                }
            }
            
            // Use document.documentElement.scrollHeight as fallback for robustness
            // This ensures we capture content that might not be in the normal document flow
            const scrollHeight = document.documentElement.scrollHeight / 96; // Convert to inches
            const calculatedHeight = (maxBottom + 5) / 96; // 5px buffer, convert to inches
            
            return Math.max(calculatedHeight, scrollHeight);
        })();
    "#;

    chrome_session.check_timeout(timeout)?;
    let content_height_inches = tab
        .evaluate(content_height_script, true)
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to measure content height: {e}"),
            detail: "Unable to calculate content dimensions for PDF".to_string(),
        })?
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
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("PDF generation failed: {e}"),
            detail: format!("Chrome's print-to-PDF operation failed. Scale: {}, Paper size: {:.1}\" x {:.1}\". Try adjusting --pdf-scale or reducing report complexity.", scale, adjusted_width, adjusted_height),
        })?;

    // Write PDF to file
    chrome_session.check_timeout(timeout)?;
    fs::write(output_path, pdf_data).map_err(|e| GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: format!("Failed to write PDF file: {e}"),
        detail: format!("Unable to write PDF to {}", output_path.display()),
    })?;

    // The ScopedLogLevel guard (_log_guard) automatically restores the original
    // log level when it goes out of scope here, ensuring no permanent global state changes.
    // Thread safety is guaranteed by the mutex synchronization in ScopedLogLevel.
    // The ChromeSession automatically cleans up Chrome processes via Drop trait.
    Ok(())
}

/// Add print-optimized CSS styles to HTML content for better PDF output
#[cfg(feature = "pdf-reports")]
pub(crate) fn add_print_css(html_content: &str) -> String {
    let print_css = r#"
        <style media="print">
            @page {
                margin: 0.1in;
                size: auto;
            }
            
            /* Force unlimited page layout - disable ALL page breaking */
            * {
                page-break-inside: avoid !important;
                break-inside: avoid !important;
                page-break-before: auto !important;
                break-before: auto !important;
                page-break-after: auto !important;
                break-after: auto !important;
            }

            body {
                font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                font-size: 12px;
                line-height: 1.4;
                color: #333;
                background: white !important;
                height: auto !important;
                min-height: 100vh;
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }
            
            /* Standard PDF styles for tables, headers, etc. */
            table {
                border-collapse: collapse;
                width: 100% !important;
                font-size: 11px;
                margin-bottom: 20px;
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }
            
            th, td {
                border: 1px solid #ddd !important;
                padding: 4px 8px !important;
                text-align: left;
            }
            
            th {
                background-color: #f5f5f5 !important;
                font-weight: bold;
            }
            
            h1, h2, h3 {
                page-break-after: auto !important;
                break-after: auto !important;
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }
        </style>
        "#
    .to_string();

    // Insert CSS before </head> or prepend if no head tag
    if let Some(head_end) = html_content.find("</head>") {
        let mut result = String::with_capacity(html_content.len() + print_css.len());
        result.push_str(&html_content[..head_end]);
        result.push_str(&print_css);
        result.push_str(&html_content[head_end..]);
        result
    } else {
        format!("{print_css}\n{html_content}")
    }
}

#[cfg(not(feature = "pdf-reports"))]
pub(crate) fn generate_pdf_from_html(
    _html_content: &str,
    _output_path: &std::path::Path,
    _scale: f64,
    _verbose: bool,
) -> Result<(), crate::GooseError> {
    Err(crate::GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: "disabled".to_string(),
        detail: "PDF reports require compiling with the 'pdf-reports' feature flag".to_string(),
    })
}

#[cfg(not(feature = "pdf-reports"))]
pub(crate) fn add_print_css(html_content: &str) -> String {
    html_content.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_print_css_with_head_tag() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Test</title>
</head>
<body>
    <h1>Test Content</h1>
</body>
</html>"#;

        let result = add_print_css(html);

        // Should contain the original content
        assert!(result.contains("Test Content"));
        assert!(result.contains("<title>Test</title>"));

        // Should contain the print CSS
        assert!(result.contains("@page"));
        assert!(result.contains("font-family"));
        assert!(result.contains("page-break-inside: avoid"));

        // CSS should be inserted before </head>
        let head_end = result.find("</head>").unwrap();
        let css_start = result.find("<style media=\"print\">").unwrap();
        assert!(css_start < head_end);
    }

    #[test]
    fn test_add_print_css_without_head_tag() {
        let html = "<h1>Simple HTML</h1>";
        let result = add_print_css(html);

        // Should contain original content
        assert!(result.contains("Simple HTML"));

        // Should contain the print CSS (prepended)
        assert!(result.contains("<style media=\"print\">"));
        assert!(result.trim_start().starts_with("<style media=\"print\">"));
    }

    #[cfg(not(feature = "pdf-reports"))]
    #[test]
    fn test_generate_pdf_without_feature() {
        use std::path::Path;

        let result = generate_pdf_from_html("test", Path::new("test.pdf"), 0.8, false);
        assert!(result.is_err());

        if let Err(crate::GooseError::InvalidOption { detail, .. }) = result {
            assert!(detail.contains("pdf-reports"));
        }
    }
}
