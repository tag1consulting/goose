//! PDF report generation functionality
//!
//! This module provides PDF report generation by converting existing HTML reports
//! to PDF format using headless Chrome. It leverages the HTML report generation
//! and converts it to PDF with configurable options.

#[cfg(feature = "pdf-reports")]
use crate::{
    config::{PdfOptions, PdfPageSize},
    GooseError,
};

#[cfg(feature = "pdf-reports")]
use headless_chrome::{Browser, LaunchOptions};

#[cfg(feature = "pdf-reports")]
use std::{ffi::OsStr, fs, path::Path};

/// Generate a PDF report from HTML content using headless Chrome
#[cfg(feature = "pdf-reports")]
pub(crate) fn generate_pdf_from_html(
    html_content: &str,
    output_path: &Path,
    pdf_options: &PdfOptions,
) -> Result<(), GooseError> {
    // Store original log level to restore later
    let original_log_level = log::max_level();

    // Temporarily reduce log level just for Chrome operations
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
            option: "--pdf".to_string(),
            value: format!("Failed to configure Chrome: {e}"),
            detail: "Unable to launch headless Chrome for PDF generation".to_string(),
        })?;

    let browser = Browser::new(launch_options).map_err(|e| GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: format!("Failed to launch Chrome: {e}"),
        detail: "Unable to start headless Chrome browser".to_string(),
    })?;

    // Create a new tab
    let tab = browser.new_tab().map_err(|e| GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: format!("Failed to create browser tab: {e}"),
        detail: "Unable to create new browser tab".to_string(),
    })?;

    // Create a data URL from the HTML content
    let encoded_html = urlencoding::encode(html_content);
    let data_url = format!("data:text/html;charset=utf-8,{encoded_html}");

    // Navigate to the data URL
    tab.navigate_to(&data_url)
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to load HTML: {e}"),
            detail: "Unable to load HTML content in browser".to_string(),
        })?;

    // Wait for the page to load
    tab.wait_until_navigated()
        .map_err(|e| GooseError::InvalidOption {
            option: "--pdf".to_string(),
            value: format!("Failed to wait for page load: {e}"),
            detail: "Page failed to load completely".to_string(),
        })?;

    // Configure PDF options based on page size
    let pdf_data = match pdf_options.page_size {
        PdfPageSize::Unlimited => {
            // For unlimited pages, dynamically calculate content height and set appropriate dimensions

            // More accurate measurement using the actual content bounds
            let content_height_script = r#"
                (function() {
                    // Get all elements in the document
                    const elements = document.querySelectorAll('*');
                    let maxBottom = 0;
                    
                    // Find the lowest bottom position of any element
                    for (let element of elements) {
                        const rect = element.getBoundingClientRect();
                        const bottom = rect.bottom + window.scrollY;
                        if (bottom > maxBottom) {
                            maxBottom = bottom;
                        }
                    }
                    
                    // Convert pixels to inches (96 DPI) and add minimal buffer
                    return (maxBottom + 5) / 96; // 5px buffer to prevent clipping
                })();
            "#;

            let content_height_inches = tab
                .evaluate(content_height_script, true)
                .map_err(|e| GooseError::InvalidOption {
                    option: "--pdf".to_string(),
                    value: format!("Failed to measure content height: {e}"),
                    detail: "Unable to calculate content dimensions for unlimited PDF".to_string(),
                })?
                .value
                .unwrap_or_default()
                .as_f64()
                .unwrap_or(11.0); // Default to Letter height if measurement fails

            // Use the exact measurement with NO additional padding
            let pdf_height = content_height_inches; // Use actual content height

            tab.print_to_pdf(Some(headless_chrome::types::PrintToPdfOptions {
                landscape: Some(false),
                display_header_footer: Some(false),
                print_background: Some(true),
                scale: Some(pdf_options.scale),
                paper_width: Some(8.5),         // Standard width in inches
                paper_height: Some(pdf_height), // Dynamic height based on content
                margin_top: Some(0.1),          // Minimal margins to reduce white space
                margin_bottom: Some(0.1),       // Minimal margins to reduce white space
                margin_left: Some(0.1),         // Minimal margins to reduce white space
                margin_right: Some(0.1),        // Minimal margins to reduce white space
                page_ranges: None,
                ignore_invalid_page_ranges: Some(false),
                header_template: None,
                footer_template: None,
                prefer_css_page_size: Some(false), // Don't let CSS override our explicit size
                transfer_mode: None,
                generate_document_outline: Some(false),
                generate_tagged_pdf: Some(false),
            }))
        }
        _ => {
            // For standard page sizes, use explicit dimensions
            let (width, height) = match pdf_options.page_size {
                PdfPageSize::A4 => (8.27, 11.7),    // A4 in inches
                PdfPageSize::Letter => (8.5, 11.0), // US Letter in inches
                PdfPageSize::Legal => (8.5, 14.0),  // US Legal in inches
                PdfPageSize::A3 => (11.7, 16.5),    // A3 in inches
                PdfPageSize::Custom { width, height } => (width, height), // Custom size in inches
                PdfPageSize::Unlimited => unreachable!(), // Already handled above
            };

            tab.print_to_pdf(Some(headless_chrome::types::PrintToPdfOptions {
                landscape: Some(false),
                display_header_footer: Some(false),
                print_background: Some(true),
                scale: Some(pdf_options.scale),
                paper_width: Some(width),
                paper_height: Some(height),
                margin_top: Some(pdf_options.margin_top),
                margin_bottom: Some(pdf_options.margin_bottom),
                margin_left: Some(pdf_options.margin_left),
                margin_right: Some(pdf_options.margin_right),
                page_ranges: None,
                ignore_invalid_page_ranges: Some(false),
                header_template: None,
                footer_template: None,
                prefer_css_page_size: Some(false),
                transfer_mode: None,
                generate_document_outline: Some(false),
                generate_tagged_pdf: Some(false),
            }))
        }
    }
    .map_err(|e| GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: format!("Failed to generate PDF: {e}"),
        detail: "PDF generation failed".to_string(),
    })?;

    // Write PDF to file
    fs::write(output_path, pdf_data).map_err(|e| GooseError::InvalidOption {
        option: "--pdf".to_string(),
        value: format!("Failed to write PDF file: {e}"),
        detail: format!("Unable to write PDF to {}", output_path.display()),
    })?;

    // Restore original log level so important messages can be displayed
    log::set_max_level(original_log_level);

    Ok(())
}

/// Add print-optimized CSS styles to HTML content for better PDF output
#[cfg(feature = "pdf-reports")]
#[allow(dead_code)]
pub(crate) fn add_print_css(html_content: &str) -> String {
    add_print_css_with_page_size(html_content, &PdfPageSize::A4)
}

/// Add print-optimized CSS styles to HTML content for better PDF output with specific page size
#[cfg(feature = "pdf-reports")]
pub(crate) fn add_print_css_with_page_size(html_content: &str, page_size: &PdfPageSize) -> String {
    let page_css = match page_size {
        PdfPageSize::Unlimited => {
            r#"
            @page {
                margin: 0.1in;
                size: auto;
            }"#
        }
        _ => {
            r#"
            @page {
                margin: 0.1in;
                size: auto;
            }"#
        }
    };

    let page_break_styles = match page_size {
        PdfPageSize::Unlimited => {
            // For unlimited pages, disable ALL page breaking to create one continuous page
            r#"
            /* Force unlimited page layout - disable ALL page breaking */
            @page {
                margin: 0.1in;
                size: auto;
            }

            /* Disable ALL page breaking for all elements */
            * {
                page-break-inside: avoid !important;
                break-inside: avoid !important;
                page-break-before: auto !important;
                break-before: auto !important;
                page-break-after: auto !important;
                break-after: auto !important;
            }

            /* Force body to be one continuous page */
            body {
                height: auto !important;
                min-height: 100vh;
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }

            /* Prevent table breaking */
            table, tr, td, th {
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }

            /* Ensure content flows continuously */
            .content, main, article, div {
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }

            /* Specific Goose report sections */
            table, .requests, .responses, .transactions, .scenarios, .status_codes, .errors, .CO,
            .chart, .graph, .goose-graph, .charts-container .chart, .info, .users {
                page-break-inside: avoid !important;
                break-inside: avoid !important;
                page-break-before: auto !important;
                break-before: auto !important;
                page-break-after: auto !important;
                break-after: auto !important;
            }
            
            h1, h2, h3 {
                page-break-after: auto !important;
                break-after: auto !important;
                page-break-inside: avoid !important;
                break-inside: avoid !important;
            }
            
            thead {
                display: table-header-group;
            }
            "#
        }
        _ => {
            // For standard page sizes, use the original page break controls
            r#"
            /* Enhanced table page break controls */
            table {
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            /* Ensure table headers repeat on new pages if table must break */
            thead {
                display: table-header-group;
            }
            
            /* Prevent orphaned headers */
            thead tr {
                page-break-after: avoid;
                break-after: avoid;
            }
            
            /* Keep table sections together when possible */
            tbody {
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            /* Prevent single rows from being orphaned */
            tr {
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            /* For very large tables, allow breaking but optimize it */
            tbody tr {
                orphans: 3; /* Minimum 3 rows at bottom of page */
                widows: 3;  /* Minimum 3 rows at top of page */
            }
            
            /* Keep entire table containers together when possible */
            .requests, .responses, .transactions, .scenarios, .status_codes, .errors, .CO {
                page-break-inside: avoid;
                break-inside: avoid;
                margin-bottom: 30px;
            }
            
            /* Keep section headings with their content */
            h1, h2, h3 {
                page-break-after: avoid;
                break-after: avoid;
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            /* Keep headings with at least some following content */
            h2 + div, h2 + table {
                page-break-before: avoid;
                break-before: avoid;
            }
            
            /* Charts and graphs */
            .chart, .graph {
                page-break-inside: avoid;
                break-inside: avoid;
                margin: 15px 0;
            }
            
            .goose-graph {
                page-break-inside: avoid;
                break-inside: avoid;
                margin: 10px 0;
            }
            
            /* Charts container */
            .charts-container .chart {
                page-break-inside: avoid;
                break-inside: avoid;
                margin-bottom: 20px;
            }
            
            /* Info sections */
            .info {
                page-break-inside: avoid;
                break-inside: avoid;
                margin-bottom: 20px;
            }
            
            /* Plan overview table */
            .info table {
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            /* Users section */
            .users {
                page-break-inside: avoid;
                break-inside: avoid;
                margin-bottom: 30px;
            }
            "#
        }
    };

    let print_css = format!(
        r#"
        <style media="print">
            {page_css}
            
            body {{
                font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                font-size: 12px;
                line-height: 1.4;
                color: #333;
                background: white !important;
            }}
            
            .goose-logo, .goose-header img {{
                max-width: 200px !important;
                height: auto !important;
            }}
            
            {page_break_styles}
            
            table {{
                border-collapse: collapse;
                width: 100% !important;
                font-size: 11px;
                margin-bottom: 20px;
            }}
            
            th, td {{
                border: 1px solid #ddd !important;
                padding: 4px 8px !important;
                text-align: left;
            }}
            
            th {{
                background-color: #f5f5f5 !important;
                font-weight: bold;
            }}
            
            h1 {{
                font-size: 18px;
                margin: 20px 0 10px 0;
                /* Don't override header colors - preserve original colors */
            }}
            
            h2 {{
                font-size: 16px;
                margin: 15px 0 8px 0;
                border-bottom: 1px solid #ddd;
                padding-bottom: 3px;
                /* Don't override header colors - preserve original colors */
            }}
            
            h3 {{
                font-size: 14px;
                margin: 12px 0 6px 0;
                /* Don't override header colors - preserve original colors */
            }}
            
            /* Charts and graphs */
            .chart, .graph, .goose-graph {{
                max-width: 100% !important;
                height: auto !important;
                margin: 15px 0;
            }}
            
            /* Ensure charts and graphs don't break */
            canvas, svg {{
                max-width: 100% !important;
                height: auto !important;
            }}
            
            /* Hide interactive elements that don't make sense in PDF */
            button, input[type="button"], input[type="submit"] {{
                display: none !important;
            }}
            
            /* Improve readability of small text */
            .small-text, .footnote {{
                font-size: 10px;
                color: #666;
            }}
            
            /* Prevent very small orphaned content */
            p, div {{
                orphans: 2;
                widows: 2;
            }}
        </style>
    "#
    );

    // Insert the print CSS before the closing </head> tag
    if let Some(head_end) = html_content.find("</head>") {
        let mut result = String::with_capacity(html_content.len() + print_css.len());
        result.push_str(&html_content[..head_end]);
        result.push_str(&print_css);
        result.push_str(&html_content[head_end..]);
        result
    } else {
        // If no </head> tag found, just prepend the CSS
        format!("{print_css}\n{html_content}")
    }
}

#[cfg(not(feature = "pdf-reports"))]
pub(crate) fn generate_pdf_from_html(
    _html_content: &str,
    _output_path: &std::path::Path,
    _pdf_options: &crate::config::PdfOptions,
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
        use crate::config::{PdfOptions, PdfPageSize};
        use std::path::Path;

        let options = PdfOptions {
            page_size: PdfPageSize::A4,
            margin: 0.5,
            scale: 1.0,
            compress: true,
        };

        let result = generate_pdf_from_html("test", Path::new("test.pdf"), &options);
        assert!(result.is_err());

        if let Err(GooseError::InvalidOption { detail, .. }) = result {
            assert!(detail.contains("pdf-reports"));
        }
    }
}
