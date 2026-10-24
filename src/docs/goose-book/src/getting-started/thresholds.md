# Thresholds

A threshold is a pass/fail rule on a load test's metrics, such as "the 95th percentile response time stays under 500 ms" or "under 1% of requests fail". Give one with `--threshold`, as often as needed. A rule that fails stops the run by default, and always makes the run fail: `GooseAttack::execute()` returns an error, so a load test whose `main` returns `Result<(), GooseError>` exits with a nonzero code that CI can see. Every report says which rules passed and which failed.

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
| `pNN`, `pNN.N` | A response time percentile, such as `p50`, `p95` or `p99.9`, greater than 0 and less than 100 | A time: a number with `us`, `ms` or `s`; a bare number is milliseconds |
| `avg` | The average response time | A time |
| `max` | The slowest response time | A time |
| `error-rate` | Failed requests as a percent of all requests | A number with an optional `%`, at most 100 |
| `rps` | Requests per second over the run so far | A number |

The operator is `<`, `<=`, `>` or `>=`. A rule passes when what Goose observed compares so with the value: `p95<500ms` passes at 499 ms and fails at 500 ms, while `p95<=500ms` passes at 500 ms too.

Without ` for <request>`, a rule judges all requests together, as the `Aggregated` row of the report's tables does. With it, the rule judges only the request the requests table names so, method and name together, such as `GET /login`, or a request's custom name, as in `POST login`.

Some rules:

- `p95<500ms`: 95% of all responses take less than 500 ms.
- `p99.9<=2s`: 99.9% of all responses take at most 2 seconds.
- `avg<150`: the average response is under 150 ms.
- `error-rate<1%`: fewer than 1% of requests fail.
- `rps>=200`: the run makes at least 200 requests a second.
- `max<5s for GET /checkout`: no `GET /checkout` takes 5 seconds or more.

A rule that does not parse is refused before any user starts, with what is wrong with it:

```
threshold "p100<1s": the percentile must be greater than 0 and less than 100, such as p95 or p99.9.
```

Thresholds judge metrics, so `--threshold` is not allowed with `--no-metrics`.

### Precision

Percentiles come from the same histogram buckets as every report's percentiles: each time under 128 µs has a bucket of its own, and from there a bucket reports the time at its middle, at most 1/128 (0.78%) away from any time it holds. A percentile rule close to its limit can therefore pass or fail by that much. The average and the slowest time are exact.

## Stopping the run

By default Goose checks every rule once a second while the load test runs, and the first rule that fails stops the run through the [killswitch](killswitch.md), with a reason that names the rule and what was observed:

```
Killswitch triggered: threshold p95<500ms failed: observed 812ms
```

A threshold failure is a killswitch stop: users stop, `test_stop` runs, and every report is written as for any other killswitch. The run then fails, as below.

With `--no-threshold-abort`, Goose never stops the run for a threshold. It runs its full time, and judges every rule only at the end.

### When checking starts and stops

Checking starts once the metrics being collected are the ones the run keeps:

- A users ramp, the default, resets its metrics when every user has started and the run enters Maintain, unless `--no-reset-metrics` is set. Checking starts then, since judging metrics about to be thrown away could stop the run for nothing. A slow ramp up never fails a rule.
- `--test-plan`, `--iterations` and `--no-reset-metrics` never reset the metrics, so checking starts in the first second.

Checking stops once the run starts to stop its users, in Decrease or Shutdown. The judgement at the end still covers the whole run.

A rule whose metric has no samples yet, such as a rule for a request that has not run, is skipped while the run goes.

### `--threshold-abort-delay`

`--threshold-abort-delay` holds the first check back by a time after checking would start, given as for `--run-time`: `30s`, `2m`, or a bare number of seconds. It never changes the judgement at the end of the run.

Use it when the first seconds of a run are slow for a reason you accept, such as a cold cache. With `--threshold-abort-delay 1m`, a warm up spike in the first minute can not stop the run, but it still counts in the final percentiles, so a rule it breaks for the whole run still fails at the end:

```bash
cargo run --release -- -H http://local.dev/ -u 50 -r 50 -t 10m \
    --threshold "p95<500ms" --threshold-abort-delay 1m
```

It needs a `--threshold`, and is not allowed with `--no-threshold-abort`, which never stops the run.

## The end of the run

When the run ends, Goose judges every rule once more on the final metrics. Here a rule whose metric has no samples fails, observed as `no samples`: a rule for a request that never ran says nothing about that request, so it can not pass.

When every rule passes, `execute()` returns `Ok(GooseMetrics)` as usual. When any fails, it returns `GooseError::ThresholdsFailed` once every report is written. The error holds the rules that failed and the run's metrics:

```rust,ignore
match goose_attack.execute().await {
    Ok(metrics) => println!("all {} thresholds passed", metrics.thresholds.len()),
    Err(GooseError::ThresholdsFailed { failed, metrics }) => {
        for result in &failed {
            eprintln!("{} failed: observed {}", result.rule, result.observed);
        }
        eprintln!("the run made {} requests", metrics.requests.len());
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
 Rule                                         |       Observed | Result
 ------------------------------------------------------------------------------
 p95<500ms                                    |          812ms | FAIL, stopped the run
 error-rate<1%                                |             0% | PASS
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

BENCHMARK_PLACEHOLDER
