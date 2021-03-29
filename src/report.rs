//! Optionally writes an html-formatted summary report after running a load test.

use crate::metrics;

use std::collections::BTreeMap;
use std::mem;

use serde::Serialize;

/// The following templates are necessary to build an html-formatted summary report.
#[derive(Debug)]
pub struct GooseReportTemplates<'a> {
    pub requests_template: &'a str,
    pub responses_template: &'a str,
    pub tasks_template: &'a str,
    pub status_codes_template: &'a str,
    pub errors_template: &'a str,
}

/// Defines the metrics reported about requests.
#[derive(Debug, Clone, Serialize)]
pub struct RequestMetric {
    pub method: String,
    pub name: String,
    pub number_of_requests: usize,
    pub number_of_failures: usize,
    pub response_time_average: String,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub requests_per_second: String,
    pub failures_per_second: String,
}

/// Defines the metrics reported about responses.
#[derive(Debug, Clone, Serialize)]
pub struct ResponseMetric {
    pub method: String,
    pub name: String,
    pub percentile_50: String,
    pub percentile_60: String,
    pub percentile_70: String,
    pub percentile_80: String,
    pub percentile_90: String,
    pub percentile_95: String,
    pub percentile_99: String,
    pub percentile_100: String,
}

/// Defines the metrics reported about tasks.
#[derive(Debug, Clone, Serialize)]
pub struct TaskMetric {
    pub is_task_set: bool,
    pub task: String,
    pub name: String,
    pub number_of_requests: usize,
    pub number_of_failures: usize,
    pub response_time_average: String,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub requests_per_second: String,
    pub failures_per_second: String,
}

/// Defines the metrics reported about status codes.
pub struct StatusCodeMetric {
    pub method: String,
    pub name: String,
    pub status_codes: String,
}

/// Helper to generate a single response metric.
pub fn get_response_metric(
    method: &str,
    name: &str,
    response_times: &BTreeMap<usize, usize>,
    total_request_count: usize,
    response_time_minimum: usize,
    response_time_maximum: usize,
) -> ResponseMetric {
    // Calculate percentiles in a loop.
    let mut percentiles = Vec::new();
    for percent in &[0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0] {
        percentiles.push(metrics::calculate_response_time_percentile(
            response_times,
            total_request_count,
            response_time_minimum,
            response_time_maximum,
            *percent,
        ));
    }

    // Now take the Strings out of the Vector and build a ResponseMetric object.
    ResponseMetric {
        method: method.to_string(),
        name: name.to_string(),
        percentile_50: mem::take(&mut percentiles[0]),
        percentile_60: mem::take(&mut percentiles[1]),
        percentile_70: mem::take(&mut percentiles[2]),
        percentile_80: mem::take(&mut percentiles[3]),
        percentile_90: mem::take(&mut percentiles[4]),
        percentile_95: mem::take(&mut percentiles[5]),
        percentile_99: mem::take(&mut percentiles[6]),
        percentile_100: mem::take(&mut percentiles[7]),
    }
}

/// Build an individual row of request metrics in the html report.
pub fn request_metrics_row(metric: RequestMetric) -> String {
    format!(
        r#"<tr>
        <td>{method}</td>
        <td>{name}</td>
        <td>{number_of_requests}</td>
        <td>{number_of_failures}</td>
        <td>{response_time_average}</td>
        <td>{response_time_minimum}</td>
        <td>{response_time_maximum}</td>
        <td>{requests_per_second}</td>
        <td>{failures_per_second}</td>
    </tr>"#,
        method = metric.method,
        name = metric.name,
        number_of_requests = metric.number_of_requests,
        number_of_failures = metric.number_of_failures,
        response_time_average = metric.response_time_average,
        response_time_minimum = metric.response_time_minimum,
        response_time_maximum = metric.response_time_maximum,
        requests_per_second = metric.requests_per_second,
        failures_per_second = metric.failures_per_second,
    )
}

/// Build an individual row of response metrics in the html report.
pub fn response_metrics_row(metric: ResponseMetric) -> String {
    format!(
        r#"<tr>
            <td>{method}</td>
            <td>{name}</td>
            <td>{percentile_50}</td>
            <td>{percentile_60}</td>
            <td>{percentile_70}</td>
            <td>{percentile_80}</td>
            <td>{percentile_90}</td>
            <td>{percentile_95}</td>
            <td>{percentile_99}</td>
            <td>{percentile_100}</td>
        </tr>"#,
        method = metric.method,
        name = metric.name,
        percentile_50 = metric.percentile_50,
        percentile_60 = metric.percentile_60,
        percentile_70 = metric.percentile_70,
        percentile_80 = metric.percentile_80,
        percentile_90 = metric.percentile_90,
        percentile_95 = metric.percentile_95,
        percentile_99 = metric.percentile_99,
        percentile_100 = metric.percentile_100,
    )
}

/// If status code metrics are enabled, add a status code metrics table to the
/// html report.
pub fn status_code_metrics_template(status_code_rows: &str) -> String {
    format!(
        r#"<div class="status_codes">
        <h2>Status Code Metrics</h2>
        <table>
            <thead>
                <tr>
                    <th>Method</th>
                    <th colspan="2">Name</th>
                    <th colspan="3">Status Codes</th>
                </tr>
            </thead>
            <tbody>
                {status_code_rows}
            </tbody>
        </table>
    </div>"#,
        status_code_rows = status_code_rows,
    )
}

/// Build an individual row of status code metrics in the html report.
pub fn status_code_metrics_row(metric: StatusCodeMetric) -> String {
    format!(
        r#"<tr>
        <td>{method}</td>
        <td colspan="2">{name}</td>
        <td colspan="3">{status_codes}</td>
    </tr>"#,
        method = metric.method,
        name = metric.name,
        status_codes = metric.status_codes,
    )
}

/// If task metrics are enabled, add a task metrics table to the html report.
pub fn task_metrics_template(task_rows: &str) -> String {
    format!(
        r#"<div class="tasks">
        <h2>Task Metrics</h2>
        <table>
            <thead>
                <tr>
                    <th colspan="2">Task</th>
                    <th># Times Run</th>
                    <th># Fails</th>
                    <th>Average (ms)</th>
                    <th>Min (ms)</th>
                    <th>Max (ms)</th>
                    <th>RPS</th>
                    <th>Failures/s</th>
                </tr>
            </thead>
            <tbody>
                {task_rows}
            </tbody>
        </table>
    </div>"#,
        task_rows = task_rows,
    )
}

/// Build an individual row of task metrics in the html report.
pub fn task_metrics_row(metric: TaskMetric) -> String {
    if metric.is_task_set {
        format!(
            r#"<tr>
            <td colspan="10" align="left"><strong>{name}</strong></td>
        </tr>"#,
            name = metric.name,
        )
    } else {
        format!(
            r#"<tr>
            <td colspan="2">{task} {name}</strong></td>
            <td>{number_of_requests}</td>
            <td>{number_of_failures}</td>
            <td>{response_time_average}</td>
            <td>{response_time_minimum}</td>
            <td>{response_time_maximum}</td>
            <td>{requests_per_second}</td>
            <td>{failures_per_second}</td>
        </tr>"#,
            task = metric.task,
            name = metric.name,
            number_of_requests = metrics::format_number(metric.number_of_requests),
            number_of_failures = metrics::format_number(metric.number_of_failures),
            response_time_average = metric.response_time_average,
            response_time_minimum = metric.response_time_minimum,
            response_time_maximum = metric.response_time_maximum,
            requests_per_second = metric.requests_per_second,
            failures_per_second = metric.failures_per_second,
        )
    }
}

/// If there are errors, add an errors table to the html report.
pub fn errors_template(error_rows: &str) -> String {
    format!(
        r#"<div class="errors">
        <h2>Errors</h2>
        <table>
            <thead>
                <tr>
                    <th>#</th>
                    <th colspan="3">Error</th>
                </tr>
            </thead>
            <tbody>
                {error_rows}
            </tbody>
        </table>
    </div>"#,
        error_rows = error_rows,
    )
}

/// Build an individual error row in the html report.
pub fn error_row(error: &metrics::GooseErrorMetric) -> String {
    format!(
        r#"<tr>
        <td>{occurrences}</td>
        <td colspan="4">{error}</strong></td>
    </tr>"#,
        occurrences = error.occurrences,
        error = error.error,
    )
}

/// Build the html report.
pub fn build_report(
    start_time: &str,
    end_time: &str,
    host: &str,
    templates: GooseReportTemplates,
) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Test Report</title>
    <style>
        .container {{
            width: 1000px;
            margin: 0 auto;
            padding: 10px;
            background: #173529;
            font-family: Arial, Helvetica, sans-serif;
            font-size: 14px;
            color: #fff;
        }}

        .info span{{
            color: #b3c3bc;
        }}

        table {{
            border-collapse: collapse;
            text-align: center;
            width: 100%;
        }}

        td, th {{
            border: 1px solid #cad9ea;
            color: #666;
            height: 30px;
        }}

        thead th {{
            background-color: #cce8eb;
            width: 100px;
        }}

        tr:nth-child(odd) {{
            background: #fff;
        }}

        tr:nth-child(even) {{
            background: #f5fafa;
        }}

        .charts-container .chart {{
            width: 100%;
            height: 350px;
            margin-bottom: 30px;
        }}

        .download {{
            float: right;
        }}

        .download a {{
            color: #00ca5a;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Goose Test Report</h1>

        <div class="info">
            <p>During: <span>{start_time} - {end_time}</span></p>
            <p>Target Host: <span>{host}</span></p>
        </div>

        <div class="requests">
            <h2>Request Metrics</h2>
            <table>
                <thead>
                    <tr>
                        <th>Method</th>
                        <th>Name</th>
                        <th># Requests</th>
                        <th># Fails</th>
                        <th>Average (ms)</th>
                        <th>Min (ms)</th>
                        <th>Max (ms)</th>
                        <th>RPS</th>
                        <th>Failures/s</th>
                    </tr>
                </thead>
                <tbody>
                    {requests_template}
                </tbody>
            </table>
        </div>

        <div class="responses">
            <h2>Response Time Metrics</h2>
            <table>
                <thead>
                    <tr>
                        <th>Method</th>
                        <th>Name</th>
                        <th>50%ile (ms)</th>
                        <th>60%ile (ms)</th>
                        <th>70%ile (ms)</th>
                        <th>80%ile (ms)</th>
                        <th>90%ile (ms)</th>
                        <th>95%ile (ms)</th>
                        <th>99%ile (ms)</th>
                        <th>100%ile (ms)</th>
                    </tr>
                </thead>
                <tbody>
                    {responses_template}
                </tbody>
            </table>
        </div>

        {status_codes_template}

        {tasks_template}

        {errors_template}

    </div>
</body>
</html>"#,
        start_time = start_time,
        end_time = end_time,
        host = host,
        requests_template = templates.requests_template,
        responses_template = templates.responses_template,
        tasks_template = templates.tasks_template,
        status_codes_template = templates.status_codes_template,
        errors_template = templates.errors_template,
    )
}
