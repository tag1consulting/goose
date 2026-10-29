# Thresholds

A threshold is a pass/fail rule on a load test's metrics, such as "the 95th percentile response time stays under 500 ms" or "under 1% of requests fail". Give one with `--threshold`, as often as needed. A rule that fails stops the run by default, and always makes the run fail: `GooseAttack::execute()` returns an error once every report is written (see [Exit code in CI](#exit-code-in-ci)). Every report says which rules passed and which failed.

```bash
cargo run --release -- -H http://local.dev/ -u 50 -r 10 -t 10m \
    --threshold "p95<500ms" \
    --threshold "error-rate<1%" \
    --threshold "p99<2s for GET /checkout"
```

Quote each rule: `<` and `>` redirect in a shell.

Thresholds give one pass or fail verdict per run. They are meant for a simple load test, one that ramps up, holds a single user count, and ramps down. To check several user counts, run a separate load test for each one. For how `rps` is counted, see [`rps` counts only the time users hold steady](#rps-counts-only-the-time-users-hold-steady).

## Syntax

A rule is `<metric><op><value>`, optionally followed by ` for <request>`.

| Metric | Measures | Value |
| ------ | -------- | ----- |
| `pNN` | A response time percentile, with optional decimals, such as `p50`, `p95` or `p99.9`, greater than 0 and less than 100 | A time: a number with `us`, `ms` or `s`; a bare number is milliseconds |
| `avg` | The average response time | A time |
| `max` | The slowest response time | A time |
| `error-rate` | Failed requests as a percent of all requests | A number with an optional `%`, at most 100 |
| `rps` | Requests per second while the run holds its users steady | A number |

The operator is `<`, `<=`, `>` or `>=`. A rule passes when the comparison holds for what Goose observed: `p95<500ms` passes at 499 ms and fails at 500 ms, while `p95<=500ms` passes at 500 ms too.

Without ` for <request>`, a rule judges all requests together, as the `Aggregated` row of the report's tables does. With it, the rule judges only the request with that name: its full name as the HTML or Markdown report shows it, method and name together, such as `GET /login`, where the name is the path or the request's custom name, as in `POST login`. The console's tables cut long names short, so copy a name from a report. When the run ends, Goose warns about a rule for a request the run never made, suggesting the name it most likely means:

```
threshold p95<1s for get /: no request named "get /"; did you mean "GET /"?
```

Some rules:

- `p95<500ms`: 95% of all responses take less than 500 ms.
- `p99.9<=2s`: 99.9% of all responses take at most 2 seconds.
- `avg<150`: the average response is under 150 ms.
- `error-rate<1%`: fewer than 1% of requests fail.
- `rps>=200`: the run makes at least 200 requests a second while it holds its users steady.
- `max<5s for GET /checkout`: no `GET /checkout` takes 5 seconds or more.

A rule that does not parse is refused before any user starts, with a `GooseError::InvalidOption` whose `detail` says what is wrong with it:

```
threshold "p100<1s": the percentile must be greater than 0 and less than 100, such as p95 or p99.9.
```

So is a rule that can never pass: any rule with `<0`, such as `p95<0` or `error-rate<0`, as well as `rps<=0` and `error-rate>100` (with or without `%`). With `--throttle-requests N`, which keeps the run at N requests a second or fewer, `rps>M` is refused when M is N or more, and `rps>=M` when M is more than N:

```
threshold "rps>=200": it can never pass; `configuration.throttle_requests` caps the run at 100 requests a second.
```

`--throttle-requests 0` means no throttle, so it refuses no `rps` rule.

Thresholds judge metrics, so `--threshold` is not allowed with `--no-metrics`.

A percentile comes from the same [histogram buckets](https://github.com/tag1consulting/goose/blob/main/src/metrics/buckets.rs) as every report's percentiles, so it can be up to 0.78% away from the exact time, while the average and the slowest time are exact.

## Stopping the run

By default Goose checks every rule once a second while the load test runs, and the first rule that fails stops the run through the [killswitch](killswitch.md), with a reason that names the rule and what was observed. In this run of the `simple` example, a local test server, not part of Goose, answered each request in 1.5 seconds until 12 seconds after the first request it received, then failed every request:

```bash
cargo run --release --example simple -- -H http://127.0.0.1:19862 -u 50 -r 50 -t 60s \
    --threshold "p95<2s" --threshold "error-rate<1%" --no-telnet
```

Once 1% or more of requests had failed, the next check stopped the run, 12 seconds after the run entered Maintain:

```
19:44:39 [INFO] Killswitch triggered: threshold error-rate<1% failed: observed 2.54%
```

A threshold failure is a killswitch stop, and works as any other [killswitch](killswitch.md) does. The run then fails, as below.

With `--no-threshold-abort`, Goose never stops the run for a threshold. It runs its full time, and judges every rule only at the end.

### When checking starts and stops

Checking starts when the run first enters Maintain, the first time it holds its users steady, in every mode. A ramp up therefore never fails a rule while the run goes. A users ramp resets its metrics as it enters Maintain. Under `--test-plan`, `--iterations` or `--no-reset-metrics` they are not reset, so the ramp up's requests still count in the first check, except in `rps`, which counts only the hold.

Once checking starts, every later Increase or Maintain step of a test plan is checked too. Checking pauses while the run stops users, in Decrease, and ends at Shutdown. The judgement at the end still counts every request since the metrics reset, or since the start of the run when there is none, including those made in Decrease. `rps` is the exception, below.

A test plan that never holds its users steady, such as `10,1m;0,1m`, never enters Maintain, so its rules are judged only at the end, and its `rps` rules can not pass. Goose warns about this at startup, unless `--no-threshold-abort` is set and no rule is an `rps` rule.

A rule whose metric has no samples yet, such as a rule for a request that has not run, is skipped while the run goes. The first check can judge only a few requests, so one early error can fail `error-rate`. The first seconds of the first hold can also fail an `rps>=` rule that the whole hold meets. `--threshold-abort-delay` gives the run time to make more requests before the first check.

### `rps` counts only the time users hold steady

An `rps` rule counts only the time the run spends holding its users steady, in Maintain. This is true in every mode, both in the judgement at the end and in any checks while the run goes. It counts the requests made while the run holds its users, over the seconds it holds them, added up over every hold of a test plan. Holds at different user counts therefore give one rate, and an `rps>=` rule judges that one rate. An `rps` rule's row gives the rate followed by `from Maintain`. A ramp up, a ramp down, and an Increase step between two holds add nothing, so none of them pulls the rate down. The report's Aggregated requests per second can differ: it covers every second since the metrics reset. A run under `--test-plan`, `--iterations` or `--no-reset-metrics` has no reset, so there the ramp up's lower rate pulls that figure down, and an `rps>=` rule judged on it would fail although the steady load meets it.

A test plan step that stays at 0 users after a ramp down and before a later ramp up is a hold too: it adds seconds and no requests, so it pulls the rate down. In `10,10s;10,20s;0,5s;0,20s;10,10s;10,20s;0,5s`, that step is `0,20s`. Leave such a step out of a test plan with an `rps` rule: without `0,20s`, the ramp down leads straight into the next ramp up, and the rate covers only the two holds at 10 users.

`--iterations` is refused at startup with `--decrease-rate` or `--decrease-time`, as a ramp down would stop users before they finish their iterations: the error reads ``` `configuration.decrease_rate` can not be set with `configuration.iterations`. ```, or the same for `decrease_time`. With `--iterations`, the run enters Maintain when the ramp up ends and stays there until the last user finishes its iterations. A run whose users all finish before the ramp up ends never enters Maintain. Once the first user finishes, the rest of the hold has fewer users making requests. That time still counts, so it pulls the rate down. A run with `--run-time` in place of `--iterations` keeps every user making requests until the hold ends, so use `--run-time` with an `rps` rule.

Until the run has held its users for a whole second there is no rate to judge. A run that never enters Maintain has no time to count, so its `rps` rules fail at the end as `no samples`.

### `--threshold-abort-delay`

`--threshold-abort-delay` holds the first check back by a time after the run first enters Maintain, given as for `--run-time`: `30s`, `2m`, or a bare number of seconds. It never changes the judgement at the end of the run.

Use it when the first seconds of a run are slow for a reason you accept, such as a cold cache. With `--threshold-abort-delay 1m`, a warm up spike in the first minute after the run enters Maintain can not stop the run, but it still counts in the final percentiles, so a rule it breaks in the final metrics still fails at the end:

```bash
cargo run --release -- -H http://local.dev/ -u 50 -r 50 -t 10m \
    --threshold "p95<500ms" --threshold-abort-delay 1m
```

It needs a `--threshold`, and is not allowed with `--no-threshold-abort`, which never stops the run.

## The end of the run

When the run ends, Goose judges every rule once more on the final metrics. Here a rule whose metric has no samples fails, observed as `no samples`: a rule for a request that never ran says nothing about that request, so it can not pass.

The rule that stopped the run always fails. Requests that finish after it stopped, and any the check had not counted yet, are counted afterwards, so by the end its value can be higher or lower than when it stopped the run. Its row shows the value at the stop and at the end, such as `2.54% at abort, 6.45% at end` in the run under [Stopping the run](#stopping-the-run), or one value when the two are the same.

When every rule passes, `execute()` returns `Ok(GooseMetrics)` as usual. When any fails, it returns `GooseError::ThresholdsFailed` once every report is written. The error holds the rules that failed and the run's metrics, as a `ThresholdMetrics` that reads as the `GooseMetrics` it holds:

```rust,ignore
match goose_attack.execute().await {
    Ok(metrics) => println!("all {} thresholds passed", metrics.thresholds.len()),
    Err(GooseError::ThresholdsFailed { failed, metrics }) => {
        for result in &failed {
            eprintln!("{} failed: observed {}", result.rule, result.observed);
        }
        eprintln!("the run lasted {} seconds", metrics.duration);
        std::process::exit(1);
    }
    Err(e) => return Err(e),
}
```

`GooseMetrics::thresholds` holds every rule's `ThresholdResult` in either case: its rule, what was observed, as text and as a number, whether it passed, and whether it stopped the run.

### Exit code in CI

A load test whose `main` returns `Result<(), GooseError>` and uses `?` on `execute()`, as every example does, exits with code 1 when a threshold fails, so a CI step fails with it. This step runs the `simple` example from a Goose checkout; in your own load test crate, use `cargo run --release --` instead:

```yaml
- name: Load test
  run: |
    cargo run --release --example simple -- \
      --host http://staging.example.com --users 20 --increase-rate 5 --run-time 5m \
      --threshold "p95<500ms" --threshold "error-rate<1%" \
      --report-file report.html
```

The console output of the run under [Stopping the run](#stopping-the-run) ends with the rule that failed, then the error `main` returns, before Goose exits with code 1:

```text
19:44:40 [ERROR] threshold error-rate<1% failed: observed 2.54% at abort, 6.45% at end
Error: ThresholdsFailed { failed: [ThresholdResult { rule: "error-rate<1%", observed: "2.54% at abort, 6.45% at end", value: Some(6.451612903225806), passed: false, aborted: true }], metrics: ThresholdMetrics { .. } }
```

## Reports

The PDF, HTML and Markdown reports have a Thresholds section with a row for each rule: the rule, what was observed, and the result. A failing row says `FAIL`, and `FAIL, stopped the run` for the rule that stopped it, so the result never rests on color alone. The JSON report keeps each result as an object in `raw_metrics.thresholds`, with `rule`, `observed`, `value`, `passed` and `aborted`, so a report rendered from it with `--render-report` shows the same section. Without thresholds there is no section. The console's final tables have the same section; the run under [Stopping the run](#stopping-the-run) printed:

```
 === THRESHOLDS ===
 ------------------------------------------------------------------------------
 Rule                                         |                     Observed | Result
 ------------------------------------------------------------------------------
 p95<2s                                       |                       1499ms | PASS
 error-rate<1%                                | 2.54% at abort, 6.45% at end | FAIL, stopped the run
 ------------------------------------------------------------------------------
```

## Setting thresholds in code

Set rules in code with `GooseDefault::Threshold`. Each call adds one rule, so code can set several. A rule that does not parse is refused by `set_default` itself:

```rust
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .set_default(GooseDefault::Threshold, "p95<500ms")?
        .set_default(GooseDefault::Threshold, "error-rate<1%")?
        // Judge them only at the end of the run.
        .set_default(GooseDefault::NoThresholdAbort, true)?;

    Ok(())
}
```

Rules given with `--threshold` replace every rule set in code; they are not added to them. `GooseDefault::ThresholdAbortDelay` sets the delay, in seconds.

## Overhead

Thresholds do no work per request: Goose judges each rule against the metrics it already keeps, once a second while it checks (see [When checking starts and stops](#when-checking-starts-and-stops)) and once at the end, or only at the end under `--no-threshold-abort`.
