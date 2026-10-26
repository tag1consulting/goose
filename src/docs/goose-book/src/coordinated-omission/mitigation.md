# Mitigation Strategies

## Overview

Goose records the time of every request it makes. That alone does not prevent coordinated omission: each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) waits for a response before it sends its next request, so while the server stalls, users send fewer requests and the stall is under counted. Coordinated omission mitigation, which is off by default, backfills the requests each user would have sent at its usual cadence.

## What Goose Records

### Complete Timing Capture

1. **Recording Every Request**: Goose records the response time of every request it makes, however long it takes, up to the request timeout
2. **No Sampling**: every response time is counted; none is sampled away
3. **One Request at a Time per User**: a `GooseUser` waits for each response before it goes on, so a slow response delays that user's next requests; this is the source of coordinated omission that mitigation corrects for

```rust
// Example: Goose times this request however long it takes, up to --timeout.
async fn user_function(user: &mut GooseUser) -> TransactionResult {
    // Even if this takes 30 seconds, Goose records the 30 seconds,
    // and this user sends nothing else while it waits.
    let _goose = user.get("/slow-endpoint").await?;
    Ok(())
}
```

### Accurate Percentile Calculation

Goose keeps response times in its own buckets:

- **Microsecond precision**: times are taken from a `Duration` and kept to the microsecond
- **Bounded error**: each bucket is never more than 1% off any time it holds, so percentiles are never more than 1% off
- **Exact extremes**: the fastest and slowest times are kept exactly

## Configuration Options

### Request Timeout Settings

A request that times out fails, and its response time is about the timeout, so a timeout shorter than the server's slowest responses cuts them off. Set one long enough to capture the slow responses you want measured:

```bash
# A 60 second request timeout (the default is 60)
cargo run --release -- --timeout 60

# For extremely slow endpoints, increase further
cargo run --release -- --timeout 300
```

### Coordinated Omission Mitigation Mode

Mitigation is disabled by default. Enable it by naming the cadence to backfill at:

```bash
# Backfill at each user's average cadence
cargo run --release -- --co-mitigation average
```

`--co-mitigation` takes `average`, `maximum`, `minimum` or `disabled`, ignoring case. In code, set it with `GooseDefault::CoordinatedOmissionMitigation` and a `GooseCoordinatedOmissionMitigation` value. Any value but `disabled` is refused with `--no-metrics`. Mitigation has no other options.

When enabled, this mode:
- Tracks how long each user takes to run through all of its transactions
- Backfills synthetic response times for the requests a stalled user did not send
- Logs an INFO message for each request that took longer than its user's usual cadence

## Understanding Your Results

With mitigation enabled and once it has backfilled a request, Goose shows two sets of metrics:
- **Raw Metrics**: the response times Goose measured
- **Adjusted Metrics**: the measured times together with the synthetic ones

A large gap between these metrics means a stall held users back during your test. The [Metrics](metrics.md) page shows where each appears.

## Choosing Your Mitigation Strategy

Goose offers four values for the `--co-mitigation` flag:

| Mode | Use Case | Behavior |
|------|----------|----------|
| `disabled` (default) | Custom analysis, external CO handling | No adjustment, raw data only |
| `average` | General performance testing | Expected cadence is the user's average time to run through its transactions |
| `minimum` | Most sensitive | Expected cadence is the user's fastest time to run through its transactions |
| `maximum` | Least sensitive | Expected cadence is the user's slowest time to run through its transactions |

### When to Use Each Mode

**Use `minimum` when:**
- Testing services with strict timing requirements
- You want any slowdown against the fastest run through the transactions backfilled
- Testing in controlled environments with little variance

**Use `average` when:**
- Simulating realistic user behavior
- Testing public-facing websites
- You want balanced synthetic data generation
- General performance regression testing

**Use `maximum` when:**
- Response times vary widely and you only want the largest stalls backfilled

**Use `disabled` when:**
- Implementing custom CO mitigation
- Performing specialized statistical analysis
- You need only actual measurements
- Comparing with other tools' raw output

## Best Practices

### 1. Use Realistic User Counts

Avoid overwhelming your system with too few users:

```bash
# Better: More users with think time
cargo run --release -- --users 1000 --increase-rate 10

# Worse: Few users hammering the system
cargo run --release -- --users 10 --increase-rate 10
```

### 2. Monitor Response Time Distributions

Read the full distribution as well as the averages. The percentile tables of the console, HTML and Markdown reports show where the long tail starts: a median that looks good can sit beside a 99th percentile many times higher.

### 3. Set Appropriate Timeouts

Balance between capturing slow responses and test duration. `--timeout` sets the timeout for every request; to give one request its own, set it on the request builder:

```rust
use goose::prelude::*;
use std::time::Duration;

async fn slow_endpoint(user: &mut GooseUser) -> TransactionResult {
    // Give this one request 30 seconds, whatever --timeout says.
    let request_builder = user
        .get_request_builder(&GooseMethod::Get, "/endpoint")?
        .timeout(Duration::from_secs(30));
    // The path names the request in the metrics, as the RequestBuilder
    // already has its URL.
    let goose_request = GooseRequest::builder()
        .path("/endpoint")
        .set_request_builder(request_builder)
        .build();
    let _goose = user.request(goose_request).await?;

    Ok(())
}
```

### 4. Use Test Plans for Controlled Load

[Test plans](../getting-started/test-plan.md) help maintain consistent request rates:

```bash
# Gradual ramp up prevents overwhelming the system: 100 users over 30 seconds,
# hold for 30 seconds, 200 users over 90 seconds, hold for 150 seconds, then
# stop every user over a minute.
cargo run --release -- --test-plan "100,30s;100,30s;200,1m30s;200,2m30s;0,1m"
```

## How It Works

Each time a [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) starts another run through all the [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html)s of its [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html), Goose measures how long the previous run took, leaving out the time the user slept between transactions because of `set_wait_time`. It keeps each user's fastest, slowest and average run. From the user's fourth run on, the expected cadence is the average, slowest or fastest run, as `--co-mitigation` says; the average includes the run just measured. When the previous run took more than twice the expected cadence, Goose treats it as a stall.

Every request the user then makes during the run that follows backfills the stall. For each one, Goose records synthetic response times: the length of the stalled run, minus the cadence, minus the request's own response time, then that value minus the cadence again, and so on for as long as the value is greater than the request's own response time. These are the [`response_time`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.response_time)s the requests the user would have sent during the stall would have seen. These synthetic times estimate the effect of the slowdown.

Independently, from a user's fourth run on, Goose logs an INFO message for each request whose response time is longer than the user's expected cadence, its [`user_cadence`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.user_cadence). Such a request will likely trigger mitigation. INFO messages are shown on the console by default (`-q` hides them), and are written to the Goose log when it is configured with `--goose-log` and the log level is raised with `-g`.

## Verification Techniques

### 1. Compare with Expected Throughput

Calculate theoretical vs actual request rates:

```text
expected_rps = users * (1000 / average_ms_per_request_including_think_time)
actual_rps   = total_requests / test_duration_seconds

co_factor    = expected_rps / actual_rps   # well above 1 suggests users were held back
```

### 2. Analyze Response Time Variance

Look for these warning signs in the measured metrics:

- A 99th percentile more than ten times the median
- A maximum response time orders of magnitude higher than the median
- A large gap between the measured and adjusted tables when mitigation is enabled

## Examples

The following run is contrived. The [`drupal_memcache`](../example/drupal-memcache.md) example ran with five users for 20 seconds against a local test server that answers each request in 5 to 25 ms, and that held every request for 3 seconds, 11 seconds into the run:

```bash
cargo run --release --example drupal_memcache -- --host http://127.0.0.1:8004/ \
    -u5 -r5 -t20s --co-mitigation average --no-telnet --no-websocket \
    --request-log requests.json --request-format json \
    --report-file report.html --report-file report.json --report-file report.md
```

Requests that took longer than their user's cadence were logged:

```bash
19:44:45 [INFO] 11.004s into goose attack: "GET http://127.0.0.1:8004/misc/drupal.js" [200] took abnormally long (3026 ms), transaction name: "(Anon) front page"
19:44:45 [INFO] 10.993s into goose attack: "GET http://127.0.0.1:8004/themes/bartik/logo.png" [200] took abnormally long (3036 ms), transaction name: "(Anon) front page"
19:44:45 [INFO] 10.984s into goose attack: "GET http://127.0.0.1:8004/misc/drupal.js" [200] took abnormally long (3046 ms), transaction name: "(Auth) front page"
19:44:45 [INFO] 11.005s into goose attack: "GET http://127.0.0.1:8004/misc/drupal.js" [200] took abnormally long (3024 ms), transaction name: "(Anon) front page"
19:44:45 [INFO] 11.004s into goose attack: "GET http://127.0.0.1:8004/" [200] took abnormally long (3026 ms), transaction name: "(Anon) front page"
```

The time before each message is when the request started, in seconds since the load test started, and the number in parentheses is its response time. With `--request-log` enabled, you can find each of these requests in the request log by its `elapsed`, the same start time in milliseconds, and its `response_time`. The first three, which started 11,004, 10,993 and 10,984 ms in and took 3,026, 3,036 and 3,046 ms:

```json
{"elapsed":11004,"scenario_index":0,"scenario_name":"anonbrowsinguser","transaction_index":0,"transaction_name":{"TransactionOnly":"(Anon) front page"},"raw":{"method":"Get","url":"http://127.0.0.1:8004/misc/drupal.js","headers":[],"body":"","custom_method":""},"name":"static asset","final_url":"http://127.0.0.1:8004/misc/drupal.js","redirected":false,"response_time":3026,"status_code":200,"success":true,"update":false,"user":1,"error":"","coordinated_omission_elapsed":0,"user_cadence":930,"response_time_fraction_us":360}
{"elapsed":10993,"scenario_index":0,"scenario_name":"anonbrowsinguser","transaction_index":0,"transaction_name":{"TransactionOnly":"(Anon) front page"},"raw":{"method":"Get","url":"http://127.0.0.1:8004/themes/bartik/logo.png","headers":[],"body":"","custom_method":""},"name":"static asset","final_url":"http://127.0.0.1:8004/themes/bartik/logo.png","redirected":false,"response_time":3036,"status_code":200,"success":true,"update":false,"user":0,"error":"","coordinated_omission_elapsed":0,"user_cadence":896,"response_time_fraction_us":748}
{"elapsed":10984,"scenario_index":1,"scenario_name":"authbrowsinguser","transaction_index":1,"transaction_name":{"TransactionOnly":"(Auth) front page"},"raw":{"method":"Get","url":"http://127.0.0.1:8004/misc/drupal.js","headers":[],"body":"","custom_method":""},"name":"static asset","final_url":"http://127.0.0.1:8004/misc/drupal.js","redirected":false,"response_time":3046,"status_code":200,"success":true,"update":false,"user":3,"error":"","coordinated_omission_elapsed":0,"user_cadence":1046,"response_time_fraction_us":50}
```

All five user threads were held up by the same stall, each loading the front page or one of its static assets: users 0, 1, 2 and 4 the anonymous front page, and user 3 the authenticated one. The front page is the transaction each user runs most often, with a weight of 15 against the node page's 10 and the user page's 3, and each front page load also requests its static assets, so most requests caught by a stall belong to it. The `user_cadence` of each line is that user's expected cadence: user 1 took on average 930 ms to run through its transactions, user 0 896 ms and user 3 1,046 ms, so a 3 second stall is more than twice any of them.

The request log also holds the requests that carry a backfill. During the run after a stall, each request a user makes appears twice: once as measured, with a [`coordinated_omission_elapsed`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.coordinated_omission_elapsed) of 0, and once more with `coordinated_omission_elapsed` set to how long the stalled run took, in milliseconds. Here is the line with the backfill for the first request of user 1's run after the stall, a front page request that took 12 ms, after a run that took 3,946 ms against a cadence of 1,182 ms, the average now that it includes the stalled run:

```json
{"elapsed":14191,"scenario_index":0,"scenario_name":"anonbrowsinguser","transaction_index":0,"transaction_name":{"TransactionOnly":"(Anon) front page"},"raw":{"method":"Get","url":"http://127.0.0.1:8004/","headers":[],"body":"","custom_method":""},"name":"/","final_url":"http://127.0.0.1:8004/","redirected":false,"response_time":12,"status_code":200,"success":true,"update":false,"user":1,"error":"","coordinated_omission_elapsed":3946,"user_cadence":1182,"response_time_fraction_us":534}
```

From this line Goose recorded three synthetic times for `GET /`: 2,752 ms (3,946 less 1,182 less 12), then 1,570 and 388 ms, each 1,182 less than the one before, stopping before the next would be 12 or less. Each synthetic time also counts as a request in `# reqs`; the line carrying the backfill does not, as the same request is counted once as measured. The synthetic times themselves are not written to the request log; normal requests have a `coordinated_omission_elapsed` of 0.

## Advanced Techniques

### Custom Metrics Collection

Log slow responses for your own analysis:

```rust
use goose::prelude::*;

async fn monitored_request(user: &mut GooseUser) -> TransactionResult {
    let goose = user.get("/endpoint").await?;

    // Write slow responses to the debug log, if --debug-log is set.
    if goose.request.response_time > 100 {
        user.log_debug(
            &format!("slow response: {} ms", goose.request.response_time),
            Some(&goose.request),
            None,
            None,
        )?;
    }

    Ok(())
}
```

### Real-time Monitoring

Use Goose's controllers or the [live dashboard](../controller/dashboard.md) to watch a test while it runs. The Telnet and WebSocket controllers are on by default, and their `metrics` command returns the metrics so far:

```bash
# The WebSocket controller listens on 0.0.0.0:5117 by default; keep it on this machine
cargo run --release -- --websocket-host 127.0.0.1 --websocket-port 5117

# Monitor for:
# - Sudden drops in request rate
# - Spikes in response times
```

## Statistical Analysis Note

While Goose provides comprehensive data for analysis, determining statistical significance of performance changes requires additional tools and expertise. Goose produces the raw data you need, but interpretation remains your responsibility.

For detailed analysis, consider:
- Kolmogorov-Smirnov or Anderson-Darling tests for distribution comparison
- Note that CO-adjusted data is derived from raw data (not statistically independent)
- Export data via `--request-log` for external analysis

## Summary

Goose accounts for coordinated omission with:

1. **Every request timed**: Goose times every request it makes
2. **Percentiles within 1%**: no bucket is more than 1% off the times it holds
3. **Configuration**: a request timeout and three mitigation modes
4. **Live metrics**: the Controllers and the dashboard show the metrics while the test runs

Enable mitigation where stalls are likely, so the results show how the system behaves under load.
