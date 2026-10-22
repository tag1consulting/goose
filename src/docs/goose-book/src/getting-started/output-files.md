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

**4. The run's folder is `--output-dir DIR`, or `goose-runs/<YYYYMMDD-HHMMSS>` from the run's local start time.** Goose creates it when it first writes into it and prints its path. Goose never writes a run into a folder that already exists and is not empty when the run starts. It uses `DIR-2`, `DIR-3`, `DIR-4` and so on instead, always counting on the original name (never `DIR-2-2`), takes the first number whose folder is missing or empty, and prints the name it chose. This also gives each run under `--no-autostart`, and each run started from the dashboard, its own folder.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --output-dir goose-runs/nightly --report-file
```

The first run writes into `goose-runs/nightly`, the next into `goose-runs/nightly-2`. A folder Goose can't write to stops the run before any load, with an error naming the folder.

## What a report holds

**5. A report always has the whole run's totals and percentiles.** Its graphs cover the seconds Goose still held when the report was written; when that is not the whole run, the report says when its graphs start and why.

The dashboard keeps only its last 10 minutes of graph data while no report needs more (rule 10). A report written then has every request in its tables, and its HTML says when its graphs start.

**6. `--notes "text"` is stored in the JSON report and shown at the top of the HTML and Markdown reports and in the dashboard.**

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file --notes "baseline before cache change"
```

**7. `--report-graph-data` adds the per second graph data to the JSON report.** Off by default. With it, a report rendered from the JSON has its graphs; without it, the rendered report says the graphs were not recorded.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file results.json --report-graph-data
```

**8. `--render-report FILE.json` writes the reports named by `--report-file` paths from an existing JSON report, then exits without running a load test.**

```bash
cargo run --example simple -- --render-report results.json --report-file results.html --report-file results.md
```

## The dashboard

**9. The dashboard's report bar, at the top of the page, shows a Save report switch: whether this run's `report.json` and `report.html` are written to its folder.** It starts on with `--dashboard`, also when `--report-file` paths are given (turn it off to get only those), and a bare `--report-file` is the same switch for runs without the dashboard. The bar also lists any `--report-file` paths from the command line, which are always written.

```bash
cargo run --features dashboard --example simple -- --host http://localhost:8080 --dashboard
```

**10. Save report can be turned on or off at any time.** While it is on, Goose keeps the run's graph data and heatmap for the whole run; while it is off, only the dashboard's last 10 minutes of graphs and 5 minutes of heatmap. Turning it on during a run keeps graphs from that point on, plus that window before it, and rule 5 labels the report.

Keep Save report off through a long soak test to keep memory flat, and turn it on before the part of the run you want graphed.

**11. Save report now writes the run's report as it stands into its folder while the run continues, and marks it as in progress.** The report at the end of the run replaces it. A second switch in the bar, **Include graph data**, is rule 7's flag and can also be changed at any time.

**12. The runs panel lists the folders that hold a `report.json` or `report.json.gz` inside the parent of the run's folder:** the folder `--output-dir` names sits in, or `goose-runs` by default. Newest first, with their notes. A report still marked in progress shows as running for this run and as incomplete for any other (a run that ended without its final report). Each run offers HTML, JSON and Markdown, rendering any format the folder lacks from its JSON.

With `--output-dir goose-runs/nightly`, the panel lists the folders in `goose-runs`. It lists only folders whose name is a letter or digit followed by letters, digits, `.`, `_` and `-`, so a run written to `goose-runs/my run` is not listed.

**13. Changing either switch, Save report now and Delete need `--dashboard-control` and the control token, like Start and Stop.** Without control the switches are shown read only.

```bash
cargo run --features dashboard --example simple -- --host http://localhost:8080 \
  --dashboard --dashboard-control --dashboard-auth-token "$TOKEN"
```

## Compression

**14. Any report or log whose name ends in `.gz` is written gzipped, in the format the name before `.gz` names.** Goose reads a `.json.gz` report anywhere it reads a JSON report: `--baseline-file`, `--render-report` and the runs panel. `--pdf-print-html`, a debugging aid, writes plain HTML whatever its name.

```bash
cargo run --example simple -- --host http://localhost:8080 -t 30s --report-file results.json.gz -R requests.csv.gz
```

**15. `--compress` gzips every file Goose writes into the run's folder under rules 3 and 9, each named with `.gz` added.** A path given to an output flag keeps exactly that name, compressed or not. `--compress LEVEL` sets the level from 1 (fastest) to 9 (smallest), default 6, for every gzipped file, including rule 14's.

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

Later, start the dashboard from the same working directory. Its runs panel lists `nightly` beside the dashboard's own runs:

```bash
cargo run --release --features dashboard --example simple -- --host http://localhost:8080 --dashboard
```

### Keep files small

Add `--compress` to gzip every file in the run's folder, or name a file with `.gz`:

```bash
cargo run --release --example simple -- --host http://localhost:8080 -t 10m \
  --report-file --report-file summary.html -R --compress
```

`summary.html` stays plain: a path keeps its name. To read the gzipped files:

```bash
zcat goose-runs/*/report.json.gz | jq .
gunzip -k goose-runs/*/report.html.gz    # then open report.html in a browser
zcat goose-runs/*/requests.json.gz | head
```

The dashboard's runs panel also opens a gzipped report. A run killed with SIGKILL leaves a gzipped log with no trailer; `zcat` reads it up to the cut, then reports an unexpected end of file.
