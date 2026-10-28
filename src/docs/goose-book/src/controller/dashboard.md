# Live Dashboard

The live dashboard is a web page Goose serves while a load test runs. It shows users, requests, failures and response times second by second, and lists the errors. You can download and compare saved runs from it. With control on, you can also start and stop the test and change its users, host, ramps and run time without restarting Goose.

![The live dashboard during a load test of the Umami demo site, with control on and no token](dashboard-hero.png)

*The top of the page with control on, as the ramp up ends. The `request rows truncated` notice is explained under [The tables](#the-tables).*

To try it against your own [Umami](../example/umami.md) site from a Goose checkout, run this and open `http://127.0.0.1:5118/`:

```bash
cargo run --release --features dashboard --example umami -- \
  --dashboard -H http://umami.example.com -u 100 -r 1
```

Most screenshots here come from one run of that command against a local Umami site, with `--dashboard-control --dashboard-no-auth`, `-t 10m`, `--no-reset-metrics` and `--notes "100, then 150 users"` added. The read-only and Errors screenshots are from other runs.

## Turning it on

Enable the optional `dashboard` feature in your load test's `Cargo.toml` (see [Requirements](../requirements.md)), then run with `--dashboard`:

```toml
goose = { version = "^0.19", features = ["dashboard"] }
```

```bash
cargo run --release -- --dashboard -H https://staging.example.com -u 50 -t 10m
```

A build without the feature refuses to start with `--dashboard`. The dashboard listens on `127.0.0.1:5118` (see [Related flags](dashboard-reference.md#related-flags) to change that).

`--dashboard` alone is read-only, but the telnet Controller can still control the run. Goose starts it by default, with no authentication, whatever the dashboard is set to:

![The read-only Control card: two ways to restart Goose with control, and the telnet Controller at 127.0.0.1:5116](dashboard-read-only.png)

To control the test from the page as well, restart Goose with `--dashboard-control` added, and one of:

- `--dashboard-auth-token <token>`, then open `http://127.0.0.1:5118/?token=<token>` (see [Browser bootstrap with `?token=`](dashboard-reference.md#browser-bootstrap-with-token)).
- `--dashboard-no-auth`, for control without a token.

### Control without a token

With `--dashboard-control --dashboard-no-auth` the page reads `Live dashboard · control, no token`. Anyone who can reach the port can then start and stop the test, point the load at any host, and delete saved runs ([the rules](dashboard-reference.md#control-without-a-token)). Even read-only and without a token, it lets anyone who can reach the port watch the run and download every saved report.

On a shared machine, use a token and set it in the load test's code with `GooseDefault::DashboardAuthToken`, since every user can see a `--dashboard-auth-token` flag in `ps`. Add `--no-telnet` too unless you need the telnet Controller, which every user can reach as well ([On a shared machine](dashboard-reference.md#on-a-shared-machine)).

### Saved reports and overhead

By default the dashboard saves each run's `report.html` and `report.json` to `goose-runs/<start time>` under the working directory ([Saving the report](dashboard-reference.md#saving-the-report)). Watching adds no requests to the site under load. `--dashboard` costs memory for the per second graph data even with no page open, and a little CPU each second while a page is open ([GraphData memory cost](dashboard-reference.md#graphdata-memory-cost)).

### Watching from another machine

Keep the dashboard on loopback and tunnel to the machine running Goose:

```bash
ssh -L 5118:127.0.0.1:5118 loadgen.example.com
```

Then open `http://127.0.0.1:5118/` on your laptop, adding `?token=<token>` if you set one ([SSH tunnels](dashboard-reference.md#ssh-tunnels)). You can instead bind a network address with `--dashboard-host` and a token, but the dashboard is plain http, so the token and the metrics cross the network unencrypted. [Bind and token rules](dashboard-reference.md#bind-and-token-rules) lists what each setup exposes.

## What it shows

The header shows the phase, the hosts under load, the duration, its `--notes`, and `SSE` with a green dot while the page is streaming. On a read-only dashboard it also shows the run settings, such as the test plan. Under it are the report bar and the Control card, then a row of tiles: **Users** against the target, **RPS**, **Fail %**, p95 and average response time. The duration and every tile but Users cover the run so far (or since the last metrics reset), so during the ramp up RPS reads lower than the requests chart.

The charts plot one point a second and keep the last 300 seconds. The chart screenshots below were taken 200 seconds in, so they go back to the start of the run; on a longer run the start scrolls off, but the reports keep all of it. When metrics reset at the end of the ramp up (the default, see [The tables](#the-tables)), the requests chart and the heatmap start over, and the users chart keeps the ramp.

Under each line chart is the average of its last 10 seconds. The average lags: the first screenshot, taken as the ramp ended, reads `Users 96 of 100` under the users chart while the Users tile already reads `100 / 100`. Once there are 20 seconds of data the average is also marked `rising`, `falling` or `steady` against the 10 seconds before.

### Active users

![The Active users chart: a ramp to 100 users, a hold, a second ramp to 150 and a hold](dashboard-users.png)

*Users ramp to 100 at one a second and hold. At 120 seconds Target is set to 150 from the Control card, with no restart, and users ramp again and hold at 150.*

### Requests per second

![The requests and failures per second chart: requests climb to about 1,000 a second, then swing around it, and failures stay at zero](dashboard-rps.png)

*The dashed line is Failures/s, at zero for the whole run.*

Compare it with the users chart: from 120 to 170 seconds users went from 100 to 150 and requests per second didn't rise, so the site was at capacity.

### The response time heatmap

![The response time heatmap: most requests take 5 to 20 ms, and from about 124 seconds a band of slower requests forms between 200 ms and 1 second](dashboard-heatmap.png)

*From about 124 seconds (roughly 104 users) the p95 climbs from 20 ms to several hundred, and the p99 to mostly between 500 ms and 1 second. The p50 stays near 8 ms.*

The **Response times by second** card is a heatmap. Each column is one second, oldest on the left, and each row a range of response times on a log scale; busier cells are darker (brighter in the dark theme). The three lines are each second's p50, p95 and p99. The newest two seconds aren't drawn yet, because their slowest requests are still arriving. Hover over a cell for its count and that second's percentiles, or pick a request at the top of the card to see only that one ([more on the heatmap](dashboard-reference.md#the-response-time-heatmap)).

### The tables

Below the charts are **Summary** (phase, duration, users, hosts and Goose version) and **Whole run** (totals and response time percentiles), then **Scenarios**, **Transactions** and **Requests**, with counts and times for each. Click a column heading to sort by it:

![The Requests table sorted by p95, slowest first, with POST /es/search/node at the top at 212 ms](dashboard-requests.png)

*Sorted by p95, the Spanish search comes first among the rows shown. Scroll right for p99, max and status codes.*

The tables cover the same span as the tiles. As in a console run, Goose resets metrics when the ramp up ends, unless you pass `--no-reset-metrics` or `--iterations` or run a test plan (see [Metrics](../getting-started/metrics.md)). This run passed `--no-reset-metrics`, so the table's 212 ms includes the first two minutes, before the site slowed down; the heatmap's lines are per second, across all requests.

Goose only sends the dashboard the 100 busiest request names. With more, as with Umami, the page shows `request rows truncated (showing top rows only).` above the Control card. A slow request that is rarely made can drop off the list. In this run the admin login and the article edits (a few requests each) are missing, and the login and the edit saves are slower than the Spanish search. The reports list every request.

### Errors

![The Errors table: the failed admin login and the 403 responses to the edits that followed](dashboard-errors.png)

*From a run with `ADMIN_PASSWORD` set wrong: the login fails with `login failed (check ADMIN_USERNAME and ADMIN_PASSWORD)`, and every article the admin then tries to edit returns 403.*

Each row is one error with its request and count. The table holds the 50 most frequent, and the notice above the Control card says `error rows truncated` when there are more. Below the table, **Transaction errors** counts the errors a transaction's own code returned; failed requests aren't counted there.

## What you can do with it

**Stop**, **Start**, **Quit**, **Target**, **Edit settings**, the report switches, **Save report now** and **Delete** need control on. You don't need control to read the page, or to download and compare saved runs.

### Stop a failing run

If the Failures/s line climbs, check the Errors table and press **Stop**. The users stop at once, Goose writes the reports as it would at the end of any run, and then it waits in Idle for the next run ([Start and Stop semantics](dashboard-reference.md#start-and-stop-semantics)). On a read-only dashboard, send `stop` to the [telnet Controller](telnet.md) on the machine running Goose instead. Under `--no-telnet`, press Ctrl-C there, which also exits Goose.

### Change users and settings

Type a count in **Target** and press **Apply**, or press **−** or **+**, which change it by the **step** value next to them and apply at once. Goose adds users at `--increase-rate` (`-r`) and removes them at `--decrease-rate`, or `--increase-rate` if that isn't set. `--increase-time` and `--decrease-time` don't apply to these changes. If you change users during the ramp up, Goose ramps to the new count and then holds it for the full run time (the hold after the ramp up). During the hold, a change leaves the hold ending when it would have, unless the ramp to the new count takes longer than the hold has left; then the run holds the new count until you stop it. [Users on a running test](dashboard-reference.md#users-on-a-running-test) covers what happens with no rate set, when Target equals the current user count, and during the ramp down.

**Edit settings** opens a form for the host, ramp up, ramp down and run time. Edits stay pending until you press **Apply settings**, or **Start with changes**, which starts the next run with them. The first host change in a page load asks you to **Confirm** before anything is sent. While a test runs, the form only takes what the run can still change, such as the run time, and tells you why the other fields are locked. See [Run settings](dashboard-reference.md#run-settings) for the rest.

![The Control card in Idle with Edit settings open, a new host, ramp up and run time pending, and the host confirmation](dashboard-settings.png)

*Taken after the run above was stopped. **Start** now reads **Start with changes**, and the new host waits for **Confirm**. With edits pending, Target's count is sent with them, so its **Apply** is greyed out and **−** and **+** only change the field.*

### Start and quit

Start Goose with `--no-autostart` and it waits in Idle until you press **Start**. **Quit**, which shuts Goose down, only appears in Idle, so stop a running test first.

### Save, compare and delete runs

The report bar has two switches: **Save report** sets whether this run's report is written to its folder, and **Include graph data** adds the per second graph data to the JSON report (or use `--report-graph-data`). **Save report now** writes the report mid run.

The **Runs** panel, at the bottom of the page, lists the saved runs in `goose-runs`, newest first. It shows the first 50, and **Show all** lists the rest. Console runs show up too if they wrote to a run folder; a bare `--report-file`, with no file name, does that ([Output Files](../getting-started/output-files.md)). To browse saved runs without load testing anything, run Goose with `--dashboard --no-autostart` from the directory that holds `goose-runs` ([see a run made without the dashboard](../getting-started/output-files.md#see-a-run-made-without-the-dashboard-in-the-dashboard)). Goose refuses `--no-autostart` when nothing could start a run, so keep a Controller on (the telnet Controller is, unless you pass `--no-telnet`) or turn control on.

![The Runs panel with three saved runs, two of them checked for comparison](dashboard-runs.png)

*Two console runs with a bare `--report-file`, and the newest from the dashboard. The two newest are checked, so **Download comparison** appears.*

From each row you can download the HTML, Markdown or JSON report, even a format the run didn't save (it's rendered from the JSON report). Check **Compare** on two rows, and **Download comparison** gives a Markdown report of the newer run against the older. **Delete** removes a run's reports and logs. [The runs panel](dashboard-reference.md#the-runs-panel) covers `--output-dir`.

## When the run ends

An autostarted run (no `--no-autostart`) that reaches its end exits Goose. If a page is connected and the report was saved, Goose waits in Idle instead so you can still download it, and prints:

```text
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Ctrl-C to exit.
```

With control on, the line ends `Press Quit in the dashboard or Ctrl-C to exit.`, and you can start another run from the page. A Controller `shutdown` also ends the wait (closing the tab leaves Goose waiting). See [Waiting after an autostarted run](dashboard-reference.md#waiting-after-an-autostarted-run).

A run started from the dashboard or a Controller goes back to Idle when it ends, with or without a page. During a run, Ctrl-C, the [killswitch](../getting-started/killswitch.md), a Controller `shutdown` or an aborting [threshold](../getting-started/thresholds.md) ends both the run and Goose, with no wait, and the page says `Goose has exited.`

## Reference

[Dashboard Reference](dashboard-reference.md) has every flag and default, the bind and token rules, the endpoints, and how to build the page from its TypeScript sources.
