//! Optionally writes an html-formatted summary report after running a load test.

use crate::metrics;

use std::collections::BTreeMap;
use std::mem;

use serde::Serialize;

/// The following templates are necessary to build an html-formatted summary report.
#[derive(Debug)]
pub(crate) struct GooseReportTemplates<'a> {
    pub raw_requests_template: &'a str,
    pub raw_responses_template: &'a str,
    pub co_requests_template: &'a str,
    pub co_responses_template: &'a str,
    pub transactions_template: &'a str,
    pub scenarios_template: &'a str,
    pub status_codes_template: &'a str,
    pub errors_template: &'a str,
    pub graph_rps_template: &'a str,
    pub graph_average_response_time_template: &'a str,
    pub graph_users_per_second: &'a str,
}

/// Defines the metrics reported about requests.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RequestMetric {
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

/// Defines the metrics reported about Coordinated Omission requests.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CORequestMetric {
    pub method: String,
    pub name: String,
    pub response_time_average: String,
    pub response_time_standard_deviation: String,
    pub response_time_maximum: usize,
}

/// Defines the metrics reported about responses.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponseMetric {
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

/// Defines the metrics reported about transactions.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransactionMetric {
    pub is_scenario: bool,
    pub transaction: String,
    pub name: String,
    pub number_of_requests: usize,
    pub number_of_failures: usize,
    pub response_time_average: String,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub requests_per_second: String,
    pub failures_per_second: String,
}

/// Defines the metrics reported about scenarios.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ScenarioMetric {
    pub name: String,
    pub users: usize,
    pub count: usize,
    pub response_time_average: String,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub count_per_second: String,
    pub iterations: String,
}

/// Defines the metrics reported about status codes.
pub(crate) struct StatusCodeMetric {
    pub method: String,
    pub name: String,
    pub status_codes: String,
}

/// Helper to generate a single response metric.
pub(crate) fn get_response_metric(
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

/// Build an individual row of raw request metrics in the html report.
pub(crate) fn raw_request_metrics_row(metric: RequestMetric) -> String {
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
pub(crate) fn response_metrics_row(metric: ResponseMetric) -> String {
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

/// If Coordinated Omission Mitigation is triggered, add a relevant request table to the
/// html report.
pub(crate) fn coordinated_omission_request_metrics_template(co_requests_rows: &str) -> String {
    format!(
        r#"<div class="CO requests">
        <h2>Request Metrics With Coordinated Omission Mitigation</h2>
        <table>
            <thead>
                <tr>
                    <th>Method</th>
                    <th>Name</th>
                    <th>Average (ms)</th>
                    <th>Standard deviation (ms)</th>
                    <th>Max (ms)</th>
                </tr>
            </thead>
            <tbody>
                {co_requests_rows}
            </tbody>
        </table>
    </div>"#,
        co_requests_rows = co_requests_rows,
    )
}

/// Build an individual row of Coordinated Omission Mitigation request metrics in
/// the html report.
pub(crate) fn coordinated_omission_request_metrics_row(metric: CORequestMetric) -> String {
    format!(
        r#"<tr>
            <td>{method}</td>
            <td>{name}</td>
            <td>{average})</td>
            <td>{standard_deviation}</td>
            <td>{maximum}</td>
        </tr>"#,
        method = metric.method,
        name = metric.name,
        average = metric.response_time_average,
        standard_deviation = metric.response_time_standard_deviation,
        maximum = metric.response_time_maximum,
    )
}

/// If Coordinated Omission Mitigation is triggered, add a relevant response table to the
/// html report.
pub(crate) fn coordinated_omission_response_metrics_template(co_responses_rows: &str) -> String {
    format!(
        r#"<div class="responses">
        <h2>Response Time Metrics With Coordinated Omission Mitigation</h2>
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
                {co_responses_rows}
            </tbody>
        </table>
    </div>"#,
        co_responses_rows = co_responses_rows,
    )
}

/// Build an individual row of Coordinated Omission Mitigation request metrics in
/// the html report.
pub(crate) fn coordinated_omission_response_metrics_row(metric: ResponseMetric) -> String {
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
pub(crate) fn status_code_metrics_template(status_code_rows: &str) -> String {
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
pub(crate) fn status_code_metrics_row(metric: StatusCodeMetric) -> String {
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

/// If transaction metrics are enabled, add a transaction metrics table to the html report.
pub(crate) fn transaction_metrics_template(transaction_rows: &str, graph: String) -> String {
    format!(
        r#"<div class="transactions">
        <h2>Transaction Metrics</h2>

        {graph}

        <table>
            <thead>
                <tr>
                    <th colspan="2">Transaction</th>
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
                {transaction_rows}
            </tbody>
        </table>
    </div>"#,
        transaction_rows = transaction_rows,
        graph = graph,
    )
}

/// Build an individual row of transaction metrics in the html report.
pub(crate) fn transaction_metrics_row(metric: TransactionMetric) -> String {
    if metric.is_scenario {
        format!(
            r#"<tr>
            <td colspan="10" align="left"><strong>{name}</strong></td>
        </tr>"#,
            name = metric.name,
        )
    } else {
        format!(
            r#"<tr>
            <td colspan="2">{transaction} {name}</strong></td>
            <td>{number_of_requests}</td>
            <td>{number_of_failures}</td>
            <td>{response_time_average}</td>
            <td>{response_time_minimum}</td>
            <td>{response_time_maximum}</td>
            <td>{requests_per_second}</td>
            <td>{failures_per_second}</td>
        </tr>"#,
            transaction = metric.transaction,
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

/// If scenario metrics are enabled, add a scenario metrics table to the html report.
pub(crate) fn scenario_metrics_template(scenario_rows: &str, graph: String) -> String {
    format!(
        r#"<div class="scenarios">
        <h2>Scenario Metrics</h2>

        {graph}

        <table>
            <thead>
                <tr>
                    <th colspan="2">Scenario</th>
                    <th># Users</th>
                    <th># Times Run</th>
                    <th>Average (ms)</th>
                    <th>Min (ms)</th>
                    <th>Max (ms)</th>
                    <th>Scenarios/s</th>
                    <th>Iterations</th>
                </tr>
            </thead>
            <tbody>
                {scenario_rows}
            </tbody>
        </table>
    </div>"#,
        scenario_rows = scenario_rows,
        graph = graph,
    )
}

/// Build an individual row of scenario metrics in the html report.
pub(crate) fn scenario_metrics_row(metric: ScenarioMetric) -> String {
    format!(
        r#"<tr>
            <td colspan="2">{name}</strong></td>
            <td>{users}</td>
            <td>{count}</td>
            <td>{response_time_average}</td>
            <td>{response_time_minimum}</td>
            <td>{response_time_maximum}</td>
            <td>{count_per_second}</td>
            <td>{iterations}</td>
        </tr>"#,
        name = metric.name,
        users = metrics::format_number(metric.users),
        count = metrics::format_number(metric.count),
        response_time_average = metric.response_time_average,
        response_time_minimum = metric.response_time_minimum,
        response_time_maximum = metric.response_time_maximum,
        count_per_second = metric.count_per_second,
        iterations = metric.iterations,
    )
}

/// If there are errors, add an errors table to the html report.
pub(crate) fn errors_template(error_rows: &str, graph: String) -> String {
    format!(
        r#"<div class="errors">
        <h2>Errors</h2>

        {graph}

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
        graph = graph,
    )
}

/// Build an individual error row in the html report.
pub fn error_row(error: &metrics::GooseErrorMetricAggregate) -> String {
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
pub(crate) fn build_report(
    users: &str,
    steps_rows: &str,
    hosts: &str,
    templates: GooseReportTemplates,
) -> String {
    let pkg_name = env!("CARGO_PKG_NAME");
    let pkg_version = env!("CARGO_PKG_VERSION");

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Goose Attack Report</title>
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

        .graph {{
            margin-bottom: 1em;
        }}
    </style>
    <script src="https://cdn.jsdelivr.net/npm/echarts@5.2.2/dist/echarts.min.js"></script>
</head>
<body>
    <div class="container">
        <h1>Goose Attack Report</h1>

        <div class="info">
            <p>Users: <span>{users}</span> </p>
            <p>Target Host: <span>{hosts}</span></p>
            <p><span><small><em>{pkg_name} v{pkg_version}</em></small></span></p>
            <h2>Plan overview</h2>
                <table>
                    <thead>
                        <tr>
                            <th>Action</th>
                            <th>Started</th>
                            <th>Stopped</th>
                            <th>Elapsed</th>
                            <th>Users</th>
                        </tr>
                    </thead>
                    <tbody>
                        {steps_rows}
                    </tbody>
                </table>
        </div>

        <div class="requests">
            <h2>Request Metrics</h2>

            {graph_rps_template}

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
                    {raw_requests_template}
                </tbody>
            </table>
        </div>

        {co_requests_template}

        <div class="responses">
            <h2>Response Time Metrics</h2>

            {graph_average_response_time_template}

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
                    {raw_responses_template}
                </tbody>
            </table>
        </div>

        {co_responses_template}

        {status_codes_template}

        {transactions_template}

        {scenarios_template}

        <div class="users">
        <h2>User Metrics</h2>
            {graph_users_per_second}
        </div>

        {errors_template}

    </div>
</body>
</html>"#,
        users = users,
        steps_rows = steps_rows,
        hosts = hosts,
        pkg_name = pkg_name,
        pkg_version = pkg_version,
        raw_requests_template = templates.raw_requests_template,
        raw_responses_template = templates.raw_responses_template,
        co_requests_template = templates.co_requests_template,
        co_responses_template = templates.co_responses_template,
        transactions_template = templates.transactions_template,
        scenarios_template = templates.scenarios_template,
        status_codes_template = templates.status_codes_template,
        errors_template = templates.errors_template,
        graph_rps_template = templates.graph_rps_template,
        graph_average_response_time_template = templates.graph_average_response_time_template,
        graph_users_per_second = templates.graph_users_per_second,
    )
}
