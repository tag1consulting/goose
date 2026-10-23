# Changelog

## 0.19.0-dev
 - [#767](https://github.com/tag1consulting/goose/pull/767) keep request, transaction and scenario times in microseconds, in buckets never more than 1% off, and show times under 10 ms to the hundredth of a millisecond (#585, #737, #366)
    o every time is taken from a `Duration` once and kept to the microsecond: a 0.4 ms request reads `0.4` where it read `0`, its minimum is 0.4 rather than 0, and the average is the mean of the real times rather than of whole milliseconds
    o percentiles come from one set of buckets in microseconds, never more than 1% off any time they hold, and a report holds only the buckets with a time. They replace the whole millisecond buckets, which rounded times from 101 to 500 ms to 10 ms, times from 501 ms to 1 s to 100 ms, and slower times to whole seconds. Now 1.4 s and 1.6 s no longer read as 1000 and 2000 ms (#737), and 2.2 ms and 2.5 ms are told apart (#366). The fastest, slowest and total times are kept exactly, so minimums, maximums and averages are exact
    o the console, HTML, Markdown and PDF reports and the dashboard show times under 10 ms to the hundredth (`2.35`) and from 10 ms in whole milliseconds (`38`), rounded half up; averages and standard deviations follow the same rule, and a baseline delta is the difference of the two values shown
    o a 0.19 JSON report loaded as a baseline has its request, coordinated omission, transaction and scenario rows rebuilt from its microsecond timing data, not read from its whole millisecond rows. A report from an older Goose keeps its whole millisecond rows, truncated, with percentiles above 100 ms rounded to 10 ms, 100 ms or whole seconds, so an unchanged time can read differently against it (`38 (+1)`)
    o `record_custom_request_duration` records a custom request from a `Duration`, keeping its microseconds; `record_custom_request` is unchanged
    o the request log's CSV format ends with a `response_time_us` column and the transaction and scenario logs' with `run_time_us`; the JSON, raw and pretty formats keep their whole millisecond fields and add `response_time_fraction_us` and `run_time_fraction_us`, the microseconds past those milliseconds
    o **Breaking**: the JSON report's minimums, maximums and percentiles stay whole milliseconds, truncated. Its averages are now means of the microseconds, and its standard deviations are worked out from the microseconds. Its timing data holds each bucket once, in `times_us`, in place of the whole millisecond `times`, beside `minimum_time_us`, `maximum_time_us` and `total_time_us` (`min_time_us` and `max_time_us` for transactions and scenarios). A JSON report from 0.18.1 or an earlier 0.19.0-dev build loads as a baseline
    o a JSON report is several times larger, as its buckets are finer and it holds them for requests, transactions and scenarios
    o a 0.19.0-dev build from before this change cannot load a JSON report from this change as a baseline, as the new report's timing data has no whole millisecond `times`; baselines are new in 0.19, so no release is affected
    o **Breaking**: the telnet Controller's `metrics-json` and the WebSocket Controller's `metrics` replies carry the same timing data as the JSON report: each timing data holds its buckets in `times_us` in place of the whole millisecond `times`, and adds `minimum_time_us`, `maximum_time_us` and `total_time_us` (`min_time_us` and `max_time_us` for transactions and scenarios) beside the whole millisecond fields
    o dashboard snapshots add `response_time_us` (fastest, slowest, p50, p95 and p99) to the aggregate, each request, scenario and transaction row and the adjusted figures. The heatmap's row table is now `buckets_us`, in microseconds, and the whole millisecond `buckets` is gone. `buckets` was only in 0.19.0-dev builds, so the snapshot stays `version: 1`. The heatmap's rows are 100 µs wide under a millisecond, then 1 ms, 2 ms, 10 ms, 50 ms, 100 ms and 1 s wide up to 61 s, 237 in all, so row indexes move. The HTML report's heatmap also drops `buckets` for `buckets_us`, and its percentile lines are in microseconds
    o the dashboard's KPI tiles share the row and each value is sized to its tile, so it stays on one line; the users tile shows the target smaller; RPS shows two decimals below 100 and whole requests a second from 100, where hovering the tile shows the two decimals. Heatmap percentiles, in the dashboard and the HTML report, are the middle of their row and show at the row's precision, truncated: to the tenth under 1 ms and in whole milliseconds from 1 ms
    o **bug fix**: transaction and scenario minimums, and their aggregated minimums, skipped 0 ms as unset, so a transaction under a millisecond read Min 1 with Avg 0.00; a time of 0 is now a minimum
    o **bug fix**: a `report.json` written by Goose 0.18 failed to load as a baseline, as it writes a transaction's name as a plain string
    o **bug fix**: a percentile's rank was taken in an `f32`, which rounds counts past 16,777,216, so past that many requests the 100th percentile could be a time short of the slowest; ranks are now taken in an `f64` from the percent as written
    o **bug fix**: a `GooseUser` made without a coordinated omission setting, such as one `GooseUser::single` makes from a parsed configuration, panicked on its first request; it now mitigates nothing
    o **Breaking**: `GooseRequestMetricTimingData`'s `times`, `minimum_time`, `maximum_time` and `total_time` (whole milliseconds) are now `times_us`, `minimum_time_us`, `maximum_time_us` and `total_time_us` (microseconds). `times_us` is a `TimeBuckets` of each bucket's fastest time and its count; it serializes as a JSON object, as the `BTreeMap` did. `minimum_time_ms()` and `maximum_time_ms()` give the fastest and slowest in whole milliseconds, truncated; `average_ms()` gives the mean in milliseconds to the microsecond; and `percentile_us()` gives a percentile in microseconds
    o **Breaking**: `TransactionMetricAggregate` and `ScenarioMetricAggregate` keep their times in `run_times`, a `GooseRequestMetricTimingData`, in place of `times`, `min_time`, `max_time`, `total_time` and `counter`
    o **Breaking**: `StatusCodeTimingSummary`'s `total_time`, `min_time` and `max_time` and `StatusCodeBreakdown`'s `min_time` and `max_time` are now `_us` fields in microseconds
    o **Breaking**: `GooseRequestMetric` has a new public field, `response_time_fraction_us`, and `TransactionMetric` and `ScenarioMetric` have `run_time_fraction_us`, so a struct literal of any of them must set it
    o **Breaking**: `GooseMetric::All` holds its scenario in a `Box`, so every metrics message stays small with the scenario's new field
 - [#766](https://github.com/tag1consulting/goose/pull/766) stop graphing `set_success()` and `set_failure()` updates as requests (#735)
    o **bug fix**: the update they send was counted as another request in the requests per second graphs and its response time was counted twice in the average response time graphs, and after `set_success()` the failure stayed in errors per second; an update now only adds or takes back the failure in errors per second
 - [#765](https://github.com/tag1consulting/goose/pull/765) say what the dashboard and `/api/v1` promise in 0.19, 0.20 and 1.0, in place of the book's "Experimental" banner (#701)
 - [#763](https://github.com/tag1consulting/goose/pull/763) add a whole run response time heatmap to the HTML report, and so to the PDF (#746)
    o on by default for every report written at the end of a run, by any `--report-file` or by the dashboard's Save report (#715); `--no-percentiles-over-time` turns it off. That flag and `GooseDefault::NoPercentilesOverTime` now both work without `--dashboard`
    o the heatmap's memory stops growing: it keeps at most 600 columns for all requests and 60 for each of the first 100 request names, and as many again for synthetic times under `--co-mitigation`. When a set of columns is full, adjacent columns merge in pairs, doubling their width
    o **behavior change**: with the heatmap on, a run that writes a report keeps the dashboard's per second response time histograms, as `--dashboard` does. Each user's batch of request metrics then holds response times from only the second it began in and the next. A request from any other second sends the batch early
    o **bug fix**: a PDF report failed with `net::ERR_ABORTED` once the HTML report's data URL passed about 2 MB, a length Chrome refuses; Chrome now gets the HTML as the content of a blank page
 - [#762](https://github.com/tag1consulting/goose/pull/762) set the host, ramps and run time from the dashboard's control panel, and start with them in one request, all or none (#712)
    o the Edit settings form sends its edits with Apply settings, or with the start through Start with changes; the first host change in a page load asks for confirmation
    o `POST /api/v1/control/settings` is new and `POST /api/v1/control/start` takes the same settings body; both, and the new error codes `invalid_value` and `no_host`, are additive under `version: 1`
    o **behavior change**: `runtime` on a running users ramp changes this run, from a Controller or the dashboard, where it was refused unless idle; `runtime 0` holds until stopped, and is refused while a ramp down is configured
    o the book's description of Stop is corrected: a Stop stops all users at once
 - [#761](https://github.com/tag1consulting/goose/pull/761) keep the dashboard's run settings in step with the server: every snapshot carries `settings`, what the next Start runs and where the running test is, the control panel and the read only header show it as one summary line, and the Target field follows changes from telnet, WebSocket and other tabs (#706); a running users ramp keeps its shape when users or the decrease rate change, and run shape defects are fixed
    o snapshots have a `settings` object: `host`, `hosts` (what the next Start will load test), `users`, the rates, `increase_time_secs`, `decrease_time_secs`, `run_time_secs`, `test_plan` in canonical form, `hold_ends_in_secs` and `stage`; additive under `version: 1`, as are a new `field` naming the setting a control refusal faults and the new error code `conflicting_settings` for a refusal that conflicts with other settings
    o **bug fix**: after one Apply or Start from the dashboard, the Target field ignored every later change from telnet, WebSocket or another tab, so `+` stepped from a stale count; it now follows the server unless being edited, and an edit that another client overtakes says it changed elsewhere (#706)
    o **bug fix**: a rate, time or run time sent to a Controller while a test plan was configured left no users, so Start weighted zero users forever and the main loop allocated until memory ran out, and the same commands during a test plan's first step panicked Goose when the step ended; they are refused, naming `users`, and `reset_metrics` no longer unwraps the users
    o **bug fix**: a Start with scenario hosts only, no default host and a `test_start` or `test_stop` transaction replied ok and then panicked; it is refused with `prepare_failed` and `"field": "host"` before it replies, and an autostarted run fails at launch; `get_base_url` returns an error instead of unwrapping a missing default host
    o **behavior change**: telnet and WebSocket refuse a host with a user name or password, a rate outside 0.01 to 100000 or sent while stopping, in Shutdown, during a users ramp's ramp down, during a running test plan, or on a running test while its own time is set, a time or run time over 30 days, and a test plan of more than 100 steps, 4096 bytes, a step over 30 days, or 100000 users in total; `runtime 0`, `increasetime 0` and `decreasetime 0` clear the setting, as before, and no longer clear the other rate or the test plan
    o **behavior change**: `users` on a running users ramp keeps its run time and ramp down: it ramps to the new count, keeps the hold's end, and ramps down for the new count; it used to hold until stopped. During the ramp down it still abandons the ramp down and holds until stopped
    o **behavior change**: a decrease rate on a running users ramp regenerates the ramp down still ahead, and adds one to a run that would stop all users at once when its hold ends; it only took effect at the next Start
    o **behavior change**: `users` is refused while a test plan runs, as the plan sets users, and a rate or time while a test plan is configured, until `users` switches to a users ramp; a rate or time used to clear the plan and say so
    o **behavior change**: the command line's cross field rules apply to Controller commands: `runtime` and `test-plan` are refused under `--iterations` and `test-plan` under `--no-reset-metrics` in every phase, and in Idle `increase-time` or `decrease-time` with fewer than 2 users, or `users 1` while either is set
    o **behavior change**: the dashboard's Target and step fields are read only, not disabled, with the reason under them, while stopping, while a test plan runs and without a token
 - [#760](https://github.com/tag1consulting/goose/pull/760) parse the command line with `clap` instead of the unmaintained `gumdrop`; every valid command line parses as it did, except an option value that begins with a hyphen
    o **API change**: `GooseConfiguration::parse_args_default` and `GooseConfiguration::parse_args_default_or_exit` are now inherent methods, so remove `use gumdrop::Options;` from load tests that imported it; `GooseConfiguration` implements `clap::Parser`, so `parse()`, `try_parse()` and `try_parse_from()` parse the same command line, with the same help
    o **API change**: `host`, `increase_time`, `run_time`, `decrease_time`, `goose_log`, `pdf_print_html`, `request_log`, `transaction_log`, `scenario_log`, `error_log`, `debug_log`, `dashboard_host`, `dashboard_auth_token`, `telnet_host`, `websocket_host`, `iterations`, `throttle_requests`, `scenarios`, `dashboard_port`, `telnet_port`, `websocket_port` and `dashboard_max_clients` in `GooseConfiguration` are now `Option`, `None` when the option is not given, so a value a user types, `0` or `""` among them, is never taken for one not given; code that set one directly wraps the value in `Some`, as in `configuration.run_time = Some("30s".to_string())`, and code that read one uses `as_deref()` or `unwrap_or`
    o **API change**: the `help` field is gone, and `parse_args_default` returns a `GooseError::InvalidOption` for `-h` and `--help`; its errors name the option and value clap refused, and give clap's reason without its usage line; they never copy the rest of the command line, which can hold a `--dashboard-auth-token`
    o **API change**: `GooseConfiguration::pdf_scale` and `pdf_timeout` are `Option<f64>` and `Option<u64>`; `GooseDefault::PdfScale` sets the scale in code, as `GooseDefault::PdfTimeout` sets the timeout
    o **behavior change**: help is split into a short `-h`, one line for each option, and a long `--help`, which explains each option's default, values, interactions and `GooseDefault`; both are regrouped under task headings with new wording, printed to stdout, and wrapped to the terminal (80 columns when none is detected, at most 120)
    o **behavior change**: an option value that begins with a hyphen goes after `=`, as in `--goose-log=-run.log`; given as its own argument it is read as an option, and the error says to use `=`. `gumdrop` took it as the value, so `--goose-log --quiet` wrote the Goose log to a file named `--quiet`. A negative number is still read as a value
    o **behavior change**: the Controller `config` command's JSON no longer has a `help` key, lists its keys in help order, and shows an option that is not set as `null`
    o **behavior change**: `--dashboard-port 0` binds a free port, which the log and the end of run message show; it bound port 5118
    o **bug fix**: `--no-debug-body` on its own leaves response bodies out of the debug log; it was reset to off unless `--no-granular-report` was also given
    o **bug fix**: `GooseDefault::NoGranularData` is applied; it had no effect
    o **bug fix**: `--no-granular-report` no longer leaves response bodies out of the debug log
    o **bug fix**: a fractional `--timeout` is kept to the millisecond; `--timeout 0.5` gave a 0 ms timeout that failed every request, and `1.5` gave 1 second; a timeout under 1 ms is refused
    o **bug fix**: a `--timeout`, `--increase-rate` or `--decrease-rate` that is not a finite number, `NaN` and `inf` among them, or the same in `GooseDefault::Timeout`, `IncreaseRate` or `DecreaseRate`, is an error naming the option; it was a warning, and the load test ran with a 60 second timeout or one user a second
    o **bug fix**: `GooseDefault::Iterations` is applied; the command line's `iterations` always won, even when `--iterations` was not given
    o **bug fix**: `--run-time 0`, `--increase-time 0`, `--decrease-time 0` and `--throttle-requests 0` undo `GooseDefault::RunTime`, `IncreaseTime`, `DecreaseTime` and `ThrottleRequests`; they were read as not given, so the default stayed, and with `--iterations`, `--increase-rate` or `--decrease-rate` the load test was refused
    o **bug fix**: an empty `--goose-log`, `--request-log`, `--transaction-log`, `--scenario-log`, `--error-log`, `--debug-log` or `--dashboard-auth-token` turns off the log or token its `GooseDefault` sets; it was read as not given, so the default stayed
    o **bug fix**: `--telnet-port 0`, `--websocket-port 0` and `--dashboard-max-clients 0` are refused; they became 5116, 5117 and 32
    o **bug fix**: an explicit `--pdf-timeout 60` now beats `GooseDefault::PdfTimeout`, and `GooseConfiguration::default()` gets a PDF scale of 0.8 and a timeout of 60, where it had 0.0 and 0 and a PDF report was refused
    o a bad `--co-mitigation` or log format value now names the valid ones, and a bad `--scenarios` value gives its reason once, from clap; it printed four lines of its own, then a generic error
    o **bug fix**: `GooseDefault::Scenarios` with a name that is not letters, digits and `*` is an error; it panicked
    o the book's runtime options page is generated from the help, and a test checks it matches
 - [#755](https://github.com/tag1consulting/goose/pull/755) replace `headless_chrome`'s `fetch` feature with Goose's own browser lookup and download, so `pdf-reports` builds again and uses an installed Chrome or Chromium first; on 2026-10-16 every `zip` release but 9.0.3 was yanked, and `fetch` needed `zip` 8.6, so every build failed to resolve, since Cargo resolves optional dependencies whatever features are enabled; on arm64 Linux `fetch` downloaded an x86_64 Chromium that could not run, even with Chromium installed (#753)
    o **behavior change**: PDF reports use `CHROME` when it is set, then an installed Chrome, Chromium or Edge, then a Chrome for Testing headless shell that Goose downloads once into the user cache directory (`goose/chrome`); `fetch` always downloaded its own Chromium snapshot and ignored both `CHROME` and the installed browser
    o **behavior change**: a `CHROME` that names a path that does not exist is an error; `headless_chrome` ignored it
    o downloading needs `unzip` on `PATH` (`tar` on Windows), and no longer needs the `zip` crate
    o a failed Chrome launch says to install Chrome or set `CHROME`, and an `Exec format error` says the binary does not match this machine's architecture
    o **behavior change**: with a `.pdf` `--report-file`, Goose finds Chrome, downloads it if needed, and renders a test page to PDF before any user starts, and a run whose Chrome can't render fails then, before any report file is created, instead of after the load test; Chrome was first looked for, downloaded and launched only when the report was written
    o **behavior change**: a `CHROME` that names a directory or a file that is not executable is an error when the configuration is checked, and a macOS app bundle gets the path of the binary inside it
    o a report that fails at the end of a run no longer stops the other `--report-file` reports or `--pdf-print-html` from being written, and a PDF report that fails leaves no empty file behind; the first error is still returned
    o a failed Chrome launch names the binary and how Goose found it, and on Linux reports a missing ELF interpreter (a glibc binary on musl) and the shared libraries `ldd` can't find; otherwise it shows what `--version` printed
    o a Chrome launch is bounded by `--pdf-timeout`; a Chrome that exited at once, such as one missing a shared library, took `headless_chrome` about five and a half minutes to report, retrying a 30 second wait eleven times
    o Goose checks `unzip` runs before it downloads, and a musl build never downloads, since Chrome for Testing needs glibc
    o a downloaded Chrome is checked against the current stable Chrome for Testing once a week and replaced when a newer one exists, older downloads are removed, and partial downloads older than an hour are removed; a failed check is a warning, and installed browsers and `CHROME` are never touched
    o a PDF report looks for Chrome again when another Goose run's weekly refresh removed the downloaded Chrome this run found at startup; it failed to launch
    o the version and download URL read from Chrome for Testing are checked before use: the version must be a Chrome version number and the URL an https URL on `storage.googleapis.com`, and the download is capped at 400MB
    o PDF and Chrome errors name the setting to change, `--report-file`, `--pdf-timeout` or `CHROME`, instead of `--pdf`, which is not an option
 - [#700](https://github.com/tag1consulting/goose/issues/700) give telnet, WebSocket and the dashboard one typed command layer, and refuse the Controller and dashboard inputs that panicked, overflowed or hung Goose, and the command line time spans and test plans that started no users or panicked or overflowed when parsed; the dashboard parsed and checked commands apart from telnet and WebSocket, values stayed strings until the parent used them, and the time span and test plan parsers unwrapped and multiplied without checks
    o **behavior change**: a test plan with no users, or more users in total than fit in a `usize`, is refused when it is parsed, so from a Controller, on the command line and by `set_default(GooseDefault::TestPlan, ..)`; starting it never got past weighting users
    o **behavior change**: a time span or a test plan step too large for a `usize`, or a time span too long in milliseconds, is an error instead of a panic or a wrapped value, from a Controller, `--run-time`, `--increase-time`, `--decrease-time`, `--test-plan`, and the matching `GooseDefault`s; `set_default(GooseDefault::TestPlan, ..)` returns an error for any plan it can't parse instead of panicking
    o **behavior change**: Controller `increaserate` and `decreaserate` must be a finite number of at least one user a day (1/86400, about 0.0000116); 0, a slower rate, and a rate too large for an `f32` (40 digits, for example) were accepted, and `.` was taken as 1
    o **behavior change**: `--increase-rate`, `--decrease-rate` and their `GooseDefault`s are refused below one user a day (1/86400), as the Controllers are; `0` keeps its `must be non-zero` refusal
    o **behavior change**: a Controller rate that clears a configured test plan replies `increase_rate configured, test plan cleared` (or `decrease_rate ...`); rates are still accepted in every phase, and the next `users` change or Start uses them
    o Controller values are read in the grammar the old Controller regexes accepted (one space after the command), narrowed to ASCII digits and ASCII white space, then parsed and bounded; a no break space in a Controller test plan is refused, and the test plan a Controller sends is parsed without printing it to stderr
    o test plans and Controller time spans read ASCII digits only: a digit of another script (`testplan ١,1s`) matched the parsers' `\d` and then panicked; it is now refused. In a command line time span it is refused before a unit (`--run-time ١h`) as `must use the ASCII digits 0 to 9`, where it panicked; such a digit is otherwise read as before (`--run-time ١` is 0)
    o **behavior change**: Controller `test-plan` must start 1 to 100000 users in total, and is refused while a Stop, `shutdown` or Ctrl-C is ramping the load test down, as `users` is, and in Shutdown, where it panicked
    o **behavior change**: Controller `runtime` is refused unless the load test is idle, as `increase-time`, `decrease-time` and `host` are; on a running load test it changed nothing in that run
    o **behavior change**: a Controller command with a value that can't be used is answered with what the value must be, for example `users must be an integer between 1 and 100000`, instead of `unrecognized command`, `failed to configure test-plan, be sure test-plan is valid` or `failed to reconfigure users, ...`, and a `users` or `test-plan` refused while a Stop, `shutdown` or Ctrl-C ramps the load test down is answered `load test is stopping, failed to reconfigure users`, and in Shutdown `load test not idle, failed to reconfigure users`, with `test-plan` in place of `users` for a test plan; telnet, WebSocket and the dashboard give the same refusal to the same value
    o **behavior change**: a Start with no users to start (a rate or run time command clears a `--test-plan` and leaves no users) is refused with `prepare_failed` instead of hanging
    o **behavior change**: a refused Start names its cause, `unable to start load test, it is not idle`, `..., no host is configured` or `..., there are no users to start`, and `host` on a running load test replies `failed to reconfigure host, load test is not idle`; before, both replies read `be sure ...` and named two causes where only one applied; status codes and the `error` codes are unchanged
    o a `--test-plan` refused for having no users or more than fit in a `usize` prints why, as a step that does not parse prints the format
    o **behavior change**: Controller rates and times are stored as parsed, so `config` shows `increaserate .1` as `0.1` and `runtime 1h` as `3600`; the units of a Controller time (`runtime`, `increasetime`, `decreasetime`) are not case sensitive, where `runtime 1H` was 0 seconds
    o **behavior change**: a Controller time of 0 written other than `0` (`runtime 0s`, `increasetime 0m`, `decreasetime 00`) is stored as `0`, which means unset, as the command line already stored `--run-time 0s`; the Controller stored the raw `0s`, so an increase time of `0s` launched every user at once, a run time of `0s` stopped the load test right after the ramp, and a decrease time of `0s` added a step to 0 users
    o Controller `shutdown` no longer holds the main loop for 250 ms, and from Idle shuts down as Ctrl-C and the dashboard's Quit do there
    o Controller `shutdown` in Shutdown replies `load test shut down` and changes nothing
    o a WebSocket read error ends that Controller connection
    o telnet reads only the bytes each read returns: a command sent with no line ending was padded with NUL bytes, or with the tail of an earlier longer line, and refused as an unrecognized command; it now runs
    o Controller commands and their aliases ignore ASCII case only; the Unicode case folding of the old regexes also read `ſtart` as `start`
    o **behavior change**: a stop, shutdown or Ctrl-C with no user running ends the load test, where it maintained 0 users forever; the dispatcher answering 16 commands a tick made this reachable from a Controller, by two `shutdown`s from Idle in one tick, a dashboard Quit then a `shutdown` in one tick, or a `stop` in the tick of its `start`
    o the parent answers up to 16 Controller commands per main loop tick, as it did for dashboard control, and skips a client that has gone; a Controller client waits at most 15 seconds for an answer and is then told `control timed out, it may still apply: check before retrying`, where one command was read per tick and a client could wait forever
    o the dashboard control endpoints, status codes and JSON shapes, and the WebSocket request and response shapes, are unchanged; only reply messages and the close of a WebSocket message over 131,072 bytes change, as listed in this entry
    o dashboard control has its own queue, answered first each tick and again before each Controller command, so no queued Controller command runs ahead of it however many Controller clients connect; the Controllers take no authentication
    o a tick answers up to 16 dashboard commands, then up to 16 more from the dashboard and Controller queues together, dashboard first; dashboard commands, which need the control token, can fill all 16 of those
    o a request whose client has gone is skipped and does not count toward either 16
    o every `metrics` and `metrics-json` in a tick is answered from one metrics request
    o **behavior change**: a WebSocket Controller message over 131,072 bytes is answered with a close frame, code 1009, reason `message too big, at most 131072 bytes`, and the connection ends; a connection buffers about 1 MiB at most, where it allowed messages of 64 MiB
    o **public API change**: `controller::ControllerCommand` carries its parsed value (`Host(String)`, `Users(usize)`, `IncreaseRate(f32)`, `DecreaseRate(f32)`, `IncreaseTime(usize)`, `DecreaseTime(usize)`, `RunTime(usize)`, `TestPlan(TestPlan)`), adds `Quit`, drops `Help` and `Exit` (the Controller answers them itself), derives only `Clone` and `Debug` (no longer `PartialEq`, `Eq` or strum's `EnumIter`), and its `FromStr` error is the `&'static str` a client is sent
    o **public API change**: `util::parse_timespan` returns `Option<usize>`, `None` when a span is too long or has a digit of another script before a unit
    o **public API change**: `TestPlan::total_users` saturates at `usize::MAX` instead of overflowing, and `GooseError::InvalidControllerCommand` is no longer returned; the variant is kept
    o removes the `async-trait`, `strum` and `strum_macros` dependencies
 - [#713](https://github.com/tag1consulting/goose/issues/713) fix whole run response time figures that an audit against an independent calculation found wrong, in the printed metrics, the reports and the dashboard
    o a response under a millisecond was never a minimum: request timings used a minimum of 0 to mean no time yet, so the minimum showed the fastest nonzero time, and percentiles below it were raised to it; the count of times now says whether there is one, for each request, each status code and every aggregate over requests
    o with coordinated omission mitigation the report's aggregate took its percentile ranks and its average over every request counted, synthetic ones included, though only measured times are in it, so its highest percentiles printed 0 and its average was too low; it now uses the count of measured times
    o the report's and the console's adjusted aggregates left out the times of requests without synthetic times, while the report still counted those requests, and the console took its adjusted maximum from the measured maximum; both now cover every request's adjusted times, its measured ones when mitigation added none, as the dashboard does
    o the reports computed and kept averages as `f32`, so an average above about 10,000 ms could print 0.01 ms off; they are `f64` now
    o **behavior change**: report averages are exact, so the JSON report carries them with more digits; values still computed as `f32` serialize as before
    o the report's transaction aggregate average divided the requests' total time by the transaction count; it now divides the transactions' total time by their count, as the console does
 - [#768](https://github.com/tag1consulting/goose/pull/768) one folder per run for the output flags given without a path, notes, a Save report switch and a runs panel in the dashboard, and gzip (#715)
    o a run writes nothing unless a flag or the dashboard asks for it, and an output flag given a path writes exactly that file, as in 0.18
    o `--report-file`, `--goose-log`, `--request-log`, `--transaction-log`, `--scenario-log`, `--error-log` and `--debug-log` take an optional path; without one they write into the run's folder
    o a bare `--report-file` writes `report.json` and `report.html`, a bare `--goose-log` writes `goose.log`, and a bare log option writes `requests`, `transactions`, `scenarios`, `errors` or `debug` with its format's extension
    o a bare `--report-file` beside `--report-file` paths writes both the folder's reports and the paths
    o **behavior change**: an output flag followed by another option is given without a path, so `--goose-log --quiet` writes `goose.log` into the run's folder; it was an error under #760, and 0.18 wrote the Goose log to a file named `--quiet`
    o new `--output-dir DIR` (`GooseDefault::OutputDir`) names the run's folder; without it the folder is `goose-runs/<YYYYMMDD-HHMMSS>`, from the run's local start time
    o a run never writes into a folder that already holds files when the run starts: it takes the first of `DIR-2`, `DIR-3` and so on that is missing or empty, also when two processes start at once
    o the run's folder is created as the run starts, and removed at its end if the run wrote nothing into it
    o a folder Goose can't write to is an error before load starts
    o when only `--dashboard` asks for the run's folder and Goose can't write to it, the run goes on with a warning and Save report off
    o a report always holds the whole run's totals and percentiles; when its graphs start after the run did, it says when and why
    o new `--notes TEXT` (`GooseDefault::Notes`) is stored in the JSON report and shown in the HTML and Markdown reports and the dashboard
    o new `--report-graph-data` (`GooseDefault::ReportGraphData`) adds the graphs' per second data to the JSON report
    o new `--render-report FILE.json` reads a JSON report and writes it to the `--report-file` paths, then exits without running a load test
    o the JSON report adds `notes`, `goose_version`, `test`, `started`, `ended`, `ended_by`, `canceled_reason`, `in_progress` and `graphs_start`; older reports still load
    o **behavior change**: `report.json` and the Controller's `metrics-json` include the test plan history, so a Markdown report rendered from the JSON has its plan overview
    o the dashboard's **Save report** switch writes `report.json` and `report.html` into the run's folder at the end of each run; it starts on with `--dashboard`, except under `--no-metrics`
    o while Save report is off and no `--report-file` path is given, Goose keeps only the dashboard's last 10 minutes of graphs and 5 minutes of heatmap
    o **Save report now** writes the report as it stands, marked in progress, while the run goes on; the report at the end of the run replaces it
    o **Include graph data** switches `--report-graph-data` on or off
    o with `--dashboard-control` the switches can change at any time; without it they stay as the command line set them
    o the runs panel lists each folder beside the run's folder that holds a `report.json` or `report.json.gz`, newest first, with its notes, and offers each report as HTML, JSON or Markdown
    o the runs panel compares two reports, and with `--dashboard-control` deletes one: its reports and Goose's logs, then its folder if that leaves it empty
    o Delete refuses the folder of the run in progress, the folder a bare `--goose-log` is open in, and a folder another Goose process is writing to
    o the end of run banner offers the report's downloads, or says why the report failed; a run with no folder shows why for the whole run
    o new routes `GET /api/v1/reports`, `GET /api/v1/reports/{name}/{file}` and `GET /api/v1/reports/{name}/compare.md`
    o new control routes `POST /api/v1/control/report`, `POST /api/v1/control/report/now`, `POST /api/v1/control/quit` and `DELETE /api/v1/reports/{name}`, with `--dashboard-control` only
    o the Quit button, shown while idle, shuts Goose down
    o the snapshot adds `report`, with `save`, `graph_data`, `folder`, `paths`, `notes`, `last_report`, `last_report_name`, `error` and `writes`; additive under `version: 1`
    o new control error codes `invalid_report`, `report_busy` and `report_failed`, in `ok: false` answers; additive under `version: 1`
    o **behavior change**: when an autostarted `--dashboard` run ends on its own and Save report wrote its report, Goose stays up while a client follows the event stream, until Quit, Ctrl-C or a Controller `shutdown`
    o **behavior change**: a report or log named `.gz` is written gzipped; 0.18 wrote such a log as plain text and refused such a report
    o Goose reads a `.json.gz` report wherever it reads a JSON report, and `--baseline-file` takes a run's folder
    o new `--compress [LEVEL]` (`GooseDefault::Compress`) gzips every file Goose writes into the run's folder, at a level from 1 to 9, 6 by default
    o a gzipped log is whole when its run ends, also when it ends early, and a gzipped Goose log is whole between runs
    o **behavior change**: `--report-file` is written even with `--no-print-metrics`
    o **behavior change**: `--no-granular-report` works with `--dashboard` as well as `--report-file`
    o **behavior change**: the metrics printed when a `--no-autostart` run returns to Idle respect `--no-print-metrics`
    o **behavior change**: the killswitch keeps its first reason, and is cleared when a run starts
    o **bug fix**: Ctrl-C while Idle after a run exits; it canceled into a plan that held 0 users with no end
    o **bug fix**: a graph series that starts after the run did is drawn from its own first second
    o **API change**: `GooseConfiguration` has the new public fields `output_dir`, `compress`, `notes`, `report_graph_data` and `render_report`, and `GooseDefault` the new variants `OutputDir`, `Compress`, `Notes` and `ReportGraphData`
    o **API change**: `config::RUN_FOLDER` is public; an output field holds it when its flag is given without a path, and a Controller's `config` and `config-json` show it as `(the run's folder)`
    o **API change**: `reset_killswitch()` is public
    o `--no-save`, `--runs-dir`, `GooseDefault::NoSave`, `GooseDefault::RunsDir`, `run.json`, `/api/v1/runs` and its routes, and the snapshot's top level `save` are removed, and the SSE `closed` event's data is `1` again; none was released, so the snapshot stays `version: 1`
 - [#694](https://github.com/tag1consulting/goose/issues/694) refuse a Users change while a Stop, `shutdown` or Ctrl-C is ramping the load test down; Users was accepted during that ramp down and brought the load test back
    o a Users request during a cancel gets `ok: false` with phase `decrease`; a test plan's own ramp down still accepts it
    o the telnet and WebSocket Controller `users` command is refused the same way during a cancel
    o the dashboard snapshot adds `stopping`, and the control panel disables the user controls while it is set
 - [#695](https://github.com/tag1consulting/goose/issues/695) stop the dashboard HTTP server when `execute()` returns, on every return path; the server task was detached and held the port and its connections until the Tokio runtime shut down, so a second `GooseAttack` in the same runtime could not bind
    o at shutdown idle connections close at once and a request in flight gets up to one second before it is closed; `execute()` returns when they are closed
 - [#696](https://github.com/tag1consulting/goose/issues/696) free a dashboard SSE client slot as soon as the client disconnects; the slot was freed only when the next snapshot failed to send, so while the main loop published none (a long `test_start`), reconnecting browsers could fill `--dashboard-max-clients` and lock viewers out
 - [#697](https://github.com/tag1consulting/goose/issues/697) harden the dashboard HTTP server; it had no Host check, no header read timeout and no connection limit
    o on a loopback bind, a request with a missing or non loopback `Host` gets 403
    o a connection that does not finish sending request headers within 10 seconds is closed, which also closes keep alive connections idle for 10 seconds
    o open connections are capped at twice `--dashboard-max-clients` plus 16; connections beyond the cap are closed unanswered
    o every response carries `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`
 - [#698](https://github.com/tag1consulting/goose/issues/698) test the dashboard control admission limit, the control body limit on every control route, and the dashboard graph kept whole when `--report-file` is set; nothing tested the first two, and the graph test never sent a command that prunes, so removing its guard left it green
    o `test_control_second_stop_refused` sends its second Stop after the first answers instead of 30 ms later at the same time; a Stop ignores the decrease rate and the main loop takes no command until its users exit, so the second Stop usually lands in Idle, and a unit test now pins the refusal during Decrease
 - [#700](https://github.com/tag1consulting/goose/issues/700) bound the telnet and WebSocket `users` command to the dashboard's 1 to 100000; [#690](https://github.com/tag1consulting/goose/pull/690) checked the range only in the dashboard, and `users 0` followed by `start` hung the main loop
    o **behavior change**: the telnet and WebSocket Controller `users` command refuses 0 and anything above 100000 (the reply is listed under the #700 entry above); `users 0` on a running test used to ramp to zero users and end the test, and every refusal read "load test not idle, failed to reconfigure users"
 - [#702](https://github.com/tag1consulting/goose/issues/702) generate the dashboard's TypeScript snapshot types from the Rust structs with `ts-rs` in a unit test that fails on drift; the hand written types could drift with nothing failing
    o the client refuses a snapshot whose `version` is not the one it was built for, and shows an error banner
 - [#704](https://github.com/tag1consulting/goose/issues/704) test that the dashboard's Target field falls back from `target_users` to `maximum_users` to `active_users` and stays empty when none is at least 1, and that a closed stream sends a poll at once; swapping the first two fallbacks left every client test green
 - [#705](https://github.com/tag1consulting/goose/issues/705) keep the dashboard token in the tab's sessionStorage so a reload keeps it; the client stored it nowhere after stripping `?token=` from the URL, so every reload got 401
    o a 401 clears the stored token; a new tab still needs `?token=`
 - [#707](https://github.com/tag1consulting/goose/issues/707) bring a polling dashboard back to the live stream, and end its polling when a retry of the stream hears that the run closed; once the stream dropped, the client polled every 2 seconds and never opened the stream again, so it could not hear `closed`, and a poll had no timeout and the next one did not wait for it, so a request that never answered stopped polling for good and slow polls overlapped
    o while polling, the client retries the stream every 10 seconds and after a poll answered 503; a retry that fails before its first snapshot is closed without a banner or an announcement
    o one poll at a time, each given up after 8 seconds
 - [#708](https://github.com/tag1consulting/goose/issues/708) fix small dashboard client gaps: the footer named telnet `:5116` and WebSocket `:5117` even when the Controllers were off or elsewhere, every page load asked for a missing `/favicon.ico`, the snapshot's status codes, failures per second and minimum and maximum times were sent but never shown, and the line charts kept the colours of the theme they were created in; the client hard coded the ports, the page had no icon link, the tables and the Whole run card left those fields out, and nothing listened for a theme change
    o `GET /api/v1/health` adds `controllers`, the `host:port` the telnet and WebSocket Controllers listen on, or `null` for one turned off
 - [#709](https://github.com/tag1consulting/goose/issues/709) fix the dashboard's quick accessibility problems; the page had no main landmark, step buttons named by their glyphs, chart canvases screen readers skipped, an invalid Target not tied to its message and a failures line told apart by colour alone, and the dark increase and shutdown badges, the pressed Adjusted/Measured toggle and the input borders were below WCAG contrast
    o the heatmap's p50, p95 and p99 lines are now solid, dashed and dotted at one width; they differed by a pixel of width, p99's dash vanished under p95 whenever both fell in one row, and lines ending at one value got stacked labels beside the wrong lines, which now share one label ("p95, p99: 16 ms") under a "last second" caption, so they no longer read as a second key
 - [#710](https://github.com/tag1consulting/goose/issues/710) make the dashboard's table sorting, control buttons and table scroll areas usable from the keyboard; headers sorted only on a mouse click with no `aria-sort`, every control button set `disabled` while its request was in flight, which dropped keyboard focus, and the table scroll areas could not take focus
    o each sortable header holds a button and the sorted header carries `aria-sort`; the sort arrow is left out of the button's accessible name
    o unavailable control buttons are `aria-disabled` instead of `disabled`, so they keep focus and stay in the tab order, and a press on one does nothing; they are dimmed by colour rather than opacity and every control button gets the accent focus ring, so the ring on a focused unavailable button is not dimmed; a focused Quit that hides after a successful quit moves focus to Start
    o the five table scroll areas are focusable labelled regions
 - [#711](https://github.com/tag1consulting/goose/issues/711) make a running load test something a screen reader user can follow; phase and connection changes were never announced, every table was rebuilt on each snapshot, losing the reader's place, and the banner was rewritten, so announced again, every second
    o one polite live region speaks only on a change: the phase, connection lost and restored, and test finished
    o the banner is written only when its text changes; the transient "polling every 2s" and "Poll failed" banners are gone, since the connection indicator and the live region say the same
    o each chart has a one sentence summary (the mean of the last 10 seconds, rising or falling past 10 percent against the 10 before; requests and failures leave out the second still counting) and a collapsed "Show data" table, which scrolls in a focusable box of its own; the heatmap's table is each second's requests, p50, p95 and p99
 - [#713](https://github.com/tag1consulting/goose/issues/713) show response times by second as a heatmap in the dashboard, for all requests or one request; the card showed only the average, which hides tails
    o the card "Response times by second" draws one column per second of the 300 second window and one row per response time bucket on a logarithmic latency axis, coloured by how many requests in that second took that long on a logarithmic scale whose ends, as the legend shows, are the sparsest and the busiest drawn cell's counts; hovering a cell gives its count and share of the second, and a select chooses one request among the first 100 request names Goose records a response time for, counted again after each metrics reset, listed by method then name
    o the heatmap draws each second's p50, p95 and p99 as lines, labelled at the right edge and in the tooltip, worked out by the same rule as the whole run figures
    o with coordinated omission mitigation active, Adjusted (the default
, as in the KPIs and tables since #716) and Measured buttons choose whether the synthetic times are counted
    o on by default with `--dashboard`; `--no-percentiles-over-time` (or `GooseDefault::NoPercentilesOverTime`) turns it off, showing the average latency chart instead; the flag is an error without `--dashboard`, while the default is then ignored
    o the KPI strip's p95 and average, the **Whole run** card (formerly Aggregate) and the tables' `avg ms`, `p50`, `p95` and `p99` say they cover the whole run, since the test started or since metrics were last reset
    o without the heatmap, batch flushing is exactly as it was: each user's batch is sent every 100 requests or 250 ms, with its requests counted by second as before. With the heatmap on, a batch keeps each request name's response times for the second it is filling and for the next one; a request from any other second sends the batch early and starts a new one that keeps the old one's count and age, so no request is sent later than before. The metrics processor adds a batch to the heatmap's histograms in the same pass that adds it to the whole run figures, which stay exactly as before. In a comparison with main, 1,000 users made about 21,800 requests a second, and each run wrote its reports, as 0.19.0-dev then did by default. Each figure is the mean of the middle 3 of 5 runs of 10 minutes. The heatmap added no measurable CPU or instructions per request: 0.66% less CPU and 0.69% fewer instructions than the dashboard with it off, within the runs' spread of about 1%. It added 3.1 MiB of peak memory. Without `--dashboard`, CPU and instructions per request were within 1.2% of main's
    o memory is bounded: one histogram per second for the 300 second window, for all requests and for up to 100 request names; a request name's pending response times are allocated only when the heatmap records one, so without it a name's entry grows by 8 bytes
    o root cause of the flush rule fix: the batch split by second followed per second graph recording rather than the heatmap, so a run that recorded graph data without the heatmap took the heatmap's path and every user held its latest request unsent; the split now follows the heatmap alone

    o the snapshot adds `series.heatmap` (only the columns that changed since the client's heatmap, with a cursor; `GET /api/v1/snapshot?heatmap_generation=&heatmap_since=` returns what a client lacks), `flags.percentiles_over_time` and `requests[].has_series`; new endpoint `GET /api/v1/series?method=&name=` returns one request's heatmap in the same shape
 - [#716](https://github.com/tag1consulting/goose/issues/716) show coordinated omission adjusted response times in the dashboard once mitigation has recorded an event; the dashboard showed measured percentiles beside "Coordinated omission: active", which read as adjusted
    o the KPI strip, aggregate panel and requests table label adjusted values and show the measured value on hover; the aggregate panel shows both
    o the snapshot adds `co_adjusted` (average, maximum and percentiles) to `aggregate` and to each request row, present only while mitigation has events; `percentile_ms` stays measured
 - [#717](https://github.com/tag1consulting/goose/issues/717) make the dashboard's `/api/v1/health` timings honest, and answer a dashboard snapshot within tens of milliseconds under sustained load; `last_build_ms` counted the wait for the main loop and the metrics processor's queue as build time, a build that timed out left no trace, and the metrics processor drained metrics until the channel was empty before answering a command, so user threads that kept up with it could hold a snapshot past the hub's 4 second timeout
    o `last_build_ms` is the metrics processor's time draining metrics and building the snapshot; the new `last_snapshot_ms` is the hub's round trip, from request to published JSON, and the new `snapshot_timeouts` counts requests not answered within 4 seconds, whose late replies are logged at `debug` and never published
    o the metrics processor handles at most 256 metrics messages before it checks for a command, and a snapshot or `GET /api/v1/series` drains at most 256 before it is built; Shutdown, Reset, metrics display and Controller metrics still drain everything
 - [#719](https://github.com/tag1consulting/goose/issues/719) refuse a controller `users` value too large for a `usize` with an error reply instead of panicking the main loop, which assumed the command's regex guaranteed a valid integer
 - [#727](https://github.com/tag1consulting/goose/pull/727) make the `user_metrics_graph_reset` tests independent of runner throughput; they required a no-reset run to record at least 50% more requests than a separate reset run, about the expected difference since users launch gradually during increase-time, so they failed on runner variance
    o each run's recorded requests are now compared with what the mock server received in that run: equal without reset, fewer with reset
 - [#739](https://github.com/tag1consulting/goose/issues/739) include batched requests in coordinated omission adjusted times; since per user batching, only failures, coordinated omission copies and unbatched requests were added to them, which pulled adjusted averages and percentiles toward the synthetic times
 - [#690](https://github.com/tag1consulting/goose/pull/690) (EXPERIMENTAL) add opt-in read-only **live web dashboard** (crate feature `dashboard` + runtime `--dashboard`, default bind `127.0.0.1:5118`)
    o compile with `--features dashboard` (not in default features; avoids axum/tower-http on every build); without the feature, `--dashboard` fails at startup with a clear rebuild hint
    o streams coalesced metric snapshots over SSE (`/api/v1/events`) with poll fallback; one-shot `GET /api/v1/snapshot`
    o trailing series charts (RPS, failures/s, active users, average latency) via vendored Chart.js
    o phase badges, connection indicator (SSE / poll / disconnected), sortable request/error tables
    o token auth on metric APIs when configured (`--dashboard-auth-token`); required for non-loopback binds; browser bootstrap via `?token=`
    o public shell/static/health; Content-Security-Policy without inline scripts
    o enables GraphData series collection (same memory class as `--report-file`); documented in Goose Book “Live Dashboard”
    o tunable SSE client cap via `--dashboard-max-clients` / `GooseDefault::DashboardMaxClients` (default 32)
    o `/api/v1/health` exposes ops counters `last_build_ms`, `build_count`, and `active_sse_clients` (no load-test metrics)
 - [#690](https://github.com/tag1consulting/goose/pull/690) (EXPERIMENTAL) add optional **dashboard runtime control** (`--dashboard-control`, requires `--dashboard` and `--dashboard-auth-token` even on loopback; same `dashboard` crate feature)
    o authenticated `POST /api/v1/control/{start,stop,users}` with structured JSON success/error; routes unregistered (404) when control is off
    o phase-aware SPA control panel: Start, Stop, absolute target users with Apply and ± step buttons
    o Stop begins a cancel ramp through Decrease (not instantaneous Idle); Start success means entered Increase (not `test_start` complete)
    o `--no-autostart` allowed with dashboard control (no Controllers required); Controllers remain power-user path (host/rates/shutdown not in dashboard)
    o documented in Goose Book “Live Dashboard” (flags, auth matrix, curl examples, semantics)
    o **behavior change**: the `--no-autostart` error when no Controller and no dashboard control is enabled now reads "`configuration.no_autostart` requires at least one Controller or dashboard control be enabled"
 - [#690](https://github.com/tag1consulting/goose/pull/690) measure the active users graph and the logs' `elapsed` from the start of the load test, with or without `--dashboard`
    o **behavior change**: with the default metrics reset, the HTML report's active users graph shows the increase ramp instead of the final user count from second 0
    o **behavior change**: `GooseUser.started` is one clock shared by every user, set when the load test starts, so `elapsed` in the request, error, transaction and scenario logs counts milliseconds since the load test started instead of since that user's client was built
 - [#639](https://github.com/tag1consulting/goose/issues/639) only capture request headers when the request log, the debug log or the error log is enabled, skipping a string allocation per header on every request otherwise
    o **behavior change**: `GooseRawRequest.headers` (for example `goose.request.raw.headers`, or the `raw_request` in `TransactionError::RequestFailed`) is now empty unless one of these logs is enabled; log output is unchanged
 - [#731](https://github.com/tag1consulting/goose/pull/731) stop reparsing the request URL on every request; `request()` serialized the already parsed URL, parsed it again and allocated its path only for three log lines, and the `debug!` one is filtered out in normal runs
    o the path is now derived only when one of those lines is emitted; log output is unchanged and there is no behavior change
 - [#754](https://github.com/tag1consulting/goose/pull/754) give each request a reference to its transaction's name instead of copying it when the name is inherited, and share one empty transaction name instead of allocating one per request; on ks3 under load, instructions per request fell 0.68% where requests inherit their transaction's name and 0.28% otherwise, with output unchanged
 - [#468](https://github.com/tag1consulting/goose/issues/468) replace `--hatch-rate` and `--startup-time` with `--increase-rate`, `--increase-time`, `--decrease-rate`, and `--decrease-time`
    o **breaking**: `--hatch-rate` / `-r` is now `--increase-rate` / `-r` (sets per-second rate users are added)
    o **breaking**: `--startup-time` / `-s` is now `--increase-time` / `-s` (sets total time to launch all users)
    o new `--decrease-rate` option sets the per-second rate users are stopped during ramp-down
    o new `--decrease-time` option sets the total time for the ramp-down phase
    o `--decrease-rate` and `--decrease-time` are mutually exclusive (same as `--increase-rate` and `--increase-time`)
    o if neither decrease option is set, Goose shuts down all users as quickly as possible (previous behavior)
    o controller commands renamed: `hatchrate` → `increaserate`, `startup-time` → `increase-time`; new `decreaserate` and `decrease-time` commands added
    o **breaking**: `GooseDefault::HatchRate` renamed to `GooseDefault::IncreaseRate`
    o **breaking**: `GooseDefault::StartupTime` renamed to `GooseDefault::IncreaseTime`
    o new `GooseDefault::DecreaseRate` and `GooseDefault::DecreaseTime` defaults
    o **breaking**: `GooseConfiguration` fields `hatch_rate` → `increase_rate`, `startup_time` → `increase_time`; new `decrease_rate` and `decrease_time` fields added
    o **breaking**: `goose::util::get_hatch_rate()` renamed to `goose::util::parse_rate()`
 - [#674](https://github.com/tag1consulting/goose/issues/674) collapse per-request channel sends: add `GooseMetric::All` variant that bundles request, transaction, and scenario metrics into a single channel message; reduces channel message volume by up to 3× in the common one-request-per-transaction-per-scenario case
    o `GooseUser::set_success()` and `GooseUser::set_failure()` now take `&mut self` instead of `&self`; this is source-compatible since transaction functions always receive `&mut GooseUser`
 - [#680](https://github.com/tag1consulting/goose/pull/680) use atomic counters for hot-path success/failure request counts ([#677](https://github.com/tag1consulting/goose/issues/677))
 - **API change**: `Transaction::set_name()` now only affects transaction names, not request names (more intuitive default behavior)
    o existing `set_name()` calls will only name the transaction for metrics organization
    o request names remain descriptive (path-based or explicitly set)
    o users requiring the old behavior must switch to `set_name_for_transaction_and_requests()`
    o introduces `TransactionName` enum with `TransactionOnly` and `InheritNameByRequests` variants
 - [#656](https://github.com/tag1consulting/goose/pull/656) fix stack overflow when cloning `GooseUser` with session data
    o **breaking**: session data types must now implement `Clone` (add `#[derive(Clone)]` to your session structs)
 - [#655](https://github.com/tag1consulting/goose/pull/655) fix `--scenarios` to use exact matching instead of substring matching; add `*` wildcard support for pattern matching
 - [#578](https://github.com/tag1consulting/goose/pull/578) add type-safe client builder for cookie configuration, optimize startup with shared clients
 - [#629](https://github.com/tag1consulting/goose/pull/629) add `--pdf-print-html` option to generate printer-friendly HTML optimized for PDF conversion; provides two-step PDF workflow without requiring Chromium dependencies; add `--pdf-timeout` option for configurable Chrome timeout in direct PDF generation (10-300s, default: 60)
 - [#663](https://github.com/tag1consulting/goose/pull/663) add response time breakdowns grouped by HTTP status code in CLI and HTML/markdown reports; breakdowns only appear when a request returns multiple different status codes
 - [#657](https://github.com/tag1consulting/goose/pull/657) add comprehensive Basic Authentication example demonstrating three approaches: custom client with default headers (recommended), helper function per-request, and manual per-request; includes flexible credential configuration and complete documentation
 - [#669](https://github.com/tag1consulting/goose/pull/669) update reqwest to 0.13, switching the default TLS backend from native-tls to rustls. HTTP/2 and post-quantum cryptography are now advertised by default, which may increase connection handshake overhead when a computationally expensive cipher suite is negotiated.
   **Breaking changes**: Certificate validation now uses bundled rustls/webpki roots instead of the OS-native certificate store, which may affect environments with custom system CAs. `--accept-invalid-certs` now works unconditionally (previously it was silently ignored unless the `rustls-tls` feature was enabled).
 - [#672](https://github.com/tag1consulting/goose/pull/672) fix `--no-autostart` initializing user states twice when a host is configured (once at startup, again on controller Start command)
 - [#673](https://github.com/tag1consulting/goose/pull/673) parallelize user state initialization across available CPUs; significantly reduces startup time with large user counts (1000+)
 - [#678](https://github.com/tag1consulting/goose/pull/678) add support for non-HTTP protocol load testing (TCP, UDP, WebSocket, etc.)
    o new `GooseMethod::Custom` variant for non-HTTP requests
    o new `GooseUser::record_custom_request(method, name, response_time_ms, success, status_code, error)` method records arbitrary request metrics that appear in all Goose reports under a user-defined method label (e.g. `TCP`, `GRPC`, `WS`)
    o `status_code` parameter allows recording protocol-specific status codes (defaults to `0` for protocols without status codes)
    o add `examples/tcp_loadtest.rs` demonstrating how to load test a raw TCP echo server
 - [#660](https://github.com/tag1consulting/goose/pull/660), [#661](https://github.com/tag1consulting/goose/pull/661) add `--baseline-file` option to compare load test results against a previous JSON report; all report formats (HTML, Markdown, JSON) show deltas for requests, response times, transactions, scenarios, errors, and coordinated omission metrics
    o `GooseMethod` now derives `Copy` and `Hash`
    o JSON reports now include `scenarios` and `hosts` fields in `raw_metrics` (previously omitted by manual serialization)
    o **breaking**: `metrics::delta` and `metrics::nullable` modules are now crate-private; `DeltaValue`, `Value`, `ApplyBaseline`, and `NullableFloat` are no longer part of the public API
 - [#683](https://github.com/tag1consulting/goose/pull/683) use `Arc<str>` consistently for scenario/transaction names in metrics structs, eliminating per-invocation `String` allocations on the hot path ([#647](https://github.com/tag1consulting/goose/issues/647))
    o **breaking**: `Scenario.name`, `Scenario.machine_name`, `ScenarioMetric.name`, `TransactionMetric.name`, `ScenarioMetricAggregate.name`, and `TransactionMetricAggregate.scenario_name` change from `String` to `Arc<str>`
    o `Arc<str>` derefs to `&str`, so most read-only usage (formatting, logging, comparisons) works unchanged
    o fix flaky `test_status_code_response_time_tracking` integration test that assumed localhost responses always take ≥1ms
 - [#685](https://github.com/tag1consulting/goose/pull/685) log info message when writing JSON and Markdown report files, matching existing HTML and PDF report behavior ([#604](https://github.com/tag1consulting/goose/issues/604))
 - [#682](https://github.com/tag1consulting/goose/pull/682) batch metrics per user: each `GooseUser` pre-aggregates its metrics locally and sends one `GooseMetric::Batch` message every 100 requests or 250ms, cutting metrics channel traffic by up to 100× ([#675](https://github.com/tag1consulting/goose/issues/675))
    o fix transaction and scenario response times over 500ms being bucketed 10× (501 to 1000ms) or 100× (over 1000ms) too low, which skewed their percentiles; request times were not affected (also fixed on `main` in [#688](https://github.com/tag1consulting/goose/pull/688) by [@aditanase](https://github.com/aditanase))
 - [#691](https://github.com/tag1consulting/goose/pull/691) fix clippy lints and deprecations reported by Rust 1.99
    o **breaking**: `TransactionError::RequestFailed { raw_request }` changes from `GooseRequestMetric` to `Box<GooseRequestMetric>`, and `TransactionError::LoggerFailed { source }` changes from `flume::SendError<Option<GooseLog>>` to `Box<flume::SendError<Option<GooseLog>>>`
 - [#690](https://github.com/tag1consulting/goose/pull/690), [#682](https://github.com/tag1consulting/goose/pull/682) fix empty or misplaced live dashboard charts without `--report-file`: batched metrics bypassed the dashboard's chart condition and series origin, so batched requests, transactions and scenarios were not charted and batched latency was indexed by absolute second after pruning
 - [#703](https://github.com/tag1consulting/goose/issues/703) split the dashboard client into TypeScript modules bundled by esbuild, with Chart.js 4.4.7 and its types from npm
    o chart configuration is now type checked against Chart.js's own types
    o `app.js` is still committed, so Cargo builds need no Node.js, and CI fails if it differs from the esbuild bundle
    o **behavior change**: `/static/chart.min.js` is no longer served (404); Chart.js is inside `app.js`
 - [#714](https://github.com/tag1consulting/goose/issues/714) add scenario and transaction tables to the live dashboard, read from the metrics Goose already keeps, with nothing added to the request path
    o the snapshot adds `scenarios`, `transactions`, `flags.scenario_metrics_disabled` and `flags.transaction_metrics_disabled`
    o with `--no-transaction-metrics` or `--no-scenario-metrics` the table says collection is off instead of showing no rows

## 0.18.1 August 14, 2025
 - [#634](https://github.com/tag1consulting/goose/pull/634) add killswitch mechanism for programmatic test termination
 - [#630](https://github.com/tag1consulting/goose/pull/630) enhanced coordinated omission metrics with severity classification, detailed tracking, and comprehensive documentation
 - [#632](https://github.com/tag1consulting/goose/pull/632) add Context7 support for enhanced AI code assistant integration
 - [#627](https://github.com/tag1consulting/goose/pull/627) add comprehensive test coverage for test plan parsing, implement `Default` for `TestPlan`
 - [#625](https://github.com/tag1consulting/goose/pull/625) standardize logging format across codebase with consistent module prefixes
 - [#633](https://github.com/tag1consulting/goose/pull/633), [#622](https://github.com/tag1consulting/goose/pull/622) fix clippy warnings for latest Rust version
 - [#620](https://github.com/tag1consulting/goose/pull/620) fix error graph display in HTML report
 - update all dependencies

## 0.18.0 February 25, 2025
 - update all dependencies
 - [#565](https://github.com/tag1consulting/goose/pull/565) add `--accept-invalid-certs` to skip validation of https certificates
 - [#568](https://github.com/tag1consulting/goose/pull/568) don't panic when truncating non utf-8 string
 - [#574](https://github.com/tag1consulting/goose/pull/574) update [`http`](https://docs.rs/http), [`itertools`](https://docs.rs/itertools) [`nix`](https://docs.rs/nix), [`rustls`](https://docs.rs/rustls/), and [`serial_test`](https://docs.rs/serial_test)
 - [#575](https://github.com/tag1consulting/goose/pull/575) add test coverage for sessions and cookies, revert [#557](https://github.com/tag1consulting/goose/pull/557) to avoid sharing the CookieJar between all users
 - [#600](https://github.com/tag1consulting/goose/pull/600) Refactor reports/metrics, add JSON and markdown report
 - [#629](https://github.com/tag1consulting/goose/pull/629) add optional PDF report generation with `pdf-reports` feature flag; supports configurable scale via `--pdf-scale` option

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
