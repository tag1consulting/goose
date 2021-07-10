# Goose

Have you ever been attacked by a goose?

[![crates.io](https://img.shields.io/crates/v/goose.svg)](https://crates.io/crates/goose)
[![Documentation](https://docs.rs/goose/badge.svg)](https://docs.rs/goose)
[![Apache-2.0 licensed](https://img.shields.io/crates/l/goose.svg)](./LICENSE)
[![CI](https://github.com/tag1consulting/goose/workflows/CI/badge.svg)](https://github.com/tag1consulting/goose/actions?query=workflow%3ACI)
[![Docker Repository on Quay](https://quay.io/repository/tag1consulting/goose/status "Docker Repository on Quay")](https://quay.io/repository/tag1consulting/goose)

### Documentation

- [README](https://github.com/tag1consulting/goose/blob/main/README.md)
- [Developer documentation](https://docs.rs/goose/)
- [Blogs and more](https://tag1.com/goose/)
  - [Goose vs Locust and jMeter](https://www.tag1consulting.com/blog/jmeter-vs-locust-vs-goose)
  - [Real-life load testing with Goose](https://www.tag1consulting.com/blog/real-life-goose-load-testing)
  - [Gaggle: a distributed load test](https://www.tag1consulting.com/blog/show-me-how-flock-flies-working-gaggle-goose)
  - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)


## Coordinated Omission Mitigation

THIS IS AN EXPERIMENTAL FEATURE THAT IS DISABLED BY DEFAULT. The following documentation is a work in progress, and may currently be misleading.

When enabled, Goose attempts to mitigate the loss of metrics data (Coordinated Omission) caused by an abnormally lengthy response to a request.

### Definition

To understand Coordinated Omission and how Goose attempts to mitigate it, it's necessary to understand how Goose is scheduling requests. Goose launches one thread per `GooseUser`. Each `GooseUser` is assigned a single `GooseTaskSet`. Each of these `GooseUser` threads then loop repeatedly through all of the `GooseTasks` defined in the assigned `GooseTaskSet`, each of which can involve any number of individual requests. However, at any given time, each `GooseUser` is only making a single request and then asynchronously waiting for the response.

If something causes the response to a request to take abnormally long, raw Goose metrics only see this slowdown as affecting a specific request (or one request per `GooseUser`). The slowdown can be caused by a variety of issues, such as a resource bottleneck (on the Goose client or the web server), garbage collection, a cache stampede, or even a network issue. A real user loading the same web page would see a much larger effect, as all requests to the affected server would stall. Even static assets such as images and scripts hosted on a reliable and fast CDN can be affected, as the web browser won't know to load them until it first loads the HTML from the affected web server. Because Goose is only making one request at a time per `GooseUser`, it may only see one or very few slow requests and then all other requests resume at normal speed. This results in a bias in the metrics to "ignore" or "hide" the true effect of a slowdown, commonly referred to as Coordinated Omission.

### Mitigation

Goose attempts to mitigate Coordinated Omission by back-filling the metrics with the statistically expected requests. To do this, it tracks the normal "cadence" of each `GooseUser`, timing how long it takes to loop through all `GooseTasks` in the assigned `GooseTaskSet`. By default, Goose will trigger Coordinated Omission Mitigation if the time to loop through a `GooseTaskSet` takes more than twice as long as the average time of all previous loops. In this case, on the next loop through the `GooseTaskSet` when tracking the actual metrics for each subsequent request in all `GooseTasks` it will also add in statistically generated "requests" with a `response_time` starting at the unexpectedly long request time, then again with that `response_time` minus the normal "cadence", continuing to generate a metric then subtract the normal "cadence" until arriving at the expected `response_time`. In this way, Goose is able to estimate the actual effect of a slowdown.

When Goose detects an abnormally slow request (one in which the individual request takes longer than the normal `user_cadence`), it will generate an INFO level message (which will be visible if Goose was started with the `-v` run time flag, or written to the log if started with the `-g` run time flag and `--goose-log` is configured). For example:

```
13:10:30 [INFO] 11.401s into goose attack: "GET http://apache/node/1557" [200] took abnormally long (1814 ms), task name: "(Anon) node page"
13:10:30 [INFO] 11.450s into goose attack: "GET http://apache/node/5016" [200] took abnormally long (1769 ms), task name: "(Anon) node page"
```

If the `--request-log` is enabled, you can get more details, in this case by looking for elapsed times matching the above messages, specifically 1814 and 1769 respectively:

```
{"coordinated_omission_elapsed":0,"elapsed":11401,"error":"","final_url":"http://apache/node/1557","method":"Get","name":"(Anon) node page","redirected":false,"response_time":1814,"status_code":200,"success":true,"update":false,"url":"http://apache/node/1557","user":2,"user_cadence":1727}
{"coordinated_omission_elapsed":0,"elapsed":11450,"error":"","final_url":"http://apache/node/5016","method":"Get","name":"(Anon) node page","redirected":false,"response_time":1769,"status_code":200,"success":true,"update":false,"url":"http://apache/node/5016","user":0,"user_cadence":1422}
```

In the requests file, you can see that two different user threads triggered Coordinated Omission Mitigation, specifically threads 2 and 0. Both `GooseUser` threads were loading the same `GooseTask` as due to task weighting this is the task loaded the most frequently. Both `GooseUser` threads loop through all `GooseTasks` in a similar amount of time: thread 2 takes on average 1.727 seconds, thread 0 takes on average 1.422 seconds.

Also if the `--request-log` is enabled, requests back-filled by Coordinated Omission Mitigation show up in the generated log file, even though they were not actually sent to the server. Normal requests not generated by Coordinated Omission Mitigation have a `coordinated_omission_elapsed` of 0.

Coordinated Omission Mitigation is disabled by default. This experimental feature can be enabled by enabling the `--co-mitigation` run time option when starting Goose. It can be configured to use the `average`, `minimum`, or `maximum` `GoouseUser` cadence when backfilling statistics.

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

## Gaggle: Distributed Load Test

Goose also supports distributed load testing. A Gaggle is one Goose process running in Manager mode, and 1 or more Goose processes running in Worker mode. The Manager coordinates starting and stopping the Workers, and collects aggregated metrics. Gaggle support is a cargo feature that must be enabled at compile-time as documented below. To launch a Gaggle, you must copy your load test application to all servers from which you wish to generate load.

It is strongly recommended that the same load test application be copied to all servers involved in a Gaggle. By default, Goose will verify that the load test is identical by comparing a hash of all load test rules. Telling it to skip this check can cause the load test to panic (for example, if a Worker defines a different number of tasks or task sets than the Manager).

### Gaggle Compile-time Feature

Gaggle support is a compile-time Cargo feature that must be enabled. Goose uses the [`nng`](https://docs.rs/nng/) library to manage network connections, and compiling `nng` requires that `cmake` be available.

The `gaggle` feature can be enabled from the command line by adding `--features gaggle` to your cargo command.

When writing load test applications, you can default to compiling in the Gaggle feature in the `dependencies` section of your `Cargo.toml`, for example:

```toml
[dependencies]
goose = { version = "^0.12", features = ["gaggle"] }
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
goose = { version = "^0.12", default-features = false, features = ["rustls"] }
```
