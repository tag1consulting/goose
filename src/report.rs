//! Optionally writes an html-formatted summary report after running a load test.

use crate::metrics;

use std::collections::BTreeMap;
use std::mem;

use serde::Serialize;

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

/// Default template used to generate an HTML report.
pub const TEMPLATE: &str = r#"<!DOCTYPE html>
<html>
<head>
    <title>Test Report</title>
    <style>
        .container {
            width: 1000px;
            margin: 0 auto;
            padding: 10px;
            background: #173529;
            font-family: Arial, Helvetica, sans-serif;
            font-size: 14px;
            color: #fff;
        }

        .info span{
            color: #b3c3bc;
        }

        table {
            border-collapse: collapse;
            text-align: center;
            width: 100%;
        }

        td, th {
            border: 1px solid #cad9ea;
            color: #666;
            height: 30px;
        }

        thead th {
            background-color: #cce8eb;
            width: 100px;
        }

        tr:nth-child(odd) {
            background: #fff;
        }

        tr:nth-child(even) {
            background: #f5fafa;
        }

        .charts-container .chart {
            width: 100%;
            height: 350px;
            margin-bottom: 30px;
        }

        .download {
            float: right;
        }

        .download a {
            color: #00ca5a;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>Goose Test Report</h1>

        <div class="info">
            <p>During: <span>{{ start_time }} - {{ end_time }}</span></p>
            <p>Target Host: <span>{{ host }}</span></p>
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
                    {{#each requests as |request| ~}}
                    <tr>
                        <td>{{ request.method }}</td>
                        <td>{{ request.name }}</td>
                        <td>{{ request.number_of_requests }}</td>
                        <td>{{ request.number_of_failures }}</td>
                        <td>{{ request.response_time_average }}</td>
                        <td>{{ request.response_time_minimum }}</td>
                        <td>{{ request.response_time_maximum }}</td>
                        <td>{{ request.requests_per_second }}</td>
                        <td>{{ request.failures_per_second }}</td>
                    </tr>
                    {{/each}}
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
                    {{#each responses as |response| ~}}
                    <tr>
                        <td>{{ response.method }}</td>
                        <td>{{ response.name }}</td>
                        <td>{{ response.percentile_50 }}</td>
                        <td>{{ response.percentile_60 }}</td>
                        <td>{{ response.percentile_70 }}</td>
                        <td>{{ response.percentile_80 }}</td>
                        <td>{{ response.percentile_90 }}</td>
                        <td>{{ response.percentile_95 }}</td>
                        <td>{{ response.percentile_99 }}</td>
                        <td>{{ response.percentile_100 }}</td>
                    </tr>
                    {{/each}}
                </tbody>
            </table>
        </div>
    </div>
</body>
</html>"#;
