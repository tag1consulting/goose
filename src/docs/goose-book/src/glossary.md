# Glossary

These are the words Goose uses in a sense of its own, each with what it means in Goose and the page that explains it.

## Adjusted Metrics
The metrics Goose shows with [coordinated omission](#coordinated-omission) mitigation on, once mitigation has added any [synthetic requests](#synthetic-request). Goose labels them "adjusted" and shows them beside the "raw" metrics of the requests actually sent. See [Coordinated Omission Metrics](./coordinated-omission/metrics.md).

## Attack Phase
The stage a [GooseAttack](#gooseattack) is in, which Goose logs as `entering GooseAttack phase:` and the [live dashboard](#live-dashboard) shows: **Idle**, **Increase** while users launch, **Maintain** once all are running, **Decrease** while they stop, and **Shutdown** as Goose exits. Idle is Goose waiting for a start from the dashboard or a [Controller](#controller), before a run or after one. The [plan overview](#plan-overview) lists Decrease as Canceling, not Decreasing, when every user finishes its [iterations](#iteration) or when `ctrl-c`, the dashboard, a Controller, a [killswitch](#killswitch) or a failed [threshold](#threshold) stops the run. See [Running A Load Test](./getting-started/running.md).

## Baseline
An earlier run's JSON report, or the [run folder](#run-folder) that holds it, given with `--baseline-file`. The PDF, HTML and Markdown reports then show each number's change from it, such as `1,234 (+200)`. See [Comparing Against A Baseline](./getting-started/common.md#comparing-against-a-baseline).

## Cadence
How long a [GooseUser](#gooseuser) takes to run once through its [scenario](#scenario), leaving out any wait time: its average, fastest or slowest time, as `--co-mitigation` chooses. [Coordinated omission](#coordinated-omission) mitigation uses it to tell how many requests a user missed while the server stalled. See [Mitigation](./coordinated-omission/mitigation.md).

## CO Event
A [request](#request) made after a stall, for which [coordinated omission](#coordinated-omission) mitigation added [synthetic requests](#synthetic-request). The CO event count counts these requests, not the stalls. See [CO Event Tracking](./coordinated-omission/metrics.md#co-event-tracking).

## Controller
The telnet or WebSocket interface that starts and stops a load test and changes its settings, such as its users, [host](#host), ramps, [run time](#run-time) or [test plan](#test-plan). The [live dashboard](#live-dashboard) can do the same with [dashboard control](#dashboard-control). See [Controllers](./controller/overview.md).

## Coordinated Omission
The error a load test makes when its users wait on a stalled server and so send fewer requests, which hides the stall from the metrics. Goose can correct for it with `--co-mitigation`. See [Coordinated Omission](./coordinated-omission/overview.md).

## Dashboard Control
What the [live dashboard](#live-dashboard) can do with `--dashboard-control`: start and stop the test, change its users and settings, turn Save report off and on, delete saved runs, and quit Goose once the test is stopped. It also needs a [dashboard token](#dashboard-token) or `--dashboard-no-auth`. Without control the dashboard is read-only: it shows the run, [saves its report](#save-report) and lists saved runs in its [runs panel](#runs-panel), but cannot change the test. See [Turning it on](./controller/dashboard.md#turning-it-on).

## Dashboard Token
A secret set with `--dashboard-auth-token`. The [live dashboard](#live-dashboard) then requires it, and a browser gives it by opening the dashboard's address with `?token=<TOKEN>` added. See [Turning it on](./controller/dashboard.md#turning-it-on).

## Decrease Rate
How many [GooseUsers](#gooseuser) stop each second at the end of a load test, set with `--decrease-rate`; `--decrease-time` spreads the stops over a time instead. With neither, all users stop at once. See [Controlling the ramp-down rate](./getting-started/common.md#controlling-the-ramp-down-rate).

## Gaggle
Goose's distributed load test, in which one Manager and one or more Workers ran a single load test across several servers. It was removed in Goose 0.17.0 and will be restored in Goose 0.20, before the 1.0 release; for a distributed load test today, use Goose 0.16.4. See [Gaggle: Distributed Load Test](./gaggle/overview.md).

## Goose Eggs
A companion crate of helpers for validating responses and for load testing Drupal sites, with its own version of the Umami example. See [Goose Eggs](./getting-started/validation.md#goose-eggs).

## Goose Log
Goose's own log of what it is doing, written with `-G` (`--goose-log`), apart from the [logs](#logs) of requests and transactions. It holds only warnings and errors unless `-g` (`--log-level`) adds info messages. See [Output Files](./getting-started/output-files.md).

## GooseAttack
The load test itself: the [scenarios](#scenario) it runs and its configuration, built with `GooseAttack::initialize()` and run with `execute()`. See [Creating A Load Test](./getting-started/creating.md).

## GooseConfiguration
The structure that holds every run-time option of a load test, parsed from the command line, with defaults the load test can set in code with [`GooseDefault`](#goosedefault). See [Custom Options](./getting-started/custom.md).

## GooseDefault
The options a load test sets in its code with `set_default()`, such as a [host](#host) or a number of users. An option given on the command line wins. See [Defaults](./config/defaults.md).

## GooseError
The error returned by [GooseAttack](#gooseattack) methods such as `initialize()`, `set_default()` and `execute()`, for example when a run-time option is invalid or no [host](#host) is configured. A [transaction](#transaction) returns a [TransactionError](#transactionerror) instead. See [Creating A Load Test](./getting-started/creating.md).

## GooseMetrics
The metrics a load test collects. `execute()` returns them when the run ends, and the JSON report holds them. See [JSON report](./getting-started/metrics.md#json-report).

## GooseRequest
A request built in a [transaction](#transaction) with `GooseRequest::builder()`, for what the shortcuts such as `user.get()` and `user.post()` do not set, such as headers or the status code to expect. See [`GooseRequest`](https://docs.rs/goose/*/goose/goose/struct.GooseRequest.html) and the [Basic Authentication example](./example/basic-auth.md).

## GooseUser
A simulated user: an async task that runs one [scenario](#scenario) over and over until the load test ends. Unless its scenario sets a wait time, it runs its [transactions](#transaction) back to back, so it makes far more requests than one person would. `--users` sets how many start. See [How many users to simulate](./getting-started/common.md#how-many-users-to-simulate).

## Graph Data
The per second numbers behind a report's graphs. When Goose will write a report at the end of the run, it keeps them in memory for every second of the run; without one, it keeps only the last 10 minutes for the [live dashboard](#live-dashboard), and none when there is no dashboard. `--report-graph-data` also adds them to the JSON report. See [Graph data and memory](./getting-started/common.md#graph-data-and-memory).

## Heatmap
A chart of response times over time, on the [live dashboard](#live-dashboard) and in the PDF and HTML reports. Each column is a span of time and each row a range of response times; a cell's color shows how many requests took that long, and lines show each column's p50, p95 and p99. See [The response time heatmap](./controller/dashboard.md#the-response-time-heatmap).

## Hold
A time when the number of users stays steady: the [run time](#run-time) after a ramp up, or a [test plan](#test-plan) step that keeps the users it has. An `rps` [threshold](#threshold), on requests per second, counts only the holds. See [Thresholds](./getting-started/thresholds.md#rps-counts-only-the-time-users-hold-steady).

## Host
The base URL a load test sends its requests to, given with `-H` (`--host`). A request's path, such as `/about`, is added to it, so one load test can run against any site. See [Host to load test](./getting-started/common.md#host-to-load-test).

## Increase Rate
How many [GooseUsers](#gooseuser) start each second at the start of a load test, set with `--increase-rate`, one a second by default; `--increase-time` sets how long starting them all takes instead. See [Specifying the increase rate](./getting-started/common.md#specifying-the-increase-rate).

## Iteration
One run of a [GooseUser](#gooseuser) through its whole [scenario](#scenario). `--iterations` stops each user after that many. See [Specifying how long the load test will run](./getting-started/common.md#specifying-how-long-the-load-test-will-run).

## Killswitch
A way to stop a running load test from your own code, by calling `goose::trigger_killswitch` with a reason. Goose stops the run as it does for `ctrl-c`, writes every report, and records the run as canceled with that reason. See [Killswitch](./getting-started/killswitch.md).

## Live Dashboard
The web page Goose serves while a load test runs with `--dashboard`. Goose must be built with the `dashboard` feature, which is off by default. The page shows users, requests, response times and errors second by second. See [Live Dashboard](./controller/dashboard.md).

## Logs
Files that record what happens during a run, each turned on with its own option, such as `--request-log`. The [request](#request), [transaction](#transaction), [scenario](#scenario) and error logs record each one as it ends. The debug log holds what the load test writes with `log_debug()`, such as a failed request and its response. See [Logging](./logging/overview.md).

## Machine Name
A [scenario](#scenario)'s name in lower case letters and digits, with `_1`, `_2` added when two names would be the same. `--scenarios` uses it to pick the scenarios that run, and `--scenarios-list` prints it. See [Limiting Scenarios](./getting-started/scenarios.md).

## Metrics Reset
When the ramp up ends, Goose prints the metrics so far and then clears them, so the totals that follow leave out the ramp up. `--no-reset-metrics` turns the reset off, so the totals include the ramp up. A run limited by `--iterations` (see [Iteration](#iteration)) or shaped by a [test plan](#test-plan) never resets. See [Specifying how long the load test will run](./getting-started/common.md#specifying-how-long-the-load-test-will-run).

## Plan Overview
A table of each step the run went through, such as Increasing, Maintaining, Decreasing or Canceling, with when it started and ended and how many users ran. The PDF, HTML and Markdown reports show it under their header, and the console prints it last. See [Running A Load Test](./getting-started/running.md).

## Request
One HTTP request a [GooseUser](#gooseuser) makes, or an operation of another protocol recorded with `GooseUser::record_custom_request`, as in the [TCP example](./example/tcp.md). See [Creating A Load Test](./getting-started/creating.md).

## Request Name
The name a [request](#request) is counted under in the metrics: its path, unless the load test names the request, or its [transaction](#transaction) names its requests with `set_name_for_transaction_and_requests()`. See [Naming Transactions](./getting-started/creating.md#naming-transactions).

## Run Folder
The folder a run writes into when an output flag such as `--report-file` is given without a path, or when [Save report](#save-report) writes the run's report. It is the folder `--output-dir` names, or else `goose-runs/<YYYYMMDD-HHMMSS>`, named for the run's local start time. See [Run folders](./getting-started/common.md#run-folders).

## Run Time
How long every user keeps running once all have started, set with `--run-time`. Starting and stopping the users takes extra time, so a run lasts longer than its run time. See [Specifying how long the load test will run](./getting-started/common.md#specifying-how-long-the-load-test-will-run).

## Running Metrics
The metrics so far, printed to the console every N seconds during a run with `--running-metrics N`, and during the ramp up also each time a user starts. See [Run-Time Options](./getting-started/runtime-options.md).

## Runs Panel
The [live dashboard](#live-dashboard)'s list of runs: this run's [run folder](#run-folder) and the others beside it that hold a report, to download, compare or, with [dashboard control](#dashboard-control), delete. See [Save, compare and delete runs](./controller/dashboard.md#save-compare-and-delete-runs).

## Save Report
A run setting that writes the run's report into its [run folder](#run-folder) when the run ends. A `--report-file` given without a path turns it on, and so does `--dashboard` unless `--no-metrics` is set; the [live dashboard](#live-dashboard) shows it as a switch that [dashboard control](#dashboard-control) can turn off and on. See [Save, compare and delete runs](./controller/dashboard.md#save-compare-and-delete-runs).

## Scenario
A list of [transactions](#transaction) one [GooseUser](#gooseuser) runs, such as browsing anonymously or logging in to post. A load test can have many, each with a [weight](#weight). See [Creating A Load Test](./getting-started/creating.md).

## Scheduler
The order in which Goose assigns weighted [scenarios](#scenario) to new users and runs a scenario's [transactions](#transaction): round robin, serial or random. See [Scheduling Scenarios And Transactions](./config/scheduler.md).

## Session Data
Data a [GooseUser](#gooseuser) keeps between its requests, such as a token from a login, stored with `set_session_data`. See [Session](./example/session.md).

## Snapshot
The JSON state the [live dashboard](#live-dashboard) serves at `/api/v1/snapshot` and streams from `/api/v1/events`, about once a second. Its `version` field numbers the snapshot's format, not the Goose release. See [Dashboard Reference](./controller/dashboard-reference.md).

## Start And Stop Transactions
[Transactions](#transaction) that run once, outside a [scenario](#scenario)'s loop. One marked with `set_on_start()` runs when each user starts, and one marked with `set_on_stop()` when it stops. `GooseAttack::test_start()` registers one that runs once before the first user starts, and `test_stop()` one that runs once after the last user stops. See [`set_on_start`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html#method.set_on_start) and [`test_start`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.test_start).

## Synthetic Request
A response time that [coordinated omission](#coordinated-omission) mitigation adds for a request a user would have sent at its usual [cadence](#cadence) had the server not stalled. See [Coordinated Omission Metrics](./coordinated-omission/metrics.md).

## Test Plan
A list of steps that shapes a load test, given with `--test-plan`. Each step is a number of users and the time to reach it, written `users,time;users,time`, to build a ramp up, a [hold](#hold), a spike or a ramp down. See [Test Plan](./getting-started/test-plan.md).

## Threshold
A pass or fail rule on a load test's metrics, such as `p95<500ms` or `error-rate<1%`, given with `--threshold`. A rule that fails stops the run by default and always makes it fail. See [Thresholds](./getting-started/thresholds.md).

## Throttle
A limit on how many requests per second all [GooseUsers](#gooseuser) together send, set with `--throttle-requests`. See [Throttle](./getting-started/throttle.md).

## Transaction
A function that makes one or more [requests](#request) and checks the responses, such as loading a page and its static assets, or logging in. See [Creating A Load Test](./getting-started/creating.md).

## TransactionError
The error a [transaction](#transaction) returns when it fails. A failed request is counted once, with the request errors. A custom message, a bad URL, or a response body that cannot be read or decoded is counted as a transaction error. See [Transaction Error Types](./getting-started/validation.md#transaction-error-types).

## TransactionResult
The type a [transaction](#transaction) function returns, `Result<(), Box<TransactionError>>`: `Ok(())` on success, or a boxed [TransactionError](#transactionerror) on failure. See [Validating Requests](./getting-started/validation.md).

## Users Ramp
A run shaped by its users, [increase rate](#increase-rate), run time and ramp down ([decrease rate](#decrease-rate)), as opposed to a [test plan](#test-plan). When the [live dashboard](#live-dashboard) or a [Controller](#controller) changes its users mid run, Goose ramps to the new count and keeps the [run time](#run-time) and ramp down where it can. See [Change users and settings](./controller/dashboard.md#change-users-and-settings).

## Weight
How often Goose picks a [transaction](#transaction) within its scenario, or what share of users a [scenario](#scenario) gets. A transaction of weight 3 runs 3 times as often as one of weight 1. See [Scheduling Scenarios And Transactions](./config/scheduler.md).
