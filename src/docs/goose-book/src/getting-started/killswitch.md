# Killswitch

Goose provides a killswitch mechanism to programmatically stop a load test when specific conditions are met. This is useful for protecting your systems and ensuring tests stop automatically when problems are detected.

To stop a run when a metric crosses a limit, such as a percentile response time or the error rate, use a [threshold](thresholds.md) instead of writing the check: a failing threshold fires this killswitch, and also makes the run fail with a nonzero exit code. A threshold failure is a killswitch stop.

When the killswitch is triggered, Goose stops the run as for Ctrl-C: users stop, `test_stop` runs, every report is written, and the run is recorded as canceled with the reason given. Only the first reason is kept until the next run starts.

A killswitch fired by your own code, as below, does not change what `execute()` returns: it returns `Ok` with the metrics of the run so far, so the process exits 0 unless your code fails it.

### Common Use Cases

* **Error Rate Threshold**: Stop when error rate exceeds acceptable limits (see [Thresholds](thresholds.md))
* **Response Time SLA Monitoring**: Halt testing when latency violates requirements (see [Thresholds](thresholds.md))
* **Health Check Integration**: Monitor system health endpoints and stop on failure
* **Resource Exhaustion Detection**: Stop when detecting connection pool or memory issues
* **Sitemap Traversal Completion**: Stop after fully crawling a site's pages
* **Data Set Processing**: Stop when all test data has been consumed
* **External Signal Integration**: Stop based on monitoring system alerts

### Example: Service Unavailable Detection

```rust
use goose::prelude::*;

async fn check_availability(user: &mut GooseUser) -> TransactionResult {
    let mut response = user.get("/api/endpoint").await?;
    
    // Stop the test if server returns 503 Service Unavailable
    if let Ok(response) = response.response {
        if response.status() == 503 {
            goose::trigger_killswitch("Server returned 503: Service Unavailable");
        }
    }
    
    Ok(())
}
```

You can also check if the killswitch has been triggered using `goose::is_killswitch_triggered()` to conditionally execute cleanup code or skip certain operations.
