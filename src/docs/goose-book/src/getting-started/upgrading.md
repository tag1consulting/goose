# Upgrading From 0.18

Change the dependency to `goose = "^0.19"`. The changes below can break a load test written for Goose 0.18, or change what it does.

## Removed options

| 0.18 | 0.19 |
|---|---|
| `--hatch-rate` (`-r`) | `--increase-rate` (`-r`), see [launching users](common.md#controlling-how-long-it-takes-goose-to-launch-all-users) |
| `--startup-time` (`-s`) | `--increase-time` (`-s`), see [launching users](common.md#controlling-how-long-it-takes-goose-to-launch-all-users) |
| `GooseDefault::HatchRate` | `GooseDefault::IncreaseRate`, see [Defaults](../config/defaults.md) |
| `GooseDefault::StartupTime` | `GooseDefault::IncreaseTime`, see [Defaults](../config/defaults.md) |
| `GooseConfiguration` fields `hatch_rate` and `startup_time` | `increase_rate` and `increase_time`, see [Custom options](custom.md#command-line-arguments) |
| `goose::util::get_hatch_rate()` | `goose::util::parse_rate()` |
| Controller commands `hatchrate` and `startuptime` | `increaserate` and `increasetime`, see [Controller commands](../controller/telnet.md#controller-commands) |
| Crate feature `gaggle` | None: Gaggle is still not available in 0.19 and is planned for 0.20, see [Gaggle](../gaggle/overview.md) |

The new `--decrease-rate` and `--decrease-time` control the ramp down; without them Goose stops all users at once, as 0.18 did. See [the ramp down rate](common.md#controlling-the-ramp-down-rate).

## API changes

- **API change**: the command line is parsed with `clap`. Remove `use gumdrop::Options;`: `GooseConfiguration::parse_args_default()` is now a method of its own, and `GooseConfiguration` implements `clap::Parser`. See [Custom options](custom.md#command-line-arguments).
- **API change**: every `GooseConfiguration` field that takes a value, such as `host`, `run_time`, `increase_time` or `iterations`, is an `Option`, `None` when not given; set one with `Some`, as in `configuration.run_time = Some("30s".to_string())`. The `help` field is gone. See [Custom options](custom.md#command-line-arguments).
- **API change**: session data must implement `Clone`; add `#[derive(Clone)]` to the struct. See the [Session example](../example/session.md).
- **API change**: `TransactionError::RequestFailed { raw_request }` holds a `Box<GooseRequestMetric>`, `TransactionError::LoggerFailed { source }` a boxed `SendError`, and the new `TransactionError::Custom(String)` lets a transaction return `Err("reason".into())`. See [Transaction error types](validation.md#transaction-error-types).
- **API change**: a `match` without a wildcard arm needs new arms for `GooseError::ThresholdsFailed` (see [Exit code in CI](thresholds.md#exit-code-in-ci)), `GooseMethod::Custom` (see [The API](../example/tcp.md#the-api)) and the new `GooseDefault` variants (see [Defaults](../config/defaults.md)).
- **API change**: scenario and transaction names are `Arc<str>` in place of `String`: `Scenario.name`, `Scenario.machine_name`, the `name` of `ScenarioMetric`, `TransactionMetric` and `ScenarioMetricAggregate`, `TransactionMetricAggregate.scenario_name`, and the `scenario_name` argument of `GooseUser::new()`. Reading one as `&str` works unchanged. See the [API documentation](https://docs.rs/goose/*/goose/goose/struct.Scenario.html).
- **API change**: times are kept in microseconds. `GooseRequestMetricTimingData` has `times_us`, `minimum_time_us`, `maximum_time_us` and `total_time_us` in place of the millisecond `times`, `minimum_time`, `maximum_time` and `total_time`; `TransactionMetricAggregate` and `ScenarioMetricAggregate` keep their times in `run_times`; a struct literal of `GooseRequestMetric`, `TransactionMetric` or `ScenarioMetric` must set the new `response_time_fraction_us` or `run_time_fraction_us`. See [Response times](metrics.md#response-times).
- **API change**: the default crate features are now `cookies`, so `default-features = false` turns off [cookie support](../config/cookie.md), where in 0.18 it turned off TLS, and the `rustls-tls` feature no longer changes the TLS that load test requests use.
- **API change**: `controller::ControllerCommand` carries each command's parsed value, and `util::parse_timespan()` returns `Option<usize>`. See [Controller commands](../controller/telnet.md#controller-commands).

## Behavior changes

- **behavior change**: `Transaction::set_name()` names only the transaction; its requests keep their own names. Use `set_name_for_transaction_and_requests()` for the 0.18 behavior. See [Naming transactions](creating.md#naming-transactions).
- **behavior change**: `--scenarios` matches machine names exactly, where 0.18 matched any part of the name; use `*` as a wildcard, as in `--scenarios "anon*"`. See [Limiting scenarios](scenarios.md#running-scenarios-by-machine-name).
- **behavior change**: HTTPS uses rustls in place of the system's native TLS library. Certificates are still checked by the operating system's verifier, so a CA the system trusts is trusted, and `--accept-invalid-certs` skips the check in every build. See the [options reference](runtime-options.md#full-reference).
- **behavior change**: an option value that begins with a hyphen goes after `=`, as in `--goose-log=-run.log`. An output option followed by another option, as in `--goose-log --quiet`, is given without a path and writes `goose.log` into the run's folder. See [Output files](output-files.md#files).
- **behavior change**: a report or log file whose name ends in `.gz` is written gzipped. See [Compression](output-files.md#compression).
- **behavior change**: `--report-file` reports are written even with `--no-print-metrics`. See [Output files](output-files.md).
- **behavior change**: `-h` prints a short summary and `--help` the full reference, both to stdout. See [Run-time options](runtime-options.md).
- **behavior change**: `--run-time 0`, `--increase-time 0`, `--decrease-time 0` and `--throttle-requests 0` undo the matching `GooseDefault`, and an empty log path, as in `--request-log ""`, turns off a log a `GooseDefault` sets; `GooseDefault::Iterations` and `GooseDefault::NoGranularData` now take effect. See [Defaults](../config/defaults.md).
- **behavior change**: values that started no users or broke Goose are refused before the load test starts: a test plan with no users, a time span too large to hold, an increase or decrease rate below one user a day, a rate or `--timeout` that is not a finite number, and a `--timeout` that rounds to 0 ms. A fractional `--timeout` is rounded to the nearest millisecond, so `0.0005` runs as 1 ms and anything below it is refused. See [Test plan](test-plan.md) and [Run-time options](runtime-options.md#full-reference).
- **behavior change**: times under 10 ms are shown to the hundredth of a millisecond, and percentiles are never more than 1% off, so figures differ from the whole milliseconds 0.18 printed. See [Response times](metrics.md#response-times).
- **behavior change**: the JSON report's timing data holds `times_us` in place of `times`, its averages are means of the microseconds, and it includes the test plan history. See [JSON report](metrics.md#json-report).
- **behavior change**: the CSV request log ends with a `response_time_us` column, and the transaction and scenario logs with `run_time_us`; the JSON, raw and pretty formats add `response_time_fraction_us` or `run_time_fraction_us`. In JSON logs the keys follow the order of the fields instead of being sorted, and `transaction_name` is an object such as `{"TransactionOnly":"login"}`, where 0.18 wrote a string. See [Request log](../logging/requests.md#log-format).
- **behavior change**: `elapsed` in the request, error, transaction and scenario logs counts milliseconds since the load test started, where 0.18 counted from when each user was built, and the HTML report's active users graph shows the increase ramp. See [Request log](../logging/requests.md#log-format).
- **behavior change**: `GooseRawRequest.headers`, as read through `goose.request.raw.headers` or a `TransactionError::RequestFailed`, is empty unless the request log, debug log or error log is enabled. See [Request log](../logging/requests.md).
- **behavior change**: Controller `users` takes 1 to 100000, and on a running load test keeps its run time and ramp down; `runtime` changes a running users ramp; a value that can't be used is answered with what it must be; `config` shows an unset option as `null` and has no `help` key. See [Controller commands](../controller/telnet.md#controller-commands).
- **behavior change**: the telnet Controller's `metrics-json` and the WebSocket Controller's `metrics` replies hold `times_us` in place of `times`, and a WebSocket message over 131,072 bytes closes the connection. See [WebSocket Controller](../controller/websocket.md#details).
- **behavior change**: the killswitch keeps the first reason it is given, and is cleared when a run starts. See [Killswitch](killswitch.md).
