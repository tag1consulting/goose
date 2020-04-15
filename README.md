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

This first example simulates 8 concurrent clients (Goose automatically allocates 1 client per available core), all loading pages as quickly as possible. In this example, Goose is running in an 8-core VM, and loading static pages from Varnish in front of Apache in another 8-core VM.

The `--host` flag is required to tell Goose which host to load all paths defined in `goosefile.rs` from. A single `-v` flag causes all `INFO` level and higher logs to be written to stdout, useful information that doesn't affect the performance of Goose. The test is configured to run for 30 minutes, or 1,800 seconds, with the `-t 30m` option. The `--print-stats` flag configures Goose to collect statistics, and to display them when the test completes or is canceled, while the `--only-summary` flag prevents Goose from showing running statistics every 15 seconds. Finally, the `--status-codes` flag configures Goose to also count and display the status codes returned per request.

```
$ cargo run --release -- --host http://apache.fosciana -v -t 30m --print-stats --status-codes --only-summary
   Compiling goose v0.2.0 (~/goose)
    Finished release [optimized] target(s) in 3.06s
     Running `target/release/goose --host 'http://apache.fosciana' -v -t 30m --print-stats --status-codes --only-summary`
21:43:55 [ INFO] Output verbosity level: INFO
21:43:55 [ INFO] Logfile verbosity level: INFO
21:43:55 [ INFO] Writing to log file: goose.log
21:43:55 [ INFO] run_time = 1800
21:43:55 [ INFO] concurrent clients defaulted to 8 (number of CPUs)
21:43:55 [ INFO] hatch_rate defaulted to 8 (number of CPUs)
21:43:55 [ INFO] launching client 1 from WebsiteTasks...
21:43:55 [ INFO] launching client 2 from WebsiteTasks...
21:43:55 [ INFO] launching client 3 from WebsiteTasks...
21:43:56 [ INFO] launching client 4 from WebsiteTasks...
21:43:56 [ INFO] launching client 5 from WebsiteTasks...
21:43:56 [ INFO] launching client 6 from WebsiteTasks...
21:43:56 [ INFO] launching client 7 from WebsiteTasks...
21:43:56 [ INFO] launching client 8 from WebsiteTasks...
21:43:56 [ INFO] launched 8 clients...
22:13:57 [ INFO] exiting after 1800 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /story.html         | 12,197,526     | 0 (0.0%)       | 6,776  | 0    
 GET /about.html         | 4,066,484      | 0 (0.0%)       | 2,259  | 0    
 GET /                   | 8,134,428      | 0 (0.0%)       | 4,519  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 24,398,438     | 0 (0.0%)       | 13,554 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /story.html         | 0.06       | 0.01       | 602.20     | 0.05      
 GET /about.html         | 0.06       | 0.01       | 602.18     | 0.05      
 GET /                   | 0.06       | 0.01       | 602.16     | 0.05      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.06       | 0.01       | 602.20     | 0.01      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /story.html         | 0.05   | 0.07   | 0.15   | 0.17   | 0.27   |   0.55
 GET /about.html         | 0.05   | 0.07   | 0.15   | 0.17   | 0.27   |   0.55
 GET /                   | 0.05   | 0.07   | 0.16   | 0.19   | 0.31   |   0.56
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05   | 0.07   | 0.15   | 0.18   | 0.29   |   0.55
-------------------------------------------------------------------------------
 Name                    | Status codes              
 ----------------------------------------------------------------------------- 
 GET /story.html         | 12,197,526 [200]         
 GET /about.html         | 4,066,484 [200]          
 GET /                   | 8,134,428 [200]          
-------------------------------------------------------------------------------
 Aggregated              | 24,398,438 [200]          
```

This next example spins up 12 clients (`-c12`) over 1.5 seconds and then resets all statistics so it only collects statistics when all clients are up and running (`--reset-stats`). It runs for 45 seconds (`-t45`), and by default displays statistics as it goes:

```
$ cargo run --release -- --host http://apache.fosciana -v --print-stats -t45 -c12 --reset-stats
    Finished release [optimized] target(s) in 0.04s
     Running `target/release/goose --host 'http://apache.fosciana' -v --print-stats -t45 -c12 --reset-stats`
04:42:54 [ INFO] Output verbosity level: INFO
04:42:54 [ INFO] Logfile verbosity level: INFO
04:42:54 [ INFO] Writing to log file: goose.log
04:42:54 [ INFO] run_time = 45
04:42:54 [ INFO] hatch_rate defaulted to 8 (number of CPUs)
04:42:54 [ INFO] launching client 1 from WebsiteTasks...
04:42:54 [ INFO] launching client 2 from WebsiteTasks...
04:42:55 [ INFO] launching client 3 from WebsiteTasks...
04:42:55 [ INFO] launching client 4 from WebsiteTasks...
04:42:55 [ INFO] launching client 5 from WebsiteTasks...
04:42:55 [ INFO] launching client 6 from WebsiteTasks...
04:42:55 [ INFO] launching client 7 from WebsiteTasks...
04:42:55 [ INFO] launching client 8 from WebsiteTasks...
04:42:55 [ INFO] launching client 9 from WebsiteTasks...
04:42:55 [ INFO] launching client 10 from WebsiteTasks...
04:42:56 [ INFO] launching client 11 from WebsiteTasks...
04:42:56 [ INFO] launching client 12 from WebsiteTasks...
04:42:56 [ INFO] launched 12 clients...
04:42:56 [ INFO] statistics reset...
04:43:11 [ INFO] printing running statistics after 15 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /story.html         | 8,668          | 0 (0.0%)       | 577    | 0    
 GET /about.html         | 2,909          | 0 (0.0%)       | 193    | 0    
 GET /                   | 5,893          | 0 (0.0%)       | 392    | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 17,470         | 0 (0.0%)       | 1,164  | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /story.html         | 0.05       | 0.01       | 0.36       | 0.04      
 GET /about.html         | 0.05       | 0.01       | 0.25       | 0.04      
 GET /                   | 0.05       | 0.02       | 0.44       | 0.04      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05       | 0.01       | 0.44       | 0.01      

04:43:26 [ INFO] printing running statistics after 30 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /story.html         | 162,195        | 0 (0.0%)       | 5,406  | 0    
 GET /about.html         | 54,279         | 0 (0.0%)       | 1,809  | 0    
 GET /                   | 108,445        | 0 (0.0%)       | 3,614  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 324,919        | 0 (0.0%)       | 10,830 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /story.html         | 0.05       | 0.01       | 1.24       | 0.05      
 GET /about.html         | 0.05       | 0.01       | 0.75       | 0.05      
 GET /                   | 0.06       | 0.02       | 3.33       | 0.04      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.06       | 0.01       | 3.33       | 0.01      

04:43:41 [ INFO] exiting after 45 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /                   | 320,240        | 0 (0.0%)       | 7,116  | 0    
 GET /story.html         | 479,804        | 0 (0.0%)       | 10,662 | 0    
 GET /about.html         | 160,077        | 0 (0.0%)       | 3,557  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 960,121        | 0 (0.0%)       | 21,336 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /                   | 0.06       | 0.02       | 3.33       | 0.04      
 GET /story.html         | 0.05       | 0.01       | 1.49       | 0.05      
 GET /about.html         | 0.05       | 0.01       | 1.47       | 0.05      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.06       | 0.01       | 3.33       | 0.03      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /                   | 0.04   | 0.07   | 0.18   | 0.22   | 0.38   |   0.63
 GET /story.html         | 0.05   | 0.07   | 0.16   | 0.19   | 0.33   |   0.57
 GET /about.html         | 0.05   | 0.07   | 0.16   | 0.20   | 0.32   |   0.59
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05   | 0.07   | 0.17   | 0.20   | 0.35   |   0.60
```

It's also possible to simply run Goose until you cancel it, by not specifying a run time. In this final example, Goose only exits when we press `ctrl-c`, still displaying statistics on exit:

```
$ cargo run --release -- --host http://apache.fosciana --print-stats --only-summary
    Finished release [optimized] target(s) in 0.04s
     Running `target/release/goose --host 'http://apache.fosciana' --print-stats --only-summary`
^Ccaught ctrl-c, exiting...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /                   | 169,100        | 0 (0.0%)       | 4,973  | 0    
 GET /about.html         | 84,865         | 0 (0.0%)       | 2,496  | 0    
 GET /story.html         | 253,336        | 0 (0.0%)       | 7,451  | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 507,301        | 0 (0.0%)       | 14,920 | 0    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /                   | 0.06       | 0.01       | 1.02       | 0.05      
 GET /about.html         | 0.05       | 0.01       | 1.00       | 0.05      
 GET /story.html         | 0.05       | 0.01       | 1.24       | 0.05      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05       | 0.01       | 1.24       | 0.20      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /                   | 0.05   | 0.07   | 0.16   | 0.19   | 0.31   |   0.56
 GET /about.html         | 0.05   | 0.07   | 0.14   | 0.17   | 0.26   |   0.57
 GET /story.html         | 0.05   | 0.07   | 0.14   | 0.17   | 0.26   |   0.58
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.05   | 0.07   | 0.15   | 0.17   | 0.28   |   0.57
```

## Roadmap

The Goose project roadmap is documented in [TODO.md](https://github.com/jeremyandrews/goose/blob/master/TODO.md).
