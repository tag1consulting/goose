# Changelog

## 0.17.3-dev
 - [#565](https://github.com/tag1consulting/goose/pull/565) add `--accept-invalid-certs` to skip validation of https certificates
 - [#568](https://github.com/tag1consulting/goose/pull/568) don't panic when truncating non utf-8 string
 - [#574](https://github.com/tag1consulting/goose/pull/574) update [`http`](https://docs.rs/http), [`itertools`](https://docs.rs/itertools) [`nix`](https://docs.rs/nix), [`rustls`](https://docs.rs/rustls/), and [`serial_test`](https://docs.rs/serial_test)
 - [#575](https://github.com/tag1consulting/goose/pull/575) add test coverage for sessions and cookies, revert [#557](https://github.com/tag1consulting/goose/pull/557) to avoid sharing the CookieJar between all users

## 0.17.2 August 28, 2023
 - [#557](https://github.com/tag1consulting/goose/pull/557) speed up user initialization on Linux
 - [#559](https://github.com/tag1consulting/goose/pull/559) disable unnecessary features in chronos, avoid potential segfault in time crate: https://rustsec.org/advisories/RUSTSEC-2020-0071

## 0.17.1 August 17, 2023
 - [#543](https://github.com/tag1consulting/goose/pull/543) remove external dependency on num_cpus(), use instead built-in available_parallelism() added in rust 1.59.0
 - [#552](https://github.com/tag1consulting/goose/pull/552) add `scenario_index`, `scenario_name`, `transaction_index` and `transaction_name` to the request log
 - [#553](https://github.com/tag1consulting/goose/pull/553) remove `serde_cbor` dependency no longer required due to [#529]
 - [#554](https://github.com/tag1consulting/goose/pull/554) update `flume`, `itertools`, `strum`, `strum_macros`, `tokio-tungstenite`, and `tungestenite` dependencies to latest versions
 - [#555](https://github.com/tag1consulting/goose/pull/555) don't panic when report has no data

## 0.17.0 December 9, 2022
 - [#529](https://github.com/tag1consulting/goose/pull/529) **API change** temporaryily removed Gaggle support `gaggle` feature) to allow upgrading Tokio and other dependencies.
   - if you require Gaggle support, use Goose 0.16.4 with Tokio 0.15 for now; Gaggle support is being added back in https://github.com/tag1consulting/goose/pull/509
   - updated Tokio to 1.23, updated tungestenite and tokio-tungstenite to 0.18; updated ctrlc to 3.2; updated num_cpus to 1.14, updated simplelog to 0.12, updated nix to 0.26, updated rustls to 0.20, updates serial_test to 0.9
   - removed `nng` dependency and `gaggle` feature
   - removed `--manager`, `--expect-workers`, `--no-hash-check`, `--manager-bind-host`, `--manager-bind-port`, `--worker`, `--manager-host`, `--manager-port` and related configuration defaults
   - removed `AttackMode::Manager` and `AttackMode::Worker`
   - ignore all Gaggle tests, will re-enable these tests when Gaggle support is re-implemented
   - box `TransactionError` to avoid over-allocating memory on the stack (see `examples/session.rs` for an example of working with this)

## 0.16.4 September 20, 2022
 - [#512](https://github.com/tag1consulting/goose/pull/512) include proper HTTP method and path in logs and html report when using `GooseRequest::builder()`
 - [#514](https://github.com/tag1consulting/goose/pull/514) fix panic when an empty wait time interval is set
 - [#516](https://github.com/tag1consulting/goose/pull/516) fix unescaped inner quotes in csv logs
 - [#519](https://github.com/tag1consulting/goose/pull/519) implement `Default` for `GooseConfiguration`
 - [#522](https://github.com/tag1consulting/goose/pull/522) display times on the report in local time (instead of UTC)

## 0.16.3 July 17, 2022
 - [#498](https://github.com/tag1consulting/goose/issues/498) ignore `GooseDefault::Host` if set to an empty string
 - [#487](https://github.com/tag1consulting/goose/pull/487) add dev-dependency on (nix)[https://docs.rs/nix] to provide test coverage confirming proper shutdown from SIGINT (ctrl-c); capture ctrl-c in a lazy_static wrapped in a RwLock so it can be reset
 - [#489](https://github.com/tag1consulting/goose/pull/489) don't panic when writing report file and shutting down with controller
 - [#505](https://github.com/tag1consulting/goose/pull/505) introduce `--scenarios` (and `GooseDefault::Scenarios`) so a subset of scenarios can be launched, and `--scenarios-list` to display internal machine names for matching

## 0.16.2 May 20, 2022
 - [#477](https://github.com/tag1consulting/goose/pull/477) introduce `--iterations` (and `GooseDefault::Iterations`) which configures each GooseUser to run a configurable number of iterations of the assigned Scenario then exit; introduces Scenario metrics which can be disabled with `--no-scenario-metrics` (`GooseDefault::NoScenarioMetrics`); introduces `--scenario-log` and `--scenario-format` (and `GooseDefault::ScenarioLog` and `GooseDefault::ScenarioFormat`)
 - [#483](https://github.com/tag1consulting/goose/pull/483) remove duplicate help (-h) output

## 0.16.1 May 12, 2022
 - [#464](https://github.com/tag1consulting/goose/pull/464) add `startuptime` (and `startup_time`) TIME to controllers, setting how long the load test should spend starting configured number of users
 - [#469](https://github.com/tag1consulting/goose/pull/469) support `users` INT command on controllers during a running load test
 - [#473](https://github.com/tag1consulting/goose/pull/473) introduce `test-plan PLAN` command allowing configuration of test plan with the controller during running and idle load tests

## 0.16.0 May 1, 2022
 - [#431](https://github.com/tag1consulting/goose/pull/431) rename `--no-granular-data` to `--no-granular-report`
 - [#415](https://github.com/tag1consulting/goose/pull/415) display granular data in HTML graphs, introduce `--no-granular-data` to disable it and display graphs as they were until this change
 - [#406](https://github.com/tag1consulting/goose/pull/406) make sure that the graphs are built correctly if the load test is interrupted during the starting phase
 - [#408](https://github.com/tag1consulting/goose/pull/408) update 'Running the load test' page in the Goose book to show HTML report
 - [#411](https://github.com/tag1consulting/goose/pull/411) **API change**: some public APIs have been made private or removed
   o `util::MovingAverage` structure and all related functions have been moved to a different namespace and made private
   o `GooseRequestMetricAggregate::requests_per_second`, `GooseRequestMetricAggregate::errors_per_second` and `GooseRequestMetricAggregate::average_response_time_per_second` have been removed
   o `GooseTaskMetricAggregate::tasks_per_second` has been removed
   o `GooseMetrics::users_per_second` has been removed
   o formerly public methods `report::task_metrics_template()` and `report::errors_template()` have been made private
   o `report::graph_rps_template()`, `report::graph_eps_template()`, `report::graph_average_response_time_template()`, `report::graph_users_per_second_template()` and `report::graph_tasks_per_second_template()` have been removed
 - [#379](https://github.com/tag1consulting/goose/pull/379) **API change**: default to `INFO` level verbosity, introduce `-q` to reduce Goose verbosity
   o **note**: `-v` now sets Goose to `DEBUG` level verbosity which when enabled will negatively impact load test performance; set `-q` to restore previous level of verbosity
 - [#379](https://github.com/tag1consulting/goose/pull/379) **API change**: remove `.print()` which is no longer required to display metrics after a load test, disable with `--no-print-metrics` or `GooseDefault::NoPrintMetrics`
 - [#422](https://github.com/tag1consulting/goose/pull/422) **API change**: introduce `--test-plan` and `GooseDefault::TestPlan`
    o internally represent all load tests as `Vec<(usize, usize)>`l test plan
    o use [FromStr] to auto convert --test-plan "{users},{timespan};{users},{timespan}", where {users} must be an integer, ie "100", and {timespan} can be integer seconds or "30s", "20m", "3h", "1h30m", etc, to internal Vec<(usize, usize)> representation
    o don't allow `--test-plan` together with `--users`, `--startup-time`, `--hatch-rate`, `--run-time`, `--no-reset-metrics`, `--manager` and `--worker`
    o internal `AttackPhase`s renamed: `Starting` -> `Increase`, `Running` -> `Maintain`, `Stopping` -> `Decrease`
 - [#449](https://github.com/tag1consulting/goose/pull/449) **API change**: rename `GooseTaskSet` -> `Scenario`, `GooseTask` -> `Transaction`, `GooseTaskResult` -> `TransationResult`, `GooseTaskEror` -> `TransactionError`, `WeightedGooseTasks` -> `WeightedTransactions`, `GooseTaskFunction` -> `TransactionFunction`, `test_start_task` -> `test_start_transaction`, `test_stop_task` -> `test_stop_transaction`, `register_task` -> `register_transaction`, `task!` -> `transaction!`, `--no-task-metrics` -> `--no-transaction-metrics`, `GooseTaskError` -> `TransactionError`
 - [#450](https://github.com/tag1consulting/goose/pull/450) add support for variable speed and multiple decrease AttackPhases
 - [#452](https://github.com/tag1consulting/goose/pull/452) **API change**: rename `--status-codes` to `--no-status-codes` and enable collecation and summary of status codes by default

## 0.15.2 December 13, 2021
 - [#391](https://github.com/tag1consulting/goose/pull/391) properly sleep for configured `set_wait_time()` walking regularly to exit quickly if the load test ends
 - [#394](https://github.com/tag1consulting/goose/pull/394) add additional graphs to the HTML report: errors per second, average response time, active users, active tasks
 - [#403](https://github.com/tag1consulting/goose/pull/403) wake up a couple times a second to handle message and allow for a quick shutdown if the load test is canceled during startup

## 0.15.1 November 19, 2021
 - [#374](https://github.com/tag1consulting/goose/pull/374) renamed `simple-with-session.rs` to `session.rs` and `simple-closure.rs` to `closure.rs` to avoid confusion with the `simple.rs` example as they all do different things
 - [#385](https://github.com/tag1consulting/goose/pull/385) properly configure `--running-metrics VALUE` when set manually
 - [#382](https://github.com/tag1consulting/goose/pull/382) set client timeout to 60 seconds by default, used for all requests made; introduce `--timeout VALUE` where VALUE is seconds as integer or a float; timeout can be configured programatically using `GooseDefault::Timeout`
 - [#381](https://github.com/tag1consulting/goose/pull/381) display requests per second graph on the HTML report

## 0.15.0 November 2, 2021
 - [#372](https://github.com/tag1consulting/goose/pull/372) de-deduplicate documentation, favoring [The Goose Book](https://book.goose.rs)
 - [#373](https://github.com/tag1consulting/goose/pull/373) **API change**: introduce `GooseRequest` and `GooseRequestBuilder` for more flexibility when making requests
    o remove `GooseUser::post_named`, `GooseUser::head_named`, `GooseUser::delete_named`, `GooseUser::goose_get`, `GooseUser::goose_put`, `GooseUser::goose_head`, `GooseUser::goose_put`, `GooseUser::goose_patch`, `GooseUser::goose_delete`, and `GooseUser::goose_send`
    o adds or modifies helpers `GooseUser::get`, `GooseUser::get_named`, `GooseUser::post`, `GooseUser::post_form`, `GooseUser::post_json`, `GooseUser::head`, and `GooseUser::delete`
    o replaces `GooseUser::goose_send` with `GooseUser::request` which accepts a `GooseRequest` object
    o fixes [#370] (see `GooseRequestBuilder::expect_status_code`)

## 0.14.1 October 13, 2021
 - [#364](https://github.com/tag1consulting/goose/pull/364) add link from the [Developer Documentation](https://docs.rs/goose) to [The Git Book](https://book.goose.rs)
 - [#368](https://github.com/tag1consulting/goose/pull/368) **fix performance regression**: optimize fastpath if no delay between tasks

## 0.14.0 September 15, 2021
 - [#361](https://github.com/tag1consulting/goose/pull/361) convert `README.md` (and enhance) into [`The Goose Book`](https://book.goose.rs/)
 - [#356](https://github.com/tag1consulting/goose/pull/356) **API change**: make `GooseAttack.execute` async, `main()` function signature changed to:
    ```rust
    #[tokio::main]
    fn main() -> Result<(), GooseError> {
    ```
 - [#355](https://github.com/tag1consulting/goose/pull/355) **API change**: add the possibility to attach custom session data `GooseUserData` to each `GooseUser`
 - [#355](https://github.com/tag1consulting/goose/pull/355) **API change**: change `GooseTask` signature to take a mutable reference of `GooseUser`:
     ```rust
     async fn example_task_function(user: &mut GooseUser) -> GooseTaskResult {
     ```
 - [#358](https://github.com/tag1consulting/goose/pull/358) **API change**: update `GooseTaskSet::set_wait_time()` to accept `std::time::Duration` instead of `usize` allowing more granularity
 - [#355](https://github.com/tag1consulting/goose/pull/355) remove `Clone` trait from `GooseUser` and `GooseAttack`
 - [#359](https://github.com/tag1consulting/goose/pull/359) use request name when displaying errors to avoid having a large volume of distinct error for the same endpoint when using path parameters
 - [#360](https://github.com/tag1consulting/goose/pull/360) updated `tungstenite` dependency to [`0.15`](https://github.com/snapview/tungstenite-rs/blob/master/CHANGELOG.md)

## 0.13.3 August 25, 2021
 - [#351](https://github.com/tag1consulting/goose/pull/351) document GooseConfiguration fields that were only documented as gumpdrop parameters (in order to generate new lines in the help output) so now they're also documented in the code
 - [#353](https://github.com/tag1consulting/goose/pull/353) fix panic when `--no-task-metrics` is enabled and metrics are printed; add tests to prevent further regressions

## 0.13.2 August 19, 2021
 - [#349](https://github.com/tag1consulting/goose/pull/349), [#345](https://github.com/tag1consulting/goose/pull/345) fix broken links within the documentation; general documentation cleanups
 - [#348](https://github.com/tag1consulting/goose/pull/348) introduce `--startup-time` which can be set together with `--users` instead of using `--hatch-rate` to configure how quickly to start users
 - [#348](https://github.com/tag1consulting/goose/pull/348) fix `--run-time` to always start counting after all users are fully started
 - [#348](https://github.com/tag1consulting/goose/pull/348) include starting and stopping time in addition to running time in text metrics and html report

## 0.13.1 August 13, 2021
 - [#338](https://github.com/tag1consulting/goose/pull/338) add test to confirm a `base_url` can include a path and be joined with a relative path
 - [#339](https://github.com/tag1consulting/goose/pull/339) fix documentation typo
 - [#340](https://github.com/tag1consulting/goose/pull/340) introduce `pretty` log format for `--error-format`, `--debug-format`, `--request-format`, and `--task-format`
  - [#341](https://github.com/tag1consulting/goose/pull/341) clippy cleanups: don't borrow references that are immediately dereferenced by the compiler: https://rust-lang.github.io/rust-clippy/master/index.html#needless_borrow
  - [#342](https://github.com/tag1consulting/goose/pull/342) consistently report users simulated, target host(s), start and end times, and total duration of test both in text metrics and html report
  - [#343](https://github.com/tag1consulting/goose/pull/343) updated httpmock dev dependency to [`0.6`](https://github.com/alexliesenfeld/httpmock/blob/master/CHANGELOG.md)

## 0.13.0 July 19, 2021
  - [#334](https://github.com/tag1consulting/goose/pull/334) **API change**: introduce `GooseRawMetric` which contains the `method`, `url`, `headers` and `body` of the client request made, and is now contained in `raw` field of the `GooseRequestMetric`
  - [#328](https://github.com/tag1consulting/goose/pull/328) enable [`gzip`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.gzip) support and set Accept-Encoding header by default in the client; disable with `--no-gzip` or `GooseDefault::NoGzip`
  - [#330](https://github.com/tag1consulting/goose/pull/330) document how to add custom cookies (https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#custom-cookies)
  - [#331](https://github.com/tag1consulting/goose/pull/331) update [`rustc_version`](https://docs.rs/rustc_version) dependency to `0.4`
  - [#334](https://github.com/tag1consulting/goose/pull/334) include client request headers in `GooseRequestMetric` so they show up in the request log and the debug log
  - [#334](https://github.com/tag1consulting/goose/pull/334) introduce `--request-body` (and `GooseDefault::RequestBody`) which when enabled shows up in the `body` field of the `GooseRawMetric`
  - [#334](https://github.com/tag1consulting/goose/pull/334) add `GooseRawMetric` to the request log, debug log and error log

## 0.12.1 July 15, 2021
 - rename `rustls` feature to `rustls-tls` so `tests/controller.rs` can build with the `rustls` library; update `tungstenite` to `0.14` and `tokio-tungstenite` = `0.15` to allow building with `rustls`
  - documentation cleanup; properly rename `GooseDefault::RequestFormat` and fix links
  - always configure `GooseConfiguration.manager` and `GooseConfiguration.worker`; confirm Manager is enabled when setting `--expect-workers`
  - moved `GooseConfiguration`, `GooseDefault`, and `GooseDefaultType` into new `src/config.rs` file; standardized configuration precedence through internal `GooseConfigure` trait defining `get_value()` for all supported types; general improvements to configuration documentation

## 0.12.0 July 8, 2021
 - **API change**: remove internal-only functions and structures from documentation, exposing only what's useful to consumers of the Goose library
    o `goose::initialize_logger()`, `Socket` reduced to `pub(crate)` scope
    o `goose::controller::GooseControllerProtocol`, `GooseControllerRequestMessage`, `GooseControllerResponseMessage`, `GooseControllerRequest`, `GooseControllerResponse`, `GooseControllerState`, `::controller_main()` reduced to `pub(crate)` scope
    o `goose::manager::manager_main()` reduced to `pub(crate)` scope
    o `goose::metrics::GooseRequestMetric::new()`, `::set_final_url()`, `::set_response_time()`, and `::set_status_code()`, `::per_second_calculations()`, `format_number()`, `merge_times()`, `update_min_time()`, `update_max_time()`, `calculate_response_time_percentile()`, and `prepare_status_codes()` reduced to `pub(crate)` scope
    o `goose::metrics::GooseRequestMetricAggregate::new()`, `::set_response_time()`, and `::set_status_code()` reduced to `pub(crate)` scope
    o `goose::metrics::GooseTaskMetric::new()` and `::set_time()` reduced to `pub(crate)` scope
    o `goose::metrics::GooseMetrics::initialize_task_metrics()` and `::print_running()`, `::fmt_requests()`, `::fmt_tasks()`, `::fmt_task_times()`, `::fmt_response_times()`, `::fmt_percentiles()`, `::fmt_status_codes()` and `::fmt_errors()` reduced to `pub(crate)` scope
    o from `goose::metrics::GooseMetrics` reduced `final_metrics`, `display_status_codes` and `display_metrics` fields to `pub(crate)` scope
    o `goose::metrics::GooseErrorMetric::new()` reduced to `pub(crate)` scope
    o `goose::logger::logger_main()` reduced to `pub(crate)` scope
    o `goose::user::user_main()` reduced to `pub(crate)` scope
    o `goose::worker::worker_main()` reduced to `pub(crate)` scope
 - **API change**: move all metrics-related stuctures and methods into `metrics.rs`, rename for consistency, and improve documentation
    o `goose::GooseRawRequest` changed to `goose::metrics::GooseRequestMetric`
    o `goose::GooseRequest` changed to `goose::metrics::GooseRequestMetricAggregate`
    o `goose::GooseRawTask` changed to `goose::metrics::GooseTaskMetric`
    o `goose::GooseRawTask` changed to `goose::metrics::GooseTaskMetricAggregate`
    o `goose::update_duration()` changed to `goose::metrics::update_duration()` and reduced to `pub(crate)` scope
    o `goose::sync_metrics()` changed to `goose::metrics::sync_metrics()` and reduced to `pub(crate)` scope
    o `goose::reset_metrics()` changed to `goose::metrics::reset_metrics()` and reduced to `pub(crate)` scope
    o `goose::receive_metrics()` changed to `goose::metrics::receive_metrics()` and reduced to `pub(crate)` scope
    o `goose::record_error()` changed to `goose::metrics::record_error()` and reduced to `pub(crate)` scope
 - expose utility functions used by Goose for use by load tests
    o `goose::util::parse_timespan()`, `::gcd()`, `::median()`, `::truncate_string()`, `::timer_expired()`, `::ms_timer_expired()`, `::get_hatch_rate()`, and `::is_valid_host()` were elevated to `pub` scope
 - introduce (disabled by default) Coordinated Omission Mitigation, configured through `--co-mitigation` with the following options: "disabled" (default0), "average", "minimum", "maximum"; (or with `GooseDefault::CoordinatedOmissionMitigation`)
  - (EXPERIMENTAL) Coordinated Omission Mitigation tracks the cadence that a GooseUser loops through all GooseTasks, (also accounting for time spent sleeping due to `.set_wait_time()`); it detects stalls (network or upstream server) that block and prevent other requests from running, and backfills the metrics to mitigate this loss of data ([based on the general implementation found in HdrHistogram](https://github.com/HdrHistogram/HdrHistogram_rust/blob/9c09314ac91848fd696b699892414cb337d9abce/src/lib.rs#L916)
  - When displaying metrics (via the cli and the html report) show both "raw" (actual) metrics and "coordinated omission mitigation" (back-filled with statistically generated) metrics, and the standard deviation between the average times for each
  - introduce `GooseLog` enum for sending `GooseDebug`, `GooseRequestMetric` and `GooseTaskMetric` objects to the Logger thread for logging to file
  - introduce `--tasks-file` run-time option for logging `GooseTaskMetric`s to file
  - rename `GooseTaskMetric` to `GooseTaskMetricAggregate`, and introduce `GooseTaskMetric` which is a subset of `GooseRequestMetric` only used for logging
  - introduce `--error-file` run-time option for logging `GooseErrorMetric`s to file
  - introduce `GooseLogFormat` enum for formatting all logs; add `--task-format` and `--error-format` using new enum, update `--requests-format` and `--debug-format`.
  - renamed `--log-file` to `--goose-log`, `--requests-file` to `--request-log`, `--requests-format` to `--request-format`, `--tasks-file` to `--task-log`, `--tasks-format` to `--task-format`, `--error-file` to `--error-log`, and `--debug-file` to `--debug-log`

## 0.11.2 June 10, 2021
 - introduce telnet Controller allowing real-time control of load test, optionally disable with `--no-telnet`, supports the following commands:
    o `help` (and `?`) display help
    o `exit` (and `quit`) exit the telnet Controller
    o `shutdown` shuts down the running load test (and exits the controller)
    o `host` (and `hosts`) HOST sets host to load test against, ie http://localhost/
    o `users` (and `user`) INT sets number of simulated users
    o `hatchrate` (and `hatch_rate`) FLOAT sets per-second rate users hatch
    o `runtime` (and `run_time`) TIME sets how long the load test should run
    o `config` displays the current load test configuration
    o `config-json` displays the current load test configuration in json format
    o `metrics` (and `stats`) displays metrics for the current load test
    o `metrics-json` (and `stats-json`) displays metrics for the current load test in json format
 - telnet Controller bind host defaults to `0.0.0.0`, can be configured with `--telnet-host`
 - telnet Controller bind port defaults to `5116`, can be configured with `--telnet-port`
 - telnet Controller defaults can be changed:
    o default to not enabling telnet Controller: `GooseDefault::NoTelnet` (bool)
    o default host to bind telnet Controller to: `GooseDefault::TelnetHost` (&str)
    o default port to bind telnet Controller to: `GooseDefault::TelnetPort` (usize)
 - introduce WebSocket Controller allowing real-time control of load test, optionally disable with `--no-websocket`, supports the same commands as the telnet Controller, except:
    o `config` and `config-json` both return the load test configuration in json format
    o `metrics` and `metrics-json` both return metrics for the current load test in json format
 - WebSocket Controller bind host defaults to `0.0.0.0`, can be configured with `--websocket-host`
 - WebSocket Controller bind port defaults to `5117`, can be configured with `--websocket-port`
 - WebSocket Controller defaults can be changed:
    o default to not enabling WebSocket Controller: `GooseDefault::NoWebSocket` (bool)
    o default host to bind WebSocket Controller to: `GooseDefault::WebSocketHost` (&str)
    o default port to bind WebSocket Controller to: `GooseDefault::WebSocketPort` (usize)
 - make it possible to start and stop a load test without completely restarting Goose
 - introduce `--no-autostart` to disable automatically starting the load test, leaves Goose in an idle state waiting for Controller commands (optionally change the default with `GooseDefault::NoAutoStart`)
    o renamed `stop` Controller command to `shutdown`
    o added new `start` Controller command, telling idle Goose load test to start
    o added new `stop` Controller command, telling running Goose load test to stop and return to idle state
 - code cleanup and logic consollidation to support Controller fixed a bug where metrics wouldn't display and the debug file, request file, and html report weren't written when load test was stopped while still launching users
 - regularly sync metrics, using a timeout to avoid hanging the main loop
 - properly reset metrics when load test is stopped and restarted
 - properly flush debug file, request file, and html report when stopping load test with Controller
 - properly (re)create debug file, request file, and html report when starting load test with Controller
 - if metrics are enabled, display when controller stops load test
 - de-duplicate code with traits, gaining compile-time validation that both Controllers are properly handling all defined commands
 - add [`async_trait`](https://docs.rs/async_trait) dependency as stable Rust doesn't otherwise support async traits
 - allow starting Goose without specifying a host if `--no-autostart` is enabled, requiring instead that the host be configured via a Controller before starting a load test
 - add test for telnet and WebSocket Controllers

## 0.11.1 May 16, 2021
 - update [`rand`](https://docs.rs/rand) dependency to `0.8` branch, update [`gen_range`](https://docs.rs/rand/0.8.*/rand/trait.Rng.html#method.gen_range) method call
 - update dependencies: [`itertools`](https://docs.rs/itertools) to `0.10`, [`simplelog`](https://docs.rs/simplelog) to `0.10`, [`url`](https://docs.rs/url) to `2`
 - update [`nng`](https://docs.rs/nng) dependency for optional `gaggle` feature
 - simplify [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami) regex when parsing form
 - allow configuration of algorithm for allocating `GooseTask`s the same as `GooseTaskSet`s; `GooseTaskSetScheduler` becomes more generically `GooseScheduler`
 - specify (and detect) minimum `rustc` requirement of `1.49.0`, due to [`flume`](https://docs.rs/flume) dependency which in turn depends on [`spinning_top`](https://docs.rs/spinning_top) which uses [`hint::spin_loop`](https://doc.rust-lang.org/std/hint/fn.spin_loop.html) which stabilized in `rustc` version `1.49.0
 - standardize links in documentation; general documentation cleanups

## 0.11.0 April 9, 2021
 - capture errors and count frequency for each, including summary in metrics report; optionally disable with `--no-error-summary`
 - clippy cleanups (prepare for Rust 2021 https://blog.rust-lang.org/inside-rust/2021/03/04/planning-rust-2021.html):
    o **API change**: all `GooseMethod`s renamed to enforce Rust naming conventions in regards to case, for example `GooseMethod::GET` becomes `GooseMethod::Get`
    o use `vec![]` macro to avoid unnecessarily pushing data into mutable vectors
    o call `format!` macro directly for improved readability
    o remove unnecessary `panic!`

## 0.10.9 March 23, 2021
 - avoid unnecessary work on Manager when starting a Gaggle
 - respect `--hatch-rate` when starting a Gaggle
 - update httpmock for running tests
 - remove unnecessary `Result()` types where no error was possible

## 0.10.8 February 13, 2021
 - introduce `--report-file` (and `GooseDefault::ReportFile`) to optionally generate an HTML report when the load test completes
 - upgrade to `tokio` 1.x, and switch to `flume` for all multi-producer, multi-consumer channels
 - make `examples/umami` more generic for easier load testing of any Drupal 9 version of the demo install profile

## 0.10.7 November 16, 2020
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

## 0.10.6 November 10, 2020
 - replace `--only-summary` with `--running-metrics <usize>`, running metrics are disabled by default
 - allow configuration of the algorithm used when allocating `GooseTaskSet`s to starting `GooseUser`s:
    o `GooseTaskSetScheduler::RoundRobin` allocates 1 of each available `GooseTaskSet` at a time (new default)
    o `GooseTaskSetScheduler::Serial` allocates all of each available `GooseTaskSet` in the order they are defined
    o `GooseTaskSetScheduler::Random` allocates 1 random `GooseTaskSet` from all available
 - when enabled, display running metrics for the entire duration of test, including ramp-up and shutdown

## 0.10.5 November 5, 2020
 - support floating point hatch rate (ie, hatch 1 user every 2 seconds with `-r .5`)

## 0.10.4 November 1, 2020
 - add new `examples/umami` for load testing Drupal 9 demo install profile
 - replace TermLogger with SimpleLogger for increased logging flexibility
 - add initial OCI Dockerfile for container-based workflows
 - use checked subtraction when calculating drift duration to prevent panic
 - update `nng-rs` dependency to fix bug when testing that the manager is ready
 
## 0.10.3 October 14, 2020
 - fixup sticky redirect tests to properly test functionality
 - add `test/sequence.rs` to confirm sequencing tests works correctly, even in Gaggle mode
 - deduplicate test logic by moving shared functionality into `tests/common.rs`; consistently test functionality both in standalone and Gaggle mode
 - properly create debug log when enabled in Gaggle mode

## 0.10.2 September 27, 2020
 - remove unnecessary `GooseAttack.number_of_cpus` instead calling `num_cpus::get()` directly
 - remove `tests/gaggle.rs`, instead mixing gaggle tests with per-feature integration tests
 - ensure `test_start` and `test_stop` run one and only one time even in Gaggle mode

## 0.10.1 September 20, 2020
 - rework `hatch_rate` to be stored in an `Option<usize>` as it can be `None` on a Worker
 - remove redundant `GooseAttack.users` instead using the `Option<usize>` in `configuration`
 - improve bounds handling of defaults, generate errors for invalid values
 - properly handle early shutdown of Gaggle distributed load test from Worker process
 - Manager starts timing Gaggle distributed load test only after all Workers start

## 0.10.0 September 13, 2020
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

## 0.9.1 August 1, 2020
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
