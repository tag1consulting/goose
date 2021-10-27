# Running A Load Test

We will use Cargo to run our example load test application. It's best to get in the habit of setting the `--release` option whenever compiling or running load tests.

```bash
$ cargo run --release
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [optimized] target(s) in 3.56s
     Running `target/release/loadtest`
Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the GooseTaskSet.set_host() function (no host defined for LoadtestTasks)." }
```

The load test fails with an error as it hasn't been told the host you want to load test. So, let's try again, this time passing in the `--host` flag. After running for a few seconds, press `ctrl-c` to stop the load test:

```bash
$ cargo run -- --host http://local.dev/
    Finished dev [optimized] target(s) in 0.07s
     Running `target/release/loadtest --host 'http://local.dev/'`

=== PER TASK METRICS ===
------------------------------------------------------------------------------
 Name                    | # times run    | # fails        | task/s | fail/s
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 2,240          | 0 (0%)         | 280.0  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 15.54      | 6          | 136        | 14

=== PER REQUEST METRICS ===
------------------------------------------------------------------------------
 Name                    | # reqs         | # fails        | req/s  | fail/s
 -----------------------------------------------------------------------------
 GET /                   | 2,240          | 0 (0%)         | 280.0  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 GET /                   | 15.30      | 6          | 135        | 14

All 8 users hatched, resetting metrics (disable with --no-reset-metrics).

^C06:03:25 [ WARN] caught ctrl-c, stopping...

=== PER TASK METRICS ===
------------------------------------------------------------------------------
 Name                    | # times run    | # fails        | task/s | fail/s
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 2,054          | 0 (0%)         | 410.8  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 20.86      | 7          | 254        | 19

=== PER REQUEST METRICS ===
------------------------------------------------------------------------------
 Name                    | # reqs         | # fails        | req/s  | fail/s
 -----------------------------------------------------------------------------
 GET /                   | 2,054          | 0 (0%)         | 410.8  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 GET /                   | 20.68      | 7          | 254        | 19
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 -----------------------------------------------------------------------------
 GET /                   | 19     | 21     | 53     | 69     | 250    | 250
```

By default, Goose will hatch 1 `GooseUser` per second, up to the number of CPU cores available on the server used for load testing. In the above example, the server has 8 CPU cores, so it took 8 seconds to hatch all users. After all users are hatched, Goose flushes all metrics collected during the hatching process so all subsequent metrics are taken with all users running. Before flushing the metrics, they are displayed to the console so the data is not lost.

The same metrics are displayed per-task and per-request. In our simple example, our single task only makes one request, so in this case both metrics show the same results.

The per-task metrics are displayed first, starting with the name of our Task Set, `LoadtestTasks`. Individual tasks in the Task Set are then listed in the order they are defined in our load test. We did not name our task, so it simply shows up as "1: ". All defined tasks will be listed here, even if they did not run, so this can be useful to confirm everything in your load test is running as expected.

Next comes the per-request metrics. Our single task makes a `GET` request for the `/` path, so it shows up in the metrics as `GET /`. Comparing the per-task metrics collected for `1: ` to the per-request metrics collected for `GET /`, you can see that they are the same.

There are two common tables found in each type of metrics. The first shows the total number of requests made (2,054), how many of those failed (0), the average number of requests per second (410.8), and the average number of failed requests per second (0).

The second table shows the average time required to load a page (20.68 milliseconds), the minimum time to load a page (7 ms), the maximum time to load a page (254 ms) and the median time to load a page (19 ms).

The per-request metrics include a third table, showing the slowest page load time for a range of percentiles. In our example, in the 50% fastest page loads, the slowest page loaded in 19 ms. In the 75% fastest page loads, the slowest page loaded in 21 ms, etc.

In real load tests, you'll most likely have multiple task sets each with multiple tasks, and Goose will show you metrics for each along with an aggregate of them all together.

Refer to the [examples](../example/overview.html) included with Goose for more complicated and useful load test examples.
