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

The following is an example of running Goose, simulating 8 concurrent clients all loading pages as quickly as possible. In this example, Goose is running in an 8-core VM, and loading static pages from Varnish in front of Apache in another 8-core VM. (Compared against Locust running in the same configuration, Goose is currently able to generate roughly 8 to 10 times as much load. It can also be configured to make considerably better use of available resources.)

```
$ cargo run --release -- --host http://apache.fosciana --print-stats -v -t1800
   Compiling goose v0.1.0 (~/goose)
    Finished release [optimized] target(s) in 3.16s
     Running `target/release/goose --host 'http://apache.fosciana' --print-stats -v -t1800`
15:26:55 [ INFO] Output verbosity level: INFO
15:26:55 [ INFO] Logfile verbosity level: INFO
15:26:55 [ INFO] Writing to log file: goose.log
15:26:55 [ INFO] run_time = 1800
15:26:55 [ INFO] concurrent clients defaulted to 8 (number of CPUs)
15:26:55 [ INFO] hatch_rate defaulted to 8 (number of CPUs)
15:26:55 [ INFO] launching WebsiteTasks client 1...
15:26:55 [ INFO] launching WebsiteTasks client 2...
15:26:56 [ INFO] launching WebsiteTasks client 3...
15:26:56 [ INFO] launching WebsiteTasks client 4...
15:26:56 [ INFO] launching WebsiteTasks client 5...
15:26:56 [ INFO] launching WebsiteTasks client 6...
15:26:56 [ INFO] launching WebsiteTasks client 7...
15:26:56 [ INFO] launching WebsiteTasks client 8...
15:26:56 [ INFO] launched 8 clients...
15:56:57 [ INFO] exiting after 1800 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /story.html         | 11,948,418     | 0 (0.0%)       | 6,638  | 0    
 GET /                   | 7,963,796      | 0 (0.0%)       | 4,424  | 0    
 GET /about.html         | 3,982,280      | 0 (0.0%)       | 2,212  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 23,894,494     | 0 (0.0%)       | 13,274 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /story.html         | 0.05       | 0.01       | 596.99     | 0.05      
 GET /                   | 0.06       | 0.01       | 596.86     | 0.05      
 GET /about.html         | 0.05       | 0.01       | 597.01     | 0.05      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05       | 0.01       | 597.01     | 0.58      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /story.html         | 0.05   | 0.07   | 0.15   | 0.17   | 0.26   |   0.58
 GET /                   | 0.05   | 0.07   | 0.16   | 0.19   | 0.31   |   0.56
 GET /about.html         | 0.05   | 0.07   | 0.15   | 0.17   | 0.27   |   0.58
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05   | 0.07   | 0.15   | 0.18   | 0.28   |   0.57
-------------------------------------------------------------------------------
```

## Roadmap

The Goose project roadmap is documented in [TODO.md](https://github.com/jeremyandrews/goose/blob/master/TODO.md).
