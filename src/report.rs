use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RequestMetric {
    pub method: String,
    pub name: String,
    pub number_of_requests: usize,
    pub number_of_failures: usize,
    pub response_time_average: usize,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub content_length_average: usize,
    pub requests_per_second: usize,
    pub failures_per_second: usize,
}

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
            <p class="download"><a href="?download=1">Download the Report</a></p>
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
                        <th>Average size (bytes)</th>
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
                        <td>{{ request.content_length_average }}</td>
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
                </tbody>
            </table>
        </div>

        <div class="failures">
            <h2>Failures Metrics</h2>
            <table>
                <thead>
                    <tr>
                        <th>Method</th>
                        <th>Name</th>
                        <th>Error</th>
                        <th>Occurrences</th>
                    </tr>
                </thead>
                <tbody>
                </tbody>
            </table>
        </div>

        <div class="exceptions">
            <h2>Exceptions Metrics</h2>
            <table>
                <thead>
                    <tr>
                        <th>Count</th>
                        <th>Message</th>
                        <th>Traceback</th>
                        <th>Nodes</th>
                    </tr>
                </thead>
                <tbody>
                </tbody>
            </table>
        </div>

        <div class="charts-container">
            <h2>Charts</h2>
        </div>
    </div>
</body>
</html>"#;
