# Metrics

Here's the output of running the loadtest. The `-v` flag sends `INFO` and more critical messages to stdout (in addition to the log file, if enabled). The `-u1024` tells Goose to spin up 1,024 users. The `-r32` option tells Goose to hatch 32 users per second. The `-t10m` option tells Goose to run the load test for 10 minutes, or 600 seconds. The `--status-codes` flag tells Goose to track metrics about HTTP status codes returned by the server, in addition to the default per-task and per-request metrics. The `--no-reset-metrics` flag tells Goose to track all metrics, instead of the default which is to flush all metrics collected during start up. And finally, the `--only-summary` flag tells Goose to only display the final metrics after the load test finishes, otherwise it would display running metrics every 15 seconds for the duration of the test.

```bash
$ cargo run --release -- --host http://local.dev -v -u1024 -r32 -t10m --status-codes --no-reset-metrics --only-summary
    Finished release [optimized] target(s) in 0.09s
     Running `target/release/examples/simple --host 'http://local.dev' -v -u1024 -r32 -t10m --status-codes --no-reset-metrics --only-summary`
10:55:04 [ INFO] Output verbosity level: INFO
10:55:04 [ INFO] Logfile verbosity level: INFO
10:55:04 [ INFO] Writing to log file: goose.log
10:55:04 [ INFO] run_time = 600
10:55:04 [ INFO] global host configured: http://local.dev
10:55:04 [ INFO] initializing user states...
10:55:09 [ INFO] launching user 1 from WebsiteUser...
10:55:09 [ INFO] launching user 2 from WebsiteUser...
10:55:09 [ INFO] launching user 3 from WebsiteUser...
```
...
```bash
10:55:42 [ INFO] launching user 1022 from WebsiteUser...
10:55:42 [ INFO] launching user 1023 from WebsiteUser...
10:55:42 [ INFO] launching user 1024 from WebsiteUser...
10:55:42 [ INFO] launched 1024 users...
All 1024 users hatched.

11:05:09 [ INFO] stopping after 600 seconds...
11:05:09 [ INFO] waiting for users to exit
11:05:09 [ INFO] exiting user 879 from WebsiteUser...
11:05:09 [ INFO] exiting user 41 from WebsiteUser...
11:05:09 [ INFO] exiting user 438 from WebsiteUser...
```
...
```bash
11:05:10 [ INFO] exiting user 268 from WebsiteUser...
11:05:10 [ INFO] exiting user 864 from WebsiteUser...
11:05:10 [ INFO] exiting user 55 from WebsiteUser...
11:05:11 [ INFO] printing metrics after 601 seconds...

=== PER TASK METRICS ===
------------------------------------------------------------------------------
 Name                    | # times run    | # fails        | task/s | fail/s
 -----------------------------------------------------------------------------
 1: WebsiteUser          |
   1:                    | 1,024          | 0 (0%)         | 1.707  | 0.000
   2:                    | 28,746         | 0 (0%)         | 47.91  | 0.000
   3:                    | 28,748         | 0 (0%)         | 47.91  | 0.000
 ------------------------+----------------+----------------+--------+---------
 Aggregated              | 58,518         | 0 (0%)         | 97.53  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 1: WebsiteUser          |
   1:                    | 5.995      | 5          | 37         | 6
   2:                    | 0.428      | 0          | 17         | 0
   3:                    | 0.360      | 0          | 37         | 0
 ------------------------+------------+------------+------------+-------------
 Aggregated              | 0.492      | 5          | 37         | 5

=== PER REQUEST METRICS ===
------------------------------------------------------------------------------
 Name                    | # reqs         | # fails        | req/s  | fail/s
 -----------------------------------------------------------------------------
 GET /                   | 28,746         | 0 (0%)         | 47.91  | 0.000
 GET /about/             | 28,748         | 0 (0%)         | 47.91  | 0.000
 POST /login             | 1,024          | 0 (0%)         | 1.707  | 0.000
 ------------------------+----------------+----------------+--------+---------
 Aggregated              | 58,518         | 29,772 (50.9%) | 97.53  | 49.62
 -------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 GET /                   | 0.412      | 0          | 17         | 0
 GET /about/             | 0.348      | 0          | 37         | 0
 POST /login             | 5.979      | 5          | 37         | 6
 ------------------------+------------+------------+------------+-------------
 Aggregated              | 0.478      | 5          | 37         | 5
 -------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 -----------------------------------------------------------------------------
 GET /                   | 0      | 1      | 3      | 4      | 5      | 5
 GET /about/             | 0      | 0      | 3      | 3      | 5      | 5
 POST /login             | 6      | 6      | 7      | 7      | 28     | 28
 ------------------------+--------+--------+--------+--------+--------+-------
 Aggregated              | 5      | 5      | 5      | 6      | 7      | 17
 -------------------------------------------------------------------------------
 Name                    | Status codes
 -----------------------------------------------------------------------------
 GET /                   | 28,746 [200]
 GET /about/             | 28,748 [200]
 POST /login             | 1,024 [200]
 -------------------------------------------------------------------------------
 Aggregated              | 58,518 [200]
 ------------------------------------------------------------------------------
 Users: 1024
 Target host: http://local.dev/
 Starting: 2021-08-12 10:55:04 - 2021-08-12 10:55:42 (duration: 00:00:38)
 Running:  2021-08-12 10:55:42 - 2021-08-12 11:05:09 (duration: 00:10:00)
 Stopping: 2021-08-12 11:05:09 - 2021-08-12 11:05:11 (duration: 00:00:02)

 goose v0.14.0
 ------------------------------------------------------------------------------
```

Additional details about how metrics are collected, stored, and displayed can be found [in the developer documentation](https://docs.rs/goose/*/goose/metrics/index.html).
