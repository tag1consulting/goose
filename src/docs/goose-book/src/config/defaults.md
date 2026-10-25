# Defaults

All run-time options can be configured with custom defaults. For example, you may want to default to the host name of your local development environment, only requiring that `--host` be set when running against a production environment. Assuming your local development environment is at "http://local.dev/" you can do this as follows:

```rust,ignore
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadtestTransactions")
            .register_transaction(transaction!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "http://local.dev/")?
        .execute()
        .await?;

    Ok(())
```

The following defaults can be configured with a `&str`:
 - host: `GooseDefault::Host`
 - per-request timeout, in seconds: `GooseDefault::Timeout`
 - users to start per second: `GooseDefault::IncreaseRate`
 - users to stop per second: `GooseDefault::DecreaseRate`
 - test plan: `GooseDefault::TestPlan`
 - only run the named scenarios: `GooseDefault::Scenarios`
 - a pass/fail threshold rule, one rule for each call: `GooseDefault::Threshold`
 - report file name: `GooseDefault::ReportFile`
 - earlier report to compare against: `GooseDefault::BaselineFile`
 - the run's folder: `GooseDefault::OutputDir`
 - notes stored with the run's reports: `GooseDefault::Notes`
 - goose log file name: `GooseDefault::GooseLog`
 - request log file name: `GooseDefault::RequestLog`
 - transaction log file name: `GooseDefault::TransactionLog`
 - scenario log file name: `GooseDefault::ScenarioLog`
 - error log file name: `GooseDefault::ErrorLog`
 - debug log file name: `GooseDefault::DebugLog`
 - host to bind telnet Controller to: `GooseDefault::TelnetHost`
 - host to bind WebSocket Controller to: `GooseDefault::WebSocketHost`
 - host to bind the dashboard to: `GooseDefault::DashboardHost`
 - dashboard auth token: `GooseDefault::DashboardAuthToken`

The following defaults can be configured with a `usize` integer:
 - total users to start: `GooseDefault::Users`
 - seconds to start all users: `GooseDefault::IncreaseTime`
 - seconds to stop all users: `GooseDefault::DecreaseTime`
 - seconds for the test to run: `GooseDefault::RunTime`
 - times each user runs its scenario: `GooseDefault::Iterations`
 - how often to print running metrics, in seconds: `GooseDefault::RunningMetrics`
 - log level: `GooseDefault::LogLevel`
 - quiet: `GooseDefault::Quiet`
 - verbosity: `GooseDefault::Verbose`
 - maximum requests per second: `GooseDefault::ThrottleRequests`
 - seconds to wait before a threshold can stop the run: `GooseDefault::ThresholdAbortDelay`
 - port to bind telnet Controller to: `GooseDefault::TelnetPort`
 - port to bind WebSocket Controller to: `GooseDefault::WebSocketPort`
 - port to bind the dashboard to: `GooseDefault::DashboardPort`
 - maximum live dashboard clients: `GooseDefault::DashboardMaxClients`
 - gzip every file in the run's folder, at a level from 1 to 9: `GooseDefault::Compress`
 - seconds to wait for a PDF report, with the `pdf-reports` feature: `GooseDefault::PdfTimeout`

The following default can be configured with an `f64` float:
 - PDF report scale, with the `pdf-reports` feature: `GooseDefault::PdfScale`

The following defaults can be configured with a `bool`:
 - do not reset metrics after all users start: `GooseDefault::NoResetMetrics`
 - do not print metrics: `GooseDefault::NoPrintMetrics`
 - do not track metrics: `GooseDefault::NoMetrics`
 - do not track transaction metrics: `GooseDefault::NoTransactionMetrics`
 - do not track scenario metrics: `GooseDefault::NoScenarioMetrics`
 - do not track status codes: `GooseDefault::NoStatusCodes`
 - do not display the error summary: `GooseDefault::NoErrorSummary`
 - judge thresholds only at the end: `GooseDefault::NoThresholdAbort`
 - add request bodies to the request log: `GooseDefault::RequestBody`
 - do not log the response body in the debug log: `GooseDefault::NoDebugBody`
 - do not start telnet Controller thread: `GooseDefault::NoTelnet`
 - do not start WebSocket Controller thread: `GooseDefault::NoWebSocket`
 - do not autostart load test, wait instead for a Controller to start: `GooseDefault::NoAutoStart`
 - do not ask for gzip compressed responses: `GooseDefault::NoGzip`
 - follow redirect of base_url: `GooseDefault::StickyFollow`
 - accept invalid TLS certificates: `GooseDefault::AcceptInvalidCerts`
 - graph only totals in reports: `GooseDefault::NoGranularData`
 - add graph data to the JSON report: `GooseDefault::ReportGraphData`
 - serve the live web dashboard: `GooseDefault::Dashboard`
 - allow control from the dashboard: `GooseDefault::DashboardControl`
 - do not track per-second response times: `GooseDefault::NoPercentilesOverTime`

A default names a path, never the run's folder. `GooseDefault::OutputDir` needs the dashboard, or an output flag given without a path on the command line. Without either, Goose refuses it with an error before the load test starts.

The following defaults can be configured with a `GooseLogFormat`:
 - request log file format: `GooseDefault::RequestFormat`
 - transaction log file format: `GooseDefault::TransactionFormat`
 - scenario log file format: `GooseDefault::ScenarioFormat`
 - error log file format: `GooseDefault::ErrorFormat`
 - debug log file format: `GooseDefault::DebugFormat`

The following defaults can be configured with a `GooseCoordinatedOmissionMitigation`:
 - default Coordinated Omission Mitigation strategy: `GooseDefault::CoordinatedOmissionMitigation`

Setting a default with a value of the wrong type returns an error that names the type the default expects.

For example, without any run-time options the following load test would automatically run against `http://local.dev/`, logging requests to `goose-requests.log` and debug to `goose-debug.log`. It will automatically launch 20 users at 4 users per second, so in 5 seconds, and run the load test for 15 minutes. Metrics will be displayed every minute during the test, and the status code table will be disabled. The order the defaults are set is not important.

```rust,ignore
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadtestTransactions")
            .register_transaction(transaction!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "http://local.dev/")?
        .set_default(GooseDefault::RequestLog, "goose-requests.log")?
        .set_default(GooseDefault::DebugLog, "goose-debug.log")?
        .set_default(GooseDefault::Users, 20)?
        .set_default(GooseDefault::IncreaseRate, "4")?
        .set_default(GooseDefault::RunTime, 900)?
        .set_default(GooseDefault::RunningMetrics, 60)?
        .set_default(GooseDefault::NoStatusCodes, true)?
        .execute()
        .await?;

    Ok(())
```

Find a complete list of all configuration options that can be configured with custom defaults [in the developer documentation](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html), as well as complete details on [how to configure defaults](https://docs.rs/goose/*/goose/config/trait.GooseDefaultType.html).
