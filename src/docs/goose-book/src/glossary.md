# Glossary

## Attack Phase
The stage a load test is in, which Goose logs as `entering GooseAttack phase:` and the [dashboard](./controller/dashboard.html) shows. A load test is **Idle** before it starts, when a [Controller](#controller) can still change its configuration (see [`--no-autostart`](./controller/overview.html)); in **Increase** while [GooseUsers](#gooseuser) launch; in **Maintain** once all are running; in **Decrease** while they stop; and in **Shutdown** as Goose exits.

## Baseline
An earlier run's JSON report, given with [`--baseline-file`](./getting-started/common.html#comparing-against-a-baseline). Every report of the current run then shows each number's change from the baseline, such as `1,234 (+200)`.

## Controller
An interface that allows real-time control of a running Goose load test. Goose provides both [Telnet](./controller/telnet.html) and [WebSocket](./controller/websocket.html) controllers for dynamically adjusting test parameters like user count, increase rate, and runtime during execution.

## Coordinated Omission
A phenomenon that occurs in load testing when the measurement system inadvertently excludes the results of requests that were affected by server slowdowns, leading to misleadingly optimistic performance metrics. Goose includes [Coordinated Omission Mitigation](./coordinated-omission/mitigation.html) functionality to detect and correct for this.

## Coordinated Omission Adjusted Values
The metrics Goose shows with [coordinated omission](#coordinated-omission) mitigation on (`--co-mitigation`), labelled "adjusted" beside the "raw" metrics that hold only the requests actually sent. While the server stalls, each [GooseUser](#gooseuser) sends fewer requests than usual. Mitigation adds a synthetic response time for each request a user would have sent at its usual cadence, and the adjusted values count these synthetic times along with the real ones. See [Coordinated Omission Metrics](./coordinated-omission/metrics.html).

## Decrease Rate
The rate at which [GooseUsers](#gooseuser) are removed during the ramp-down phase of a load test, typically specified as users per second. Configured with [`--decrease-rate`](./getting-started/common.html#controlling-the-ramp-down-rate). If not configured, Goose shuts down all users immediately when the load test completes.

## Gaggle
Goose's distributed load test, in which one Manager and many Workers ran a single load test across several servers. Gaggle support was removed in Goose 0.17.0, and Goose 0.19 cannot run a Gaggle. See [Gaggle: Distributed Load Test](./gaggle/overview.html).

## GooseAttack
A load test defined by one or more [Scenarios](#scenario) with one or more [Transactions](#transaction).

## GooseConfiguration
A structure that defines all configuration options for a Goose load test, including user count, increase rate, runtime, host, and various other parameters. Set from command line arguments, with defaults that the load test can set in code with [`GooseDefault`](./config/defaults.html).

## GooseError
The error returned by [GooseAttack](#gooseattack) methods such as `initialize()`, `set_default()` and `execute()`, for example when a run-time option is invalid or no host is configured. A [Transaction](#transaction) returns a different error, a `TransactionError`, inside its [TransactionResult](#transactionresult).

## GooseUser
A Tokio task that repeatedly runs a single [**scenario**](./getting-started/metrics.html#scenarios) for the duration of the load test. For example, when Goose starts, you may use the [`--users`](./getting-started/common.html#how-many-users-to-simulate) command line option to configure how many GooseUsers are started. This is not intended to be a 1:1 correlation between GooseUsers and real website users.

## Heatmap
A chart of how response times are spread over time, in the HTML report and the [live dashboard](./controller/dashboard.html#the-response-time-heatmap). Each column is a span of time, starting at one second, and each row a range of response times; a cell's color shows how many requests that started in that column took that long. Lines across it show each column's p50, p95 and p99. `--no-percentiles-over-time` turns it off. See [Response times](./getting-started/metrics.html#response-times).

## Increase Rate
The rate at which new [GooseUsers](#gooseuser) are launched during the ramp-up phase of a load test, typically specified as users per second. Configured with [`--increase-rate`](./getting-started/common.html#specifying-the-increase-rate). See also [Decrease Rate](#decrease-rate).

## Killswitch
A way to stop a running load test from your own code, by calling `goose::trigger_killswitch` with a reason. Goose stops the run as it does for `ctrl-c`, writes every report, and records the run as canceled with that reason. See [Killswitch](./getting-started/killswitch.html).

## Request
A single [**request**](./getting-started/metrics.html#requests) based around HTTP verbs, or a non-HTTP operation recorded with `GooseUser::record_custom_request`, as in the [TCP example](./example/tcp.html).

## Run Folder
The folder a run writes into when an output flag such as `--report-file` is given without a path, or when the dashboard's Save report writes the run's report: `--output-dir`, or `goose-runs/<YYYYMMDD-HHMMSS>` from the run's local start time. See [Output files](./getting-started/output-files.html).

## Saved Run
A run whose report is in its [run folder](#run-folder), as `report.json` (or `report.json.gz`) beside `report.html`: written by a bare `--report-file`, or by the dashboard's Save report, which is on by default with `--dashboard`. The dashboard's runs panel lists saved runs, and a saved run's folder can be given to `--baseline-file`. See [Output files](./getting-started/output-files.html#the-dashboard).

## Scenario
A [**scenario**](./getting-started/metrics.html#scenarios) is a collection of [**transactions**](./getting-started/metrics.html#transactions) (aka steps) a user would undertake to achieve a specific user journey.

## Snapshot Version
The `version` field at the top of every snapshot the [live dashboard](./controller/dashboard.html) serves under `/api/v1`. It numbers the snapshot's format, not the Goose release, which is the snapshot's `goose_version`. It is `1` in Goose 0.19, and goes up only when a change to the snapshot is not additive, such as a field removed or renamed. The bundled dashboard refuses a snapshot of any other version.

## Test Plan
A flexible approach to scheduling load test phases, allowing you to define complex load patterns like gradual ramp-up, sustained load periods, spike testing, and graceful ramp-down. Test plans use the format `users,duration;users,duration` to specify multiple phases. See [Test Plan](./getting-started/test-plan.html).

## Threshold
A pass/fail rule on a load test's metrics, such as `p95<500ms` or `error-rate<1%`, given with `--threshold`. A rule that fails stops the run by default and always makes it fail. See [Thresholds](./getting-started/thresholds.html).

## Throttle
A limit on how many requests per second all [GooseUsers](#gooseuser) together send, set with `--throttle-requests`. See [Throttle](./getting-started/throttle.html).

## Transaction
A [**transaction**](./getting-started/metrics.html#transactions) is a collection of one or more [**requests**](./getting-started/metrics.html#requests) and any desired validation. For example, this may include loading the front page and all contained static assets, logging into the website, or adding one or more items to a shopping cart. Transactions typically include assertions or expectation validation.

## TransactionResult
A [`Result`](https://doc.rust-lang.org/std/result/enum.Result.html) returned by [Transaction](#transaction) functions. It is `Result<(), Box<TransactionError>>`: a transaction returns `Ok(())` on success, or a boxed `TransactionError` on failure. The `TransactionError::Custom` variant allows returning custom error messages using string literals or formatted strings with `.into()`.

## Weight
A value that controls the frequency with which a [Transaction](#transaction) or [Scenario](#scenario) runs, relative to the other transactions in the same scenario, or scenarios in the same load test. For example, if one transaction has a weight of 3 and another transaction in the same scenario has a weight of 1, the first transaction will run 3 times as often as the second.
