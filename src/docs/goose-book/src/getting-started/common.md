# Common Run Time Options

As seen on the previous page, Goose has a lot of run time options which can be overwhelming. The following are a few of the more common and more important options to be familiar with. In these examples we only demonstrate one option at a time, but it's generally useful to combine many options.

## Verbose Output

By default Goose is not very verbose, and only outputs metrics at the end of a load test. It can be preferable to get more insight into what's going by enabling the `-v` flag to increase verbosity.

### Example
_Enable verbose output while running load test._

```bash
cargo run --release -- -v
```

## Host to load test

Load test plans typically contain relative paths, and so Goose must be told which host to run the load test against in order for it to start. This allows a single load test plan to be used for testing different environments, for example "http://local.example.com", "https://qa.example.com", and "https://www.example.com".

### Example
_Load test the https://www.example.com domain._

```bash
cargo run --release -- -H https://www.example.com
```

## How many users to simulate

By default, Goose will launch one user per available CPU core. Often you will want to simulate considerably more users than this, and this can be done by setting the "--user" run time option.

### Example
_Launch 1,000 GooseUsers._

```bash
cargo run --release -- -u 1000
```

## Controlling how long it takes Goose to launch all users

There are two ways to configure how long Goose will take to launch all configured GooseUsers. You can user either `--hatch-rate` or `--startup-time`, but not both together.

### Specifying the hatch rate

By default, Goose starts one GooseUser per second. So if you configure `--users` to 10 it will take ten seconds to fully start the load test. If you set `--hatch-rate 5` then Goose will start 5 users every second, taking two seconds to start up. If you set `--hatch-rate 0.5` then Goose will start 1 user every 2 seconds, taking twenty seconds to start all 10 users.

### Example
_Launch one user every two seconds._

```bash
cargo run --release -- -r .5
```

### Specifying the total startup time

Alternatively, you can tell Goose how long you'd like it to take to start all GooseUsers. So, if you configure `--users` to 10 and set `--startup-time 10` it will launch 1 user every second. If you set `--start-time 1m` it will start 1 user every 6 seconds, starting all users over one minute. And if you set `--start-time 2s` it will launch five users per second, launching all users in two seconds.

### Example
_Launch all users in 5 seconds._

```bash
cargo run --release -- -s 5
```

## Specifying how long the load test will run

The `--run-time` option is not affected by how long Goose takes to start up. Thus, if you configure a load test with `--users 100 --start-time 30m --run-time 5m` Goose will run for a total of 35 minutes, first ramping up for 30 minutes and then running at full load for 5 minutes. If you want Goose to exit immediately after all users start, you can set a very small run time, for example `--users 100 --hatch-rate .25 --run-time 1s`.

If you do not configure a run time, Goose will run until it's canceled with `ctrl-c`.

### Example
_Run the load test for 30 minutes._

```bash
cargo run --release -- -t 30m
```

## Writing An HTML-formatted Report

By default, Goose displays [text-formatted metrics](metrics.md) when a load test finishes. It can also optionally write an HTML-formatted report if you enable the `--report-file <NAME>` run-time option, where `<NAME>` is an absolute or relative path to the report file to generate. Any file that already exists at the specified path will be overwritten.

### Example
_Write an HTML-formatted report to `report.html` when the load test finishes._

```bash
cargo run --release -- --report-file report.html
```

