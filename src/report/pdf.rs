//! PDF report generation functionality
//!
//! This module provides PDF report generation by converting existing HTML reports
//! to PDF format using headless Chrome. It leverages the HTML report generation
//! from the print module and converts it to PDF with configurable options.
//!
//! **Note**: This entire module is only available when compiled with the `pdf-reports` feature flag.

use crate::logger::ScopedLogLevel;
use crate::report::chrome::ResolvedChrome;
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
    use crate::report::chrome::ResolvedChrome;
    use crate::GooseError;
    use std::time::Duration;

    /// Creates a consistent error for Chrome launch failures
    pub fn chrome_launch_error(inner_error: String, chrome: &ResolvedChrome) -> GooseError {
        GooseError::InvalidOption {
            option: "--report-file".to_string(),
            value: format!("Chrome launch failed: {inner_error}"),
            detail: super::launch_diagnosis::diagnose(&inner_error, chrome),
        }
    }

    /// Creates a consistent error for Chrome configuration failures
    pub fn chrome_config_error(inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--report-file".to_string(),
            value: format!("Chrome configuration failed: {inner_error}"),
            detail: "Goose could not build the options to launch Chrome with. This is a bug in Goose; please report it at https://github.com/tag1consulting/goose/issues with this error.".to_string(),
        }
    }

    /// Creates a consistent error for Chrome operation failures
    pub fn chrome_operation_error(operation: &str, inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--report-file".to_string(),
            value: format!("{operation} failed: {inner_error}"),
            detail: format!("Chrome {operation} operation failed. This may indicate Chrome instability or insufficient system resources."),
        }
    }

    /// Creates a consistent error for timeout scenarios
    pub fn timeout_error(duration: Duration) -> GooseError {
        GooseError::InvalidOption {
            option: "--pdf-timeout".to_string(),
            value: "timeout".to_string(),
            detail: format!("Chrome operation timed out after {duration:?}. This may indicate Chrome is unresponsive or the system is under heavy load."),
        }
    }

    /// Creates a consistent error for file write failures
    pub fn file_write_error(inner_error: String, output_path: &std::path::Path) -> GooseError {
        GooseError::InvalidOption {
            option: "--report-file".to_string(),
            value: format!("Failed to write PDF file: {inner_error}"),
            detail: format!("Unable to write PDF to {}", output_path.display()),
        }
    }

    /// Creates a consistent error for content measurement failures
    pub fn content_measurement_error(inner_error: String) -> GooseError {
        GooseError::InvalidOption {
            option: "--report-file".to_string(),
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
            option: "--report-file".to_string(),
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
    /// The launch is bounded by `timeout`. `headless_chrome` waits 30 seconds
    /// for Chrome to report its DevTools address, and on a port it picked
    /// itself retries ten more times, so a Chrome that exits at once (a
    /// missing shared library) would otherwise take five minutes to fail.
    /// Goose picks the port, so a failure is not retried, except when the
    /// port turned out to be taken.
    ///
    /// # Returns
    /// * `Result<ChromeSession, GooseError>` - The Chrome session or an error
    fn new(
        chrome: &ResolvedChrome,
        args: Vec<&'static OsStr>,
        timeout: Duration,
    ) -> Result<Self, GooseError> {
        let start_time = Instant::now();

        debug!(
            "Launching {} ({}) for PDF generation",
            chrome.path.display(),
            chrome.source.describe()
        );
        let mut attempts = 0;
        let browser = loop {
            attempts += 1;
            let launch_options = LaunchOptions::default_builder()
                .path(Some(chrome.path.clone()))
                .headless(true)
                .args(args.clone())
                .port(free_port())
                .build()
                .map_err(|e| pdf_errors::chrome_config_error(e.to_string()))?;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(Browser::new(launch_options));
            });
            let remaining = timeout.saturating_sub(start_time.elapsed());
            match rx.recv_timeout(remaining) {
                Ok(Ok(browser)) => break browser,
                Ok(Err(e)) if attempts < 3 && e.to_string().contains("already in use") => continue,
                Ok(Err(e)) => return Err(pdf_errors::chrome_launch_error(e.to_string(), chrome)),
                // The launch thread ends on its own, and kills Chrome if it started.
                Err(_) => {
                    return Err(pdf_errors::chrome_launch_error(
                        format!(
                            "Chrome did not report its DevTools address within {} seconds",
                            timeout.as_secs()
                        ),
                        chrome,
                    ))
                }
            }
        };

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

/// A free local port for Chrome's DevTools, or `None` to let
/// `headless_chrome` pick one.
fn free_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .ok()
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
    chrome: &ResolvedChrome,
) -> Result<(), GooseError> {
    let rendered = render_pdf(html_content, scale, verbose, timeout_seconds, chrome)?;
    fs::write(output_path, rendered.pdf)
        .map_err(|e| pdf_errors::file_write_error(e.to_string(), output_path))
}

/// Confirms `chrome` can render a PDF, the way [`generate_pdf_from_html`]
/// does, and returns Chrome's version when it reports one.
pub(crate) fn preflight_pdf(
    scale: f64,
    verbose: bool,
    timeout_seconds: u64,
    chrome: &ResolvedChrome,
) -> Result<Option<String>, GooseError> {
    render_pdf(
        "<!DOCTYPE html><html><body><p>Goose PDF report check</p></body></html>",
        scale,
        verbose,
        timeout_seconds,
        chrome,
    )
    .map(|rendered| rendered.version)
}

/// The HTML Chrome prints, in a file only this user can read, removed when
/// dropped.
struct PageFile {
    path: std::path::PathBuf,
    url: url::Url,
}

impl PageFile {
    fn write(html: &str) -> std::io::Result<Self> {
        use std::io::Write;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path =
            std::env::temp_dir().join(format!("goose-report-{}-{nanos}.html", std::process::id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let url = url::Url::from_file_path(&path).map_err(|()| {
            std::io::Error::other(format!("{} is not an absolute path", path.display()))
        })?;
        let mut file = options.open(&path)?;
        // Created: from here on, dropping the page removes it.
        let page = PageFile { path, url };
        file.write_all(html.as_bytes())?;
        Ok(page)
    }
}

impl Drop for PageFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// A PDF rendered in memory.
struct Rendered {
    pdf: Vec<u8>,
    /// Chrome's version, such as `141.0.7390.122`.
    version: Option<String>,
}

/// Launches Chrome and prints `html_content` to a PDF in memory.
fn render_pdf(
    html_content: &str,
    scale: f64,
    verbose: bool,
    timeout_seconds: u64,
    chrome: &ResolvedChrome,
) -> Result<Rendered, GooseError> {
    // Get appropriate Chrome arguments based on verbosity
    let chrome_args = get_chrome_launch_args(verbose);

    // Create a thread-safe scoped logging guard that temporarily adjusts log level for headless Chrome operations
    // This suppresses Rust logging output during PDF generation while maintaining thread safety
    let _log_guard = if !verbose {
        Some(ScopedLogLevel::new(LevelFilter::Error)?)
    } else {
        None
    };

    // Set timeout for Chrome operations (configurable)
    let timeout = Duration::from_secs(timeout_seconds);

    // Launch Chrome with appropriate arguments and automatic resource management
    let chrome_session = ChromeSession::new(chrome, chrome_args, timeout)?;

    // The product is reported as `HeadlessChrome/141.0.7390.122`.
    let version = chrome_session.browser().get_version().ok().map(|version| {
        match version.product.split_once('/') {
            Some((_, number)) => number.to_string(),
            None => version.product,
        }
    });

    // Create a new tab
    chrome_session.check_timeout(timeout)?;
    let tab = chrome_session
        .browser()
        .new_tab()
        .map_err(|e| pdf_errors::chrome_operation_error("tab creation", e.to_string()))?;

    // Load the HTML from a file: Chrome refuses a data URL of more than
    // about 2 MB, which a report with many request names passes.
    let page = PageFile::write(html_content)
        .map_err(|e| pdf_errors::chrome_operation_error("HTML loading", e.to_string()))?;
    chrome_session.check_timeout(timeout)?;
    tab.navigate_to(page.url.as_str())
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

    chrome_session.check_timeout(timeout)?;

    // The ScopedLogLevel guard (_log_guard) automatically restores the original
    // log level when it goes out of scope here, ensuring no permanent global state changes.
    // Thread safety is guaranteed by the mutex synchronization in ScopedLogLevel.
    // The ChromeSession automatically cleans up Chrome processes via Drop trait.
    Ok(Rendered {
        pdf: pdf_data,
        version,
    })
}

/// Explains why Chrome did not start. Each step that can't run is skipped,
/// and no step can hang: every command it runs has a short timeout.
mod launch_diagnosis {
    use crate::report::chrome::ResolvedChrome;
    use std::io::Read;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const HOW_TO: &str = "Install Chrome or Chromium, or set the CHROME environment variable to the full path of a working Chrome or Chromium binary.";

    /// How the launch failed, from the error text.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Failure {
        /// The binary is for another architecture (ENOEXEC).
        ExecFormat,
        /// ENOENT: the binary, or on Linux its ELF interpreter, is missing.
        NotFound,
        /// EACCES: the binary is not executable.
        PermissionDenied,
        Other,
    }

    pub(super) fn classify(error: &str) -> Failure {
        // `Command::spawn` errors end with the OS error number.
        if error.contains("Exec format error") || error.contains("os error 8)") {
            Failure::ExecFormat
        } else if error.contains("No such file or directory") || error.contains("os error 2)") {
            Failure::NotFound
        } else if error.contains("Permission denied") || error.contains("os error 13)") {
            Failure::PermissionDenied
        } else {
            Failure::Other
        }
    }

    /// The libraries `ldd` reports missing. glibc's `ldd` prints
    /// `libnss3.so => not found`; musl's prints
    /// `Error loading shared library libnss3.so: No such file or directory`.
    pub(super) fn missing_libraries(ldd_output: &str) -> Vec<String> {
        let mut missing: Vec<String> = Vec::new();
        for line in ldd_output.lines().map(str::trim) {
            let library = if let Some((library, rest)) = line.split_once("=>") {
                (rest.trim() == "not found").then(|| library.trim())
            } else if let Some(rest) = line.strip_prefix("Error loading shared library ") {
                rest.split(':').next().map(str::trim)
            } else {
                None
            };
            if let Some(library) = library.filter(|library| !library.is_empty()) {
                if !missing.iter().any(|seen| seen == library) {
                    missing.push(library.to_string());
                }
            }
        }
        missing
    }

    /// What Goose learned about the failed launch.
    pub(super) struct Findings<'a> {
        pub(super) failure: Failure,
        pub(super) exists: bool,
        pub(super) linux: bool,
        pub(super) missing: &'a [String],
        pub(super) version_stderr: Option<&'a str>,
    }

    /// Writes the explanation from what was learned.
    pub(super) fn explain(path: &Path, source: &str, findings: &Findings) -> String {
        let mut text = format!(
            "Goose could not start the Chrome binary {} ({source}).",
            path.display()
        );
        match findings.failure {
            Failure::ExecFormat => text.push_str(&format!(
                " It does not match this machine's architecture ({}).",
                std::env::consts::ARCH
            )),
            Failure::NotFound if !findings.exists => text.push_str(" The file does not exist."),
            Failure::NotFound if findings.linux => text.push_str(
                " The file exists, so the ELF interpreter it was linked against is missing: usually a glibc binary on a musl system such as Alpine, or the reverse.",
            ),
            Failure::PermissionDenied => {
                text.push_str(" The file is not executable by this user.")
            }
            _ => {}
        }
        if !findings.missing.is_empty() {
            text.push_str(&format!(
                " It needs shared libraries that are not installed: {}. Installing your distribution's Chromium package pulls them in.",
                findings.missing.join(", ")
            ));
        }
        if let Some(stderr) = findings.version_stderr.filter(|s| !s.trim().is_empty()) {
            text.push_str(&format!(
                " Running it with --version printed: {}",
                stderr.trim()
            ));
        }
        text.push(' ');
        text.push_str(HOW_TO);
        text
    }

    /// Diagnoses a failed launch of `chrome`.
    pub(super) fn diagnose(error: &str, chrome: &ResolvedChrome) -> String {
        let failure = classify(error);
        let exists = chrome.path.is_file();
        let linux = cfg!(target_os = "linux");
        // Nothing more to learn from a binary that is missing or for another architecture.
        let probe = exists && failure != Failure::ExecFormat;

        let mut missing = Vec::new();
        if probe && linux {
            if let Some((stdout, stderr)) = run(
                Command::new("ldd").arg(&chrome.path),
                Duration::from_secs(5),
            ) {
                missing = missing_libraries(&format!("{stdout}\n{stderr}"));
            }
        }
        let interpreter_missing = linux && exists && failure == Failure::NotFound;
        let version_stderr = if probe && missing.is_empty() && !interpreter_missing {
            run(
                Command::new(&chrome.path).arg("--version"),
                Duration::from_secs(5),
            )
            .map(|(_, stderr)| stderr.lines().take(5).collect::<Vec<_>>().join(" / "))
        } else {
            None
        };

        explain(
            &chrome.path,
            chrome.source.describe(),
            &Findings {
                failure,
                exists,
                linux,
                missing: &missing,
                version_stderr: version_stderr.as_deref(),
            },
        )
    }

    /// Runs a command and returns its output, or `None` if it can't start or
    /// doesn't finish within `timeout`.
    fn run(command: &mut Command, timeout: Duration) -> Option<(String, String)> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut stdout);
        }
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        Some((stdout, stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::launch_diagnosis::{classify, explain, missing_libraries, Failure, Findings};
    use std::path::Path;

    fn findings(failure: Failure) -> Findings<'static> {
        Findings {
            failure,
            exists: true,
            linux: false,
            missing: &[],
            version_stderr: None,
        }
    }

    #[test]
    fn classifies_launch_errors() {
        assert_eq!(
            classify("Exec format error (os error 8)"),
            Failure::ExecFormat
        );
        assert_eq!(
            classify("No such file or directory (os error 2)"),
            Failure::NotFound
        );
        assert_eq!(
            classify("Permission denied (os error 13)"),
            Failure::PermissionDenied
        );
        assert_eq!(
            classify("Could not auto detect a chrome executable"),
            Failure::Other
        );
        // os error 28 is not os error 2.
        assert_eq!(
            classify("No space left on device (os error 28)"),
            Failure::Other
        );
    }

    #[test]
    fn launch_error_names_chrome_variable_and_binary() {
        let detail = explain(
            Path::new("/usr/bin/chromium"),
            "installed",
            &findings(Failure::Other),
        );
        assert!(detail.contains("CHROME"), "{}", detail);
        assert!(
            detail.contains("/usr/bin/chromium (installed)"),
            "{}",
            detail
        );
        assert!(!detail.to_lowercase().contains("download"), "{}", detail);
        assert!(!detail.contains("architecture"), "{}", detail);
    }

    #[test]
    fn launch_error_reports_architecture_mismatch() {
        let detail = explain(
            Path::new("/usr/bin/chromium"),
            "installed",
            &findings(Failure::ExecFormat),
        );
        assert!(detail.contains("architecture"), "{}", detail);
        assert!(detail.contains(std::env::consts::ARCH), "{}", detail);
        assert!(detail.contains("CHROME"), "{}", detail);
        assert!(!detail.to_lowercase().contains("download"), "{}", detail);
    }

    #[test]
    fn launch_error_reports_missing_interpreter_on_linux() {
        let mut found = findings(Failure::NotFound);
        found.linux = true;
        let detail = explain(Path::new("/opt/chrome"), "set by CHROME", &found);
        assert!(detail.contains("ELF interpreter"), "{}", detail);
        assert!(detail.contains("musl"), "{}", detail);

        // A missing file is just missing.
        found.exists = false;
        let detail = explain(Path::new("/opt/chrome"), "set by CHROME", &found);
        assert!(detail.contains("does not exist"), "{}", detail);
        assert!(!detail.contains("ELF"), "{}", detail);

        // Off Linux there is no ELF interpreter to blame.
        found.exists = true;
        found.linux = false;
        let detail = explain(Path::new("/opt/chrome"), "set by CHROME", &found);
        assert!(!detail.contains("ELF"), "{}", detail);
    }

    #[test]
    fn launch_error_lists_missing_libraries_and_stderr() {
        let missing = vec!["libnss3.so".to_string(), "libatk-1.0.so.0".to_string()];
        let mut found = findings(Failure::Other);
        found.linux = true;
        found.missing = &missing;
        let detail = explain(Path::new("/opt/chrome"), "installed", &found);
        assert!(detail.contains("libnss3.so, libatk-1.0.so.0"), "{}", detail);
        assert!(detail.contains("Chromium package"), "{}", detail);

        let mut found = findings(Failure::Other);
        found.version_stderr = Some("  cannot open display  ");
        let detail = explain(Path::new("/opt/chrome"), "installed", &found);
        assert!(
            detail.contains("--version printed: cannot open display"),
            "{}",
            detail
        );

        // Empty stderr adds nothing.
        found.version_stderr = Some("  ");
        let detail = explain(Path::new("/opt/chrome"), "installed", &found);
        assert!(!detail.contains("--version"), "{}", detail);
    }

    #[test]
    fn parses_missing_libraries_from_ldd() {
        let glibc = "\
\tlinux-vdso.so.1 (0x0000ffff9a5e0000)
\tlibdl.so.2 => /lib/aarch64-linux-gnu/libdl.so.2 (0x0000ffff9a590000)
\tlibnss3.so => not found
\tlibnssutil3.so => not found
\tlibnss3.so => not found
\tlibc.so.6 => /lib/aarch64-linux-gnu/libc.so.6 (0x0000ffff9a3e0000)
";
        assert_eq!(
            missing_libraries(glibc),
            vec!["libnss3.so", "libnssutil3.so"]
        );

        let musl = "\
\t/lib/ld-musl-x86_64.so.1 (0x7f0e1c4a6000)
Error loading shared library libglib-2.0.so.0: No such file or directory (needed by /opt/chrome)
Error loading shared library libnss3.so: No such file or directory (needed by /opt/chrome)
";
        assert_eq!(
            missing_libraries(musl),
            vec!["libglib-2.0.so.0", "libnss3.so"]
        );

        assert!(missing_libraries("\tnot a dynamic executable\n").is_empty());
        assert!(missing_libraries("").is_empty());
    }
}
