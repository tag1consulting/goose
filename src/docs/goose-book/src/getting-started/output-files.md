# Output files

Goose writes reports and logs only when you ask for them. This page gives the rules for every file Goose writes, then three recipes.

## Files

**1. A run writes nothing unless a flag or the dashboard asks for it.**

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s
```

This run prints its metrics and writes no file and no folder.

**2. An output flag given a path writes exactly that file and nothing more.**

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file results.html
```

This writes `results.html`, as Goose 0.18 did.

**3. An output flag given without a path writes into the run's folder.** `--report-file` writes `report.json` and `report.html`. `--goose-log`, `--request-log`, `--transaction-log`, `--scenario-log`, `--error-log` and `--debug-log` write `goose.log`, `requests`, `transactions`, `scenarios`, `errors` and `debug`, each with its format's extension (`.json`, `.csv`, or `.log` for raw and pretty), plus `.gz` under rule 15.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file -R --request-format csv
```

This writes `report.json`, `report.html` and `requests.csv` into the run's folder.

**4. The run's folder is `--output-dir DIR`, or `goose-runs/<YYYYMMDD-HHMMSS>` from the run's local start time.** Goose creates it as the run starts and prints its path. If the run writes nothing into it, Goose removes it when the run ends. Goose never writes a run into a folder that already exists and is not empty when the run starts. It uses `DIR-2`, `DIR-3`, `DIR-4` and so on instead, always numbered from the original name (never `DIR-2-2`). It takes the first number whose folder is missing or empty, and prints the name it chose. Two processes that start at the same moment still get a folder each. The numbering also gives each run under `--no-autostart`, and each run started from the dashboard, its own folder.

While the run lasts, the folder holds a `.goose-run` file, which reserves it so no other run takes it. A bare `--goose-log` keeps the file there until the next run starts or the load test ends. A process that is killed leaves the file behind; remove it by hand to free the folder. Before the first run starts, a bare `--goose-log` keeps its lines in memory, up to 1 MiB, and writes them into the first run's folder. Lines past 1 MiB are dropped, and if no run starts, none are written.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --output-dir goose-runs/nightly --report-file
```

The first run writes into `goose-runs/nightly`, the next into `goose-runs/nightly-2`. `--output-dir` needs an output flag given without a path, or `--dashboard` without `--no-metrics`; otherwise it is an error, as the folder would hold nothing. A folder Goose can't write to stops the run before any load, with an error naming the folder. The one exception is in rule 9.

## What a report holds

**5. A report always has the whole run's totals and percentiles.** Its graphs cover the seconds Goose still kept when the report was written; when that is not the whole run, the report says when its graphs start and why.

While Save report is off and no `--report-file` path is given, Goose keeps only the dashboard's last 10 minutes of graph data (rule 10). A report written later, after Save report is turned on or by Save report now, still has every request in its tables, and says when its graphs start and why.

**6. `--notes "text"` is stored in the JSON report and shown at the top of the HTML and Markdown reports and in the dashboard.** It needs `--report-file` or `--dashboard`.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file --notes "baseline before cache change"
```

**7. `--report-graph-data` adds the per second graph data to the JSON report.** Off by default, and it needs `--report-file` or `--dashboard`. With it, a report rendered from the JSON has its graphs; without it, the rendered report says the graphs were not recorded.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file results.json --report-graph-data
```

**8. `--render-report FILE.json` writes the reports named by `--report-file` paths from an existing JSON report, then exits without running a load test.** It refuses `--notes` and `--report-graph-data`, as the rendered report keeps the JSON report's own.

```bash
cargo run --example simple -- --render-report results.json --report-file results.html --report-file results.md
```

## The dashboard

[The live dashboard](../controller/dashboard.md) describes the report bar, the runs panel and the routes behind them.

**9. The dashboard's report bar, at the top of the page, shows a Save report switch: whether this run's `report.json` and `report.html` are written to its folder.** It starts on with `--dashboard`, except under `--no-metrics`. It also starts on when `--report-file` paths are given; turn it off to get only those. For a run without the dashboard, a bare `--report-file` does what the switch does. The bar also lists any `--report-file` paths from the command line, which are always written.

When only `--dashboard` asks for the run's folder, with no output flag given without a path and no `--output-dir`, a folder Goose can't write to does not stop the run. Goose prints a warning naming the folder and the reason before any load, and runs with Save report off. A banner at the top of the dashboard says why, and stays for the whole run.

```bash
cargo run --features dashboard --example simple -- --host http://localhost:8080 --dashboard
```

**10. Save report can be turned on or off at any time.** While it is on, Goose keeps the run's graph data and heatmap for the whole run; while it is off and no `--report-file` path is given, only the dashboard's last 10 minutes of graphs and 5 minutes of heatmap. Turning it on during a run keeps graphs from that point on, plus that window before it, and rule 5 labels the report.

On a long soak test with `--dashboard-control` and no `--report-file` path, turn Save report off to keep memory flat, and turn it on before the part of the run you want graphed. Without `--dashboard-control`, Save report stays on: in a 60 minute run of `examples/simple` with 300 users, about 28 requests a second over 2 request names, the process grew by about 0.8 MiB an hour, from 37.9 MiB at 5 minutes to 38.7 MiB at 60. Add `--dashboard-control` to be able to turn Save report off.

**11. Save report now writes the run's report as it stands into its folder while the run continues, and marks it as in progress.** It needs a run in progress. The runs panel lists the report once it is written. If it can't be written, the runs panel says why. The report at the end of the run replaces it, also when Save report is off. A second switch in the bar, **Include graph data**, is rule 7's flag and can also be changed at any time.

**12. The runs panel lists the folders beside the run's folder that hold a `report.json` or `report.json.gz`.** It looks in the folder that holds the run's folder: the parent of the `--output-dir` folder, or `goose-runs` by default. Rows are newest first and show each run's notes. A report still marked in progress shows as running for the run in progress and as incomplete for any other (a run that ended without its final report). Each row offers HTML, JSON and Markdown, rendering any format the folder lacks from its JSON. A report stored gzipped downloads as plain HTML, JSON or Markdown. With control, each row also has a Delete button. It deletes the folder's reports and the logs Goose names there, then the folder if that leaves it empty. The folder of the run in progress can't be deleted. Nor can the folder a bare `--goose-log` is open in: Goose keeps that log open in the last run's folder until the next run starts. Nor can a folder another Goose process is writing to, which holds `.goose-run` (rule 4).

With `--output-dir goose-runs/nightly`, the panel lists the folders in `goose-runs`. It lists only folders whose name is a letter or digit followed by any mix of letters, digits, `.`, `_` and `-`, so a run written to `goose-runs/my run` is not listed. For such a folder, the banner at the end of the run says where the report was saved and offers no downloads.

**13. Save report now, Delete, and changing either switch need `--dashboard-control` and the control token, like Start and Stop.** Without control the switches are shown read only, so Save report stays on and every `--dashboard` run writes `report.json` and `report.html` into its folder, except under `--no-metrics`. Goose then also keeps the run's graph data for the whole run (rule 10).

```bash
cargo run --features dashboard --example simple -- --host http://localhost:8080 \
  --dashboard --dashboard-control --dashboard-auth-token "$TOKEN"
```

## Compression

**14. Any report or log whose name ends in `.gz` is written gzipped.** A report's format is the one the name before `.gz` names. A log keeps the format its `--request-format` or other format option sets. Goose reads a `.json.gz` report anywhere it reads a JSON report: `--baseline-file`, `--render-report` and the runs panel. `--pdf-print-html`, a debugging aid, writes plain HTML whatever its name.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file results.json.gz -R requests.csv.gz --request-format csv
```

**15. `--compress` gzips every file Goose writes into the run's folder under rules 3 and 9, each named with `.gz` added.** A path given to an output flag keeps exactly that name, and is gzipped only when that name ends in `.gz` (rule 14). `--compress LEVEL` sets the level from 1 (fastest) to 9 (smallest), default 6, for every gzipped file, including rule 14's.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file -R --compress 9
```

This writes `report.json.gz`, `report.html.gz` and `requests.json.gz` into the run's folder.

## Recipes

### One report file

```bash
cargo run --release --example simple -- --host http://localhost:8080 -t 5m --report-file results.html
```

Goose writes `results.html` and nothing else.

### See a run made without the dashboard in the dashboard

Write the run's report into a folder under `goose-runs`:

```bash
cargo run --release --example simple -- --host http://localhost:8080 -t 5m \
  --output-dir goose-runs/nightly --report-file
```

Later, start the dashboard from the same working directory. With `--no-autostart` it waits for Start instead of running a new load test, and its runs panel lists `nightly`:

```bash
cargo run --release --features dashboard --example simple -- --host http://localhost:8080 --dashboard --no-autostart
```

### Keep files small

Add `--compress` to gzip every file in the run's folder, or name a file with `.gz`:

```bash
cargo run --release --example simple -- --host http://localhost:8080 -t 10m \
  --report-file --report-file summary.html -R --compress
```

`summary.html` stays plain: a path keeps its name. To read the gzipped files:

```bash
gzip -dc goose-runs/*/report.json.gz | jq .
gunzip -k goose-runs/*/report.html.gz    # then open report.html in a browser
gzip -dc goose-runs/*/requests.json.gz | head
```

The dashboard's runs panel also opens a gzipped report. A gzipped log is whole once its run ends. A gzipped Goose log is also whole between runs, so `gzip -dc` reads it while the dashboard waits for the next run. During a run, and after a run killed with SIGKILL, `gzip -dc` reads a gzipped log up to the last part written, then reports an unexpected end of file. To follow a run as it goes, use a plain log.
