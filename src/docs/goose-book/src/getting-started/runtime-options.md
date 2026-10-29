# Run-Time Options

Every Goose load test takes the run-time options below. Three of them need a crate feature: `--pdf-scale` and `--pdf-timeout` exist only when Goose is built with `pdf-reports`, and `--dashboard` is refused unless Goose is built with `dashboard`. `-h` prints a summary of each one, and `--help` prints the full reference: what each option does, its default, the values it takes, the options it works with or excludes, an example, and the `GooseDefault` that sets it in code, where they apply. To see the summary for your own load test, run `cargo run --release -- -h` in its folder, or `cargo run --release --example simple -- -h` in a checkout of Goose.

The help on this page is from Goose 0.19.0. When you run `-h` yourself, its first line names the version of Goose your load test was built with, and `-V` (`--version`) prints the library's name and version, such as `goose 0.19.0`. The output below comes from a build with the `pdf-reports` feature:

```text
{{#include runtime-options-short.txt}}
```

Help wraps to the terminal's width, up to 120 columns. When the output is not a terminal, it wraps to `COLUMNS` if that is set, also up to 120, and to 80 columns otherwise. Both outputs on this page are the help at 80 columns.

## Where each option is explained

The options a load test needs first are on [Common Options](common.md). Every other option is explained on the page linked beside it, or here in the table.

| Options | Explained |
|---------|-----------|
| `-H`/`--host` | [Common Options](common.md#host-to-load-test) |
| `-u`/`--users` | [Common Options](common.md#how-many-users-to-simulate) |
| `-t`/`--run-time`, `--iterations` | [Common Options](common.md#specifying-how-long-the-load-test-will-run) |
| `-r`/`--increase-rate`, `-s`/`--increase-time` | [Common Options](common.md#controlling-how-long-it-takes-goose-to-launch-all-users) |
| `--decrease-rate`, `--decrease-time` | [Common Options](common.md#controlling-the-ramp-down-rate) |
| `--test-plan` | [Test Plan](test-plan.md) |
| `--scenarios`, `--scenarios-list` | [Limiting Scenarios](scenarios.md) |
| `--throttle-requests` | [Throttle](throttle.md) |
| `-h`/`--help` | `-h` prints the summary and `--help` the full reference, both shown on this page. |
| `-V`/`--version` | Prints the Goose library's name and version, then exits. |
| `-l`/`--list` | Prints each scenario by its display name, with its transactions and their weights, then exits without load testing; `--scenarios-list` prints the names `--scenarios` takes. |
| `-v`/`--verbose`, `-q`/`--quiet` | Without either, the console shows info, warning and error messages while the load test runs. `-v` adds debug messages and `-vv` trace messages; `-q` leaves out the info messages. |
| `-G`/`--goose-log` | Writes Goose's own log of the run to a file, or into the run folder when given no path; [Output Files](output-files.md#files) names the file. |
| `-g`/`--log-level` | Adds info messages to the Goose log that `--goose-log` writes, debug messages with `-gg` and trace messages with `-ggg`. Without `-g`, that log holds only warnings and errors. |
| `--running-metrics` | Prints the metrics so far to the console every N seconds, and during the ramp up also each time a user starts. The metrics at the end are printed too. With `--no-print-metrics`, neither is printed. |
| `--no-print-metrics` | [Common Options](common.md#console-output) |
| `--no-error-summary` | Leaves the errors and transaction errors tables out of the console metrics and the reports. The [error log](../logging/errors.md) still records every error. |
| `--report-file`, `--pdf-print-html`, `--pdf-scale`, `--pdf-timeout` | [Common Options](common.md#writing-an-html-formatted-report) |
| `--baseline-file` | [Common Options](common.md#comparing-against-a-baseline) |
| `--output-dir`, `--notes` | [Common Options](common.md#run-folders), then [Output Files](output-files.md#files) |
| `--report-graph-data`, `--render-report` | [Common Options](common.md#graph-data-and-memory), then [Output Files](output-files.md) |
| `--compress` | [Output Files](output-files.md#compression) |
| `--no-granular-report` | [Metrics](metrics.md#requests) |
| `--no-reset-metrics` | Keeps the metrics from the ramp up, which Goose otherwise clears when the ramp up ends. A test plan or `--iterations` run never clears them, and the option is refused with either. |
| `--no-metrics` | Collects no metrics, so Goose writes no report. It cannot be used with `--report-file`, `--pdf-print-html`, `--threshold`, `--co-mitigation`, or the request, transaction, scenario and error logs. |
| `--no-transaction-metrics`, `--no-scenario-metrics` | `--no-transaction-metrics` leaves each transaction's metrics out of the console metrics and the reports, and the transaction log stays empty. `--no-scenario-metrics` does the same for scenarios and the scenario log. |
| `--no-status-codes` | Leaves the count of each HTTP status code out of the console metrics and the reports. |
| `--co-mitigation` | [Mitigation](../coordinated-omission/mitigation.md) |
| `--threshold`, `--no-threshold-abort`, `--threshold-abort-delay` | [Thresholds](thresholds.md) |
| `-R`/`--request-log`, `--request-format`, `--request-body` | [Request Log](../logging/requests.md) |
| `-T`/`--transaction-log`, `--transaction-format` | [Transaction Log](../logging/transactions.md) |
| `-S`/`--scenario-log`, `--scenario-format` | [Scenario Log](../logging/scenarios.md) |
| `-E`/`--error-log`, `--error-format` | [Error Log](../logging/errors.md) |
| `-D`/`--debug-log`, `--debug-format`, `--no-debug-body` | [Debug Log](../logging/debug.md) |
| `--dashboard` | [Common Options](common.md#watching-a-run), then [Live Dashboard](../controller/dashboard.md) |
| `--dashboard-host`, `--dashboard-port`, `--dashboard-auth-token`, `--dashboard-control`, `--dashboard-no-auth`, `--dashboard-max-clients`, `--no-percentiles-over-time` | [Dashboard Reference](../controller/dashboard-reference.md#related-flags) |
| `--no-autostart` | [Controllers](../controller/overview.md) |
| `--no-telnet`, `--telnet-host`, `--telnet-port` | [Telnet Controller](../controller/telnet.md) |
| `--websocket`, `--websocket-host`, `--websocket-port` | [WebSocket Controller](../controller/websocket.md) |
| `--timeout` | [Tips](tips.md#timeouts) |
| `--no-gzip` | By default requests ask for gzip compressed responses, which Goose decompresses; `--no-gzip` stops asking. |
| `--sticky-follow` | When a request is redirected to another host, the GooseUser sends its later requests to that host too. |
| `--accept-invalid-certs` | Skips TLS certificate validation, for test servers with self-signed or expired certificates. |

`--no-websocket` is not in the help: the WebSocket Controller is off unless `--websocket` turns it on, and a command line from Goose 0.18 that still passes `--no-websocket` runs with a warning. [Upgrading From 0.18](upgrading.md) lists every option that was removed or now behaves differently.

## Full reference

`cargo run --release -- --help` prints:

```text
{{#include runtime-options-long.txt}}
```

The developer documentation describes [`GooseConfiguration`](https://docs.rs/goose/*/goose/config/struct.GooseConfiguration.html), the structure that holds these options.
