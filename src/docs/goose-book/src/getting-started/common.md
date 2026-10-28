# Common Run Time Options

These are the options you need in your first week of load testing: where to send the load, how many users, how fast they start, how long to run, how to watch the run, and which reports to keep. Combine them as you need. [Run-Time Options](runtime-options.md) lists every option and the page that explains it.

Each example other than the host example also needs `--host`, unless the load test sets a host itself with `GooseDefault::Host` or `Scenario::set_host`. Options that take a time, such as `--run-time` and `--increase-time`, take a whole number of seconds, or hours, minutes and seconds in that order, such as `1h30m` or `2m30s`.

## Host to load test

A load test's requests usually name only a path, such as `/about`, so Goose must be told which host to send them to. The same load test can then run against `http://local.example.com`, `https://qa.example.com` or `https://www.example.com`.

### Host example
_Load test the `https://www.example.com` domain._

```bash
cargo run --release -- -H https://www.example.com
```

## How many users to simulate

By default, Goose will launch one user per available CPU core. Often you will want to simulate considerably more users than this, and this can be done by setting the `--users` run time option.

(_Alternatively, you can use [`--test-plan`](test-plan.md) to build both simple and more complex traffic patterns that can include a varying number of users._)

### Users example
_Launch 1,000 GooseUsers._

```bash
cargo run --release -- -u 1000
```

At the default rate of one user a second, starting 1,000 users takes over 16 minutes; the next section shows how to start them faster.

## Controlling how long it takes Goose to launch all users

Goose starts users at a rate, with `--increase-rate`, or over a time, with `--increase-time`, but not both together. (_These replace `--hatch-rate` and `--startup-time` from Goose 0.18 and earlier, which no longer exist; see [Upgrading From 0.18](upgrading.md)._)

Both are best effort limits: Goose will not start users faster than you configure, but the machine running Goose may not be able to start them that fast.

### Specifying the increase rate

By default, Goose starts one GooseUser per second. So if you configure `--users` to 10 it will take ten seconds to fully start the load test. If you set `--increase-rate 5` then Goose will start 5 users every second, taking two seconds to start up. If you set `--increase-rate 0.5` then Goose will start 1 user every 2 seconds, taking twenty seconds to start all 10 users. The slowest rate Goose accepts is one user a day, about `0.0000116`; a slower rate is an error.

Goose does not round a rate that is not a whole number: `--increase-rate 1.5` starts a user every 667 milliseconds, so the increase phase of 9 users lasts 6 seconds. `--decrease-rate`, `--increase-time`, `--decrease-time` and the steps of a [test plan](test-plan.md) are not rounded either.

### Increase rate example
_Launch one user every two seconds._

```bash
cargo run --release -- -r .5
```

### Specifying the total increase time

Alternatively, you can tell Goose how long you would like it to take to start all GooseUsers. So, if you configure `--users` to 10 and set `--increase-time 10` it will launch 1 user every second. If you set `--increase-time 1m` it will start 1 user every 6 seconds, starting all users over one minute. And if you set `--increase-time 2s` it will launch five users per second, launching all users in two seconds.

### Increase time example
_Launch all users in 5 seconds._

```bash
cargo run --release -- -s 5
```

## Specifying how long the load test will run

The `--run-time` option is not affected by how long Goose takes to start up. Thus, if you configure a load test with `--users 100 --increase-time 30m --run-time 5m` Goose will run for a total of 35 minutes, first ramping up for 30 minutes and then running at full load for 5 minutes. If you want Goose to exit immediately after all users start, you can set a very small run time, for example `--users 100 --increase-rate .25 --run-time 1s`.

Instead of a run time, `--iterations` sets how many times each GooseUser runs through its scenario before it stops, and a [`--test-plan`](test-plan.md) sets how long each of its steps lasts.

Goose resets its metrics when the ramp up ends. Its reports then leave the ramp up out of the totals and out of every graph but the active users graph. `--no-reset-metrics` keeps the ramp up in. A run limited by `--iterations` or shaped by a test plan never resets, and refuses `--no-reset-metrics`.

If you do not configure a run time, Goose runs until it is canceled with `ctrl-c`.

### Run time example
_Run the load test for 30 minutes._

```bash
cargo run --release -- -t 30m
```

### Iterations example
_Each GooseUser will take as long as it takes to fully run its assigned Scenario 5 times and then stop._

```bash
cargo run --release -- --iterations 5
```

## Watching a run

With the `dashboard` feature turned on in your load test's `Cargo.toml`, `--dashboard` serves a live web dashboard while the load test runs. Open `http://127.0.0.1:5118/` in a browser to see the users, requests per second, response times and errors as they change.

![The Goose live dashboard, read-only, just over two minutes into a load test of the Umami demo site: the header with the Maintain phase, the host and the time run; the Save report checkbox, checked; a notice that the request rows are truncated; the read-only Control card, which names the options that turn on control; the KPI row; and the requests per second and active users charts](common-dashboard.png)

*A read-only dashboard 2 minutes 14 seconds into a run of 50 users with `--no-reset-metrics`: the users chart climbs by one user a second to 50 and holds, and the requests chart climbs with it.*

By default the dashboard only watches: it cannot start, stop or change the test, and its Control card says how to turn control on. With **Save report** on, as it is by default, Goose writes the run's `report.html` and `report.json` into its [run folder](#run-folders) when the run ends. `--dashboard-control` lets the dashboard start and stop the test, change its users and settings, and, once the test is stopped, quit Goose. It also needs a token, set with `--dashboard-auth-token <TOKEN>` and given by opening `http://127.0.0.1:5118/?token=<TOKEN>`, or `--dashboard-no-auth`, which lets anyone who can reach the port control the test. [Live Dashboard](../controller/dashboard.md) tours every part of the page, including the truncation notice.

### Dashboard example
_Turn on the `dashboard` feature, then watch a load test in the browser._

```toml
goose = { version = "^0.19", features = ["dashboard"] }
```

```bash
cargo run --release -- --dashboard
```

## Writing reports {#writing-an-html-formatted-report}

`--report-file <FILE>` writes a report when the load test finishes. The file's extension sets its type: `.pdf` for a PDF report, when Goose is built with the `pdf-reports` feature; `.html` or `.htm` for HTML; `.md` for Markdown; and `.json` for JSON. Give `--report-file` once for each report you want. A `--report-file` given without a path writes `report.html` and `report.json` into the [run folder](#run-folders). A file that already exists at the path is overwritten. [Report Formats](metrics.md#report-formats) shows what each report holds. The HTML report is the easiest to start with.

### PDF report example
_Build with the `pdf-reports` feature and write a PDF report, waiting up to two minutes for Chrome._

```bash
cargo run --release --features goose/pdf-reports -- --report-file report.pdf --pdf-timeout 120
```

A PDF report needs Chrome or Chromium; see [Requirements](../requirements.md#chrome-for-pdf-reports). `--pdf-timeout` is how many seconds Goose waits for Chrome to start and to render the PDF, from 10 to 600, default 60. `--pdf-scale`, from 0.1 to 2.0, default 0.8, sets how much Chrome scales the pages. Both exist only with the `pdf-reports` feature.

### HTML report example
_Write an HTML report to `report.html` when the load test finishes._

```bash
cargo run --release -- --report-file report.html
```

![The Request Metrics graph of an HTML report: requests per second over a run of about three minutes, a line for all requests and one for each request, with the ramp up shaded](rps.png)

*The HTML report's requests per second graph, from a run of 50 users with `--no-reset-metrics`: the total climbs through the shaded ramp up and levels off once every user is running.*

### Printer-friendly HTML report example
_Write an HTML report laid out for printing to PDF._

```bash
cargo run --release -- --pdf-print-html print-report.html
```

`--pdf-print-html` writes the HTML report laid out for printing, to turn into a PDF with your browser's "Print to PDF" or to print on paper. It needs neither Chrome nor the `pdf-reports` feature, has the same graphs and response time heatmap as the HTML report, and cannot be used with `--no-metrics`. It can also be written beside a `--report-file`:

```bash
cargo run --release -- --report-file report.html --pdf-print-html print-report.html
```

### HTML and Markdown report example
_Write an HTML and a Markdown report when the load test finishes._

```bash
cargo run --release -- --report-file report.html --report-file report.md
```

### JSON report example
_Write a JSON report, which a later run can compare against with [`--baseline-file`](#comparing-against-a-baseline)._

```bash
cargo run --release -- --report-file report.json
```

### Console output

Goose prints [ASCII metrics](metrics.md#ascii-metrics) to the console when a load test finishes, whether or not it writes a report. `--no-print-metrics` leaves them out, and every requested report is still written.

## Run folders

An output flag given without a path, such as a bare `--report-file` or `--goose-log`, writes into the run folder: `goose-runs/<YYYYMMDD-HHMMSS>` from the run's local start time, unless `--output-dir` names one. A run watched with `--dashboard` saves its report there too. `--notes` labels the run in its reports and on the dashboard. [Output Files](output-files.md) gives the rules for every file Goose writes.

```bash
cargo run --release -- --report-file --notes "after the cache change"
```

When running Goose in Docker, mount a folder for the runs:

```bash
docker run -v "$PWD/goose-runs:/goose-runs" my-load-test --host https://example.com --output-dir /goose-runs/nightly --report-file
```

In CI, upload `goose-runs/` as a build artifact to keep every run's reports.

## Comparing Against A Baseline

To see what changed between two runs, such as before and after a deployment, give the later run an earlier run's JSON report with `--baseline-file`. A run folder that holds a `report.json` works too, such as the one a `--dashboard` run or a bare `--report-file` writes:

```bash
cargo run --release -- --report-file --baseline-file goose-runs/20261028-120502
```

The numeric metrics in the PDF, HTML and Markdown reports then include a delta showing the difference from the baseline. For example, instead of just showing `1,234` requests, the HTML report shows `1,234 (+200)` if the current run had 200 more requests than the baseline, or `1,234 (-50)` if it had 50 fewer; the Markdown report shows its request counts without thousands separators, `1234 (+200)`. The JSON report holds only the current run's values, so it can be the next run's baseline. The console metrics show no deltas.

Deltas are calculated for:

- **Requests**: number of requests, failures, average/min/max response time, requests/failures per second
- **Response time percentiles**: p50, p60, p70, p80, p90, p95, p99, p100
- **Transactions**: request count, failures, response times, rates
- **Scenarios**: user count, iteration count, response times, rates
- **Errors**: occurrence count
- **Transaction errors**: occurrence count
- **Coordinated Omission**: adjusted response times and standard deviation

Metrics are matched between runs by their key (HTTP method + endpoint name for requests, transaction + name for transactions, etc.). A request in the current run but not in the baseline is shown without a delta, and one only in the baseline is not shown.

The baseline must be a JSON report generated by Goose, plain or gzipped (`.json.gz`), or a run folder, whose `report.json` or `report.json.gz` is then used. If the file does not exist, contains invalid JSON, or has no request metrics, Goose will report an error. A load test can also set a default baseline in code with [`GooseDefault::BaselineFile`](../config/defaults.md), which `--baseline-file` overrides.

### Baseline comparison example
_Run a load test comparing against a previous run, generating both HTML and JSON reports._

```bash
cargo run --release -- --report-file report.html --report-file latest.json --baseline-file baseline.json
```

The resulting `latest.json` can then be used as the baseline for the next comparison, enabling a chain of run-over-run comparisons.

### Baselines from older Goose releases

A JSON report from Goose 0.18.1 or later loads as a baseline, including one from a run that received more than one status code. A baseline without transaction errors, such as one from Goose 0.18.1, still loads, and the run's transaction errors then show no delta.

A time's delta is the current time minus the baseline time, each as the report prints it. A 0.19 baseline's request, transaction and scenario times come from its microsecond timing data. A report from an older Goose holds whole milliseconds: its minimums and maximums are truncated, and its percentiles above 100 ms are rounded to 10 ms, 100 ms or whole seconds. So an unchanged time can read differently against it, such as `38 (+1)`.

## Controlling the ramp-down rate

You can configure how quickly Goose stops users at the end of a load test with `--decrease-rate` or `--decrease-time`. These are mutually exclusive, just like their increase counterparts, and `--decrease-rate` has the same slowest rate of one user a day. If neither is set, Goose stops all users at once.

### Decrease rate example
_Stop one user every two seconds._

```bash
cargo run --release -- --decrease-rate .5
```

### Decrease time example
_Spread the stops over 30 seconds._

```bash
cargo run --release -- --decrease-time 30s
```

Goose divides the time by the number of users and stops one user each interval, the first at once, so with 4 users it stops one every 7.5 seconds and the last 22.5 seconds in.

## Graph data and memory

Goose keeps per second graph data in memory until the run ends whenever it will write a report then: for a `--report-file` path, for `--pdf-print-html`, and while Save report is on, as it is with `--dashboard` or a bare `--report-file`. It holds every second of the run, the ramp up included. After the metrics reset, Goose keeps zeros in memory for the ramp up seconds of the request, error, response time, transaction and scenario data, and the report leaves those seconds out of its graphs. When Save report is turned on mid run, the data starts up to 10 minutes before that moment. The graph data costs about 12 bytes per request name per second, and 4 bytes more per second for each name that has failed. Goose holds it in buffers that grow by doubling, so it can take up to twice that.

Writing an HTML report costs far more while it is written: about 300 bytes more per request name for each second of the run. `--pdf-print-html` builds the same HTML and costs the same. A PDF report costs about three times as much in Goose, about 950 bytes per request name for each second of the run. Chrome needs memory of its own to print it, from about 400 MB for a small report to about 750 MB for a 20 minute run with 100 request names. A Markdown or JSON report adds almost nothing. With 20 and 100 request names and none failing, counting the doubling:

| run length | 20 names, graph data | 20 names, writing HTML | 100 names, graph data | 100 names, writing HTML |
|-|-|-|-|-|
| 10 minutes | 0.3 MB | 4 MB | 1.3 MB | 20 MB |
| 1 hour | 1.1 MB | 24 MB | 5 MB | 114 MB |
| 24 hours | 35 MB | 570 MB | 160 MB | 2.8 GB |
| 7 days | 280 MB | 4 GB | 1.3 GB | 20 GB |

The per second costs were measured on 10 to 35 minute runs of Goose 0.19, and every row is computed from them. The table leaves out the response time heatmap, which stops growing once its columns fill, after about 10 minutes. At 100 request names it takes about 5 MB when response times are fast and even, 10 MB when they are typical, and up to about 40 MB when they spread widely, or more with a `--timeout` over 60 seconds; `--no-percentiles-over-time` turns it off.

To keep the HTML cost out of a long soak test, write a JSON report with its graph data, `--report-file report.json --report-graph-data`, and turn it into an HTML report after the run with `--render-report report.json --report-file report.html`. The render needs more memory than writing the HTML during the run would, about 1.3 times as much for a 10 minute run and more for shorter ones, but it no longer competes with the load test, and it can run later or on another machine. [Output Files](output-files.md) gives the rules.

Without a report, as on a `--dashboard` run with Save report off, Goose keeps only the last 10 minutes of per second graph data in memory; the dashboard's charts show the last 5 minutes of it. On a long soak test with `--dashboard-control`, no `--report-file` path and no `--pdf-print-html`, turn Save report off to keep memory flat, and turn it on before the part of the run you want graphed.

## Other options

When you need more than this page, these come next:

- [Test Plan](test-plan.md): a load shape of steps, such as a ramp up, a hold and a spike.
- [Throttle](throttle.md): a cap on requests per second.
- [Thresholds](thresholds.md): pass and fail rules on the metrics.
- [Limiting Scenarios](scenarios.md): running only some of a load test's scenarios.
- [Logging](../logging/overview.md): the request, transaction, scenario, error and debug logs.
