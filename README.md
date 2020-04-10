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

The following is an example of running Goose, simulating 8 concurrent clients all loading pages as quickly as possible. In this example, Goose is running in an 8-core VM, and loading static pages from Varnish in front of Apache in another 8-core VM. (Compared against Locust running in the same configuration, Goose is currently able to generate nearly 8-10 times as much load.)

```
$ cargo run --release -- -t1800 -v --print-stats 
   Compiling goose v0.1.0 (~/goose)
    Finished release [optimized] target(s) in 2.94s
     Running `target/release/goose -t1800 -v --print-stats`
12:11:18 [ INFO] Output verbosity level: INFO
12:11:18 [ INFO] Logfile verbosity level: INFO
12:11:18 [ INFO] Writing to log file: goose.log
12:11:18 [ INFO] run_time = 1800
12:11:18 [ INFO] concurrent clients defaulted to 8 (number of CPUs)
12:11:18 [ INFO] hatch_rate defaulted to 8 (number of CPUs)
12:11:18 [ INFO] launching WebsiteTasks client 1...
12:11:18 [ INFO] launching WebsiteTasks client 2...
12:11:18 [ INFO] launching WebsiteTasks client 3...
12:11:18 [ INFO] launching WebsiteTasks client 4...
12:11:19 [ INFO] launching WebsiteTasks client 5...
12:11:19 [ INFO] launching WebsiteTasks client 6...
12:11:19 [ INFO] launching WebsiteTasks client 7...
12:11:19 [ INFO] launching WebsiteTasks client 8...
12:11:19 [ INFO] launched 8 clients...
12:41:18 [ INFO] exiting after 1800 seconds...
-------------------------------------------------------------------------------
WebsiteTasks:
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /index.html         | 8,757,164      | 0 (0.0%)       | 4,865  | 0    
 GET /story.html         | 13,136,007     | 0 (0.0%)       | 7,297  | 0    
 GET /about.html         | 4,375,590      | 0 (0.0%)       | 2,430  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 26,268,761     | 0 (0.0%)       | 14,593 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /index.html         | 0.06       | 0.01       | 595.81     | 0.04      
 GET /story.html         | 0.05       | 0.01       | 595.86     | 0.04      
 GET /about.html         | 0.05       | 0.01       | 595.68     | 0.04      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05       | 0.01       | 595.86     | 0.03      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /index.html         | 0.04   | 0.07   | 0.16   | 0.18   | 0.30   |   0.52
 GET /story.html         | 0.04   | 0.06   | 0.14   | 0.16   | 0.25   |   0.52
 GET /about.html         | 0.04   | 0.06   | 0.14   | 0.16   | 0.25   |   0.52
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.04   | 0.07   | 0.14   | 0.17   | 0.27   |   0.52

```

## Roadmap

The Goose project roadmap is documented in [TODO.md](https://github.com/jeremyandrews/goose/blob/master/TODO.md).
