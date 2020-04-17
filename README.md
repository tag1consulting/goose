# Goose

Have you ever been attacked by a goose?

## Overview

Goose is a Rust load testing tool based on [Locust](https://locust.io/).
User behavior is defined with standard Rust code.

Goose load tests are built using Cargo to build a new application with a
dependency on the Goose library.

Passing the included `simple` example the `-h` flag you can see the
run-time configuration options available to Goose load tests:

```
$ cargo run --release --example simple -- -h
    Finished release [optimized] target(s) in 0.05s
     Running `target/release/examples/simple -h`
client 0.4.0
Configuration options required for launching a Goose loadtest

USAGE:
    simple [FLAGS] [OPTIONS]

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
    -c, --clients <clients>          Number of concurrent Goose users (defaults to available CPUs)
    -r, --hatch-rate <hatch-rate>    How many users to spawn per second (defaults to available CPUs)
    -H, --host <host>                Host to load test in the following format: http://10.21.32.33 [default: ]
        --log-file <log-file>         [default: goose.log]
    -t, --run-time <run-time>        Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.)
                                     [default: ]
```

## Examples

### Simple

The `examples/simple.rs` example copies the simple load test documented on the locust.io web page, rewritten in Goose. It uses minimal advanced functionality, but demonstrates how to GET and POST pages. It defines a single Task Set which has the client log in and then load a couple of pages.

Goose can make use of all available CPU cores. By default, it will launch 1 client per core, and it can be configured to launch many more. The following was configured instead to launch 1,024 clients. Each client randomly pauses 5 to 15 seconds after each task is loaded, so it's possible to spin up a large number of clients. Here is a snapshot of `top` on an 8-core VM that ran this example with 10G of available RAM -- there were ample resources to launch considerably more "clients", though `ulimit` had to be reasied:

```
top - 11:14:57 up 16 days,  4:40,  2 users,  load average: 0.00, 0.04, 0.01
Tasks: 129 total,   1 running, 128 sleeping,   0 stopped,   0 zombie
%Cpu(s):  0.3 us,  0.3 sy,  0.0 ni, 99.2 id,  0.0 wa,  0.0 hi,  0.2 si,  0.0 st
MiB Mem :   9993.6 total,   6695.1 free,   1269.3 used,   2029.2 buff/cache
MiB Swap:  10237.0 total,  10234.7 free,      2.3 used.   8401.5 avail Mem 

  PID USER      PR  NI    VIRT    RES    SHR S  %CPU  %MEM     TIME+ COMMAND                                                   
19776 goose     20   0    9.8g 874688   8252 S   6.3   8.5   0:42.90 simple                                                    
```

Here's the output of running the loadtest. The `-v` flag sends `INFO` and higher messages to stdout (in addition to the log file). The `c1024` tells Goose to spin up 1,024 clients. The `-r32` option tells Goose to spin up 32 clients per second. The `-t 10m` option tells Goose to run the load test for 10 minutes, or 600 seconds. The `--print-stats` flag tells Goose to collect statistics during the load test, and the `--status-codes` flag tells it to include statistics about HTTP Status codes returned by the server. Finally, the `--only-summary` flag tells Goose to only display the statistics when the load test finishes, otherwise it would display running statistics every 15 seconds for the duration of the test.

```
$ cargo run --release --example simple -- --host http://apache.fosciana -v -c1024 -r32 -t 10m --print-stats --status-codes --only-summary
    Finished release [optimized] target(s) in 0.05s
     Running `target/release/examples/simple --host 'http://apache.fosciana' -v -c1024 -r32 -t 10m --print-stats --status-codes --only-summary`
15:05:36 [ INFO] Output verbosity level: INFO
15:05:36 [ INFO] Logfile verbosity level: INFO
15:05:36 [ INFO] Writing to log file: goose.log
15:05:36 [ INFO] run_time = 600
15:05:36 [ INFO] global host configured: http://apache.fosciana
15:05:42 [ INFO] launching client 1 from WebsiteUser...
15:05:42 [ INFO] launching client 2 from WebsiteUser...
15:05:42 [ INFO] launching client 3 from WebsiteUser...
15:05:42 [ INFO] launching client 4 from WebsiteUser...
15:05:42 [ INFO] launching client 5 from WebsiteUser...
15:05:42 [ INFO] launching client 6 from WebsiteUser...
15:05:42 [ INFO] launching client 7 from WebsiteUser...
15:05:42 [ INFO] launching client 8 from WebsiteUser...
```
...
```
15:06:14 [ INFO] launching client 1022 from WebsiteUser...
15:06:14 [ INFO] launching client 1023 from WebsiteUser...
15:06:14 [ INFO] launching client 1024 from WebsiteUser...
15:06:14 [ INFO] launched 1024 clients...
15:16:14 [ INFO] stopping after 600 seconds...
------------------------------------------------------------------------------ 
 Name                    | # reqs         | # fails        | req/s  | fail/s
 ----------------------------------------------------------------------------- 
 GET /about/             | 22,793         | 288 (1.3%)     | 35     | 0    
 POST /login             | 22,776         | 320 (1.4%)     | 35     | 0    
 GET /                   | 22,788         | 244 (1.1%)     | 35     | 0    
 ------------------------+----------------+----------------+-------+---------- 
 Aggregated              | 68,357         | 852 (1.2%)     | 106    | 1    
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Mean      
 ----------------------------------------------------------------------------- 
 GET /about/             | 8.42       | 0.01       | 603.77     | 0.08      
 POST /login             | 9.36       | 0.01       | 607.22     | 0.19      
 GET /                   | 8.47       | 0.01       | 604.42     | 0.09      
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 8.75       | 0.01       | 607.22     | 0.19      
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 ----------------------------------------------------------------------------- 
 GET /about/             | 0.08   | 0.10   | 0.36   | 500.29 | 554.02 | 601.97
 POST /login             | 0.19   | 0.22   | 1.28   | 500.48 | 572.90 | 600.21
 GET /                   | 0.09   | 0.11   | 0.34   | 500.32 | 568.10 | 600.48
 ------------------------+------------+------------+------------+------------- 
 Aggregated              | 0.10   | 0.18   | 0.61   | 500.38 | 566.55 | 601.97
-------------------------------------------------------------------------------
 Name                    | Status codes              
 ----------------------------------------------------------------------------- 
 GET /about/             | 22,505 [200]             
 POST /login             | 16 [503], 22,456 [200]   
 GET /                   | 22,544 [200]             
-------------------------------------------------------------------------------
 Aggregated              | 67,505 [200], 16 [503]    
```

## Roadmap

The Goose project roadmap is documented in [TODO.md](https://github.com/jeremyandrews/goose/blob/master/TODO.md).
