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
            <h2>Request Statistics</h2>
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
                </tbody>
            </table>
        </div>

        <div class="responses">
            <h2>Response Time Statistics</h2>
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
            <h2>Failures Statistics</h2>
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
            <h2>Exceptions Statistics</h2>
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
