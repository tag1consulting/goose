# Defaults

All run-time options can be configured with custom defaults. For example, you may want to default to the the host name of your local development environment, only requiring that `--host` be set when running against a production environment. Assuming your local development environment is at "http://local.dev/" you can do this as follows:

```rust
    GooseAttack::initialize()?
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "http://local.dev/")?
        .execute()?
        .print();

    Ok(())
```

The following defaults can be configured with a `&str`:
 - host: `GooseDefault::Host`
 - log file name: `GooseDefault::LogFile`
 - html-formatted report file name: `GooseDefault::ReportFile`
 - requests log file name: `GooseDefault::RequestsFile`
 - requests log file format: `GooseDefault::RequestsFormat`
 - debug log file name: `GooseDefault::DebugFile`
 - debug log file format: `GooseDefault::DebugFormat`
 - host to bind telnet Controller to: `GooseDefault::TelnetHost`
 - host to bind WebSocket Controller to: `GooseDefault::WebSocketHost`
 - host to bind Manager to: `GooseDefault::ManagerBindHost`
 - host for Worker to connect to: `GooseDefault::ManagerHost`

The following defaults can be configured with a `usize` integer:
 - total users to start: `GooseDefault::Users`
 - users to start per second: `GooseDefault::HatchRate`
 - how often to print running metrics: `GooseDefault::RunningMetrics`
 - number of seconds for test to run: `GooseDefault::RunTime`
 - log level: `GooseDefault::LogLevel`
 - verbosity: `GooseDefault::Verbose`
 - maximum requests per second: `GooseDefault::ThrottleRequests`
 - number of Workers to expect: `GooseDefault::ExpectWorkers`
 - port to bind telnet Controller to: `GooseDefault::TelnetPort`
 - port to bind WebSocket Controller to: `GooseDefault::WebSocketPort`
 - port to bind Manager to: `GooseDefault::ManagerBindPort`
 - port for Worker to connect to: `GooseDefault::ManagerPort`

The following defaults can be configured with a `bool`:
 - do not reset metrics after all users start: `GooseDefault::NoResetMetrics`
 - do not track metrics: `GooseDefault::NoMetrics`
 - do not track task metrics: `GooseDefault::NoTaskMetrics`
 - do not start telnet Controller thread: `GooseDefault::NoTelnet`
 - do not start WebSocket Controller thread: `GooseDefault::NoWebSocket`
 - do not autostart load test, wait instead for a Controller to start: `GooseDefault::NoAutoStart`
 - track status codes: `GooseDefault::StatusCodes`
 - follow redirect of base_url: `GooseDefault::StickyFollow`
 - enable Manager mode: `GooseDefault::Manager`
 - ignore load test checksum: `GooseDefault::NoHashCheck`
 - enable Worker mode: `GooseDefault::Worker`

The following defaults can be configured with a `GooseCoordinatedOmissionMitigation`:
 - default Coordinated Omission Mitigation strategy: `GooseDefault::CoordinatedOmissionMitigation`

For example, without any run-time options the following load test would automatically run against `local.dev`, logging metrics to `goose-metrics.log` and debug to `goose-debug.log`. It will automatically launch 20 users in 4 seconds, and run the load test for 15 minutes. Metrics will be displayed every minute during the test and will include additional status code metrics. The order the defaults are set is not important.

```rust
    GooseAttack::initialize()?
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "local.dev")?
        .set_default(GooseDefault::RequestsFile, "goose-requests.log")?
        .set_default(GooseDefault::DebugFile, "goose-debug.log")?
        .set_default(GooseDefault::Users, 20)?
        .set_default(GooseDefault::HatchRate, 4)?
        .set_default(GooseDefault::RunTime, 900)?
        .set_default(GooseDefault::RunningMetrics, 60)?
        .set_default(GooseDefault::StatusCodes, true)?
        .execute()?
        .print();

    Ok(())
```
