# Simple Example

The `-h` flag will show all run-time configuration options available to Goose load tests. For example, you can pass the `-h` flag to the `simple` example as follows, `cargo run --example simple -- -h`:

```
Usage: target/debug/examples/simple [OPTIONS]

Options available when launching a Goose load test.


Optional arguments:
  -h, --help                 Displays this help
  -V, --version              Prints version information
  -l, --list                 Lists all tasks and exits

  -H, --host HOST            Defines host to load test (ie http://10.21.32.33)
  -u, --users USERS          Sets concurrent users (default: number of CPUs)
  -r, --hatch-rate RATE      Sets per-second user hatch rate (default: 1)
  -t, --run-time TIME        Stops after (30s, 20m, 3h, 1h30m, etc)
  -G, --goose-log NAME       Enables Goose log file and sets name
  -g, --log-level            Sets Goose log level (-g, -gg, etc)
  -v, --verbose              Sets Goose verbosity (-v, -vv, etc)

Metrics:
  --running-metrics TIME     How often to optionally print running metrics
  --no-reset-metrics         Doesn't reset metrics after all users have started
  --no-metrics               Doesn't track metrics
  --no-task-metrics          Doesn't track task metrics
  --no-error-summary         Doesn't display an error summary
  --report-file NAME         Create an html-formatted report
  -R, --request-log NAME     Sets request log file name
  --request-format FORMAT    Sets request log format (csv, json, raw)
  -T, --task-log NAME        Sets task log file name
  --task-format FORMAT       Sets task log format (csv, json, raw)
  -E, --error-log NAME       Sets error log file name
  --error-format FORMAT      Sets error log format (csv, json, raw)
  -D, --debug-log NAME       Sets debug log file name
  --debug-format FORMAT      Sets debug log format (csv, json, raw)
  --no-debug-body            Do not include the response body in the debug log
  --status-codes             Tracks additional status code metrics

Advanced:
  --no-telnet                Doesn't enable telnet Controller
  --telnet-host HOST         Sets telnet Controller host (default: 0.0.0.0)
  --telnet-port PORT         Sets telnet Controller TCP port (default: 5116)
  --no-websocket             Doesn't enable WebSocket Controller
  --websocket-host HOST      Sets WebSocket Controller host (default: 0.0.0.0)
  --websocket-port PORT      Sets WebSocket Controller TCP port (default: 5117)
  --no-autostart             Doesn't automatically start load test
  --co-mitigation STRATEGY   Sets coordinated omission mitigation strategy
  --throttle-requests VALUE  Sets maximum requests per second
  --sticky-follow            Follows base_url redirect with subsequent requests

Gaggle:
  --manager                  Enables distributed load test Manager mode
  --expect-workers VALUE     Sets number of Workers to expect
  --no-hash-check            Tells Manager to ignore load test checksum
  --manager-bind-host HOST   Sets host Manager listens on (default: 0.0.0.0)
  --manager-bind-port PORT   Sets port Manager listens on (default: 5115)
  --worker                   Enables distributed load test Worker mode
  --manager-host HOST        Sets host Worker connects to (default: 127.0.0.1)
  --manager-port PORT        Sets port Worker connects to (default: 5115)
```

The `examples/simple.rs` example copies the simple load test documented on the locust.io web page, rewritten in Rust for Goose. It uses minimal advanced functionality, but demonstrates how to GET and POST pages. It defines a single Task Set which has the user log in and then load a couple of pages.

Goose can make use of all available CPU cores. By default, it will launch 1 user per core, and it can be configured to launch many more. The following was configured instead to launch 1,024 users. Each user randomly pauses 5 to 15 seconds after each task is loaded, so it's possible to spin up a large number of users. Here is a snapshot of `top` when running this example on a 1-core VM with 10G of available RAM -- there were ample resources to launch considerably more "users", though `ulimit` had to be resized:

```
top - 06:56:06 up 15 days,  3:13,  2 users,  load average: 0.22, 0.10, 0.04
Tasks: 116 total,   3 running, 113 sleeping,   0 stopped,   0 zombie
%Cpu(s):  1.7 us,  0.7 sy,  0.0 ni, 96.7 id,  0.0 wa,  0.0 hi,  1.0 si,  0.0 st
MiB Mem :   9994.9 total,   7836.8 free,   1101.2 used,   1056.9 buff/cache
MiB Swap:  10237.0 total,  10237.0 free,      0.0 used.   8606.9 avail Mem

  PID USER      PR  NI    VIRT    RES    SHR S  %CPU  %MEM     TIME+ COMMAND
 1339 goose     20   0 1235480 758292   8984 R   3.0   7.4   0:06.56 simple
```

Here's the output of running the loadtest. The `-v` flag sends `INFO` and more critical messages to stdout (in addition to the log file). The `-u1024` tells Goose to spin up 1,024 users. The `-r32` option tells Goose to hatch 32 users per second. The `-t10m` option tells Goose to run the load test for 10 minutes, or 600 seconds. The `--status-codes` flag tells Goose to track metrics about HTTP status codes returned by the server, in addition to the default per-task and per-request metrics. The `--no-reset-metrics` flag tells Goose to start tracking the 10m run-time from when the first user starts, instead of the default which is to flush all metrics and start timing after all users have started. And finally, the `--only-summary` flag tells Goose to only display the final metrics after the load test finishes, otherwise it would display running metrics every 15 seconds for the duration of the test.

```
$ cargo run --release --example simple -- --host http://local.dev -v -u1024 -r32 -t10m --status-codes --no-reset-metrics --only-summary
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
```
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
```
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
```
