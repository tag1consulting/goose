# Thresholds

A threshold is a pass/fail rule on a load test's metrics, such as "the 95th percentile response time stays under 500 ms" or "under 1% of requests fail". Give one with `--threshold`, as often as needed. A rule that fails stops the run by default, and always makes the run fail: `GooseAttack::execute()` returns an error once every report is written (see [Exit code in CI](#exit-code-in-ci)). Every report says which rules passed and which failed.

```bash
cargo run --release -- -H http://local.dev/ -u 50 -r 10 -t 10m \
    --threshold "p95<500ms" \
    --threshold "error-rate<1%" \
    --threshold "p99<2s for GET /checkout"
```

Quote each rule: `<` and `>` redirect in a shell.

## Syntax

A rule is `<metric><op><value>`, optionally followed by ` for <request>`.

| Metric | Measures | Value |
| ------ | -------- | ----- |
| `pNN` | A response time percentile, with optional decimals, such as `p50`, `p95` or `p99.9`, greater than 0 and less than 100 | A time: a number with `us`, `ms` or `s`; a bare number is milliseconds |
| `avg` | The average response time | A time |
| `max` | The slowest response time | A time |
| `error-rate` | Failed requests as a percent of all requests | A number with an optional `%`, at most 100 |
| `rps` | Requests per second, from when checking starts | A number |

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
- `rps>=200`: the run makes at least 200 requests a second.
- `max<5s for GET /checkout`: no `GET /checkout` takes 5 seconds or more.

A rule that does not parse is refused before any user starts, with a `GooseError::InvalidOption` whose `detail` says what is wrong with it:

```
threshold "p100<1s": the percentile must be greater than 0 and less than 100, such as p95 or p99.9.
```

So is a rule that can never pass: any rule with `<0`, such as `p95<0` or `error-rate<0`, as well as `rps<=0` and `error-rate>100` (with or without `%`). With `--throttle-requests N`, which keeps the run at N requests a second or fewer, `rps>M` is refused when M is N or more, and `rps>=M` when M is more than N:

```
threshold "rps>=200": it can never pass; `configuration.throttle_requests` caps the run at 100 requests a second.
```

Thresholds judge metrics, so `--threshold` is not allowed with `--no-metrics`.

A percentile comes from the same [histogram buckets](https://github.com/tag1consulting/goose/blob/main/src/metrics/buckets.rs) as every report's percentiles, so it can be up to 0.78% away from the exact time, while the average and the slowest time are exact.

## Stopping the run

By default Goose checks every rule once a second while the load test runs, and the first rule that fails stops the run through the [killswitch](killswitch.md), with a reason that names the rule and what was observed:

```
Killswitch triggered: threshold p95<500ms failed: observed 812ms
```

A threshold failure is a killswitch stop, and works as any other [killswitch](killswitch.md) does. The run then fails, as below.

With `--no-threshold-abort`, Goose never stops the run for a threshold. It runs its full time, and judges every rule only at the end.

### When checking starts and stops

Checking starts when the run first enters Maintain, the first time it holds its users steady, in every mode. A ramp up therefore never fails a rule while the run goes. A users ramp resets its metrics as it enters Maintain. Under `--test-plan`, `--iterations` or `--no-reset-metrics` they are not reset, so the ramp up's requests still count in the first check.

Once checking starts, every later Increase or Maintain step of a test plan is checked too. Checking pauses while the run stops users, in Decrease, and ends at Shutdown. The judgement at the end still covers the whole run.

A test plan that never holds its users steady, such as `10,1m;0,1m`, never enters Maintain, so its rules are judged only at the end. Goose warns about this at startup.

A rule whose metric has no samples yet, such as a rule for a request that has not run, is skipped while the run goes. The first check can judge only a few requests, so one early error can fail `error-rate`; `--threshold-abort-delay` gives the run time to make more.

### `rps` counts from when checking starts

An `rps` rule counts the requests made since checking started, over the seconds since then, and its row reads as `213.5 from Maintain`. The count ends when checking pauses as the run stops its users, unless a later step checks again, so a ramp down does not pull the final value down either. The report's Aggregated requests per second can differ, as it covers the whole run: under `--test-plan`, the ramp up's lower rate would pull it down and fail an `rps>=` rule that the steady load meets. Before a whole second has passed there is no rate to judge. When checking never starts, under `--no-threshold-abort` or in a run that never enters Maintain, `rps` covers the whole run.

### `--threshold-abort-delay`

`--threshold-abort-delay` holds the first check back by a time after the run first enters Maintain, given as for `--run-time`: `30s`, `2m`, or a bare number of seconds. It never changes the judgement at the end of the run.

Use it when the first seconds of a run are slow for a reason you accept, such as a cold cache. With `--threshold-abort-delay 1m`, a warm up spike in the first minute after the run enters Maintain can not stop the run, but it still counts in the final percentiles, so a rule it breaks for the whole run still fails at the end:

```bash
cargo run --release -- -H http://local.dev/ -u 50 -r 50 -t 10m \
    --threshold "p95<500ms" --threshold-abort-delay 1m
```

It needs a `--threshold`, and is not allowed with `--no-threshold-abort`, which never stops the run.

## The end of the run

When the run ends, Goose judges every rule once more on the final metrics. Here a rule whose metric has no samples fails, observed as `no samples`: a rule for a request that never ran says nothing about that request, so it can not pass.

The rule that stopped the run always fails. Requests that finish after it stopped, and any the check had not counted yet, are counted afterwards, so by the end its value can be back inside the limit. Its row shows the value when it stopped the run and the value at the end, such as `100% at abort, 0.8% at end`, or one value when the two are the same.

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

A load test whose `main` returns `Result<(), GooseError>` and uses `?` on `execute()`, as every example does, exits with code 1 when a threshold fails, so a CI step fails with it:

```yaml
- name: Load test
  run: |
    cargo run --release --example simple -- \
      --host http://staging.example.com --users 20 --increase-rate 5 --run-time 5m \
      --threshold "p95<500ms" --threshold "error-rate<1%" \
      --report-file report.html
```

## Reports

The console's final tables, and the HTML, Markdown and JSON reports, have a Thresholds section with a row for each rule: the rule, what was observed, and the result. A failing row says `FAIL`, and `FAIL, stopped the run` for the rule that stopped it, so the result never rests on colour alone. The JSON report keeps the results, so a report rendered from it with `--render-report` shows the same section. Without thresholds there is no section.

```
 === THRESHOLDS ===
 ------------------------------------------------------------------------------
 Rule                                         |                   Observed | Result
 ------------------------------------------------------------------------------
 error-rate<1%                                | 100% at abort, 0.8% at end | FAIL, stopped the run
 p95<500ms                                    |                      312ms | PASS
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

Thresholds do no work per request, as Goose reads the metrics once a second: over 5 runs of 10 minutes per build, alternating builds, with 32 users sending requests back to back to nginx on the same 18 core MacBook Pro, the averages of the middle 3 runs by requests per second put throughput and p95 within run to run noise, and CPU time 1% to 2% higher with 3 thresholds than before thresholds.
