# Goose

Have you ever been attacked by a goose?

## Overview

Goose begins as a minimal port of [Locust](https://locust.io/) to Rust.

Define user behaviour with Rust code.

The MVP is support for the following subset of commands:

```
$ cargo run -- -h
    Finished dev [unoptimized + debuginfo] target(s) in 0.59s
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
    -c, --clients <clients>              Rust module file to import, e.g. '../other.rs'. Number of concurrent Goose
                                         users [default: 1]
    -r, --hatch-rate <hatch-rate>        The rate per second in which clients are spawned [default: 1]
    -H, --host <host>                    Host to load test in the following format: http://10.21.32.33 [default: ]
        --log-file <log-file>             [default: goose.log]
    -t, --run-time <run-time>            Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.)
                                         [default: ]
    -s, --stop-timeout <stop-timeout>    Number of seconds to wait for a simulated user to complete any executing task
                                         before existing. Default is to terminate immediately [default: 0]
```

Once the above is complete, additional planned features include:
 - gaggle support for distributed load testing
 - a web UI for controlling and monitoring Goose
