# Goose Book screenshots

This folder retakes the Book's screenshots from real load tests. `shots.json` lists every image:
the file it writes, the run it needs, when to take it and what to capture. `shots.mjs` builds
Goose, starts each run, drives the dashboard in Chromium and writes the PNG files into
`../src/`. CI never runs it.

## What it needs

- Rust and Cargo, to build the examples. The rig builds each example once per feature set.
- Node.js 20 or later.
- Chromium for Playwright: `npx playwright install chromium` once, after `npm ci`.
- The Umami target, a Drupal 11 site installed with the Umami demo profile, with CSS and JS
  aggregation turned off (`examples/umami` does not decode `&amp;` in the asset links it parses,
  so aggregated asset links fail). Its admin account must be `admin` with password
  `P@ssw0rd1234`, the defaults of `examples/umami`. Set its address in `GOOSE_SHOTS_UMAMI`.
- For the Examples chapter's shots, four small local targets, each named by an environment
  variable. They are not in this repository; any server that answers the same way will do.
  - `GOOSE_SHOTS_HTTP`, such as `http://127.0.0.1:8080`, for `simple`, `closure` and `basic_auth`:
    pages at `/`, `/about/`, `/about` and `/our-team`; `POST /login` answers 200;
    `/basic-auth/user/passwd` answers 401 unless the request has Basic auth with the user `user`
    and the password `passwd`; `/user-agent`, `/json` and `POST /post` answer 200 with a JSON body.
  - `GOOSE_SHOTS_SESSION`, such as `http://127.0.0.1:8081`, for `session`: `POST /signup` answers
    `{"jwtToken":"goose-example-token"}`, and `/` answers 401 unless the request carries
    `Authorization: Bearer goose-example-token`.
  - `GOOSE_SHOTS_GRAPHQL`, such as `http://127.0.0.1:4400`, for `graphql_loadtest`: `POST /graphql`
    answers each of the example's four operations with a 200 and a JSON `data` body.
  - `GOOSE_SHOTS_TCP`, such as `tcp://127.0.0.1:9000`, for `tcp_loadtest`: a TCP echo server.
    The example reads the port from it.
- The `errors` run sets a wrong admin password on purpose, so each pass makes one failed admin
  login. Drupal blocks the account after 5 failed logins in 6 hours, and a good login clears the
  count; the other runs log in with the right password, so a full pass leaves the count at zero.
  Taking `--only dashboard-errors` 5 times in a row would lock the account.
- No failed requests before a pass. Drupal can serve a cached page that still names a Spanish
  translations file it has since replaced, and every Spanish page then gets a 404 for it. Check
  that `curl -s $GOOSE_SHOTS_UMAMI/es | grep -o 'languages/es_[^"]*'` names a file that answers
  200; if not, rebuild the site's caches (`drush cr`).
- Free ports 5116 (telnet Controller) and 5118 (dashboard). The rig refuses to start a run while
  an HTTP server answers on the dashboard port.
- `oxipng`, optional. When it is installed, every image goes through `oxipng -o 4`.

## Running it

From this folder:

```bash
npm ci
npx playwright install chromium
GOOSE_SHOTS_UMAMI=http://umami.example.com \
  GOOSE_SHOTS_HTTP=http://127.0.0.1:8080 GOOSE_SHOTS_SESSION=http://127.0.0.1:8081 \
  GOOSE_SHOTS_GRAPHQL=http://127.0.0.1:4400 GOOSE_SHOTS_TCP=tcp://127.0.0.1:9000 \
  node shots.mjs
```

The rig stops before the first run if any selected run's target or env variable is not set.

That builds Goose, runs every run in `shots.json` one after another, and writes every image. A
pass takes as long as its runs, most of it waiting for runs to reach the times the shots need: each
run lasts until its last shot's or event's `at`, after any preludes, plus the stop.
The seven Examples runs alone take about 15 minutes, and two to three more for each run taken again.
The rig's log prints its times in UTC; run folders and file times are local.

To take some shots only, name them:

```bash
GOOSE_SHOTS_UMAMI=http://umami.example.com node shots.mjs --only dashboard-hero,dashboard-rps
```

`GOOSE_SHOTS_BEFORE_RUN`, when set, is a shell command the rig runs before each run, such as one
that reopens an ssh tunnel to the target, so no run outlives a connection the network limits.

A run still waits for every event it has, so a later shot takes as long as the run needs to reach
it. Images that one page shows together should come from one run, so the rig warns when `--only`
names some of a run's shots and not the others. `--skip-build` reuses the binaries the last build
copied into `.work/bin`. `--list` lists the shots and exits.

Each run gets a fresh folder, `.work/runs/<run>/`, which holds Goose's log (`goose.log`), its
run folders, and its images in `shots/`, which the rig copies into `../src/` once the run passes
its check; if one image is over its size limit, none of that run's images are copied. When the
last shot of a run is taken, the rig sends Goose Ctrl-C and waits for it to exit, then kills it if
it is still up after 20 seconds.
If the rig itself fails or is interrupted, it kills every Goose process it started.

The rig fails with a list of problems when an image is over 500 KB, or over its shot's `maxKB`. A
run that should show no errors but had failed requests, such as from a dropped connection to the
target, is taken again, up to `attempts` times, and only the attempt kept reports its images'
problems. Each failed attempt's folder is kept as `.work/runs/<run>.attempt-<n>`, so its
`goose.log` says which requests failed, until the next pass of that run. A run that still has
failed requests on its last attempt stays in `.work/runs/<run>`, and the pass stops there.

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
| `prelude` | Runs to finish first in the same folder, each a list of flags, so that the runs panel has earlier runs to list |
| `events` | Actions at set times that take no image, each `{ "name", "at", "actions" }` |
| `allowFailures` | `true` when the run's images are about errors. Otherwise, a run with any failed request is taken again, its images with it |
| `attempts` | How many times to take a run before giving up, 3 by default |

A shot:

| Field | Meaning |
|-------|---------|
| `name` | The name `--only` takes |
| `output` | The image, relative to `src/docs/goose-book/src/` |
| `run` | The run it needs |
| `at` | Seconds after the run's Goose process started. The shot waits until then, and every element and band shot waits until the element it captures holds data, and a viewport or page shot until its `ready` element does |
| `viewport` | The viewport for this shot, when not the default |
| `actions` | What to do in the page before capturing |
| `settle` | Seconds to wait after the actions |
| `capture` | What to capture |
| `maxKB` | A lower limit than 500 KB for this image, in KB; a thumbnail sets 100 |

A capture is one of:

- `{ "type": "element", "selector": "#errors" }`: one element. `pad` adds a margin, and `width` and
  `height` cut it to at most that many CSS pixels from the top left corner of the padded box.
  `right` and `bottom`, each a selector, end the cut at that element's right or bottom edge,
  such as a table column's header cell (`#requests-table th:nth-child(7)`) or a row, so a cut
  never splits a column or a row however wide the data is. They set that edge, with no `pad`
  beyond it, when it falls inside the box.
- `{ "type": "band", "from": "#a", "to": "#b" }`: the box around two elements. It takes `width`,
  `height`, `right`, `bottom` and `pad` as `element` does.
- `{ "type": "viewport", "column": "#kpis", "pad": 16 }`: the first screen, cut to the left and
  right edges of the `column` element. Without `column`, the whole first screen.
- `{ "type": "page" }`: the whole page.
- `{ "type": "report", "file": "report.html", "selector": "#graphs" }`: a report from the
  newest of the run's run folders that holds `file`, opened in the browser; the whole page without
  `selector`. With `selector` it takes `width`, `height`, `right`, `bottom` and `pad` as `element`
  does.

A cut made from element boxes, by `band`, by `report` with `selector`, and by `element` with any
of `width`, `height`, `right`, `bottom` or a non-zero `pad`, is snapped inward to whole device
pixels, so it never takes a pixel from beyond the cut's rectangle (the boxes plus `pad`) and loses
at most one device pixel of its own edge. Other captures, a `viewport` `column` cut among them,
are not snapped.

`viewport` and `page` take `ready`, a selector to wait for, and `element` waits on its own
selector; all three take `text`, a regular expression that element's text must match before the
capture. An element is not ready while its text says
`No data yet`, `No reports yet` or `connecting`.

An action is `{ "do": ... }` with one of:

| `do` | Fields | What it does |
|------|--------|--------------|
| `click` | `selector` | Clicks it |
| `fill` | `selector`, `value` | Types the value into a field |
| `select` | `selector`, `value` | Picks the option with that value or that label |
| `check` | `selector` | Checks a checkbox |
| `open` | `selector` | Opens a `<details>` element, if closed |
| `scroll` | `selector` | Scrolls it into view |
| `wait` | `seconds` | Waits |
| `waitFor` | `selector`, `text`, `timeout` | Waits until the element holds data and its text matches |
| `api` | `method`, `path`, `body` | Sends a request to the dashboard, with the run's token |
| `telnet` | `command`, `port` | Sends one command to the telnet Controller |

Shots and events run in the order of `at`, so a later shot sees what an earlier action did.

## Adding a shot

1. Pick the run it needs, or add one to `runs`. Shots that can share a run should: every run costs
   its full length.
2. Add the shot with a time at least 90 seconds into the run, so the panels hold data.
3. Take it alone with `--only <name>`, and look at it at the Book's column width, about 750
   pixels.
4. Reference it from the Book page with alt text that says what it shows and a one sentence
   caption.
