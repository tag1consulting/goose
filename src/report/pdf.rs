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
use std::{fs, path::Path};

/// Generate a PDF report from HTML content using headless Chrome
#[cfg(feature = "pdf-reports")]
pub(crate) fn generate_pdf_from_html(
    html_content: &str,
    output_path: &Path,
    pdf_options: &PdfOptions,
) -> Result<(), GooseError> {
    // Launch headless Chrome
    let launch_options = LaunchOptions::default_builder()
        .headless(true)
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

    // Configure PDF options
    let (width, height) = match pdf_options.page_size {
        PdfPageSize::A4 => (8.27, 11.7),    // A4 in inches
        PdfPageSize::Letter => (8.5, 11.0), // US Letter in inches
        PdfPageSize::Legal => (8.5, 14.0),  // US Legal in inches
        PdfPageSize::A3 => (11.7, 16.5),    // A3 in inches
        PdfPageSize::Custom { width, height } => (width, height), // Custom size in inches
    };

    // Generate PDF
    let pdf_data = tab
        .print_to_pdf(Some(headless_chrome::types::PrintToPdfOptions {
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

    Ok(())
}

/// Add print-optimized CSS styles to HTML content for better PDF output
#[cfg(feature = "pdf-reports")]
pub(crate) fn add_print_css(html_content: &str) -> String {
    let print_css = r#"
        <style media="print">
            @page {
                margin: 0.5in;
                size: auto;
            }
            
            body {
                font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                font-size: 12px;
                line-height: 1.4;
                color: #333;
                background: white !important;
            }
            
            .goose-logo, .goose-header img {
                max-width: 200px !important;
                height: auto !important;
            }
            
            /* Enhanced table page break controls */
            table {
                page-break-inside: avoid;
                break-inside: avoid;
                border-collapse: collapse;
                width: 100% !important;
                font-size: 11px;
                margin-bottom: 20px;
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
            
            th, td {
                border: 1px solid #ddd !important;
                padding: 4px 8px !important;
                text-align: left;
            }
            
            th {
                background-color: #f5f5f5 !important;
                font-weight: bold;
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
                color: #333 !important;
            }
            
            /* Keep headings with at least some following content */
            h2 + div, h2 + table {
                page-break-before: avoid;
                break-before: avoid;
            }
            
            h1 {
                font-size: 18px;
                margin: 20px 0 10px 0;
            }
            
            h2 {
                font-size: 16px;
                margin: 15px 0 8px 0;
                border-bottom: 1px solid #ddd;
                padding-bottom: 3px;
            }
            
            h3 {
                font-size: 14px;
                margin: 12px 0 6px 0;
            }
            
            /* Charts and graphs */
            .chart, .graph {
                page-break-inside: avoid;
                break-inside: avoid;
                margin: 15px 0;
                max-width: 100% !important;
                height: auto !important;
            }
            
            .goose-graph {
                max-width: 100% !important;
                height: auto !important;
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
            
            /* Ensure charts and graphs don't break */
            canvas, svg {
                max-width: 100% !important;
                height: auto !important;
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
            
            /* Hide interactive elements that don't make sense in PDF */
            button, input[type="button"], input[type="submit"] {
                display: none !important;
            }
            
            /* Improve readability of small text */
            .small-text, .footnote {
                font-size: 10px;
                color: #666;
            }
            
            /* Prevent very small orphaned content */
            p, div {
                orphans: 2;
                widows: 2;
            }
        </style>
    "#;

    // Insert the print CSS before the closing </head> tag
    if let Some(head_end) = html_content.find("</head>") {
        let mut result = String::with_capacity(html_content.len() + print_css.len());
        result.push_str(&html_content[..head_end]);
        result.push_str(print_css);
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
        assert!(result.starts_with("<style media=\"print\">"));
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
