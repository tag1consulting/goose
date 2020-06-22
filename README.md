# Goose

Have you ever been attacked by a goose?

[![crates.io](https://img.shields.io/crates/v/goose.svg)](https://crates.io/crates/goose)
[![Documentation](https://docs.rs/goose/badge.svg)](https://docs.rs/goose)
[![Apache-2.0 licensed](https://img.shields.io/crates/l/goose.svg)](./LICENSE)
[![CI](https://github.com/tag1consulting/goose/workflows/CI/badge.svg)](https://github.com/tag1consulting/goose/actions?query=workflow%3ACI)

## Overview

Goose is a Rust load testing tool inspired by [Locust](https://locust.io/).
User behavior is defined with standard Rust code. Load tests are applications
that have a dependency on the Goose library. Web requests are made with the
[Reqwest](https://docs.rs/reqwest) HTTP Client.

## Getting Started

The 
[in-line documentation](https://docs.rs/goose/*/goose/#creating-a-simple-goose-load-test)
offers much more detail about Goose specifics. For a general background to help you get
started with Rust and Goose, read on.

[Cargo](https://doc.rust-lang.org/cargo/) is the Rust package manager. To create a new
load test, use Cargo to create a new application (you can name your application anything,
we've generically selected `loadtest`):

```bash
$ cargo new loadtest
     Created binary (application) `loadtest` package
$ cd loadtest/
```

This creates a new directory named `loadtest/` containing `loadtest/Cargo.toml` and
`loadtest/src/main.rs`. Start by editing `Cargo.toml` adding Goose under the dependencies
heading:


```toml
[dependencies]
goose = "^0.7"
```

At this point it's possible to compile all dependencies, though the
resulting binary only displays "Hello, world!":

```
$ cargo run
    Updating crates.io index
  Downloaded goose v0.7.5
      ...
   Compiling goose v0.7.5
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [unoptimized + debuginfo] target(s) in 52.97s
     Running `target/debug/loadtest`
Hello, world!
```

To create an actual load test, you first have to add the following boilerplate
to the top of `src/main.rs`:

```rust
use goose::prelude::*;
```

Then create a new load testing function. For our example we're simply going
to load the front page of the website we're load-testing. Goose passes all
load testing functions a mutable pointer to a GooseClient object, which is used
to track statistics and make web requests. Thanks to the Reqwest library, the
Goose client manages things like cookies, headers, and sessions for you. Load
testing functions must be declared async.

In load tests functions you typically do not set the host, and instead configure
the host at run time, so you can easily run your load test against different
environments without recompiling:

```rust
async fn loadtest_index(client: &mut GooseClient) {
    let _response = client.get("/");
}
```

Finally, edit the `main()` function, removing the hello world text and replacing
it as follows:

```rust
fn main() {
    GooseAttack::initialize()
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .execute();
}
```

And that's it, you've created your first load test! Let's run it and see what
happens.

```bash
$ cargo run
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [unoptimized + debuginfo] target(s) in 3.56s
     Running `target/debug/loadtest`
12:09:56 [ERROR] Host must be defined globally or per-TaskSet. No host defined for LoadtestTasks.
```

Goose is unable to run, as it doesn't know the domain you want to load test. So,
let's try again, this time passing in the `--host` flag. After running for a few
seconds, we then press `ctrl-c` to stop Goose:

```bash
$ cargo run -- --host http://apache.fosciana/
    Finished dev [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/loadtest --host 'http://apache.fosciana/'`
^C12:12:47 [ WARN] caught ctrl-c, stopping...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /                   | 905            | 0 (0%)         | 301    | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median    
 ----------------------------------------------------------------------------- 
 GET /                   | 3139       | 952        | 102412     | 3000      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /                   | 3000   | 4000   | 5000   | 6000   | 8000   |   8000
```

When printing statistics, Goose displays three tables. The first shows the total
number of requests made (905), how many of those failed (0), the everage number
of requests per second (301), and the average number of failed requests per
second (0).

The second table shows the average time required to load a page (3139 milliseconds),
the mininimum time to load a page (952 ms), the maximum time to load a page (102412
ms) and the median time to load a page (3000 ms).

The final table shows the slowest page load time for a range of percentiles. In our
example, in the 50% fastest page loads, the slowest page loaded in 3000 ms. In the
75% fastest page loads, the slowest page loadd in 4000 ms, etc.

In most load tests you'll make have different tasks being run, and each will be
split out in the statistics, along with a line showing all totaled together in
aggregate.

Refer to the
[examples directory](https://github.com/tag1consulting/goose/tree/master/examples)
for more complicated and useful load test examples.

## Tips

* Avoid `unwrap()` in your task functions -- Goose generates a lot of load, and this tends
to trigger errors. Embrace Rust's warnings and properly handle all possible errors, this
will save you time debugging later.
* When running your load test for real, use the cargo `--release` flag to generate
optimized code. This can generate considerably more load test traffic.

## Simple Example

The `-h` flag will show all run-time configuration options available to Goose
load tests. For example, pass the `-h` flag to the `simple` example,
`cargo run --example simple -- -h`:

```
client 0.7.5
CLI options available when launching a Goose load test

USAGE:
    simple [FLAGS] [OPTIONS]

FLAGS:
    -h, --help             Prints help information
    -l, --list             Shows list of all possible Goose tasks and exits
    -g, --log-level        Log level (-g, -gg, -ggg, etc.)
        --manager          Enables manager mode
        --no-hash-check    Ignore worker load test checksum
        --no-stats         Don't print stats in the console
        --only-summary     Only prints summary stats
        --reset-stats      Resets statistics once hatching has been completed
        --status-codes     Includes status code counts in console stats
        --sticky-follow    Client follows redirect of base_url with subsequent requests
    -V, --version          Prints version information
    -v, --verbose          Debug level (-v, -vv, -vvv, etc.)
        --worker           Enables worker mode

OPTIONS:
    -c, --clients <clients>                        Number of concurrent Goose users (defaults to available CPUs)
        --expect-workers <expect-workers>
            Required when in manager mode, how many workers to expect [default: 0]

    -r, --hatch-rate <hatch-rate>                  How many users to spawn per second [default: 1]
    -H, --host <host>                              Host to load test, for example: http://10.21.32.33 [default: ]
        --log-file <log-file>                      Log file name [default: goose.log]
        --manager-bind-host <manager-bind-host>    Define host manager listens on, formatted x.x.x.x [default: 0.0.0.0]
        --manager-bind-port <manager-bind-port>    Define port manager listens on [default: 5115]
        --manager-host <manager-host>              Host manager is running on [default: 127.0.0.1]
        --manager-port <manager-port>              Port manager is listening on [default: 5115]
    -t, --run-time <run-time>
            Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.) [default: ]
```

The `examples/simple.rs` example copies the simple load test documented on the locust.io web page, rewritten in Rust for Goose. It uses minimal advanced functionality, but demonstrates how to GET and POST pages. It defines a single Task Set which has the client log in and then load a couple of pages.

Goose can make use of all available CPU cores. By default, it will launch 1 client per core, and it can be configured to launch many more. The following was configured instead to launch 1,024 clients. Each client randomly pauses 5 to 15 seconds after each task is loaded, so it's possible to spin up a large number of clients. Here is a snapshot of `top` when running this example on an 8-core VM with 10G of available RAM -- there were ample resources to launch considerably more "clients", though `ulimit` had to be resized:

```
top - 11:14:57 up 16 days,  4:40,  2 users,  load average: 0.00, 0.04, 0.01
Tasks: 129 total,   1 running, 128 sleeping,   0 stopped,   0 zombie
%Cpu(s):  0.3 us,  0.3 sy,  0.0 ni, 99.2 id,  0.0 wa,  0.0 hi,  0.2 si,  0.0 st
MiB Mem :   9993.6 total,   6695.1 free,   1269.3 used,   2029.2 buff/cache
MiB Swap:  10237.0 total,  10234.7 free,      2.3 used.   8401.5 avail Mem 

  PID USER      PR  NI    VIRT    RES    SHR S  %CPU  %MEM     TIME+ COMMAND                                                   
19776 goose     20   0    9.8g 874688   8252 S   6.3   8.5   0:42.90 simple                                                    
```

Here's the output of running the loadtest. The `-v` flag sends `INFO` and more critical messages to stdout (in addition to the log file). The `-c1024` tells Goose to spin up 1,024 clients. The `-r32` option tells Goose to spin up 32 clients per second. The `-t 10m` option tells Goose to run the load test for 10 minutes, or 600 seconds. The `--print-stats` flag tells Goose to collect statistics during the load test, and the `--status-codes` flag tells it to include statistics about HTTP Status codes returned by the server. Finally, the `--only-summary` flag tells Goose to only display the statistics when the load test finishes, otherwise it would display running statistics every 15 seconds for the duration of the test.

```
$ cargo run --release --example simple -- --host http://apache.fosciana -v -c1024 -r32 -t 10m --print-stats --status-codes --only-summary
    Finished release [optimized] target(s) in 0.05s
     Running `target/release/examples/simple --host 'http://apache.fosciana' -v -c1024 -r32 -t 10m --print-stats --status-codes --only-summary`
18:42:48 [ INFO] Output verbosity level: INFO
18:42:48 [ INFO] Logfile verbosity level: INFO
18:42:48 [ INFO] Writing to log file: goose.log
18:42:48 [ INFO] run_time = 600
18:42:48 [ INFO] global host configured: http://apache.fosciana
18:42:53 [ INFO] launching client 1 from WebsiteUser...
18:42:53 [ INFO] launching client 2 from WebsiteUser...
18:42:53 [ INFO] launching client 3 from WebsiteUser...
18:42:53 [ INFO] launching client 4 from WebsiteUser...
18:42:53 [ INFO] launching client 5 from WebsiteUser...
18:42:53 [ INFO] launching client 6 from WebsiteUser...
18:42:53 [ INFO] launching client 7 from WebsiteUser...
18:42:53 [ INFO] launching client 8 from WebsiteUser...

```
...
```
18:43:25 [ INFO] launching client 1022 from WebsiteUser...
18:43:25 [ INFO] launching client 1023 from WebsiteUser...
18:43:25 [ INFO] launching client 1024 from WebsiteUser...
18:43:25 [ INFO] launched 1024 clients...
18:53:26 [ INFO] stopping after 600 seconds...
18:53:26 [ INFO] waiting for clients to exit
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /                   | 34,077         | 582 (1.7%)     | 53     | 0    
 GET /about/             | 34,044         | 610 (1.8%)     | 53     | 0    
 POST /login             | 1,024          | 0 (0%)         | 1      | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 69,145         | 1,192 (1.7%)   | 107    | 1    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median      
 ----------------------------------------------------------------------------- 
 GET /                   | 12.38      | 0.01       | 1001.10    | 0.09      
 GET /about/             | 12.80      | 0.01       | 1001.10    | 0.08      
 POST /login             | 0.21       | 0.15       | 1.82       | 0.20      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 12.41      | 0.01       | 1001.10    | 0.02      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /                   | 0.09   | 0.10   | 345.18 | 500.60 | 1000.93 | 1001.09
 GET /about/             | 0.08   | 0.10   | 356.65 | 500.61 | 1000.94 | 1001.08
 POST /login             | 0.20   | 0.22   | 0.27   | 0.34   | 1.36   |   1.82
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.08   | 0.10   | 349.40 | 500.60 | 1000.93 | 1001.09
-------------------------------------------------------------------------------
 Name                    | Status codes              
 ----------------------------------------------------------------------------- 
 GET /                   | 33,495 [200], 582 [0]      
 GET /about/             | 33,434 [200], 610 [0]      
 POST /login             | 1,024 [200]              
-------------------------------------------------------------------------------
 Aggregated              | 67,953 [200]              
```

## Gaggle: Distributed Load Test

Goose also supports distributed load testing. A Gaggle is one Goose process
running in manager mode, and 1 or more Goose processes running in worker mode.
The manager coordinates starting and stopping the workers, and collects
aggregated statistics. Gaggle support is a cargo feature that must be enabled
at compile-time as documented below. To launch a Gaggle, you must copy your
load test application to all servers from which you wish to generate load.

### Gaggle Compile-time Feature

Gaggle support is a compile-time Cargo feature that must be enabled. Goose uses
the [`nng`](https://docs.rs/nng/) library to manage network connections, and
compiling `nng` requires that `cmake` be available.

The `gaggle` feature can be enabled from the command line by adding
`--features gaggle` to your cargo command.

When writing load test applications, you can default to compiling in the Gaggle
feature in the `dependencies` section of your `Cargo.toml`, for example:

```toml
[dependencies]
goose = { version = "^0.7", features = ["gaggle"] }
```

### Goose Manager

To launch a Gaggle, you first must start a Goose application in manager
mode. All configuration happens in the manager. To start, add the `--manager`
flag and the `--expect-workers` flag, the latter necessary to tell the Manager
process how many Worker processes it will be coordinating. For example:

```
cargo run --features gaggle --example simple -- --manager --expect-workers 2 --host http://local.dev/ -v
```

This configures a Goose manager to listen on all interfaces on the default
port (0.0.0.0:5115) for 2 Goose worker processes.

### Goose Worker

At this time, a Goose process can be either a manager or a worker, not both.
Therefor, it makes sense to launch your first worker on the same server that
the manager is running on. If not otherwise configured, a Goose worker will
try to connect to the manager on the localhost. This can be done as folows:

```
cargo run --features gaggle --example simple -- --worker -v
```

In our above example, we expected 2 workers. The second Goose process should
be started on a different server. This will require telling it the host where
the Goose manager proocess is running. For example:

```
cargo run --example simple -- --worker --manager-host 192.168.1.55 -v
```

Once all expected workers are running, the distributed load test will
automatically start. We set the `-v` flag so Goose provides verbose output
indicating what is happening. In our example, the load test will run until
it is canceled. You can cancel the manager or either of the worker processes,
and the test will stop on all servers.

### Goose Run-time Flags

* `--manager`: starts a Goose process in manager mode. There currently can only be one manager per Gaggle.
* `--worker`: starts a Goose process in worker mode. How many workers are in a given Gaggle is defined by the `--expect-workers` option, documented below.
* `--no-hash-check`: tells Goose to ignore if the load test applications don't match between worker(s) and manager. Not recommended.

The `--no-stats`, `--only-summary`, `--reset-stats`, `--status-codes`, and `--no-hash-check` flags must be set on the manager. Workers inheret these flags from the manager

### Goose Run-time Options

* `--manager-bind-host <manager-bind-host>`: configures the host that the manager listens on. By default Goose will listen on all interfaces, or `0.0.0.0`.
* `--manager-bind-port <manager-bind-port>`: configures the port that the manager listens on. By default Goose will listen on port `5115`.
* `--manager-host <manager-host>`: configures the host that the worker will talk to the manager on. By default, a Goose worker will connect to the localhost, or `127.0.0.1`. In a distributed load test, this must be set to the IP of the Goose manager.
* `--manager-port <manager-port>`: configures the port that a worker will talk to the manager on. By default, a Goose worker will connect to port `5115`.

The `--clients`, `--hatch-rate`, `--host`, and `--run-time` options must be set on the manager. Workers inheret these options from the manager.

### Technical Details

Goose uses [`nng`](https://docs.rs/nng/) to send network messages between
the manager and all workers. [Serde](https://docs.serde.rs/serde/index.html)
and [Serde CBOR](https://github.com/pyfisch/cbor) are used to serialize messages
into [Concise Binary Object Representation](https://tools.ietf.org/html/rfc7049).

Workers initiate all network connections, and push a HashMap containing load test
statistics up to the manager process.

## Roadmap

The Goose project roadmap is documented in [TODO.md](https://github.com/tag1consulting/goose/blob/master/TODO.md).
