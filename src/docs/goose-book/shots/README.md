# Goose Book screenshots

This folder retakes the Book's screenshots from real load tests. `shots.json` lists every image:
the file it writes, the run it needs, when to take it and what to capture. `shots.mjs` builds
Goose, starts each run, drives the dashboard in Chromium and writes the PNG files into
`../src/`. CI never runs it.

## What it needs

- Rust and Cargo, to build the examples. The rig builds each example once per feature set.
- Node.js 20 or later.
- Chromium for Playwright: `npx playwright install chromium` once, after `npm ci`.
- For the `metrics` run, built with `pdf-reports`: network access, so Goose can download a Chrome
  for Testing headless shell when it finds no Chrome, and so Chrome can load the HTML report's
  graphs from their CDN while it writes the PDF. An installed Chrome, or one named in `CHROME`,
  saves the download.
- Network access for every `report` capture of a graph (`report-rps`, `metrics-html-*`): the
  report loads ECharts from a CDN, and offline the graphs come out blank with no error from the rig.
- The load target, a Drupal 11 site installed with the Umami demo profile and changed as
  `examples/umami` needs: search and contact enabled, a "Website feedback" contact form, an alias
  for the renamed mushroom article (`favourite`), image styles generated ahead, and CSS and JS
  aggregation turned off (`examples/umami` does not decode `&amp;` in the asset links it parses,
  so aggregated asset links fail). Its admin account must be `admin` with password
  `P@ssw0rd1234`, the defaults of `examples/umami`. Set its address in `GOOSE_SHOTS_UMAMI`.
- The `errors` run sets a wrong admin password on purpose, so each pass makes one failed admin
  login. Drupal blocks the account after 5 failed logins in 6 hours, and a good login clears the
  count; the other runs log in with the right password, so a full pass leaves the count at zero.
  Taking `--only dashboard-errors` 5 times in a row would lock the account.
- No failed requests before a pass. Drupal can serve a cached page that still names a Spanish
  translations file it has since replaced, and every Spanish page then gets a 404 for it. Check
  that `curl -s $GOOSE_SHOTS_UMAMI/es | grep -o 'languages/es_[^"]*'` names a file that answers
  200; if not, rebuild the site's caches (`drush cr`).
- Free ports 5116 (telnet Controller) and 5118 (dashboard), and 5126 and 5128 for the `metrics`
  run, which moves its ports there. The rig refuses to start a run while something answers on that
  run's dashboard port.
- `pdftoppm` from Poppler, for the `pdf` capture (`brew install poppler`, `apt install poppler-utils`).
- For the `co-average` run, a local server that stalls a share of requests; see
  [The stall server](#the-stall-server). Name the command that starts it in
  `GOOSE_SHOTS_STALL_SERVER`.
- `oxipng`, optional. When it is installed, every image goes through `oxipng -o 4`.

## Running it

From this folder:

```bash
npm ci
npx playwright install chromium
GOOSE_SHOTS_UMAMI=http://umami.example.com \
  GOOSE_SHOTS_STALL_SERVER="node /path/to/stall-server.mjs" node shots.mjs
```

That builds Goose, runs every run in `shots.json` one after another, and writes every image. A
full pass takes about 25 minutes, most of it waiting for runs to reach the times the shots need.

To take some shots only, name them:

```bash
GOOSE_SHOTS_UMAMI=http://umami.example.com node shots.mjs --only dashboard-hero,dashboard-rps
```

The coordinated omission shots need only the stall server, not the Umami site:

```bash
GOOSE_SHOTS_STALL_SERVER="node /path/to/stall-server.mjs" node shots.mjs \
  --only co-dashboard,co-dashboard-whole-run,co-html-requests,co-html-percentiles,co-html-events
```

`GOOSE_SHOTS_BEFORE_RUN`, when set, is a shell command the rig runs before each run, such as one
that reopens an ssh tunnel to the target, so no run outlives a connection the network limits.

A run still waits for every event it has, so a later shot takes as long as the run needs to reach
it. `--skip-build` reuses the binaries the last build copied into `.work/bin`. `--list` lists the
shots and exits.

Each run gets a fresh folder, `.work/runs/<run>/`, which holds Goose's log (`goose.log`) and its
run folders. When the last shot of a run is taken, the rig sends Goose Ctrl-C and waits for it to
exit, then kills it if it is still up after 20 seconds. If the rig itself fails or is
interrupted, it kills every Goose process it started.

The rig fails with a list of problems when an image is over 500 KB. A run that should show no errors but had failed requests, such as from a dropped connection to the target, is taken again, up to `attempts` times.

## The manifest

```json
{
  "defaults": { "viewport": { "width": 1440, "height": 900 }, "scale": 2, "colorScheme": "light", "locale": "en-US" },
  "runs": { "<run>": { ... } },
  "shots": [ { ... } ]
}
```

`timezone` may be added to `defaults`; without it the browser uses the machine's time zone, as Goose does when it names run folders, so the Runs panel's Started column agrees with the folder names.

A run:

| Field | Meaning |
|-------|---------|
| `example` | The example to build and run, such as `umami` |
| `features` | Cargo features to build it with, such as `["dashboard"]` |
| `target` | The host to load test, passed as `-H`. `${NAME}` is read from the environment, and the rig stops when it is not set |
| `flags` | The rest of the command line |
| `env` | Environment variables for the run |
| `dashboard` | The dashboard's address, `http://127.0.0.1:5118` by default |
| `token` | The dashboard token, when the flags set one |
| `prelude` | Runs to finish first in the same folder, each a list of flags, so that the Runs panel has earlier runs to list |
| `events` | Actions at set times that take no image, each `{ "name", "at", "actions" }` |
| `allowFailures` | `true` when the run's images are about errors. Otherwise, a run with any failed request is taken again, its images with it |
| `attempts` | How many times to take a run before giving up, 3 by default |
| `server` | A load target the rig starts before the run and stops after it: `command` (with `${NAME}` read from the environment), `args`, and `ready`, an address that answers once the server is up |

A shot:

| Field | Meaning |
|-------|---------|
| `name` | The name `--only` takes |
| `output` | The image, relative to `src/docs/goose-book/src/` |
| `run` | The run it needs |
| `at` | Seconds after the run's Goose process started. The shot waits until then, and every `element` and `band` shot waits until the elements it captures hold data; a `viewport` or `page` shot waits for its `ready` element when it names one, a `report` shot waits 2 seconds after the page loads, and a `pdf` shot does not wait |
| `viewport` | The viewport for this shot, when not the default |
| `actions` | What to do in the page before capturing |
| `settle` | Seconds to wait after the actions |
| `capture` | What to capture |

A capture is one of:

- `{ "type": "element", "selector": "#errors" }`: one element. `width` and `height` cut it to at
  most that many CSS pixels from its top left corner.
- `{ "type": "band", "from": "#a", "to": "#b" }`: the box around two elements.
- `{ "type": "viewport", "column": "#kpis", "pad": 16 }`: the first screen, cut to the left and
  right edges of the `column` element. Without `column`, the whole first screen.
- `{ "type": "page" }`: the whole page.
- `{ "type": "report", "file": "report.html", "selector": "#graphs" }`: a report from the newest run
  folder that holds it, or from the run's own folder when no run folder holds it (a `--report-file`
  path without a folder), opened in the browser; the whole page without `selector`. `selector`
  may be a list, to capture the box around them all.
- `{ "type": "pdf", "file": "report.pdf", "dpi": 150, "height": 1500 }`: the top of a PDF report,
  found as for `report`, drawn by `pdftoppm` at `dpi` and cut to `height` pixels (and, with `height`,
  to `width` pixels when given).

`element`, `band` and `report` take `width`, `height` and `pad`, a margin in CSS pixels, and
`rows`: the image then ends under that many table rows, after the header row, of the last element
named.

`element` takes `text`, a regular expression its own text must match before the capture;
`viewport` and `page` take `ready`, a selector to wait for, and `text` for that element. An element is not ready while its text says
`No data yet`, `No reports yet` or `connecting`.

An action is `{ "do": ... }` with one of:

| `do` | Fields | What it does |
|------|--------|--------------|
| `click` | `selector` | Clicks it |
| `fill` | `selector`, `value` | Types the value into a field |
| `select` | `selector`, `value` | Picks an option |
| `check` | `selector` | Checks a checkbox |
| `open` | `selector` | Opens a `<details>` element, if closed |
| `scroll` | `selector` | Scrolls it into view |
| `wait` | `seconds` | Waits |
| `waitFor` | `selector`, `text`, `timeout` | Waits until the element holds data and its text matches |
| `api` | `method`, `path`, `body` | Sends a request to the dashboard, with the run's token |
| `telnet` | `command`, `port` | Sends one command to the telnet Controller |

Shots and events run in the order of `at`, so a later shot sees what an earlier action did.

## The stall server

The `co-average` run measures coordinated omission against a server that stalls every `--every`th
request for `--stall-ms` milliseconds and answers the others after a random delay from `--min-ms`
to `--max-ms` milliseconds. It is not in this repository; any server that takes these options and
answers `GET /stats`, the run's `ready` address, with 200 will do. The run starts it as:

```text
$GOOSE_SHOTS_STALL_SERVER --port 18900 --every 1000 --stall-ms 1000 --min-ms 5 --max-ms 15
```

It must answer every request with `200`, with a JSON body for a `POST` (the run uses
`examples/graphql_loadtest`), and count requests across all connections, so that one request
in 1,000 stalls whatever the number of users.

## Adding a shot

1. Pick the run it needs, or add one to `runs`. Shots that can share a run should: every run costs
   its full length.
2. Add the shot with a time at least 90 seconds into the run, so the panels hold data.
3. Take it alone with `--only <name>`, and look at it at the Book's column width, about 750
   pixels.
4. Reference it from the Book page with alt text that says what it shows and a one sentence
   caption.
