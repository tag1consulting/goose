# Goose

Have you ever been attacked by a goose?

[![crates.io](https://img.shields.io/crates/v/goose.svg)](https://crates.io/crates/goose)
[![Documentation](https://docs.rs/goose/badge.svg)](https://docs.rs/goose)
[![Apache-2.0 licensed](https://img.shields.io/crates/l/goose.svg)](./LICENSE)
[![CI](https://github.com/tag1consulting/goose/workflows/CI/badge.svg)](https://github.com/tag1consulting/goose/actions?query=workflow%3ACI)
[![Docker Repository on Quay](https://quay.io/repository/tag1consulting/goose/status "Docker Repository on Quay")](https://quay.io/repository/tag1consulting/goose)

## Overview

Goose is a Rust load testing tool inspired by [Locust](https://locust.io/). User behavior is defined with standard Rust code. Load tests are applications that have a dependency on the Goose library. Web requests are made with the [Reqwest](https://docs.rs/reqwest) HTTP Client.

### Documentation

- [README](https://github.com/tag1consulting/goose/blob/main/README.md)
- [Developer documentation](https://docs.rs/goose/)
- [Blogs and more](https://tag1.com/goose/)
  - [Goose vs Locust and jMeter](https://www.tag1consulting.com/blog/jmeter-vs-locust-vs-goose)
  - [Real-life load testing with Goose](https://www.tag1consulting.com/blog/real-life-goose-load-testing)
  - [Gaggle: a distributed load test](https://www.tag1consulting.com/blog/show-me-how-flock-flies-working-gaggle-goose)
  - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)

### Requirements

- Minimum required `rustc` version is `1.49.0`: `goose` depends on [`flume`](https://docs.rs/flume) for communication between threads, which in turn depends on [`spinning_top`](https://docs.rs/spinning_top) which uses `hint::spin_loop` which stabilized in `rustc` version `1.49.0`. More detail in https://github.com/rust-lang/rust/issues/55002.

## Getting Started

The [in-line documentation](https://docs.rs/goose/*/goose/#creating-a-simple-goose-load-test) offers much more detail about Goose specifics. For a general background to help you get started with Rust and Goose, read on.

[Cargo](https://doc.rust-lang.org/cargo/) is the Rust package manager. To create a new load test, use Cargo to create a new application (you can name your application anything, we've generically selected `loadtest`):

```bash
$ cargo new loadtest
     Created binary (application) `loadtest` package
$ cd loadtest/
```

This creates a new directory named `loadtest/` containing `loadtest/Cargo.toml` and `loadtest/src/main.rs`. Start by editing `Cargo.toml` adding Goose under the dependencies heading:

```toml
[dependencies]
goose = "^0.11"
```

At this point it's possible to compile all dependencies, though the resulting binary only displays "Hello, world!":

```
$ cargo run
    Updating crates.io index
  Downloaded goose v0.11.2
      ...
   Compiling goose v0.11.2
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [unoptimized + debuginfo] target(s) in 52.97s
     Running `target/debug/loadtest`
Hello, world!
```

To create an actual load test, you first have to add the following boilerplate to the top of `src/main.rs` to make Goose's functionality available to your code:

```rust
use goose::prelude::*;
```

Then create a new load testing function. For our example we're simply going to load the front page of the website we're load-testing. Goose passes all load testing functions a pointer to a GooseUser object, which is used to track metrics and make web requests. Thanks to the Reqwest library, the Goose client manages things like cookies, headers, and sessions for you. Load testing functions must be declared async, which helps ensure that your simulated users don't become CPU-locked.

In load test functions you typically do not set the host, and instead configure the host at run time, so you can easily run your load test against different environments without recompiling. The following `loadtest_index` function simply loads the front page of our web page:

```rust
async fn loadtest_index(user: &GooseUser) -> GooseTaskResult {
    let _goose_metrics = user.get("/").await?;

    Ok(())
}
```

The function is declared `async` so that we don't block a CPU-core while loading web pages. All Goose load test functions are passed in a reference to a `GooseUser` object, and return a `GooseTaskResult` which is either an empty `Ok(())` on success, or a `GooseTaskError` on failure. We use the `GooseUser` object to make requests, in this case we make a `GET` request for the front page, `/`. The `.await` frees up the CPU-core while we wait for the web page to respond, and the tailing `?` passes up any unexpected errors that may be returned from this request. When the request completes, Goose returns metrics which we store in the  `_goose_metrics` variable. The variable is prefixed with an underscore (`_`) to tell the compiler we are intentionally not using the results. Finally, after making a single successful request, we return `Ok(())` to let Goose know this task function completed successfully.

We have to tell Goose about our new task function. Edit the `main()` function, setting a return type and replacing the hello world text as follows:

```rust
fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .execute()?
        .print();

    Ok(())
}
```

If you're new to Rust, `main()`'s return type of `Result<(), GooseError>` may look strange. It essentially says that `main` will return nothing (`()`) on success, and will return a `GooseError` on failure. This is helpful as several of `GooseAttack`'s methods can fail, returning an error. In our example, `initialize()` and `execute()` each may fail. The `?` that follows the method's name tells our program to exit and return an error on failure, otherwise continue on. The `print()` method consumes the `GooseMetrics` object returned by `GooseAttack.execute()` and prints a summary if metrics are enabled. The final line, `Ok(())` returns the empty result expected on success.

And that's it, you've created your first load test! Let's run it and see what happens.

```bash
$ cargo run
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [unoptimized + debuginfo] target(s) in 3.56s
     Running `target/debug/loadtest`
Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the GooseTaskSet.set_host() function (no host defined for LoadtestTasks)." }
```

Goose is unable to run, as it hasn't been told the host you want to load test. So, let's try again, this time passing in the `--host` flag. After running for a few seconds, we then press `ctrl-c` to stop the load test:

```bash
$ cargo run -- --host http://local.dev/
    Finished dev [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/loadtest --host 'http://local.dev/'`

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

By default, Goose will hatch 1 GooseUser per second, up to the number of CPU cores available on the server used for load testing. In the above example, the server has 8 CPU cores, so it took 8 seconds to hatch all users. After all users are hatched, Goose flushes all metrics collected during the hatching process so all subsequent metrics are taken with all users running. Before flushing the metrics, they are displayed to the console so the data is not lost.

The same metrics are displayed per-task and per-request. In our simple example, our single task only makes one request, so in this case both metrics show the same results.

The per-task metrics are displayed first, starting with the name of our Task Set, `LoadtestTasks`. Individual tasks in the Task Set are then listed in the order they are defined in our load test. We did not name our task, so it simply shows up as `1: `. All defined tasks will be listed here, even if they did not run, so this can be useful to confirm everything in your load test is running as expected.

Next comes the per-request metrics. Our single task makes a `GET` request for the `/` path, so it shows up in the metrics as `GET /`. Comparing the per-task metrics collected for `1: ` to the per-request metrics collected for `GET /`, you can see that they are the same.

There are two common tables found in each type of metrics. The first shows the total number of requests made (2,054), how many of those failed (0), the average number of requests per second (410.8), and the average number of failed requests per second (0).

The second table shows the average time required to load a page (20.68 milliseconds), the minimum time to load a page (7 ms), the maximum time to load a page (254 ms) and the median time to load a page (19 ms).

The per-request metrics include a third table, showing the slowest page load time for a range of percentiles. In our example, in the 50% fastest page loads, the slowest page loaded in 19 ms. In the 75% fastest page loads, the slowest page loaded in 21 ms, etc.

In real load tests, you'll most likely have multiple task sets each with multiple tasks, and Goose will show you metrics for each along with an aggregate of them all together.

Refer to the [examples directory](https://github.com/tag1consulting/goose/tree/master/examples) for more complicated and useful load test examples.

## Tips

* Avoid `unwrap()` in your task functions -- Goose generates a lot of load, and this tends
to trigger errors. Embrace Rust's warnings and properly handle all possible errors, this
will save you time debugging later.
* When running your load test for real, use the cargo `--release` flag to generate
optimized code. This can generate considerably more load test traffic.

## Simple Example

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

## Scheduling GooseTaskSets

When starting a load test, Goose assigns one `GooseTaskSet` to each `GooseUser` thread. By default, it assigns `GooseTaskSet`s (and then `GooseTask`s within the task set) in a round robin order. As new `GooseUser` threads are launched, the first will be assigned the first defined `GooseTaskSet`, the next will be assigned the next defined `GooseTaskSet`, and so on, looping through all available `GooseTaskSet`s. Weighting is respected during this process, so if one `GooseTaskSet` is weighted heavier than others, that `GooseTaskSet` will get assigned to `GooseUser`s more at the end of the launching process.

The `GooseScheduler` can be configured to instead launch `GooseTaskSet`s and `GooseTask`s in a `Serial` or a `Random order`. When configured to allocate in a `Serial` order, `GooseTaskSet`s and `GooseTask`s are launched in the extact order they are defined in the load test (see below for more detail on how this works). When configured to allocate in a `Random` order, running the same load test multiple times can lead to different amounts of load being generated.

Prior to Goose `0.10.6` `GooseTaskSet`s were allocated in a serial order. Prior to Goose `0.11.1` `GooseTask`s were allocated in a serial order. To restore the old behavior, you can use the `GooseAttack::set_scheduler()` method as follows:

```rust
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::Serial)
```

To instead randomize the order that `GooseTaskSet`s and `GooseTask`s are allocated, you can instead configure as follows:

```rust
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::Random)
```

The following configuration is possible but superfluous because it is the scheduling default, and is therefor how Goose behaves even if the `.set_scheduler()` method is not called at all:

```rust
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::RoundRobin)
```

### Scheduling Example

The following simple example helps illustrate how the different schedulers work.

```rust
    GooseAttack::initialize()?
        .register_taskset(taskset!("TaskSet1")
            .register_task(task!(task1).set_weight(2)?)
            .register_task(task!(task2))
            .set_weight(2)?
        )
        .register_taskset(taskset!("TaskSet2")
            .register_task(task!(task1))
            .register_task(task!(task2).set_weight(2)?)
        )
        .execute()?
        .print();

    Ok(())
```

### Round Robin

This first example assumes the default of `.set_scheduler(GooseScheduler::RoundRobin)`.

If Goose is told to launch only two users, the first GooseUser will run `TaskSet1` and the second user will run `TaskSet2`. Even though `TaskSet1` has a weight of 2 `GooseUser`s are allocated round-robin so with only two users the second instance of `TaskSet1` is never launched.

The `GooseUser` running `TaskSet1` will then launch tasks repeatedly in the following order: `task1`, `task2`, `task1`. If it runs through twice, then it runs all of the following tasks in the following order: `task1`, `task2`, `task1`, `task1`, `task2`, `task1`.

### Serial

This second example assumes the manual configuration of `.set_scheduler(GooseScheduler::Serial)`.

If Goose is told to launch only two users, then both `GooseUser`s will launch `TaskSet1` as it has a weight of 2. `TaskSet2` will not get assigned to either of the users.

Both `GooseUser`s running `TaskSet1` will then launch tasks repeatedly in the following order: `task1`, `task1`, `task2`. If it runs through twice, then it runs all of the following tasks in the following order: `task1`, `task1`, `task2`, `task1`, `task1`, `task2`.

### Random

This third example assumes the manual configuration of `.set_scheduler(GooseScheduler::Random)`.

If Goose is told to launch only two users, the first will be randomly assigned either `TaskSet1` or `TaskSet2`. Regardless of which is assigned to the first user, the second will again be randomly assigned either `TaskSet1` or `TaskSet2`. If the load test is stopped and run again, there users are randomly re-assigned, there is no consistency between load test runs.

Each `GooseUser` will run tasks in a random order. The random order will be determined at start time and then will run repeatedly in this random order as long as the user runs.

## Defaults

All run-time options can be configured with custom defaults. For example, you may want to default to the the host name of your local development environment, only requiring that `--host` be set when running against a production environment. Assuming your local development environment is at "http://local.dev/" you can do this as follows:

```rust
    GooseAttack::initialize()?
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "http://local.dev/")?
        .execute()?
        .print();

    Ok(())
```

The following defaults can be configured with a `&str`:
 - host: `GooseDefault::Host`
 - log file name: `GooseDefault::LogFile`
 - html-formatted report file name: `GooseDefault::ReportFile`
 - requests log file name: `GooseDefault::RequestsFile`
 - requests log file format: `GooseDefault::RequestsFormat`
 - debug log file name: `GooseDefault::DebugFile`
 - debug log file format: `GooseDefault::DebugFormat`
 - host to bind telnet Controller to: `GooseDefault::TelnetHost`
 - host to bind WebSocket Controller to: `GooseDefault::WebSocketHost`
 - host to bind Manager to: `GooseDefault::ManagerBindHost`
 - host for Worker to connect to: `GooseDefault::ManagerHost`

The following defaults can be configured with a `usize` integer:
 - total users to start: `GooseDefault::Users`
 - users to start per second: `GooseDefault::HatchRate`
 - how often to print running metrics: `GooseDefault::RunningMetrics`
 - number of seconds for test to run: `GooseDefault::RunTime`
 - log level: `GooseDefault::LogLevel`
 - verbosity: `GooseDefault::Verbose`
 - maximum requests per second: `GooseDefault::ThrottleRequests`
 - number of Workers to expect: `GooseDefault::ExpectWorkers`
 - port to bind telnet Controller to: `GooseDefault::TelnetPort`
 - port to bind WebSocket Controller to: `GooseDefault::WebSocketPort`
 - port to bind Manager to: `GooseDefault::ManagerBindPort`
 - port for Worker to connect to: `GooseDefault::ManagerPort`

The following defaults can be configured with a `bool`:
 - do not reset metrics after all users start: `GooseDefault::NoResetMetrics`
 - do not track metrics: `GooseDefault::NoMetrics`
 - do not track task metrics: `GooseDefault::NoTaskMetrics`
 - do not start telnet Controller thread: `GooseDefault::NoTelnet`
 - do not start WebSocket Controller thread: `GooseDefault::NoWebSocket`
 - do not autostart load test, wait instead for a Controller to start: `GooseDefault::NoAutoStart`
 - track status codes: `GooseDefault::StatusCodes`
 - follow redirect of base_url: `GooseDefault::StickyFollow`
 - enable Manager mode: `GooseDefault::Manager`
 - ignore load test checksum: `GooseDefault::NoHashCheck`
 - enable Worker mode: `GooseDefault::Worker`

The following defaults can be configured with a `GooseCoordinatedOmissionMitigation`:
 - default Coordinated Omission Mitigation strategy: `GooseDefault::CoordinatedOmissionMitigation`

For example, without any run-time options the following load test would automatically run against `local.dev`, logging metrics to `goose-metrics.log` and debug to `goose-debug.log`. It will automatically launch 20 users in 4 seconds, and run the load test for 15 minutes. Metrics will be displayed every minute during the test and will include additional status code metrics. The order the defaults are set is not important.

```rust
    GooseAttack::initialize()?
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .set_default(GooseDefault::Host, "local.dev")?
        .set_default(GooseDefault::RequestsFile, "goose-requests.log")?
        .set_default(GooseDefault::DebugFile, "goose-debug.log")?
        .set_default(GooseDefault::Users, 20)?
        .set_default(GooseDefault::HatchRate, 4)?
        .set_default(GooseDefault::RunTime, 900)?
        .set_default(GooseDefault::RunningMetrics, 60)?
        .set_default(GooseDefault::StatusCodes, true)?
        .execute()?
        .print();

    Ok(())
```

## Coordinated Omission Mitigation

By default, Goose attempts to mitigate the loss of metrics data (Coordinated Omission) caused by an abnormally lengthy response to a request.

### Definition

To understand Coordinated Omission and how Goose attempts to mitigate it, it's necessary to understand how Goose is scheduling requests. Goose launches one thread per `GooseUser`. Each `GooseUser` is assigned a single `GooseTaskSet`. Each of these `GooseUser` threads then loop repeatedly through all of the `GooseTasks` defined in the assigned `GooseTaskSet`, each of which can involve any number of individual requests. However, at any given time, each `GooseUser` is only making a single request and then asynchronously waiting for the response.

If something causes the response to a request to take abnormally long, raw Goose metrics only see this slowdown as affecting a specific request (or one request per `GooseUser`). The slowdown can be caused by a variety of issues, such as a resource bottleneck (on the Goose client or the web server), garbage collection, a cache stampede, or even a network issue. A real user loading the same web page would see a much larger effect, as all requests to the affected server would stall. Even static assets such as images and scripts hosted on a reliable and fast CDN can be affected, as the web browser won't know to load them until it first loads the HTML from the affected web server. Because Goose is only making one request at a time per `GooseUser`, it may only see one or very few slow requests and then all other requests resume at normal speed. This results in a bias in the metrics to "ignore" or "hide" the true effect of a slowdown, commonly referred to as Coordinated Omission.

### Mitigation

Goose attempts to mitigate Coordinated Omission by back-filling the metrics with the statistically expected requests. To do this, it tracks the normal "cadence" of each `GooseUser`, timing how long it takes to loop through all `GooseTasks` in the assigned `GooseTaskSet`. By default, Goose will trigger Coordinated Omission Mitigation if the time to loop through a `GooseTaskSet` takes more than twice as long as the average time of all previous loops. In this case, on the next loop through the `GooseTaskSet` when tracking the actual metrics for each subsequent request in all `GooseTasks` it will also add in statistically generated "requests" with a `response_time` starting at the unexpectedly long request time, then again with that `response_time` minus the normal "cadence", continuing to generate a metric then subtract the normal "cadence" until arriving at the expected `response_time`. In this way, Goose is able to estimate the actual effect of a slowdown.

When Coordinated Omission Mitigation detects an abnormally slow request, Goose will generate an INFO level message (which will be visible if Goose was started with the `-v` run time flag, or written to the log if started with the `-g` run time flag and `--goose-log` is configured). For example:

```
10:10:02 [INFO] coordinated omission alert 6.957s into goose attack: "GET http://apache/node/8848" [200] took abnormally long (2932 ms), task name: "(Anon) node page"
10:10:02 [INFO] coordinated omission alert 7.019s into goose attack: "GET http://apache/node/1960" [200] took abnormally long (2873 ms), task name: "(Anon) node page"
10:10:02 [INFO] coordinated omission alert 7.314s into goose attack: "GET http://apache/node/1297" [200] took abnormally long (2578 ms), task name: "(Anon) node page"
```

If the `--request-log` is enabled, you can get more details, in this case by looking for elapsed times matching the above messages, specifically 6957, 7019, and 7314 respectively:

```
{"coordinated_omission_cadence":1651,"coordinated_omission_elapsed":0,"elapsed":6957,"error":"","final_url":"http://apache/node/8848","method":"Get","name":"(Anon) node page","redirected":false,"response_time":2932,"status_code":200,"success":true,"update":false,"url":"http://apache/node/8848","user":2}
{"coordinated_omission_cadence":1439,"coordinated_omission_elapsed":0,"elapsed":7019,"error":"","final_url":"http://apache/node/1960","method":"Get","name":"(Anon) node page","redirected":false,"response_time":2873,"status_code":200,"success":true,"update":false,"url":"http://apache/node/1960","user":0}
{"coordinated_omission_cadence":1812,"coordinated_omission_elapsed":0,"elapsed":7314,"error":"","final_url":"http://apache/node/1297","method":"Get","name":"(Anon) node page","redirected":false,"response_time":2578,"status_code":200,"success":true,"update":false,"url":"http://apache/node/1297","user":3}
```

In the requests file, you can see that three different user threads triggered Coordinated Omission Mitigation, specifically threads 2, 0, and 3. All `GooseUser` threads were loading the same `GooseTask` as due to task weighting this is the task loaded the most frequently. Each `GooseUser` thread loops through all `GooseTasks` in a similar amount of time: thread 2 takes on average 1.651 seconds, thread 0 takes on average 1.439 seconds, and thread 3 takes on average 1.812 seconds.

Also if the `--request-log` is enabled, requests back-filled by Coordinated Omission Mitigation show up in the generated log file, even though they were not actually sent to the server. In the following example, Coordinated Omission Mitigation was triggered when the server took 11,965 milliseconds to loop through all requests, instead of the average cadence of 3,162 milliseconds. This causes it to backfill a block of requests that statistically should have happened, with a `response_time` decreasing by the expected request cadence.

```json
{"coordinated_omission_cadence":3161,"coordinated_omission_elapsed":11965,"elapsed":185835,"error":"","final_url":"http://example.com/misc/jquery.js?v=1.4.4","method":"Get","name":"static asset","redirected":false,"response_time":11965,"status_code":200,"success":true,"update":false,"url":"http://example.com/misc/jquery.js?v=1.4.4","user":2}
{"coordinated_omission_cadence":3161,"coordinated_omission_elapsed":11965,"elapsed":185835,"error":"","final_url":"http://example.com/misc/jquery.js?v=1.4.4","method":"Get","name":"static asset","redirected":false,"response_time":8804,"status_code":200,"success":true,"update":false,"url":"http://example.com/misc/jquery.js?v=1.4.4","user":2}
{"coordinated_omission_cadence":3161,"coordinated_omission_elapsed":11965,"elapsed":185835,"error":"","final_url":"http://example.com/misc/jquery.js?v=1.4.4","method":"Get","name":"static asset","redirected":false,"response_time":5643,"status_code":200,"success":true,"update":false,"url":"http://example.com/misc/jquery.js?v=1.4.4","user":2}
{"coordinated_omission_cadence":3161,"coordinated_omission_elapsed":11965,"elapsed":185835,"error":"","final_url":"http://example.com/misc/jquery.js?v=1.4.4","method":"Get","name":"static asset","redirected":false,"response_time":2482,"status_code":200,"success":true,"update":false,"url":"http://example.com/misc/jquery.js?v=1.4.4","user":2}
```

Normal requests not generated by Coordinated Omission Mitigation have a `coordinated_omission_elapsed` of 0.

Coordinated Omission Mitigation can be disabled by setting `--co-mitigation disabled` when starting Goose. By default it uses the average cadence when backfilling, but it can also be configured to use the `minimum` or `maximum` cadence to allow for different server configuration and testing plans operating on different assumptions.

### Metrics

When Coordinated Omission Mitigation kicks in, Goose tracks both the "raw" metrics and the "adjusted" metrics. It shows both together when displaying metrics, first the "raw" (actually seen) metrics, followed by the "adjusted" metrics. As the minimum response time is never changed by Coordinated Omission Mitigation, this column is replacd with the "standard deviation" between the average "raw" response time, and the average "adjusted" response time.

The following example was "contrived". The `drupal_loadtest` example was run for 15 seconds, and after 10 seconds the upstream Apache server was manually "paused" for 3 seconds, forcing some abnormally slow queries. (More specifically, the apache web server was started by running `. /etc/apache2/envvars && /usr/sbin/apache2 -DFOREGROUND`, it was "paused" by pressing `ctrl-z`, and it was resumed three seconds later by typing `fg`.) In the "PER REQUEST METRICS" Goose shows first the "raw" metrics", followed by the "adjusted" metrics:

```
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 GET (Anon) front page    |       11.73 |          3 |          81 |         12
 GET (Anon) node page     |       81.76 |          5 |       3,390 |         37
 GET (Anon) user page     |       27.53 |         16 |          94 |         26
 GET (Auth) comment form  |       35.27 |         24 |          50 |         35
 GET (Auth) front page    |       30.68 |         20 |         111 |         26
 GET (Auth) node page     |       97.79 |         23 |       3,326 |         35
 GET (Auth) user page     |       25.20 |         21 |          30 |         25
 GET static asset         |        9.27 |          2 |          98 |          6
 POST (Auth) comment form |       52.47 |         43 |          59 |         52
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |       17.04 |          2 |       3,390 |          8
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |    Std Dev |         Max |     Median
 ------------------------------------------------------------------------------
 GET (Anon) front page    |      419.82 |     288.56 |       3,153 |         14
 GET (Anon) node page     |      464.72 |     270.80 |       3,390 |         40
 GET (Anon) user page     |      420.48 |     277.86 |       3,133 |         27
 GET (Auth) comment form  |      503.38 |     331.01 |       2,951 |         37
 GET (Auth) front page    |      489.99 |     324.78 |       2,960 |         33
 GET (Auth) node page     |      530.29 |     305.82 |       3,326 |         37
 GET (Auth) user page     |      500.67 |     336.21 |       2,959 |         27
 GET static asset         |      427.70 |     295.87 |       3,154 |          9
 POST (Auth) comment form |      512.14 |     325.04 |       2,932 |         55
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |      432.98 |     294.11 |       3,390 |         14
 ```

From these two tables, it is clear that there was a statistically significant event affecting the load testing metrics. In particular, note that the standard deviation between the "raw" average and the "adjusted" average is considerably larger than the "raw" average, calling into questing whether or not your load test was "valid". (The answer to that question depends very much on your specific goals and load test.)

Goose also shows multiple percentile graphs, again showing first the "raw" metrics followed by the "adjusted" metrics. The "raw" graph would suggest that less than 1% of the requests for the `GET (Anon) node page` were slow, and less than 0.1% of the requests for the `GET (Auth) node page` were slow. However, through Coordinated Omission Mitigation we can see that statistically this would have actually affected all requests, and for authenticated users the impact is visible on >25% of the requests.

```
 ------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 GET (Anon) front page    |     12 |     15 |     25 |     27 |     81 |     81
 GET (Anon) node page     |     37 |     43 |     60 |  3,000 |  3,000 |  3,000
 GET (Anon) user page     |     26 |     28 |     34 |     93 |     94 |     94
 GET (Auth) comment form  |     35 |     37 |     50 |     50 |     50 |     50
 GET (Auth) front page    |     26 |     34 |     45 |     88 |    110 |    110
 GET (Auth) node page     |     35 |     38 |     58 |     58 |  3,000 |  3,000
 GET (Auth) user page     |     25 |     27 |     30 |     30 |     30 |     30
 GET static asset         |      6 |     14 |     21 |     22 |     81 |     98
 POST (Auth) comment form |     52 |     55 |     59 |     59 |     59 |     59
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |      8 |     16 |     47 |     53 |  3,000 |  3,000
 ------------------------------------------------------------------------------
 Adjusted for Coordinated Omission:
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 GET (Anon) front page    |     14 |     21 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Anon) node page     |     40 |     55 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Anon) user page     |     27 |     32 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Auth) comment form  |     37 |    400 |  2,951 |  2,951 |  2,951 |  2,951
 GET (Auth) front page    |     33 |    410 |  2,960 |  2,960 |  2,960 |  2,960
 GET (Auth) node page     |     37 |    410 |  3,000 |  3,000 |  3,000 |  3,000
 GET (Auth) user page     |     27 |    420 |  2,959 |  2,959 |  2,959 |  2,959
 GET static asset         |      9 |     20 |  3,000 |  3,000 |  3,000 |  3,000
 POST (Auth) comment form |     55 |    390 |  2,932 |  2,932 |  2,932 |  2,932
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |     14 |     42 |  3,000 |  3,000 |  3,000 |  3,000
 ```

 The Coordinated Omission metrics will also show up in the HTML report generated when Goose is started with the `--report-file` run-time option. If Coordinated Omission mitigation kicked in, the HTML report will include both the "raw" metrics and the "adjusted" metrics.

## Controlling Running Goose Load Test

By default, Goose will launch a telnet Controller thread that listens on `0.0.0.0:5116`, and a WebSocket Controller thread that listens on `0.0.0.0:5117`. The running Goose load test can be controlled through these Controllers. Goose can optionally be started with the `--no-autostart` run time option to prevent the load test from automatically starting, requiring instead that it be started with a Controller command. When Goose is started this way, a host is not required and can instead be configured via the Controller.

NOTE: The controller currently is not Gaggle-aware, and only functions correctly when running Goose as a single process in standalone mode.

### Telnet Controller

The host and port that the telnet Controller listens on can be configured at start time with `--telnet-host` and `--telnet-port`. The telnet Controller can be completely disabled with the `--no-telnet` command line option. The defaults can be changed with `GooseDefault::TelnetHost`,`GooseDefault::TelnetPort`, and `GooseDefault::NoTelnet`.

To learn about all available commands, telnet into the Controller thread and enter `help` (or `?`), for example:
```
% telnet localhost 5116
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'.
goose> ?
goose 0.11.2 controller commands:
 help (?)           this help
 exit (quit)        exit controller
 start              start an idle load test
 stop               stop a running load test and return to idle state
 shutdown           shutdown running load test (and exit controller)
 host HOST          set host to load test, ie http://localhost/
 users INT          set number of simulated users
 hatchrate FLOAT    set per-second rate users hatch
 runtime TIME       set how long to run test, ie 1h30m5s
 config             display load test configuration
 config-json        display load test configuration in json format
 metrics            display metrics for current load test
 metrics-json       display metrics for current load test in json format
goose>
```

### WebSocket Controller

The host and port that the WebSocket Controller listens on can be configured at start time with `--websocket-host` and `--websocket-port`. The WebSocket Controller can be completely disabled with the `--no-websocket` command line option. The defaults can be changed with `GooseDefault::WebSocketHost`,`GooseDefault::WebSocketPort`, and `GooseDefault::NoWebSocket`.

The WebSocket Controller supports the same commands listed above. Requests and Response are in JSON format.

Requests must be made in the following format:
```json
{
  "request": String,
}
```

For example, a client should send the follow json to request the current load test metrics:
```json
{
  "request": "metrics",
}
```

Responses will always be in the following format:
```json
{
  "response": String,
  "success": Boolean,
}
```

For example:
```
% websocat ws://127.0.0.1:5117
foo
{"response":"unable to parse json, see Goose README.md","success":false}
{"request": "foo"}
{"response":"unrecognized command, see Goose README.md","success":false}
{"request": "config"}
{"response":"{\"help\":false,\"version\":false,\"list\":false,\"host\":\"http://apache/\",\"users\":5,\"hatch_rate\":\".5\",\"run_time\":\"\",\"log_level\":0,\"goose_log\":\"\",\"verbose\":1,\"running_metrics\":null,\"no_reset_metrics\":false,\"no_metrics\":false,\"no_task_metrics\":false,\"no_error_summary\":false,\"report_file\":\"\",\"request_log\":\"\",\"request_format\":\"json\",\"debug_log\":\"\",\"debug_format\":\"json\",\"no_debug_body\":false,\"status_codes\":false,\"no_telnet\":false,\"telnet_host\":\"0.0.0.0\",\"telnet_port\":5116,\"no_websocket\":false,\"websocket_host\":\"0.0.0.0\",\"websocket_port\":5117,\"no_autostart\":true,\"throttle_requests\":0,\"sticky_follow\":false,\"manager\":false,\"expect_workers\":null,\"no_hash_check\":false,\"manager_bind_host\":\"\",\"manager_bind_port\":0,\"worker\":false,\"manager_host\":\"\",\"manager_port\":0}","success":true}
{"request": "stop"}
{"response":"load test not running, failed to stop","success":false}
{"request": "exit"}
{"response":"goodbye!","success":true}
```

## Throttling Requests

By default, Goose will generate as much load as it can. If this is not desirable, the throttle allows optionally limiting the maximum number of requests per second made during a load test. This can be helpful to ensure consistency when running a load test from multiple different servers with different available resources.

The throttle is specified as an integer. For example:

```rust
$ cargo run --example simple -- --host http://local.dev/ -u100 -r20 -v --throttle-requests 5
```

In this example, Goose will launch 100 GooseUser threads, but the throttle will prevent them from generating a combined total of more than 5 requests per second. The `--throttle-requests` command line option imposes a maximum number of requests, not a minimum number of requests.

## Logging Load Test Errors

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-file=error.log` command line option, where `error.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--error-file` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":2239,"error":"503 Service Unavailable: /comment/reply/8151","final_url":"http://apache/comment/reply/8151","method":"Post","name":"(Auth) comment form","redirected":false,"response_time":26,"status_code":503,"url":"http://apache/comment/reply/8151","user":1}
{"elapsed":2261,"error":"503 Service Unavailable: /node/9577","final_url":"http://apache/node/9577","method":"Get","name":"(Anon) node page","redirected":false,"response_time":143,"status_code":503,"url":"http://apache/node/9577","user":2}
{"elapsed":2267,"error":"503 Service Unavailable: /","final_url":"http://apache/","method":"Get","name":"(Auth) front page","redirected":false,"response_time":138,"status_code":503,"url":"http://apache/","user":1}
{"elapsed":2404,"error":"503 Service Unavailable: /user/4375","final_url":"http://apache/user/4375","method":"Get","name":"(Anon) user page","redirected":false,"response_time":5,"status_code":503,"url":"http://apache/user/4375","user":2}
```

Logs include the entire [`GooseErrorMetric`] object as defined in `src/goose.rs`, which are created when requests result in an error.

By default Goose logs errors in JSON Lines format. The `--errors-format` option can be used to log in `csv`, `json` or `raw` format. The `raw` format is Rust's debug output of the entire [`GooseErrorMetric`] object.

For example, `csv` output of similar errors as those logged above would like like:
```csv
elapsed,method,name,url,final_url,redirected,response_time,status_code,user,error
6250,GET,"(Auth) node page","http://apache/node/3781","http://apache/node/3781",false,5,503,1,"503 Service Unavailable: /node/3781"
6256,GET,"(Auth) front page","http://apache/","http://apache/",false,5,503,1,"503 Service Unavailable: /"
6262,GET,"(Auth) node page","http://apache/node/5452","http://apache/node/5452",false,8,503,1,"503 Service Unavailable: /node/5452"
6265,GET,"(Anon) node page","http://apache/node/1819","http://apache/node/1819",false,5,503,0,"503 Service Unavailable: /node/1819"
```

## Logging Load Test Requests

Goose can optionally log details about all load test requests to a file. To enable, add the `--request-log=request.log` command line option, where `request.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--request-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"coordinated_omission_cadence":3361,"coordinated_omission_elapsed":0,"elapsed":24172,"error":"","final_url":"http://local.dev/misc/feed.png","method":"Get","name":"static asset","redirected":false,"response_time":4,"status_code":200,"success":true,"update":false,"url":"http://local.dev/misc/feed.png","user":7}
{"coordinated_omission_cadence":2183,"coordinated_omission_elapsed":0,"elapsed":24149,"error":"","final_url":"http://local.dev/user/4816","method":"Get","name":"(Anon) user page","redirected":false,"response_time":28,"status_code":200,"success":true,"update":false,"url":"http://local.dev/user/4816","user":2}
{"coordinated_omission_cadence":2738,"coordinated_omission_elapsed":0,"elapsed":24168,"error":"","final_url":"http://local.dev/themes/bartik/logo.png","method":"Get","name":"static asset","redirected":false,"response_time":14,"status_code":200,"success":true,"update":false,"url":"http://local.dev/themes/bartik/logo.png","user":1}
{"coordinated_omission_cadence":2514,"coordinated_omission_elapsed":0,"elapsed":24171,"error":"","final_url":"http://local.dev/themes/bartik/logo.png","method":"Get","name":"static asset","redirected":false,"response_time":11,"status_code":200,"success":true,"update":false,"url":"http://local.dev/themes/bartik/logo.png","user":4}
```

Logs include the entire [`GooseRequestMetric`] object as defined in `src/goose.rs`, which are created on all requests.

In the first line of the above example, `GooseUser` thread 7 made a successful `GET` request for `/misc/feed.png`, which takes 4 milliseconds. The second line is `GooseUser` thread 2 making a successful `GET` request for `/user/4816`, which takes 28 milliseconds.

By default Goose logs requests in JSON Lines format. The `--request-format` option can be used to log in `csv`, `json` or `raw` format. The `raw` format is Rust's debug output of the entire [`GooseRequestMetric`] object.

For example, `csv` output of similar requests as those logged above would like like:
```csv
elapsed,method,name,url,final_url,redirected,response_time,status_code,success,update,user,error,coordinated_omission_elapsed,coordinated_omission_cadence
22116,GET,"(Auth) node page","http://apache/node/3891","http://apache/node/3891",false,45,200,true,false,6,,0,3106
22158,GET,"static asset","http://apache/misc/feed.png","http://apache/misc/feed.png",false,3,200,true,false,1,,0,2477
22146,GET,"static asset","http://apache/misc/drupal.js?q9apdy","http://apache/misc/drupal.js?q9apdy",false,15,200,true,false,0,,0,1751
22160,GET,"static asset","http://apache/misc/jquery.js?v=1.4.4","http://apache/misc/jquery.js?v=1.4.4",false,5,200,true,false,5,,0,2293
22141,GET,"(Anon) node page","http://apache/node/9581","http://apache/node/9581",false,28,200,true,false,3,,0,2072
```

## Logging Load Test Tasks

Goose can optionally log details about all load test tasks to a file. To enable, add the `--task-log=task.log` command line option, where `task.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--task-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":22060,"name":"(Anon) front page","run_time":97,"success":true,"task_index":0,"taskset_index":0,"user":0}
{"elapsed":22118,"name":"(Anon) node page","run_time":41,"success":true,"task_index":1,"taskset_index":0,"user":5}
{"elapsed":22157,"name":"(Anon) node page","run_time":6,"success":true,"task_index":1,"taskset_index":0,"user":0}
{"elapsed":22078,"name":"(Auth) front page","run_time":109,"success":true,"task_index":1,"taskset_index":1,"user":6}
{"elapsed":22157,"name":"(Anon) user page","run_time":35,"success":true,"task_index":2,"taskset_index":0,"user":4}
```

Logs include the entire [`GooseTaskMetric`] object as defined in `src/goose.rs`, which are created each time any task is run.

In the first line of the above example, `GooseUser` thread 0 succesfully ran the `(Anon) front page` task in 97 milliseconds. In the second line `GooseUser` thread 5 succesfully ran the `(Anon) node page` task in 41 milliseconds.

By default Goose logs tass in JSON Lines format. The `--task-format` option can be used to log in `csv`, `json` or `raw` format. The `raw` format is Rust's debug output of the entire [`GooseTaskMetric`] object.

For example, `csv` output of similar tasks as those logged above would like like:
```csv
elapsed,taskset_index,task_index,name,run_time,success,user
21936,0,0,"(Anon) front page",83,true,0
21990,1,3,"(Auth) user page",34,true,1
21954,0,0,"(Anon) front page",84,true,5
22009,0,1,"(Anon) node page",34,true,2
21952,0,0,"(Anon) front page",95,true,7
```

## Load Test Debug Logging

Goose can optionally and efficiently log arbitrary details, and specifics about requests and responses for debug purposes. A central logging thread maintains a buffer to minimize the IO overhead, and controls the writing to ensure that multiple threads don't corrupt each other's messages.

To write to the debug log, you must invoke `client.log_debug(tag, Option<request>, Option<headers>, Option<body>)` from your load test task functions. The `tag` field is required and can be any arbitrary string: it can identify where in the load test the log was generated, and/or why debug is being written, and/or other details such as the contents of a form the load test posts. The `request` field is an optional reference to the [`GooseRawRequest`](https://docs.rs/goose/*/goose/goose/struct.GooseRawRequest) object and provides details such as what URL was requested and if it redirected, how long into the load test the request was made, which GooseUser thread made the request, and what status code the server responded with. The `headers` field is an optional reference to all the HTTP headers returned by the remote server for this request. The `body` field is an optional reference to the entire web page body returned by the server for this request.

See `examples/drupal_loadtest` for an example of how you might invoke log_debug from a load test.

Calls to `client.set_failure(tag, Option<request>, Option<headers>, Option<body>)` can be used to tell Goose that a request failed even though the server returned a successful status code, and will automatically invoke `log_debug()` for you. See `examples/drupal_loadtest` and `examples/umami` to see how you might use `set_failure` to generate useful debug logs.

When the load test is run with the `--debug-log=foo` command line option, where `foo` is either a relative or an absolute path, Goose will log all debug generated by calls to `client.log_debug()` (or to `client.set_failure()`) to this file. If the file already exists it will be overwritten. The following is an example debug log file entry:

```json
{"body":"<!DOCTYPE html>\n<html>\n  <head>\n    <title>503 Backend fetch failed</title>\n  </head>\n  <body>\n    <h1>Error 503 Backend fetch failed</h1>\n    <p>Backend fetch failed</p>\n    <h3>Guru Meditation:</h3>\n    <p>XID: 923425</p>\n    <hr>\n    <p>Varnish cache server</p>\n  </body>\n</html>\n","header":"{\"date\": \"Wed, 01 Jul 2020 10:27:31 GMT\", \"server\": \"Varnish\", \"content-type\": \"text/html; charset=utf-8\", \"retry-after\": \"5\", \"x-varnish\": \"923424\", \"age\": \"0\", \"via\": \"1.1 varnish (Varnish/6.1)\", \"x-varnish-cache\": \"MISS\", \"x-varnish-cookie\": \"SESSd7e04cba6a8ba148c966860632ef3636=hejsW1mQnnsHlua0AicCjEpUjnCRTkOLubwL33UJXRU\", \"content-length\": \"283\", \"connection\": \"keep-alive\"}","request":{"elapsed":4192,"final_url":"http://local.dev/node/3247","method":"GET","name":"(Auth) comment form","redirected":false,"response_time":8,"status_code":503,"success":false,"update":false,"url":"http://local.dev/node/3247","user":4},"tag":"post_comment: no form_build_id found on node/3247"}
```

If `--debug-log=foo` is not specified at run time, nothing will be logged and there is no measurable overhead in your load test.

By default Goose writes debug logs in JSON Lines format. The `--debug-format` option can be used to log in `json` or `raw` format. The `raw` format is Rust's debug output of the `GooseDebug` object.

## Gaggle: Distributed Load Test

Goose also supports distributed load testing. A Gaggle is one Goose process running in Manager mode, and 1 or more Goose processes running in Worker mode. The Manager coordinates starting and stopping the Workers, and collects aggregated metrics. Gaggle support is a cargo feature that must be enabled at compile-time as documented below. To launch a Gaggle, you must copy your load test application to all servers from which you wish to generate load.

It is strongly recommended that the same load test application be copied to all servers involved in a Gaggle. By default, Goose will verify that the load test is identical by comparing a hash of all load test rules. Telling it to skip this check can cause the load test to panic (for example, if a Worker defines a different number of tasks or task sets than the Manager).

### Gaggle Compile-time Feature

Gaggle support is a compile-time Cargo feature that must be enabled. Goose uses the [`nng`](https://docs.rs/nng/) library to manage network connections, and compiling `nng` requires that `cmake` be available.

The `gaggle` feature can be enabled from the command line by adding `--features gaggle` to your cargo command.

When writing load test applications, you can default to compiling in the Gaggle feature in the `dependencies` section of your `Cargo.toml`, for example:

```toml
[dependencies]
goose = { version = "^0.11", features = ["gaggle"] }
```

### Gaggle Manager

To launch a Gaggle, you first must start a Goose application in Manager mode. All configuration happens in the Manager. To start, add the `--manager` flag and the `--expect-workers` flag, the latter necessary to tell the Manager process how many Worker processes it will be coordinating. For example:

```
cargo run --features gaggle --example simple -- --manager --expect-workers 2 --host http://local.dev/ -v
```

This configures a Goose Manager to listen on all interfaces on the default port (0.0.0.0:5115) for 2 Goose Worker processes.

### Gaggle Worker

At this time, a Goose process can be either a Manager or a Worker, not both. Therefor, it usually makes sense to launch your first Worker on the same server that the Manager is running on. If not otherwise configured, a Goose Worker will try to connect to the Manager on the localhost. This can be done as follows:

```
cargo run --features gaggle --example simple -- --worker -v
```

In our above example, we expected 2 Workers. The second Goose process should be started on a different server. This will require telling it the host where the Goose Manager process is running. For example:

```
cargo run --example simple -- --worker --manager-host 192.168.1.55 -v
```

Once all expected Workers are running, the distributed load test will automatically start. We set the `-v` flag so Goose provides verbose output indicating what is happening. In our example, the load test will run until it is canceled. You can cancel the Manager or either of the Worker processes, and the test will stop on all servers.

### Gaggle Run-time Flags

* `--manager`: starts a Goose process in Manager mode. There currently can only be one Manager per Gaggle.
* `--worker`: starts a Goose process in Worker mode. How many Workers are in a given Gaggle is defined by the `--expect-workers` option, documented below.
* `--no-hash-check`: tells Goose to ignore if the load test application doesn't match between Worker(s) and the Manager. This is not recommended, and can cause the application to panic.

The `--no-metrics`, `--only-summary`, `--no-reset-metrics`, `--status-codes`, and `--no-hash-check` flags must be set on the Manager. Workers inherit these flags from the Manager

### Gaggle Run-time Options

* `--manager-bind-host <manager-bind-host>`: configures the host that the Manager listens on. By default Goose will listen on all interfaces, or `0.0.0.0`.
* `--manager-bind-port <manager-bind-port>`: configures the port that the Manager listens on. By default Goose will listen on port `5115`.
* `--manager-host <manager-host>`: configures the host that the Worker will talk to the Manager on. By default, a Goose Worker will connect to the localhost, or `127.0.0.1`. In a distributed load test, this must be set to the IP of the Goose Manager.
* `--manager-port <manager-port>`: configures the port that a Worker will talk to the Manager on. By default, a Goose Worker will connect to port `5115`.

The `--users`, `--hatch-rate`, `--host`, and `--run-time` options must be set on the Manager. Workers inherit these options from the Manager.

The `--throttle-requests` option must be configured on each Worker, and can be set to a different value on each Worker if desired.

### Technical Details

Goose uses [`nng`](https://docs.rs/nng/) to send network messages between the Manager and all Workers. [Serde](https://docs.serde.rs/serde/index.html) and [Serde CBOR](https://github.com/pyfisch/cbor) are used to serialize messages into [Concise Binary Object Representation](https://tools.ietf.org/html/rfc7049).

Workers initiate all network connections, and push metrics to the Manager process.

## RustLS

By default Reqwest (and therefore Goose) uses the system-native transport layer security to make HTTPS requests. This means `schannel` on Windows, `Security-Framework` on macOS, and `OpenSSL` on Linux. If you'd prefer to use a [pure Rust TLS implementation](https://github.com/ctz/rustls), disable default features and enable `rustls` in `Cargo.toml` as follows:

```toml
[dependencies]
goose = { version = "^0.11", default-features = false, features = ["rustls"] }
```
