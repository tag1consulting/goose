# Changelog

## 0.10.0-dev
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
 - where possible, passs error up the stack instead of calling `exit(1)`
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
