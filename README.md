# Goose

Have you ever been attacked by a goose?

## Overview

Goose begins as a minimal port of [Locust](https://locust.io/) to Rust.

User behaviour is defined with standard Rust code.

The goal of the MVP is to support the following subset of commands:

```
$ cargo run -- -h
    Finished dev [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/goose -h`
client 0.1.0

USAGE:
    goose [FLAGS] [OPTIONS]

FLAGS:
    -h, --help            Prints help information
    -l, --list            Shows list of all possible Goose classes and exits
    -g, --log-level       Log level (-g, -gg, -ggg, etc.)
        --only-summary    Only prints summary stats
        --print-stats     Prints stats in the console
        --reset-stats     Resets statistics once hatching has been completed
    -V, --version         Prints version information
    -v, --verbose         Debug level (-v, -vv, -vvv, etc.)

OPTIONS:
    -c, --clients <clients>              Number of concurrent Goose users [default: 1]
    -r, --hatch-rate <hatch-rate>        The rate per second in which clients are spawned [default: 1]
    -H, --host <host>                    Host to load test in the following format: http://10.21.32.33 [default: ]
        --log-file <log-file>             [default: goose.log]
    -t, --run-time <run-time>            Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.)
                                         [default: ]
    -s, --stop-timeout <stop-timeout>    Number of seconds to wait for a simulated user to complete any executing task
                                         before exiting. Default is to terminate immediately [default: 0]
```

## Examples

The following is an example of running Goose, simulating 8 concurrent clients all loading pages as quickly as possible. In this example, Goose is running in an 8-core VM, and loading static pages from Apache in another 8-core VM. (Compared against Locust running in the same configuration, Goose is currently able to generate nearly ten times as much load.)

```
$ cargo run --release -- -t1800 -v --print-stats
   Compiling goose v0.1.0 (~/goose)
    Finished release [optimized] target(s) in 2.76s
     Running `target/release/goose -t1800 -v --print-stats`
09:32:12 [ INFO] Output verbosity level: INFO
09:32:12 [ INFO] Logfile verbosity level: INFO
09:32:12 [ INFO] Writing to log file: goose.log
09:32:12 [ INFO] run_time = 1800
09:32:12 [ INFO] concurrent clients defaulted to 8 (number of CPUs)
09:32:12 [ INFO] hatch_rate defaulted to 8 (number of CPUs)
09:32:12 [ INFO] launching WebsiteTasks client 1...
09:32:13 [ INFO] launching WebsiteTasks client 2...
09:32:13 [ INFO] launching WebsiteTasks client 3...
09:32:13 [ INFO] launching WebsiteTasks client 4...
09:32:13 [ INFO] launching WebsiteTasks client 5...
09:32:13 [ INFO] launching WebsiteTasks client 6...
09:32:13 [ INFO] launching WebsiteTasks client 7...
09:32:13 [ INFO] launching WebsiteTasks client 8...
09:32:13 [ INFO] launched 8 clients...
10:02:13 [ INFO] exiting after 1800 seconds...
-------------------------------------------------------------------------------
WebsiteTasks:
-------------------------------------------------------------------------------
 Name                    | # reqs         | # fails        | req/s | fail/s
-------------------------------------------------------------------------------
 GET /index.html         | 8,186,635      | 0 (0.0%)       | 4,548 | 0    
 GET /story.html         | 12,283,960     | 0 (0.0%)       | 6,824 | 0    
 GET /about.html         | 4,095,219      | 0 (0.0%)       | 2,275 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
-------------------------------------------------------------------------------
 GET /index.html         | 0.05       | 0.01       | 607.01     | 0.04      
 GET /story.html         | 0.06       | 0.01       | 607.09     | 0.04      
 GET /about.html         | 0.06       | 0.01       | 601.24     | 0.04      
```

## Roadmap

Once the above is complete, additional planned features include:
 - gaggle support for distributed load testing (leader/worker)
 - a web UI for controlling and monitoring Goose
