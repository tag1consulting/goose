//! Print-optimized HTML generation functionality
//!
//! This module provides print-ready HTML generation by adding CSS styles optimized
//! for PDF conversion and printing. These functions are always available regardless
//! of feature flags and work with any external PDF generation tool.

/// Generate print-optimized HTML content by adding CSS styles that match Chrome's internal PDF generation
///
/// This function is always available regardless of feature flags and can be used for both
/// PDF generation and standalone HTML+CSS output (e.g., --pdf-print-html).
pub(crate) fn generate_print_optimized_html_content(html_content: &str) -> String {
    add_print_css(html_content)
}

/// Add print-optimized CSS styles to HTML content that matches Chrome's internal PDF generation
pub(crate) fn add_print_css(html_content: &str) -> String {
    let print_css = r#"
        <style media="print">
            @page {
                margin: 0.1in;
                size: 8.5in auto;
            }
            
            /* Match Chrome's internal PDF generation behavior */
            html, body {
                font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                font-size: 12px;
                line-height: 1.4;
                color: #333;
                background: white !important;
                margin: 0;
                padding: 0;
                height: auto;
                overflow: visible;
            }
            
            /* Prevent page breaks in critical elements but allow natural flow */
            h1, h2, h3, h4, h5, h6 {
                page-break-after: avoid;
                break-after: avoid;
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            /* Table handling - prevent breaking table headers but allow content to flow */
            table {
                border-collapse: collapse;
                width: 100% !important;
                font-size: 11px;
                margin-bottom: 20px;
            }
            
            thead {
                display: table-header-group;
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            tbody {
                display: table-row-group;
            }
            
            tr {
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            th, td {
                border: 1px solid #ddd !important;
                padding: 4px 8px !important;
                text-align: left;
                page-break-inside: avoid;
                break-inside: avoid;
            }
            
            th {
                background-color: #f5f5f5 !important;
                font-weight: bold;
            }
            
            /* Allow natural page breaks for content sections */
            .metrics-section, .overview-section {
                page-break-inside: auto;
                break-inside: auto;
            }
            
            /* Ensure charts and images scale properly */
            img, canvas, svg {
                max-width: 100% !important;
                height: auto !important;
                page-break-inside: avoid;
                break-inside: avoid;
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
}
