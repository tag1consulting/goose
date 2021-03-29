# Changelog

## 0.10.10-dev
 - capture errors and count frequency for each, including summary in metrics report; optionally disable with `--no-error-summary`

## 0.10.9 March 23, 2021
 - avoid unnecessary work on Manager when starting a Gaggle
 - respect `--hatch-rate` when starting a Gaggle
 - update httpmock for running tests
 - remove unnecessary `Result()` types where no error was possible

## 0.10.8 Feb 13, 2021
 - introduce `--report-file` (and `GooseDefault::ReportFile`) to optionally generate an HTML report when the load test completes
 - upgrade to `tokio` 1.x, and switch to `flume` for all multi-producer, multi-consumer channels
 - make `examples/umami` more generic for easier load testing of any Drupal 9 version of the demo install profile

## 0.10.7 Nov 16, 2020
 - account for time spent doing things other than sleeping, maintaining more consistency when displaying statistics and shutting down
 - start each debug log file with a line feed in case the page is too big for the buffer; increase the debug logger buffer size from 8K to 8M
 - introduce `--no-debug-body` flag to optionally prevent debug log from including the response body
 - rename the metrics file to requests file to better reflect what it is
    o `--metrics-file` becomes `--requests-file`
    o `--metrics-format` becomes `--requests-format`
    o `GooseDebug::MetricsFile` becomes `GooseDebug::RequestsFile`
    o `GooseDebug::MetricsFormat` becomes `GooseDebug::RequestsFormat`
 - reset drift timer any time the attack_phase changes
 - document all public high level files and functions

## 0.10.6 Nov 10, 2020
 - replace `--only-summary` with `--running-metrics <usize>`, running metrics are disabled by default
 - allow configuration of the algorithm used when allocating `GooseTaskSet`s to starting `GooseUser`s:
    o `GooseTaskSetScheduler::RoundRobin` allocates 1 of each available `GooseTaskSet` at a time (new default)
    o `GooseTaskSetScheduler::Serial` allocates all of each available `GooseTaskSet` in the order they are defined
    o `GooseTaskSetScheduler::Random` allocates 1 random `GooseTaskSet` from all available
 - when enabled, display running metrics for the entire duration of test, including ramp-up and shutdown

## 0.10.5 Nov 5, 2020
 - support floating point hatch rate (ie, hatch 1 user every 2 seconds with `-r .5`)

## 0.10.4 Nov 1, 2020
 - add new `examples/umami` for load testing Drupal 9 demo install profile
 - replace TermLogger with SimpleLogger for increased logging flexibility
 - add initial OCI Dockerfile for container-based workflows
 - use checked subtraction when calculating drift duration to prevent panic
 - update `nng-rs` dependency to fix bug when testing that the manager is ready
 
## 0.10.3 Oct 14, 2020
 - fixup sticky redirect tests to properly test functionality
 - add `test/sequence.rs` to confirm sequencing tests works correctly, even in Gaggle mode
 - deduplicate test logic by moving shared functionality into `tests/common.rs`; consistently test functionality both in standalone and Gaggle mode
 - properly create debug log when enabled in Gaggle mode

## 0.10.2 Sep 27, 2020
 - remove unnecessary `GooseAttack.number_of_cpus` instead calling `num_cpus::get()` directly
 - remove `tests/gaggle.rs`, instead mixing gaggle tests with per-feature integration tests
 - ensure `test_start` and `test_stop` run one and only one time even in Gaggle mode

## 0.10.1 Sep 20, 2020
 - rework `hatch_rate` to be stored in an `Option<usize>` as it can be `None` on a Worker
 - remove redundant `GooseAttack.users` instead using the `Option<usize>` in `configuration`
 - improve bounds handling of defaults, generate errors for invalid values
 - properly handle early shutdown of Gaggle distributed load test from Worker process
 - Manager starts timing Gaggle distributed load test only after all Workers start

## 0.10.0 Sep 13, 2020
 - default to resetting statistics, disable with `--no-reset-stats`, display spawning statistics before resetting
 - only run gaggle integration tests when feature is enabled
 - prevent time-drift when launching users and throttling requests
 - add per-task statistics in addition to per-request statistics, disable with `--no-task-stats`
 - rename `stats` and `statistics` to `metrics` for consistency and clarity
    o `--no-stats` became `--no-metrics`
    o `--no-reset-stats` became `--no-reset-metrics`
    o `--no-task-stats` became `--no-task-metrics`
    o `--stats-log-file` became `--metrics-log-file`
    o `--stats-log-format` became `--metrics-log-format`
 - shorten some configuration options to fit standard console width, preparation for switch to gumdrop
    o `--debug-log-file` became `--debug-file`
    o `--debug-log-format` became `--debug-format`
    o `--metrics-log-file` became `--metrics-file`
    o `--metrics-log-format` became `--metrics-format`
 - reworded errors for consistency, made error.detail required
 - replace `structopt` with `gumdrop`
    o restructured help page to logically group related options
    o rewrote/simplified configuration descriptions to fit standard console width
 - update prelude documentation
 - increase precision of metrics for smaller values
 - consistently build configuration from arguments
 - replace `GooseAttack::set_host()` with more generic `GooseAttack::set_default()`, exposes the following defaults:
    o default host: `GooseDefault::Host` (&str)
    o default users to start: `GooseDefault::Users` (usize)
    o default users to start per second: `GooseDefault::HatchRate` (usize)
    o default number of seconds for test to run: `GooseDefault::RunTime` (usize)
    o default log level: `GooseDefault::LogLevel` (usize)
    o default log file name: `GooseDefault::LogFile` (&str)
    o default verbosity: `GooseDefault::Verbose` (usize)
    o default to only printing final summary metrics: `GooseDefault::OnlySummary` (bool)
    o default to not resetting metrics after all users start: `GooseDefault::NoResetMetrics` (bool)
    o default to not tracking metrics: `GooseDefault::NoMetrics` (bool)
    o default to not tracking task metrics: `GooseDefault::NoTaskMetrics` (bool)
    o default metrics log file name: `GooseDefault::MetricsFile` (&str)
    o default metrics log file format: `GooseDefault::MetricsFormat` (&str)
    o default debug log file name: `GooseDefault::DebugFile` (&str)
    o default debug log file format: `GooseDefault::DebugFormat` (&str)
    o default to tracking status codes: `GooseDefault::StatusCodes` (bool)
    o default maximum requests per second: `GooseDefault::ThrottleRequests` (usize)
    o default to following redirect of base_url: `GooseDefault::StickyFollow` (bool)
    o default to enabling Manager mode: `GooseDefault::Manager` (bool)
    o default number of Workers to expect: `GooseDefault::ExpectWorkers` (usize)
    o default to ignoring load test checksum: `GooseDefault::NoHashCheck` (bool)
    o default host to bind Manager to: `GooseDefault::ManagerBindHost` (&str)
    o default port to bind Manager to: `GooseDefault::ManagerBindPort` (usize)
    o default to enabling Worker mode: `GooseDefault::Worker` (bool)
    o default host for Worker to connect to: `GooseDefault::ManagerHost` (&str)
    o default port for Worker to connect to: `GooseDefault::ManagerPort` (usize)

## 0.9.1 Aug 1, 2020
 - return `GooseStats` from `GooseAttack` `.execute()`
 - rework as methods of `GooseStats`: `.print()`, `.print_running()`, `fmt_requests()`,
   `fmt_response_times()`, `fmt_percentiles()`, and `fmt_status_codes()`
 - display `GooseStats` with fmt::Display (ie `print!("{}", goose_stats);`)
 - make it possible to pass a closure to GooseTask::new
 - fix display of `GooseError` and `GooseTaskError`

## 0.9.0 July 23, 2020
 - fix code documentation, requests are async and require await
 - properly support setting host when registering task set 
 - rename `response` wrapper to `goose`, so we end up with `goose.request` and `goose.response`
 - add `--throttle-requests` to optionally limit the maximum requests per second (api change)
 - introduce `GooseError` and `GooseTaskError`
 - change task function signature, tasks must return a `GooseTaskResult`
 - change `GooseAttack` method signatures where an error is possible
 - where possible, pass error up the stack instead of calling `exit(1)`
 - introduce `GooseAttack.display()` which consumes the load test state and displays statistics
 - `panic!()` on unexpected errors instead of `exit(1)`

## 0.8.2 July 2, 2020
 - `client.log_debug()` will write debug logs to file when specified with `--debug-log-file=`
 - add `-debug-log-format=` to switch between `json` (default) and `raw` formats
 - cleanup code with clippy, automate clippy with PRs
 - add optional compile-time `rustls` feature

## 0.8.1 June 30, 2020
 - sort stats by method:name to ease comparisons
 - optionally log all requests to file specified with `--stats-log-file=`
 - add `--stats-log-format=` to switch between `json` (default), `csv` and `raw` formats

## 0.8.0 June 26, 2020
 - properly subtract previous statistic when handling `set_failure()` and `set_success()`
 - detect and track redirects in `GooseRawRequest`
 - `--sticky-follow` makes redirect of GooseClient base_url sticky, affecting subsequent requests
 - changed `GooseClient` to `GooseUser`

## 0.7.5 June 10, 2020
 - store actual URL requested in GooseRawRequest
 - add `set_client_builder`, allow load test to build Reqwest clients with custom options
 - properly fix documentation links

## 0.7.4 June 5, 2020
 - fix gaggles to not panic, add test
 - fix test_start and test_stop to not panic, add tests
 - optimize NNG usage, write directly to Message instead of first to buffer
 - fix documentation links

## 0.7.3 June 5, 2020
 - move client out of GooseClient into global GooseClientState
 - introduce `test_start_task` and `test_stop_task` allowing global setup and teardown
 - don't panic if a load test doesn't define any normal tasks
 - pass immutable GooseClient to tasks
 - integrate httpmock into testing load test

## 0.7.2 June 1, 2020
 - don't shuffle order of weighted task sets when launching clients
 - remove GooseClientMode as it serves no useful purpose
 - push statistics from client threads to parent in real-time
 - simplify `set_failure` and `set_success` to pass in request

## 0.7.1 May 26, 2020
 - no longer compile Reqwest blocking client
 - remove need to declare `use std::boxed::Box` in load tests
 - remove unnecessary mutexes
 - introduce `use goose::prelude::*`

## 0.7.0 May 25, 2020
 - initial async support

## 0.6.3-dev
 - nng does not support udp as a transport protocol, and tcp overhead isn't
   problematic; remove to-do to add udp, hard-code tcp
 - add worker id for tracing gaggle worker threads
 - cleanup gaggle logic and comments

## 0.6.2 May 18, 2020
 - replace `unsafe` code blocks with `lazy_static` singleton
 - perform checksum to confirm workers are running same load test,
   `--no-hash-check` to ignore
 - code and documentation consistency

## 0.6.1 May 16, 2020
 - replace `--print-stats` with `--no-stats`, default to printing stats
 - make gaggle an optional compile-time feature
 - GooseState is now GooseAttack

## 0.6.0 May 14, 2020
 - Initial support for gaggles: distributed load testing
