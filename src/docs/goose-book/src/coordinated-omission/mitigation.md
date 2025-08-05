# Mitigation Strategies

## Overview

Goose provides comprehensive protection against coordinated omission through its metrics collection architecture. By recording all request timings and maintaining detailed percentile distributions, Goose ensures that slow responses are properly represented in your load test results.

## Built-in Protection

### Complete Timing Capture

Goose's fundamental design prevents coordinated omission by:

1. **Recording Every Request**: All request start and end times are captured, regardless of duration
2. **No Sampling**: Unlike some tools, Goose doesn't sample metrics - every data point is recorded
3. **Async Architecture**: Non-blocking request handling ensures slow responses don't prevent new requests

```rust
// Example: How Goose captures all timings
async fn user_function(user: &mut GooseUser) -> TransactionResult {
    // Start time is automatically recorded
    let _goose = user.get("/slow-endpoint").await?;
    // End time is recorded regardless of response duration
    // Even if this takes 30 seconds, it's properly tracked
    Ok(())
}
```

### Accurate Percentile Calculation

Goose uses the [hdrhistogram](https://docs.rs/hdrhistogram/) crate to maintain high-resolution timing distributions:

- **Microsecond precision**: Timings are recorded with microsecond accuracy
- **Dynamic range**: Handles response times from microseconds to minutes
- **Memory efficient**: Compressed histogram format maintains accuracy without excessive memory use

## Configuration Options

### Request Timeout Settings

Configure appropriate timeouts to ensure all responses are captured:

```bash
# Set a 60-second request timeout (default is 60)
cargo run --release -- --request-timeout 60

# For extremely slow endpoints, increase further
cargo run --release -- --request-timeout 300
```

### Coordinated Omission Mitigation Mode

Enable explicit coordinated omission mitigation for traditional closed-loop testing:

```bash
# Enable CO mitigation mode
cargo run --release -- --co-mitigation enabled

# With custom parameters
cargo run --release -- --co-mitigation enabled \
    --co-mitigation-expected-interval 100 \
    --co-mitigation-accuracy 2
```

When enabled, this mode:
- Tracks expected vs actual request intervals
- Adjusts metrics to account for delayed requests
- Provides warnings when significant delays are detected

## Understanding Your Results

Goose provides two sets of metrics:
- **Raw Metrics**: Actual measurements from completed requests
- **CO-Adjusted Metrics**: Include synthetic data points for requests that should have been made

Significant differences between these metrics indicate CO events occurred during your test.

## Choosing Your Mitigation Strategy

Goose offers four CO mitigation modes via the `--co-mitigation` flag:

| Mode | Use Case | Behavior |
|------|----------|----------|
| `disabled` | Custom analysis, external CO handling | No adjustment, raw data only |
| `average` (default) | General performance testing | Uses average response time as baseline |
| `minimum` | Strict SLA compliance, microservices | Uses minimum response time as baseline |
| `maximum` | Conservative testing, worst-case analysis | Uses maximum response time as baseline |

### When to Use Each Mode

**Use `minimum` when:**
- Testing microservices with strict timing requirements
- Validating SLA compliance
- You need to detect ANY performance degradation
- Testing in controlled environments

**Use `average` when:**
- Simulating realistic user behavior
- Testing public-facing websites
- You want balanced synthetic data generation
- General performance regression testing

**Use `maximum` when:**
- Conservative performance testing
- Analyzing worst-case scenarios
- You want minimal synthetic data generation
- Testing systems with high variance

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
cargo run --release -- --users 1000 --hatch-rate 10

# Worse: Few users hammering the system
cargo run --release -- --users 10 --hatch-rate 10
```

### 2. Monitor Response Time Distributions

Always review the full distribution, not just averages:

```text
Response Time Percentiles:
50%: 45ms      # Median looks good
95%: 127ms     # 95th percentile reasonable
99%: 894ms     # 99th shows degradation
99.9%: 5,234ms # Long tail reveals issues
```

### 3. Set Appropriate Timeouts

Balance between capturing slow responses and test duration:

```rust
use goose::prelude::*;

// Configure per-request timeouts
let _goose = user.get("/endpoint")
    .set_timeout(Duration::from_secs(30))
    .await?;
```

### 4. Use Test Plans for Controlled Load

[Test plans](../getting-started/test-plan.md) help maintain consistent request rates:

```toml
[testplan]
# Gradual ramp-up prevents overwhelming the system
"0s" = "0"
"30s" = "100"
"1m" = "100"
"2m30s" = "200"
"5m" = "200"
"6m" = "0"
```

## How It Works

When using `average` mode (default when CO mitigation is enabled), Goose will trigger Coordinated Omission Mitigation if the time to loop through a [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) takes more than twice as long as the average time of all previous loops. In this case, on the next loop through the [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) when tracking the actual metrics for each subsequent request in all [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) it will also add in statistically generated "requests" with a [`response_time`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.response_time) starting at the unexpectedly long request time, then again with that [`response_time`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.response_time) minus the normal "cadence", continuing to generate a metric then subtract the normal "cadence" until arriving at the expected [`response_time`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.response_time). In this way, Goose is able to estimate the actual effect of a slowdown.

When Goose detects an abnormally slow request (one in which the individual request takes longer than the normal [`user_cadence`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.user_cadence)), it will generate an INFO level message (which will be visible on the command line (unless `--no-print-metrics` is enabled), and written to the log if started with the `-g` run time flag and `--goose-log` is configured).

## Verification Techniques

### 1. Compare with Expected Throughput

Calculate theoretical vs actual request rates:

```python
# Expected requests per second
expected_rps = users * (1000 / avg_think_time_ms)

# Compare with actual from Goose metrics
actual_rps = total_requests / test_duration_seconds

# Large discrepancies indicate CO issues
co_factor = expected_rps / actual_rps
```

### 2. Analyze Response Time Variance

High variance often indicates coordinated omission:

```bash
# Look for these warning signs in metrics:
# - Standard deviation > mean response time
# - 99th percentile > 10x median
# - Maximum response time orders of magnitude higher
```

### 3. Monitor Active Transaction Counts

Track concurrent in-flight requests:

```rust
// Use GooseMetrics to monitor active transactions
// Sustained high counts indicate queueing/delays
```

## Examples

An example of a request triggering Coordinate Omission mitigation:

```bash
13:10:30 [INFO] 11.401s into goose attack: "GET http://apache/node/1557" [200] took abnormally long (1814 ms), transaction name: "(Anon) node page"
13:10:30 [INFO] 11.450s into goose attack: "GET http://apache/node/5016" [200] took abnormally long (1769 ms), transaction name: "(Anon) node page"
```

If the `--request-log` is enabled, you can get more details, in this case by looking for elapsed times matching the above messages, specifically 1,814 and 1,769 respectively:

```json
{"coordinated_omission_elapsed":0,"elapsed":11401,"error":"","final_url":"http://apache/node/1557","method":"Get","name":"(Anon) node page","redirected":false,"response_time":1814,"status_code":200,"success":true,"update":false,"url":"http://apache/node/1557","user":2,"user_cadence":1727}
{"coordinated_omission_elapsed":0,"elapsed":11450,"error":"","final_url":"http://apache/node/5016","method":"Get","name":"(Anon) node page","redirected":false,"response_time":1769,"status_code":200,"success":true,"update":false,"url":"http://apache/node/5016","user":0,"user_cadence":1422}
```

In the requests file, you can see that two different user threads triggered Coordinated Omission Mitigation, specifically threads 2 and 0. Both [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) threads were loading the same [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) as due to transaction weighting this is the transaction loaded the most frequently. Both [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) threads loop through all [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) in a similar amount of time: thread 2 takes on average 1.727 seconds, thread 0 takes on average 1.422 seconds.

Also if the `--request-log` is enabled, requests back-filled by Coordinated Omission Mitigation show up in the generated log file, even though they were not actually sent to the server. Normal requests not generated by Coordinated Omission Mitigation have a [`coordinated_omission_elapsed`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html#structfield.coordinated_omission_elapsed) of 0.

## Advanced Techniques

### Custom Metrics Collection

Implement additional CO detection:

```rust
use goose::prelude::*;
use std::time::Instant;

async fn monitored_request(user: &mut GooseUser) -> TransactionResult {
    let intended_start = Instant::now();
    
    // Your actual request
    let result = user.get("/endpoint").await?;
    
    let actual_start = result.request.start_time;
    let schedule_delay = actual_start.duration_since(intended_start);
    
    // Log if request was significantly delayed
    if schedule_delay.as_millis() > 100 {
        user.log_debug(&format!(
            "Request delayed by {}ms", 
            schedule_delay.as_millis()
        ))?;
    }
    
    Ok(())
}
```

### Real-time Monitoring

Use Goose's controllers for live detection:

```bash
# Enable real-time metrics via WebSocket
cargo run --release -- --websocket-host 0.0.0.0 --websocket-port 5117

# Monitor for:
# - Sudden drops in request rate
# - Spikes in response times
# - Increasing queue depths
```

## Statistical Analysis Note

While Goose provides comprehensive data for analysis, determining statistical significance of performance changes requires additional tools and expertise. Goose produces the raw data you need, but interpretation remains your responsibility.

For detailed analysis, consider:
- Kolmogorov-Smirnov or Anderson-Darling tests for distribution comparison
- Note that CO-adjusted data is derived from raw data (not statistically independent)
- Export data via `--request-log` for external analysis

## Summary

Goose's architecture inherently protects against coordinated omission through:

1. **Comprehensive data collection** - Every request is tracked
2. **Accurate percentile calculations** - Full distributions preserved
3. **Flexible configuration** - Timeouts and modes for various scenarios
4. **Real-time visibility** - Monitor and detect issues during tests

By following these practices and utilizing Goose's built-in protections, you can ensure your load test results accurately reflect real-world system behavior under load.
