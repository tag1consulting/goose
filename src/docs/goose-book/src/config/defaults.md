# Defaults

All run-time options can be configured with custom defaults. For example, you may want to default to the the host name of your local development environment, only requiring that `--host` be set when running against a production environment. Assuming your local development environment is at "http://local.dev/" you can do this as follows:

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
 - set a per-request timeout: `GooseDefault::Timeout`
 - users to start per second: `GooseDefault::HatchRate`
 - html-formatted report file name: `GooseDefault::ReportFile`
 - goose log file name: `GooseDefault::GooseLog`
 - request log file name: `GooseDefault::RequestLog`
 - transaction log file name: `GooseDefault::TransactionLog`
 - error log file name: `GooseDefault::ErrorLog`
 - debug log file name: `GooseDefault::DebugLog`
 - test plan: `GooseDefault::TestPlan`
 - host to bind telnet Controller to: `GooseDefault::TelnetHost`
 - host to bind WebSocket Controller to: `GooseDefault::WebSocketHost`
 - host to bind Manager to: `GooseDefault::ManagerBindHost`
 - host for Worker to connect to: `GooseDefault::ManagerHost`

The following defaults can be configured with a `usize` integer:
 - total users to start: `GooseDefault::Users`
 - how quickly to start all users: `GooseDefault::StartupTime`
 - how often to print running metrics: `GooseDefault::RunningMetrics`
 - number of seconds for test to run: `GooseDefault::RunTime`
 - log level: `GooseDefault::LogLevel`
 - quiet: `GooseDefault::Quiet`
 - verbosity: `GooseDefault::Verbose`
 - maximum requests per second: `GooseDefault::ThrottleRequests`
 - number of Workers to expect: `GooseDefault::ExpectWorkers`
 - port to bind telnet Controller to: `GooseDefault::TelnetPort`
 - port to bind WebSocket Controller to: `GooseDefault::WebSocketPort`
 - port to bind Manager to: `GooseDefault::ManagerBindPort`
 - port for Worker to connect to: `GooseDefault::ManagerPort`

The following defaults can be configured with a `bool`:
 - do not reset metrics after all users start: `GooseDefault::NoResetMetrics`
 - do not print metrics: `GooseDefault::NoPrintMetrics`
 - do not track metrics: `GooseDefault::NoMetrics`
 - do not track transaction metrics: `GooseDefault::NoTransactionMetrics`
 - do not log the request body in the error log: `GooseDefault::NoRequestBody`
 - do not display the error summary: `GooseDefault::NoErrorSummary`
 - do not log the response body in the debug log: `GooseDefault::NoDebugBody`
 - do not start telnet Controller thread: `GooseDefault::NoTelnet`
 - do not start WebSocket Controller thread: `GooseDefault::NoWebSocket`
 - do not autostart load test, wait instead for a Controller to start: `GooseDefault::NoAutoStart`
 - do not gzip compress requests: `GooseDefault::NoGzip`
 - do not track status codes: `GooseDefault::NoStatusCodes`
 - follow redirect of base_url: `GooseDefault::StickyFollow`
 - enable Manager mode: `GooseDefault::Manager`
 - enable Worker mode: `GooseDefault::Worker`
 - ignore load test checksum: `GooseDefault::NoHashCheck`
 - do not collect granular data in the HTML report: `GooseDefault::NoGranularData`

The following defaults can be configured with a `GooseLogFormat`:
 - request log file format: `GooseDefault::RequestFormat`
 - transaction log file format: `GooseDefault::TransactionFormat`
 - error log file format: `GooseDefault::ErrorFormat`
 - debug log file format: `GooseDefault::DebugFormat`

The following defaults can be configured with a `GooseCoordinatedOmissionMitigation`:
 - default Coordinated Omission Mitigation strategy: `GooseDefault::CoordinatedOmissionMitigation`

For example, without any run-time options the following load test would automatically run against `local.dev`, logging metrics to `goose-metrics.log` and debug to `goose-debug.log`. It will automatically launch 20 users in 4 seconds, and run the load test for 15 minutes. Metrics will be displayed every minute during the test, and the status code table will be disabled. The order the defaults are set is not important.

```rust,ignore
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadtestTransactions")
            .register_transaction(transaction!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "local.dev")?
        .set_default(GooseDefault::RequestLog, "goose-requests.log")?
        .set_default(GooseDefault::DebugLog, "goose-debug.log")?
        .set_default(GooseDefault::Users, 20)?
        .set_default(GooseDefault::HatchRate, 4)?
        .set_default(GooseDefault::RunTime, 900)?
        .set_default(GooseDefault::RunningMetrics, 60)?
        .set_default(GooseDefault::NoStatusCodes, true)?
        .execute()
        .await?;

    Ok(())
```

Find a complete list of all configuration options that can be configured with custom defaults [in the developer documentation](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html), as well as complete details on [how to configure defaults](https://docs.rs/goose/*/goose/config/trait.GooseDefaultType.html).
