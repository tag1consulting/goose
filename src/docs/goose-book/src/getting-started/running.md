# Running A Load Test

Run the load test from [Creating A Load Test](creating.md) with Cargo. Pass the `--release` option whenever you compile or run a load test: a debug build is much slower and can't generate as much load.

```bash
$ cargo run --release
    Finished `release` profile [optimized] target(s) in 0.07s
     Running `target/release/loadtest`
17:13:37 [INFO] Output verbosity level: INFO
17:13:37 [INFO] Logfile verbosity level: WARN
17:13:37 [INFO] users defaulted to number of CPUs = 18
Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the Scenario.set_host() function (no host defined for LoadtestTransactions)." }
```

The load test fails because it has not been told which host to load test.

Run it again, this time passing the `--host` option with the address of a site you run or are allowed to load test. The `--` passes the options after it to the load test rather than to Cargo. Without `-u`, Goose starts one user for each CPU; without `-t`, it runs until you stop it. This load test sets no wait time, so each user sends its next request as soon as the last one is answered. Against your own site, start small (see [How many users to simulate](common.md#how-many-users-to-simulate) and [Specifying how long the load test will run](common.md#specifying-how-long-the-load-test-will-run)):

```bash
$ cargo run --release -- --host https://your-site.example -u 2 -t 30s --report-file=report.html --no-reset-metrics
```

`--report-file` with a `.html` file extension [writes an HTML report](common.md#writing-an-html-formatted-report), and `--no-reset-metrics` keeps the ramp up in that report (explained below). Stop a run without `-t` with one Ctrl-C. The run below is the same command against a test server on port 19060 of this machine, without `-u` and `-t`. It ran until Ctrl-C was pressed once, about 8 seconds after `entering GooseAttack phase: Maintain`:

```bash
$ cargo run --release -- --host http://localhost:19060 --report-file=report.html --no-reset-metrics
    Finished `release` profile [optimized] target(s) in 0.06s
     Running `target/release/loadtest --host 'http://localhost:19060' --report-file=report.html --no-reset-metrics`
17:13:38 [INFO] Output verbosity level: INFO
17:13:38 [INFO] Logfile verbosity level: WARN
17:13:38 [INFO] users defaulted to number of CPUs = 18
17:13:38 [INFO] no_reset_metrics = true
17:13:38 [INFO] report_file = ["report.html"]
17:13:38 [INFO] global host configured: http://localhost:19060
17:13:38 [INFO] [scheduler]: allocating transactions and scenarios with RoundRobin scheduler
17:13:38 [INFO] [scheduler]: initializing 18 user states...
17:13:38 [INFO] [controller]: Telnet listening on: 127.0.0.1:5116
17:13:38 [INFO] entering GooseAttack phase: Increase
17:13:38 [INFO] [user 1]: launching user from LoadtestTransactions
17:13:39 [INFO] [user 2]: launching user from LoadtestTransactions
17:13:40 [INFO] [user 3]: launching user from LoadtestTransactions
...
17:13:54 [INFO] [user 17]: launching user from LoadtestTransactions
17:13:55 [INFO] [user 18]: launching user from LoadtestTransactions
All 18 users launched.

17:13:56 [INFO] entering GooseAttack phase: Maintain
17:14:04 [WARN] caught ctrl-c, stopping...
17:14:04 [INFO] Killswitch triggered: SIGINT received
17:14:05 [INFO] entering GooseAttack phase: Decrease
17:14:05 [INFO] [user 14]: exiting from LoadtestTransactions
17:14:05 [INFO] [user 5]: exiting from LoadtestTransactions
...
17:14:05 [INFO] html report file written to: report.html
17:14:05 [INFO] entering GooseAttack phase: Shutdown
17:14:05 [INFO] printing final metrics after 27 seconds...
```

All `INFO` and higher-level log messages are displayed on the console while the load test runs. The `entering GooseAttack phase:` lines name each [phase](../glossary.md#attack-phase) the load test enters. `-q` (`--quiet`) hides the `INFO` lines and leaves warnings and errors, and `-v` (`--verbose`) adds low-level debug messages. In Goose 0.19, `-v` against a fast server can hang the run at the metrics reset; add `--no-reset-metrics` with it, and see [When a run never ends](tips.md#when-a-run-never-ends).

Goose launches one user a second, so this machine's 18 users took 18 seconds to ramp up; the overview below shows this as the Increasing step. With no wait time, this run's console metrics, not shown here, report over 9,000 requests a second against the local test server.

By default, once every user has started, Goose prints the metrics so far, [resets them](../glossary.md#metrics-reset), and prints `All 18 users launched, resetting metrics (disable with --no-reset-metrics).` The results then describe the run at full load. `--no-reset-metrics`, used here, skips that early print of the metrics and the reset, and Goose prints only `All 18 users launched.` The tables and graphs in `report.html` then include the ramp up; the [User Metrics](metrics.md#users) graph shows the ramp up with or without the reset.

## What the run produced

Once every user has exited, Goose writes `report.html` in the directory you ran it from; open it in a browser. Then Goose enters Shutdown and prints its final console metrics, which end with the [plan overview](../glossary.md#plan-overview). The overview lists each step of the run, when it started and stopped, how long it took, and how many users were running:

```text
 === OVERVIEW ===
 ------------------------------------------------------------------------------
 Action       Started               Stopped             Elapsed    Users
 ------------------------------------------------------------------------------
 Increasing:  2026-10-28 18:13:38 - 2026-10-28 18:13:56 (00:00:18, 0 -> 18)
 Maintaining: 2026-10-28 18:13:56 - 2026-10-28 18:14:05 (00:00:09, 18)
 Canceling:   2026-10-28 18:14:05 - 2026-10-28 18:14:05 (00:00:00, 0 <- 18)

 Target host: http://localhost:19060/
 goose v0.19.0-dev
 ------------------------------------------------------------------------------
```

Because Ctrl-C stopped this run, the overview lists its Decrease phase as Canceling; a run that ends at its `-t` time lists it as Decreasing. The log lines in the run above carry UTC times; the plan overview uses local time, an hour ahead on this machine.

To watch a run live, change the `goose` line in `Cargo.toml` to `goose = { version = "^0.19", features = ["dashboard"] }` and run with `--dashboard` (see [Watching a run](common.md#watching-a-run)). Give `--report-file` once for each report you want; its extension sets the type (see [Writing reports](common.md#writing-an-html-formatted-report)). A `.pdf` report also needs the `pdf-reports` feature, as in `features = ["dashboard", "pdf-reports"]`, and Chrome, which Goose downloads if it finds none (see [Requirements](../requirements.md#chrome-for-pdf-reports)).

[Metrics](metrics.md) shows every view of a run and when to use each, from the live dashboard through the PDF, HTML, Markdown and JSON reports to the console metrics. Its [HTML report](metrics.md#html-report) section walks through the report you just opened, and its [console metrics](metrics.md#ascii-metrics) section explains the tables printed before the plan overview.

The [examples](../example/overview.md) included with Goose show larger load tests.
