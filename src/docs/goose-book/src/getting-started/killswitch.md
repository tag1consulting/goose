# Killswitch

Goose provides a killswitch mechanism to programmatically stop a load test when specific conditions are met. This is useful for protecting your systems and ensuring tests stop automatically when problems are detected.

To stop a run when a metric crosses a limit, such as a percentile response time or the error rate, use a [threshold](thresholds.md) instead of writing the check.

When the killswitch is triggered, Goose logs `Killswitch triggered:` with the reason and stops the run as for Ctrl-C: users stop, `test_stop` runs, every report is written, and the run is recorded as canceled with the reason given. Under `--no-autostart` too, a killswitch stop then shuts Goose down rather than returning it to Idle, though a connected dashboard can keep it up until Quit, as at the end of an autostarted run. Only the first reason is kept: later calls change nothing until Goose clears the killswitch as the next run starts.

A killswitch fired by your own code, as below, is not an error in itself: `execute()` returns `Ok` with the metrics of the run so far, unless a threshold fails when the run ends.

### Common Use Cases

* **Health Check Integration**: Monitor system health endpoints and stop on failure
* **Resource Exhaustion Detection**: Stop when detecting connection pool or memory issues
* **Sitemap Traversal Completion**: Stop after fully crawling a site's pages
* **Data Set Processing**: Stop when all test data has been consumed
* **External Signal Integration**: Stop based on monitoring system alerts

### Example: Service Unavailable Detection

```rust
use goose::prelude::*;

async fn check_availability(user: &mut GooseUser) -> TransactionResult {
    let goose = user.get("/api/endpoint").await?;

    // Stop the test if server returns 503 Service Unavailable
    if let Ok(response) = goose.response {
        if response.status() == 503 {
            goose::trigger_killswitch("Server returned 503: Service Unavailable");
        }
    }

    Ok(())
}
```

You can also check if the killswitch has been triggered using `goose::is_killswitch_triggered()` to conditionally execute cleanup code or skip certain operations.

`goose::reset_killswitch()` clears it. Goose calls it itself as each run starts. A test suite that runs several load tests in one process can call it after a test that triggered the killswitch, so the tests after it are not canceled.
