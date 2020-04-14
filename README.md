# Goose

Have you ever been attacked by a goose?

## Overview

Goose begins as a minimal port of [Locust](https://locust.io/) to Rust.

User behaviour is defined with standard Rust code.

The goal of the MVP is to support the following subset of commands:

```
$ cargo run --release -- -h
    Finished release [optimized] target(s) in 0.05s
     Running `target/release/goose -h`
client 0.2.0

USAGE:
    goose [FLAGS] [OPTIONS] --host <host>

FLAGS:
    -h, --help            Prints help information
    -l, --list            Shows list of all possible Goose tasks and exits
    -g, --log-level       Log level (-g, -gg, -ggg, etc.)
        --only-summary    Only prints summary stats
        --print-stats     Prints stats in the console
        --reset-stats     Resets statistics once hatching has been completed
        --status-codes    Includes status code counts in console stats
    -V, --version         Prints version information
    -v, --verbose         Debug level (-v, -vv, -vvv, etc.)

OPTIONS:
    -c, --clients <clients>              Number of concurrent Goose users (defaults to available CPUs)
    -r, --hatch-rate <hatch-rate>        How many users to spawn per second (defaults to available CPUs)
    -H, --host <host>                    Host to load test in the following format: http://10.21.32.33
        --log-file <log-file>             [default: goose.log]
    -t, --run-time <run-time>            Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.)
                                         [default: ]
    -s, --stop-timeout <stop-timeout>    Number of seconds to wait for a simulated user to complete any executing task
                                         before exiting. Default is to terminate immediately [default: 0]
```

## Examples

The following is an example of running Goose, simulating 8 concurrent clients all loading pages as quickly as possible. In this example, Goose is running in an 8-core VM, and loading static pages from Varnish in front of Apache in another 8-core VM. (Compared against Locust running in the same configuration, Goose is currently able to generate roughly 8 to 10 times as much load. It can also be configured to make considerably better use of available resources.)

The `--host` flag is required to tell Goose which host to load all paths defined in `goosefile.rs` from. A single `-v` flag causes all `INFO` level and higher logs to be written to stdout. The test is configured to run for 30 minutes, or 1800 seconds, with the `-t 30m` option. The `--print-stats` flag configures Goose to collect statistics, and to display them when the test completes or is canceled. Finally, the `--status-codes` flag configures Goose to also count and display the status codes returned per request.

```
$ cargo run --release -- --host http://apache.fosciana -v -t 30m --print-stats --status-codes
   Compiling goose v0.1.0 (~/goose)
    Finished release [optimized] target(s) in 3.06s
     Running `target/release/goose --host 'http://apache.fosciana' -v -t 30m --print-stats --status-codes`
06:56:38 [ INFO] Output verbosity level: INFO
06:56:38 [ INFO] Logfile verbosity level: INFO
06:56:38 [ INFO] Writing to log file: goose.log
06:56:38 [ INFO] run_time = 1800
06:56:38 [ INFO] concurrent clients defaulted to 8 (number of CPUs)
06:56:38 [ INFO] hatch_rate defaulted to 8 (number of CPUs)
06:56:38 [ INFO] launching WebsiteTasks client 1...
06:56:38 [ INFO] launching WebsiteTasks client 2...
06:56:39 [ INFO] launching WebsiteTasks client 3...
06:56:39 [ INFO] launching WebsiteTasks client 4...
06:56:39 [ INFO] launching WebsiteTasks client 5...
06:56:39 [ INFO] launching WebsiteTasks client 6...
06:56:39 [ INFO] launching WebsiteTasks client 7...
06:56:39 [ INFO] launching WebsiteTasks client 8...
06:56:39 [ INFO] launched 8 clients...
07:26:40 [ INFO] exiting after 1800 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /                   | 7,585,519      | 0 (0.0%)       | 4,214  | 0    
 GET /story.html         | 11,377,237     | 0 (0.0%)       | 6,320  | 0    
 GET /about.html         | 3,792,596      | 0 (0.0%)       | 2,106  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 22,755,352     | 0 (0.0%)       | 12,641 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /                   | 0.06       | 0.01       | 609.18     | 0.05      
 GET /story.html         | 0.06       | 0.01       | 609.11     | 0.05      
 GET /about.html         | 0.06       | 0.01       | 609.08     | 0.05      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.06       | 0.01       | 609.18     | 0.03      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /                   | 0.05   | 0.08   | 0.16   | 0.19   | 0.31   |   0.58
 GET /story.html         | 0.05   | 0.07   | 0.15   | 0.17   | 0.28   |   0.59
 GET /about.html         | 0.05   | 0.07   | 0.15   | 0.17   | 0.28   |   0.62
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05   | 0.07   | 0.16   | 0.18   | 0.29   |   0.59
-------------------------------------------------------------------------------
 Name                    | Status codes              
 ----------------------------------------------------------------------------- 
 GET /                   | 7,585,519 [200]          
 GET /story.html         | 11,377,237 [200]         
 GET /about.html         | 3,792,596 [200]          
```

## Roadmap

The Goose project roadmap is documented in [TODO.md](https://github.com/jeremyandrews/goose/blob/master/TODO.md).
