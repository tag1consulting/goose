//! PDF report generation functionality
//!
//! This module provides PDF report generation by converting print-ready HTML reports
//! to PDF format using either built-in headless Chrome or external tools.

use crate::GooseError;
use std::{fs, path::Path, process::Command};

#[cfg(feature = "pdf-reports")]
use std::ffi::OsStr;

#[cfg(feature = "pdf-reports")]
use headless_chrome::{Browser, LaunchOptions};

/// Default PDF generator command - built-in Chrome
const DEFAULT_PDF_GENERATOR: &str = "__builtin__";

/// Generate PDF using the specified generator
pub(crate) fn generate_pdf(
    html_content: &str,
    output_path: &Path,
    generator: Option<&str>,
    scale: Option<f64>,
) -> Result<(), GooseError> {
    let generator_cmd = generator.unwrap_or(DEFAULT_PDF_GENERATOR);

    if generator_cmd == DEFAULT_PDF_GENERATOR {
        #[cfg(feature = "pdf-reports")]
        return generate_builtin_pdf(html_content, output_path, scale);

        #[cfg(not(feature = "pdf-reports"))]
        return Err(GooseError::InvalidOption {
            option: "--report-file".to_string(),
            value: output_path.display().to_string(),
            detail: "PDF generation requires external tool or the 'pdf-reports' feature. Use --pdf-generator to specify a custom command.".to_string(),
        });
    } else {
        run_external_pdf_generator(generator_cmd, html_content, output_path)
    }
}

/// Generate PDF using built-in Chrome (when pdf-reports feature is enabled)
///
/// # Thread Safety Note
/// This function temporarily modifies the global log level to reduce Chrome's verbose output.
/// It is designed for single-threaded use at the end of load tests when generating the final
/// report. Since Goose generates only one report at test completion, concurrent PDF generation
/// is not a concern for the intended use case.
#[cfg(feature = "pdf-reports")]
fn generate_builtin_pdf(html_content: &str, output_path: &Path, scale: Option<f64>) -> Result<(), GooseError> {
    // Store original log level to restore later
    let original_log_level = log::max_level();

    // Temporarily reduce log level just for Chrome operations
    // NOTE: This modifies the global log level but is safe for our use case since
    // PDF generation only occurs once at the end of a load test
    log::set_max_level(log::LevelFilter::Error);

    // Launch headless Chrome
    let launch_options = LaunchOptions::default_builder()
        .headless(true)
        .args(vec![
            OsStr::new("--no-sandbox"),
            OsStr::new("--disable-dev-shm-usage"),
            OsStr::new("--disable-logging"),
            OsStr::new("--log-level=3"), // Only show fatal errors
            OsStr::new("--silent"),
        ])
        .build()
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf-generator".to_string(),
            value: format!("Failed to configure Chrome: {e}"),
            detail: "Unable to launch headless Chrome for PDF generation".to_string(),
        })?;

    let browser = Browser::new(launch_options).map_err(|e| GooseError::InvalidOption {
        option: "--pdf-generator".to_string(),
        value: format!("Failed to launch Chrome: {e}"),
        detail: "Unable to start headless Chrome browser".to_string(),
    })?;

    // Create a new tab
    let tab = browser.new_tab().map_err(|e| GooseError::InvalidOption {
        option: "--pdf-generator".to_string(),
        value: format!("Failed to create browser tab: {e}"),
        detail: "Unable to create new browser tab".to_string(),
    })?;

    // Create a data URL from the HTML content
    let encoded_html = urlencoding::encode(html_content);
    let data_url = format!("data:text/html;charset=utf-8,{encoded_html}");

    // Navigate to the data URL
    tab.navigate_to(&data_url)
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf-generator".to_string(),
            value: format!("Failed to load HTML: {e}"),
            detail: "Unable to load HTML content in browser".to_string(),
        })?;

    // Wait for the page to load
    tab.wait_until_navigated()
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf-generator".to_string(),
            value: format!("Failed to wait for page load: {e}"),
            detail: "Page failed to load completely".to_string(),
        })?;

    // Improved content height measurement with fallback for robustness
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

    let content_height_inches = tab
        .evaluate(content_height_script, true)
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf-generator".to_string(),
            value: format!("Failed to measure content height: {e}"),
            detail: "Unable to calculate content dimensions for PDF".to_string(),
        })?
        .value
        .unwrap_or_default()
        .as_f64()
        .unwrap_or(11.0);

    // Use provided scale or default to 0.8
    let scale = scale.unwrap_or(0.8);

    // Calculate adjusted page dimensions based on scale factor
    let base_width = 8.5;
    let adjusted_width = base_width * scale;
    let adjusted_height = content_height_inches * scale;

    let pdf_data = tab
        .print_to_pdf(Some(headless_chrome::types::PrintToPdfOptions {
            landscape: Some(false),
            display_header_footer: Some(false),
            print_background: Some(true),
            scale: Some(scale),
            paper_width: Some(adjusted_width),
            paper_height: Some(adjusted_height),
            margin_top: Some(0.1),
            margin_bottom: Some(0.1),
            margin_left: Some(0.1),
            margin_right: Some(0.1),
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
            option: "--pdf-generator".to_string(),
            value: format!("Failed to generate PDF: {e}"),
            detail: "PDF generation failed".to_string(),
        })?;

    // Write PDF to file
    fs::write(output_path, pdf_data).map_err(|e| GooseError::InvalidOption {
        option: "--pdf-generator".to_string(),
        value: format!("Failed to write PDF file: {e}"),
        detail: format!("Unable to write PDF to {}", output_path.display()),
    })?;

    // Restore original log level so important messages can be displayed
    log::set_max_level(original_log_level);

    Ok(())
}

/// Run external PDF generator with {input} and {output} placeholder substitution
fn run_external_pdf_generator(
    generator_cmd: &str,
    html_content: &str,
    output_path: &Path,
) -> Result<(), GooseError> {
    // Write HTML to temporary file
    let temp_dir = std::env::temp_dir();
    let temp_html_path = temp_dir.join(format!("goose-report-{}.html", std::process::id()));

    fs::write(&temp_html_path, html_content).map_err(|e| GooseError::InvalidOption {
        option: "--pdf-generator".to_string(),
        value: format!("Failed to write temporary HTML file: {e}"),
        detail: "Unable to create temporary HTML file for PDF generation".to_string(),
    })?;

    // Replace placeholders in the command
    let cmd_with_args = generator_cmd
        .replace("{input}", temp_html_path.to_string_lossy().as_ref())
        .replace("{output}", output_path.to_string_lossy().as_ref());

    // Parse command and arguments
    let mut cmd_parts = cmd_with_args.split_whitespace();
    let command = cmd_parts.next().ok_or_else(|| GooseError::InvalidOption {
        option: "--pdf-generator".to_string(),
        value: generator_cmd.to_string(),
        detail: "PDF generator command cannot be empty".to_string(),
    })?;

    let args: Vec<&str> = cmd_parts.collect();

    // Execute the command
    let output =
        Command::new(command)
            .args(&args)
            .output()
            .map_err(|e| GooseError::InvalidOption {
                option: "--pdf-generator".to_string(),
                value: format!("Failed to execute command '{}': {}", command, e),
                detail: "Make sure the PDF generator is installed and in your PATH".to_string(),
            })?;

    // Clean up temporary file
    let _ = fs::remove_file(&temp_html_path);

    // Check if command succeeded
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GooseError::InvalidOption {
            option: "--pdf-generator".to_string(),
            value: format!("Command failed with status {}: {}", output.status, stderr),
            detail: format!("PDF generator command: {}", cmd_with_args),
        });
    }

    Ok(())
}

/// Add print-optimized CSS styles to HTML content for better PDF output
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

    #[test]
    fn test_external_command_placeholder_replacement() {
        let generator_cmd = "wkhtmltopdf --scale 0.8 {input} {output}";
        let input_path = std::path::PathBuf::from("/tmp/test.html");
        let output_path = std::path::PathBuf::from("/tmp/test.pdf");

        let cmd_with_args = generator_cmd
            .replace("{input}", input_path.to_string_lossy().as_ref())
            .replace("{output}", output_path.to_string_lossy().as_ref());

        assert_eq!(
            cmd_with_args,
            "wkhtmltopdf --scale 0.8 /tmp/test.html /tmp/test.pdf"
        );
    }

    #[cfg(not(feature = "pdf-reports"))]
    #[test]
    fn test_generate_pdf_without_feature() {
        use std::path::Path;

        let result = generate_pdf("test", Path::new("test.pdf"), None, None);
        assert!(result.is_err());

        if let Err(GooseError::InvalidOption { detail, .. }) = result {
            assert!(detail.contains("external tool or the 'pdf-reports' feature"));
        }
    }
}
