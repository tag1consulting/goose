# Tips

The tips follow a load test's life: writing it, checking it, the first runs, scaling up, reading the results and debugging. If you are back with a problem, start here:

- A request that should fail passes: [Check the page, not only the status code](#check-the-page-not-only-the-status-code).
- The machine running Goose is the limit: [Check that Goose is not the bottleneck](#check-that-goose-is-not-the-bottleneck).
- Requests fail to connect once you add users: [Raise the open file limit](#raise-the-open-file-limit).
- The reports are missing: [Write the reports, and keep each run](#write-the-reports-and-keep-each-run).
- A CI job should fail: [Make a CI job fail on slow responses](#make-a-ci-job-fail-on-slow-responses).
- Response times look too good: [When response times look too good](#when-response-times-look-too-good).
- Requests time out: [When requests time out](#timeouts).
- A run never ends: [When a run never ends](#when-a-run-never-ends).
- A command from an older post fails: [Options renamed since the older posts](#options-renamed-since-the-older-posts).

## Writing a load test

### Handle every error

A request method such as `get` returns `Ok` even when the request fails, with the failure inside the response, so `?` on the call does not stop the transaction. Goose still counts the request as failed; see [A failed request does not fail its transaction](#a-failed-request-does-not-fail-its-transaction). Under load, an `unwrap()` on `goose.response` panics the first time a request gets no response, as when it times out, and one on its body panics when the body cannot be read. Handle those with `if let` or `match`, and return other errors with `?`. Goose records an error a transaction returns and keeps the user running.

```rust,ignore
async fn website_index(user: &mut GooseUser) -> TransactionResult {
    let goose = user.get("/").await?;
    if let Ok(response) = goose.response {
        let _text = response.text().await;
    }
    Ok(())
}
```

### Name requests that carry an ID

Goose groups requests by name, and a request's name is its path unless you give one. A test that requests `/node/1`, `/node/2` and so on gets a row for every node; one name for them all gives one row whose numbers cover every node.

```rust,ignore
let path = format!("/node/{nid}");
let _goose = user.get_named(&path, "/node/%nid").await?;
```

### Give users a wait time

A scenario without a wait time sends each user's next request as soon as the last one ends, so each user loads the server as hard as it can. `set_wait_time` pauses each user for a random time in a range after each transaction, as a person would. With a wait time, the same load needs more users.

```rust,ignore
scenario!("WebsiteUser")
    .set_wait_time(Duration::from_secs(5), Duration::from_secs(15))?
```

### Stop the run from your own code

`goose::trigger_killswitch` stops a run as Ctrl-C does, with every report written; see [Killswitch](killswitch.md). A [threshold](thresholds.md), set on the command line or in code with `GooseDefault::Threshold`, stops the run when a metric such as the error rate crosses a limit.

```rust,ignore
goose::trigger_killswitch("Server returned 503: Service Unavailable");
```

## Checking a load test

### Check the page, not only the status code

A `200` with an error page is still a `200`. Check the response for the content you expect, and mark the request failed with `set_failure` when it is not there. Pass the page to `set_failure` so the debug log keeps it; [Request failures](../logging/debug.md#request-failures) explains `set_failure`, and [Validating Requests](validation.md) shows other checks. In Goose 0.19 the request log still shows such a request as a success, with `"success":true`; see [List every failed request](#list-every-failed-request).

```rust,ignore
let mut goose = user.get("/").await?;
if let Ok(response) = goose.response {
    let text = response.text().await.unwrap_or_default();
    if !text.contains("<title>Home</title>") {
        return user.set_failure("title not found", &mut goose.request, None, Some(&text));
    }
}
```

### Start with one user and a debug log

Run one user for a short time with a debug log before adding load, so each request is easy to follow. Goose writes an entry to the debug log each time the load test calls [`log_debug`](../logging/debug.md) and each time `set_failure` marks a request as failed; a test that does neither gets an empty debug log. Add `-R` for a line for every request as well. Without file names, `-D` writes `debug.json` and `-R` writes `requests.json` into the run's folder, `goose-runs/<start time>/`; see [Output Files](output-files.md).

```bash
cargo run --release -- -H http://localhost:8080 -u 1 -t 30s -D -R
```

## First runs

### Build with `--release`

`cargo run` without `--release` builds a debug binary, which sends far less load: Tag1 measured about 13 times the traffic from a release build in [The golden Goose egg](https://www.tag1.com/blog/golden-goose-egg-compile-time-adventure/). Every `cargo run` on this page uses `--release`.

```bash
cargo run --release -- -H http://localhost:8080
```

### Always name the host

A load test can set a default host in its code, as the [Umami example](../example/umami.md) does, and a run without `-H` (`--host`) then loads that host. A load test with no default host refuses to start without `-H`. Pass it every time.

```bash
cargo run --release -- -H https://staging.example.com
```

### Stop with one Ctrl-C

Ctrl-C stops the run cleanly: users stop, every report is written and the final metrics print. A second Ctrl-C exits at once and writes none of them, so wait for the first to finish. The run in [Running A Load Test](running.md) shows the line Goose prints:

```text
17:14:04 [WARN] caught ctrl-c, stopping...
```

### Start the run from the dashboard

With the `dashboard` feature, `--no-autostart` starts Goose idle, so you can check the settings in the dashboard, at `http://127.0.0.1:5118/`, before you press Start. Pressing Start needs `--dashboard-control`, which in turn needs `--dashboard-auth-token` or `--dashboard-no-auth`; the command below uses `--dashboard-no-auth`, which suits a test on your own machine only. See [Live Dashboard](../controller/dashboard.md).

```bash
cargo run --release --features goose/dashboard -- -H http://localhost:8080 \
  --dashboard --dashboard-control --dashboard-no-auth --no-autostart
```

## Scaling up

### Check that Goose is not the bottleneck

When the machine running Goose runs out of CPU, the results measure Goose, not the server: response times grow and requests per second stop rising as you add users. Watch the Goose process in `top` or `htop` while the test runs; replace `loadtest` below with your binary's name, which is the crate's name unless you set another. Goose uses every core, so `top` can show it at up to 100% times the number of cores; near that, or with every core busy in `htop`, Goose is the limit. Build with `--release`, send less load from this machine with fewer users or [`--throttle-requests`](#cap-the-request-rate), or use a larger machine.

```bash
top -p $(pgrep -nf target/release/loadtest)      # Linux
top -pid $(pgrep -nf target/release/loadtest)    # macOS
```

### Raise the open file limit

By default each user holds its own connections, and each connection is an open file. On Linux and macOS the default limit is often 1,024 or lower, and Goose does not raise it. Past the limit, requests fail with status code `0` and `client error (Connect)`, and nothing in the output names open files. A test with no wait time can also run on past `-t`, and its first Ctrl-C does not stop it; a second ends it, with no reports. Raise the limit in the shell that runs Goose; [A Goose in the clouds](https://www.tag1.com/blog/goose-clouds-load-testing-scale/) mentions raising it for a large run.

```bash
ulimit -n 100000
```

### Ramp up slowly

Goose starts one user a second by default. A high `-r` (`--increase-rate`, users a second) starts many users at once, which can overwhelm a cold server and measure its warm up, not its steady state. For a large run, spread the start over time with `-s` (`--increase-time`), or give the run a shape with a [test plan](test-plan.md).

```bash
cargo run --release -- -H http://localhost:8080 -u 500 --increase-time 5m -t 30m
```

### Cap the request rate

`--throttle-requests` caps the requests Goose sends each second, whatever the number of users. Use it to stay under a rate limit, or to hold a steady load while you change something else; see [Throttle](throttle.md).

```bash
cargo run --release -- -H http://localhost:8080 -u 100 --throttle-requests 200
```

### Use every core of one machine

One Goose process runs its users across every CPU core; set `-u` for the load you need rather than starting more processes. Distributed load tests with Gaggles, which the older posts describe, return in Goose 0.20; see [Gaggle](../gaggle/overview.md).

```bash
cargo run --release -- -H http://localhost:8080 -u 1000 -s 5m
```

## Reading the results

### Write the reports, and keep each run

A run writes no report files unless you ask for them. A `--report-file` without a file name, or `--dashboard`, writes `report.html` and `report.json` into the run's folder, `goose-runs/<start time>/` under the directory you ran Goose from; Goose logs the folder as the run starts (see [Output Files](output-files.md)). A file name you give is written where you name it, relative to the directory you ran Goose from. Reports are also missing after a second Ctrl-C (see [Stop with one Ctrl-C](#stop-with-one-ctrl-c)), and `--report-file` is refused with `--no-metrics`. `--notes` labels the run, and `--baseline-file` compares a later run with an earlier one; see [Comparing against a baseline](common.md#comparing-against-a-baseline).

```bash
cargo run --release -- -H http://localhost:8080 --report-file --notes "before the cache change"
```

### The tables start once every user is running

By default Goose resets its metrics once every user has started, so the tables describe the run at full load. `--no-reset-metrics` keeps the ramp up in them. A run with a test plan or `--iterations` never resets. See [Console metrics](metrics.md#ascii-metrics).

```bash
cargo run --release -- -H http://localhost:8080 --no-reset-metrics
```

### A failed request does not fail its transaction

A request fails on an error status code, when no response comes back, or when your code calls `set_failure`. A transaction fails only when its function returns an error. This run of the [`simple` example](../example/simple.md), with a 1 second timeout, loaded a local server that answers `/` after 3 seconds. Both requests for `/` timed out, but transaction `0.1`, which requests `/`, did not fail. The example's transactions have no names, so its Markdown report numbers them by scenario and transaction, from 0:

```bash
cargo run --release --example simple -- -H http://127.0.0.1:19050 \
  -u 2 -r 2 -t 15s --timeout 1 --no-telnet --report-file report.md
```

```markdown
## Request Metrics

| Method | Name | # Requests | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ------ | ---- | ---------: | ------: | -----------: | -------: | -------: | --: | ---------: |
| GET | / | 2 | 2 | 1001 | 1001 | 1001 | 0.13 | 0.13 |
...
## Transaction Metrics

| Transaction | # Times Run | # Fails | Average (ms) | Min (ms) | Max (ms) | RPS | Failures/s |
| ----------- | ----------: | ------: | -----------: | -------: | -------: | --: | ---------: |
| **WebsiteUser** |
...
| 0.1  | 2 | 0 | 1001 | 1001 | 1001 | 0.13 | 0.00 |
```

To make a failed request fail its transaction, build it with `GooseRequest::builder().error_on_fail()` and send it with `user.request`, or return what `set_failure` returns, as [Check the page](#check-the-page-not-only-the-status-code) does; see [Validating Requests](validation.md).

### Make a CI job fail on slow responses

`--threshold` judges the run against a rule. A rule that fails makes the run fail and, by default, stops it while it runs; with `--no-threshold-abort` the run goes to the end and is judged then. Either way `execute()` returns an error, so a `main` that returns it with `?` exits with a nonzero status; see [Exit code in CI](thresholds.md#exit-code-in-ci).

```bash
cargo run --release -- -H http://localhost:8080 -t 5m --threshold "p95<500ms" --threshold "error-rate<1%"
```

### When response times look too good

First check `# Fails` in the request table, and that the pages are the ones you expect (see [Check the page](#check-the-page-not-only-the-status-code)): fast failures look like fast responses. When a server stalls a request, its user waits instead of sending the requests it would have sent, so the wait shows in one response time and the requests it never sent are missing from the measured times. `--co-mitigation average` adds synthetic response times for those missing requests, without sending more load; [Was my run affected?](../coordinated-omission/metrics.md#was-my-run-affected) shows how to read the result.

```bash
cargo run --release -- -H http://localhost:8080 --co-mitigation average --report-file
```

## Debugging

### When requests time out {#timeouts}

Goose times out a request after 60 seconds. A timeout is usually the finding, since the server could not answer in time at this load. Raise the limit only when you expect slow answers: the requests that would have timed out then count as slow responses, not failures. A timed-out request counts at the limit in the response times. In [the run above](#a-failed-request-does-not-fail-its-transaction), with `--timeout 1`, `GET /` shows 1,001 ms as its average, minimum and maximum. `--timeout` sets the limit in seconds, decimals allowed.

```bash
cargo run --release -- -H http://localhost:8080 --timeout 120
```

To remove the limit, build a client of your own with [`GooseUser::set_client_builder`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_client_builder); for a limit no request will reach, set a very long timeout, such as `--timeout 86400` for a day.

### See the page a request got

Run with `-D` (`--debug-log`), as in [Start with one user and a debug log](#start-with-one-user-and-a-debug-log): each entry in `debug.json` is a JSON object, and `body` holds the page your code passed to `set_failure` or `log_debug`, as [Check the page](#check-the-page-not-only-the-status-code) does. [jq](https://jqlang.org/) or [jaq](https://crates.io/crates/jaq) can pull it out to open in a browser; you may need to turn off JavaScript to view it. The request log below records every request, but not the page it got.

```bash
head -n 1 goose-runs/<start time>/debug.json | jq -r .body > page.html
```

### List every failed request

The error table, in the reports and in the console's final metrics, says why requests failed, as in this Markdown report of [the timeout run](#a-failed-request-does-not-fail-its-transaction):

```markdown
## Error Metrics

| Method | Name |  #  | Error |
| ------ | ---- | --: | ----- |
| GET | / | 2 | error sending request /: operation timed out |
```

`-R` (`--request-log`) writes a line for every request; without a file name it writes `requests.json` into the run's folder, and jq can list the ones that failed. The `simple` example with `-R`, run again against the slow server for 10 seconds, gave these two lines:

```bash
cargo run --release --example simple -- -H http://127.0.0.1:19052 \
  -u 2 -r 2 -t 10s --timeout 1 --no-telnet -R -q
jq -c 'select(.success == false) | {name, status_code, error}' goose-runs/<start time>/requests.json
```

```json
{"name":"/","status_code":0,"error":"error sending request /: operation timed out"}
{"name":"/","status_code":0,"error":"error sending request /: operation timed out"}
```

A status code of `0` means no response came back; see [Response time breakdowns by status code](metrics.md#response-time-breakdowns-by-status-code). A request your code marks failed with `set_failure` still shows `"success":true` in the request log, so the jq command above skips it; `-E` (`--error-log`) writes a line for each of them, as for every other failed request.

### When a run never ends

A run without `-t` or `--iterations`, or with a test plan whose last step still has users, goes on until Ctrl-C (end a plan with a step to 0 users, such as `0,0s`), and a run past the open file limit can too (see [Raise the open file limit](#raise-the-open-file-limit)). When `-t` runs out, or at the first Ctrl-C, each user first finishes its transaction: after `entering GooseAttack phase: Decrease`, each request left in it can wait up to the [timeout](#timeouts), 60 seconds by default, against a stalled server, before the reports are written. Unless a threshold or the killswitch stops it, a run started from the dashboard or a Controller leaves Goose up when it ends, and so does a `--dashboard` run that ends on its own with its report saved and a dashboard tab open. Goose then stays up, even after the tab is closed, until Ctrl-C, a Controller `shutdown`, or the dashboard's Quit button, which needs `--dashboard-control` (see [When the run ends](../controller/dashboard.md#when-the-run-ends)). In Goose 0.19, `-v` can hang a run partway through printing its metrics, where not even two Ctrl-Cs stop it: leave out `--running-metrics` when you use `-v`, and add `--no-reset-metrics` against a fast server. From another terminal, end a hung run with `kill` and its process ID, which writes no reports; replace `loadtest` with your binary's name:

```bash
kill $(pgrep -nf target/release/loadtest)
```

## Options renamed since the older posts

Tag1's blog posts about Goose from 2020 and 2021, such as [Using Gaggles to distribute your load testing](https://www.tag1.com/blog/using-gaggles-distribute-your-load-testing/), use options that have been renamed or removed. Their code also uses the names from before Goose 0.16: `taskset!` is now `scenario!`, `register_taskset` is `register_scenario`, `task!` is `transaction!`, `register_task` is `register_transaction`, `GooseTaskSet` is `Scenario`, `GooseTaskError` is `TransactionError`, and `GooseTaskResult` is `TransactionResult`.

| In the posts | Now |
|---|---|
| `-c`, `--clients` | `-u`, `--users` |
| `--hatch-rate` | `-r`, `--increase-rate` |
| `--startup-time` | `-s`, `--increase-time` |
| `--log-file` | `-G`, `--goose-log` |
| `--debug-file`, `--debug-log-file` | `-D`, `--debug-log` |
| `--requests-file`, `--metrics-file`, `--stats-log-file` | `-R`, `--request-log` |
| `--metrics-format`, `--requests-format` | `--request-format` |
| `--task-log` | `-T`, `--transaction-log` |
| `--task-format` | `--transaction-format` |
| `--print-stats` | the default; `--no-print-metrics` turns the printing off, `--no-metrics` the collecting |
| `--only-summary` | the default; `--running-metrics N` adds running metrics |
| `--reset-stats` | the default |
| `--no-stats` | `--no-metrics` |
| `--status-codes` | the default; `--no-status-codes` turns it off |
| `--no-task-metrics` | `--no-transaction-metrics` |
| `--manager`, `--worker` and the other Gaggle options | none in Goose 0.19; Gaggles return in 0.20, see [Gaggle](../gaggle/overview.md) |
