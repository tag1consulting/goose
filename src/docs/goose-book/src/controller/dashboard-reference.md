# Dashboard Reference

This page has every flag, rule and endpoint of the [Live Dashboard](dashboard.md), and the snapshot fields a client most needs. The dashboard sends a metrics snapshot over Server-Sent Events (SSE), and falls back to a short poll when SSE isn't available. Compatibility rules are in [Stability](#stability).

## Enabling the dashboard

The dashboard is behind the `dashboard` crate feature, which isn't on by default. Build with `--features dashboard` and run with `--dashboard`:

```bash
cargo run --release --features dashboard --example simple -- \
  --dashboard \
  -H https://staging.example.com \
  -u 50 -t 10m
```

Then open:

```text
http://127.0.0.1:5118/
```

A build without the feature refuses `--dashboard` at startup and tells you to rebuild with `--features dashboard`.

The dashboard's port opens when the Controllers' ports do, once Goose has built a client for every user (which takes longer with more users and fewer cores). With `--no-autostart` users are built on Start, so the port opens at once.

The server stays up until the load test exits, which after an autostarted run can be well after the run ends ([Waiting after an autostarted run](#waiting-after-an-autostarted-run)). Before `GooseAttack::execute()` returns, finished or failed, connected pages get `event: closed`, open connections get up to one second to finish, and the server closes. A program can then run another `GooseAttack` on the same port right away. On the event stream, the `closed` event's data is `1`.

### Related flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--dashboard` | off | Serve the dashboard, read-only |
| `--dashboard-control` | off | Turn on control: Start, Stop, users, the run settings (which can point the load at any host), Quit, the report switches, Save report now, and deleting saved reports and logs. Refused at startup without `--dashboard`, which it doesn't turn on |
| `--dashboard-host HOST` | `127.0.0.1` | Bind address ([Loopback default and auth policy](#loopback-default-and-auth-policy)) |
| `--dashboard-port PORT` | `5118` | Bind port. `0` picks a free port, which the startup log shows: `[dashboard]: listening on http://127.0.0.1:60096 (read-only)`. A port in use stops Goose when it binds, with `[dashboard]: failed to bind ...: ... (is the port already in use?)`; pick another port. On a shared machine, see [Keeping the token private](#keeping-the-token-private) |
| `--dashboard-auth-token TOKEN` | empty | The token for the metric, report and control routes ([What requires a token](#what-requires-a-token)). The [Security matrix](#security-matrix-observe-vs-control) lists when it's required |
| `--dashboard-no-auth` | off | Serve with no token, control included under `--dashboard-control` ([Control without a token](#control-without-a-token)). Refused at startup without `--dashboard` |
| `--dashboard-max-clients N` | `32` | Maximum concurrent clients on the event stream (`GET /api/v1/events`); the next one gets HTTP 503. `0` is refused at startup (`must be at least 1`). The server also holds at most `2 × N + 16` connections (80 by default), and closes any connection beyond that as soon as it's accepted |
| `--no-percentiles-over-time` | off | Turn off the response time heatmap and the histograms behind it, on the dashboard and in the HTML report ([The response time heatmap](#the-response-time-heatmap)). Works without `--dashboard` |

Each has a default you can set in code: `GooseDefault::Dashboard`, `GooseDefault::DashboardControl`, `GooseDefault::DashboardNoAuth`, `GooseDefault::DashboardHost`, `GooseDefault::DashboardPort`, `GooseDefault::DashboardAuthToken`, `GooseDefault::DashboardMaxClients` and `GooseDefault::NoPercentilesOverTime`.

### Observe vs control at a glance

| Capability | Dashboard (read-only) | Dashboard with control | Controllers ([telnet](telnet.md) / [WebSocket](websocket.md)) |
|------------|---------------------|------------------------|----------------------------------|
| Live rates, percentiles, errors | Yes | Yes | `metrics` / `metrics-json` |
| Trailing RPS / users / latency charts | Yes | Yes | No |
| Start / stop / set users | No | Yes | Yes |
| Host / ramps / run time | No | Yes | Yes |
| Test plan | No | Only through `POST /api/v1/control/settings` or `POST /api/v1/control/start` | Yes (`test-plan`) |
| `config` / `config-json` | No | No | Yes |
| Shut Goose down | No | Quit, from Idle only | Yes (`shutdown`, any phase) |
| Default bind | `127.0.0.1:5118` | `127.0.0.1:5118` | `127.0.0.1:5116` / `:5117`, WebSocket only with `--websocket` |
| Auth | [Security matrix](#security-matrix-observe-vs-control) | [Security matrix](#security-matrix-observe-vs-control) | None |

The Controllers stay available alongside the dashboard. Dashboard control uses the same [phase](../glossary.md#attack-phase) transitions they do.

## Bind and token rules

### Loopback default and auth policy

The dashboard binds to loopback (`127.0.0.1`) by default. Goose treats a bind host as loopback when it's:

- `localhost` (any case, with no DNS lookup)
- an IPv4 address in `127.0.0.0/8`, such as `127.0.0.1` or `127.0.0.2`
- `::1`, with or without brackets (`[::1]`)

Goose decides this from the `--dashboard-host` value as written, so a hostname that resolves to a loopback address isn't loopback. Any other bind, including `0.0.0.0`, `::`, a LAN address or a hostname, needs a token (`--dashboard-auth-token` or `GooseDefault::DashboardAuthToken`) or `--dashboard-no-auth`. With neither, Goose refuses to start.

Goose also checks the `Host` header of every request, the page, its static files and `/api/v1/health` included:

| Bind | `Host` it takes, with any port | Refusal |
|------|--------------------------------|---------|
| Loopback, whatever the token setting | `localhost`, an address in `127.0.0.0/8`, or `[::1]` | 403, plain text `host not allowed` |
| Off loopback, under `--dashboard-no-auth` | An IP address (`10.0.0.5:5118`, `[fe80::1]:5118`), `localhost`, or the `--dashboard-host` value | 403, `{ "ok": false, "reason": "host_not_allowed" }` |
| Off loopback, with a token | Any | None |

Where Goose checks `Host`, a missing one is refused. The check blocks DNS rebinding, where a web page points its own domain at the dashboard's address and sends that domain as `Host`.

Under `--dashboard-no-auth`, to browse by a DNS name, set `--dashboard-host` to that name; Goose then binds an address it resolves to.

### Security matrix (observe vs control)

| Bind | Observe (`--dashboard`) | Control (`--dashboard-control`) |
|------|-------------------------|----------------------------------|
| Loopback, no token | Metrics open | Startup error (control needs a token or `--dashboard-no-auth`) |
| Loopback, token set | Metrics need the token | Control needs the token |
| Loopback, `--dashboard-no-auth` | Metrics open | Control open; startup warning; cross-origin requests refused |
| Off loopback, no token | Startup error | Startup error |
| Off loopback, token set | Metrics need the token | Control needs the token |
| Off loopback, `--dashboard-no-auth` | Metrics open; startup warning; page banner | Control open; startup warning; page banner; cross-origin requests refused |
| Any bind, `--dashboard-no-auth` with `--dashboard-auth-token` | Startup error | Startup error |

"Metrics" here means every `GET` route marked Required in [What requires a token](#what-requires-a-token), the report routes included; [On a shared machine](#on-a-shared-machine) shows what they reveal and to whom. There is one token: under `--dashboard-control`, anyone you give it to for watching can also start, stop and redirect the test, and delete saved reports and logs.

### What requires a token

| Path | When a token is set | Notes |
|------|---------------------|-------|
| `GET /` (the HTML shell of the single page app) | Public | |
| `GET /static/app.js`, `GET /static/app.css` | Public | |
| `GET /api/v1/health` | Public | No load test metrics ([Health endpoint exception](#health-endpoint-exception)) |
| `GET /api/v1/snapshot` | Required | |
| `GET /api/v1/events` (SSE) | Required | |
| `GET /api/v1/series` | Required | [Request heatmap endpoint](#request-heatmap-endpoint) |
| `GET /api/v1/reports` and `/api/v1/reports/*` | Required | [Report endpoints](#report-endpoints) |
| `POST /api/v1/control/*` | Required | Only with `--dashboard-control`; Bearer header only ([Endpoints](#endpoints)) |
| `DELETE /api/v1/reports/{name}` | Required | Only with `--dashboard-control`; Bearer header only ([Endpoints](#endpoints)) |

#### Browser bootstrap with `?token=`

A browser's `EventSource` can't set an `Authorization` header, so to open a dashboard that has a token, put the token in the URL:

```text
http://{host}:{port}/?token=SECRET
```

The page reads `token` from the URL and strips it from the address bar with `history.replaceState`. It adds `?token=` to its `/api/v1/snapshot`, `/api/v1/events` and `/api/v1/series` requests.

The page keeps the token in the tab's `sessionStorage` (never `localStorage`). A reload keeps it and closing the tab drops it; a new tab or window on the bare URL needs `?token=` again. A tab duplicated from the browser menu gets a copy of the token, and a browser that restores closed tabs may restore the token too. A token in the URL replaces a stored one. When the server answers 401 to a snapshot, event stream or series request, or to a start, stop, users, quit or settings request, the page drops the stored token, so a stale one (for example after Goose restarts with a new token) isn't sent again. A 401 on the report switches, Save report now, a Delete or a report route only shows the error.

If the page loads without a token when one is required, it still renders, but the metric calls get 401 and the page shows:

```text
Open this dashboard as http://host:port/?token=… (token required for metrics).
```

Other clients can send the token either way for metrics:

```bash
curl -sS -H "Authorization: Bearer $DASHBOARD_TOKEN" http://127.0.0.1:5118/api/v1/snapshot
# or
curl -sS "http://127.0.0.1:5118/api/v1/snapshot?token=$DASHBOARD_TOKEN"
```

### Control without a token

With `--dashboard-no-auth`, the dashboard's control works with no token and no `?token=` URL:

```bash
cargo run --release --features dashboard --example simple -- \
  --dashboard --dashboard-control --dashboard-no-auth \
  -H https://staging.example.com \
  -u 50 -t 10m
```

With no token, the control routes (`POST /api/v1/control/*` and `DELETE /api/v1/reports/{name}`) refuse the requests another site's page could send. A request that sends an `Origin` other than `http://<its Host>`, or lacks `Content-Type: application/json`, gets 403 with `{ "ok": false, "reason": "cross_origin" }`. (A form, a link or a plain `fetch` from another site can't send that content type without the browser first asking the server, and Goose sends no CORS headers, so the browser never sends the request.) The dashboard page sends `Content-Type: application/json` on every control request, with `{}` as the body of a POST that has nothing else to send; its Delete sends the header with no body.

`curl` and scripts send no `Origin`, so they only need the content type:

```bash
curl -sS -H 'Content-Type: application/json' -d '{}' http://127.0.0.1:5118/api/v1/control/start
```

Behind a reverse proxy, control under `--dashboard-no-auth` works only while the page's `Origin` matches the `Host` Goose sees:

| Proxy | Control under `--dashboard-no-auth` |
|-------|-------------------------------------|
| Serves https, or rewrites `Host` | Refused |
| Reached by IP address, serves plain http, passes `Host` unchanged, in front of an off loopback bind | Works |
| Reached by a name that resolves to Goose's machine, serves plain http, passes `Host` unchanged, with `--dashboard-host` set to that name | Works |
| In front of a loopback bind | Refused: the proxy must pass a loopback `Host`, which never matches the page's `Origin`. Use a token |

### Off loopback

Under `--dashboard-no-auth` on a bind off loopback, such as `--dashboard-host 0.0.0.0`, Goose starts and writes this warning to standard error (where neither `-q` nor the log level hides it) and to the Goose log at warn level:

```text
==============================================================================
WARNING: the dashboard on http://0.0.0.0:5118 has no token (--dashboard-no-auth).
Anyone who can reach this port can watch the test, start and stop it,
and point the load at any host.
Safer: bind 127.0.0.1 and use an ssh forward (ssh -L 5118:127.0.0.1:5118 host),
or use --dashboard-auth-token in place of --dashboard-no-auth. Only IP addresses,
localhost and the --dashboard-host name are accepted as Host; set --dashboard-host
to the DNS name you browse to.
==============================================================================
```

Without `--dashboard-control` the second sentence reads `Anyone who can reach this port can watch the test.` The page shows the first two sentences in a banner, with the address the page was opened at. The banner stays up while the page is open.

### Startup lines

```text
[dashboard]: listening on http://127.0.0.1:5118 (control enabled)
[dashboard]: open http://127.0.0.1:5118/?token=<token> to control the test
```

Goose never prints the token; `<token>` is literal. Each line starts with `[dashboard]: `:

| Setup | `listening on` suffix | Second line | Second line's level |
|-------|-----------------------|-------------|-------|
| Read-only, no token | `(read-only)` | `read-only; for control add --dashboard-control and either --dashboard-no-auth (no token) or --dashboard-auth-token <token>` | info |
| Read-only, token set | `(read-only)` | `read-only; for control add --dashboard-control` | info |
| Read-only, `--dashboard-no-auth` | `(read-only, no token)` | `read-only; for control add --dashboard-control` | info |
| Control, token set | `(control enabled)` | `open http://<address>/?token=<token> to control the test` | info |
| Control, `--dashboard-no-auth` | `(control enabled, no token)` | On loopback, `--dashboard-no-auth: anyone on this machine can start, stop and redirect the test` | warn |

Off loopback, `--dashboard-no-auth` adds the warning block in [Off loopback](#off-loopback), which replaces the warn line. On a `0.0.0.0` or `::` bind the listening line shows the bind address and the `open` line shows `127.0.0.1` or `[::1]`.

### Example: bind on all interfaces

```bash
read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN

cargo run --release --features dashboard --example simple -- \
  --dashboard \
  --dashboard-host 0.0.0.0 \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 200 -t 1h
```

On a shared machine, set the token in code instead ([Keeping the token private](#keeping-the-token-private)).

From another machine, open `http://<this machine's address>:5118/?token=<token>`. For encryption, keep the default loopback bind and use an SSH tunnel or a reverse proxy instead ([SSH tunnels](#ssh-tunnels)).

## Exposure and logging

### On a shared machine

Every user of the machine running Goose can reach what Goose serves on loopback. Off loopback, read "Any local user" in the three token rows (with no token set; under `--dashboard-no-auth` with `--dashboard-control`; with a token set) as anyone who can reach the port. The token and the metrics then also cross the network as unencrypted http. The Controllers keep their own binds; the other rows stay local.

| Who | What they can reach | To close it |
|-----|---------------------|-------------|
| Any local process | The telnet Controller, on by default at `127.0.0.1:5116` with no authentication, and every command it takes ([telnet](telnet.md)), `shutdown` included | `--no-telnet` |
| Any local process, or any page in a local browser | The WebSocket Controller, under `--websocket`. It takes no authentication and accepts a connection from any web page | Leave `--websocket` off |
| Any local user, with no token set | The metrics, the run's notes, `--report-file` paths and run settings, and every saved report in the [runs folder](#the-runs-panel) | A token, set in code ([Keeping the token private](#keeping-the-token-private)) |
| Any local user, under `--dashboard-no-auth` with `--dashboard-control` | The row above, plus everything `--dashboard-control` turns on ([Related flags](#related-flags)), the host the load goes to included | A token in place of `--dashboard-no-auth` |
| Any local user, with a token set | The page's HTML and static files, and [`GET /api/v1/health`](#health-endpoint-exception). They can also fill the connection cap and keep your browser out ([Security notes](#security-notes-summary)) | No setting closes the lockout. Run where you trust the other users, or behind a firewall when the bind is off loopback |
| Any local user, through the file system | The run folders under `goose-runs` (reports and logs), written with the process's umask, which usually lets every local user read them | `umask 077`, or an `--output-dir` inside a folder only you can read, such as `~/private/nightly` with `~/private` at mode 700 |
| Any local user, with the token on the command line | The token, in `ps` | Set it in code |

With no token set, any local user can read the request names, hosts, error strings and rates in the snapshot, the event stream and the per request heatmap. Saved reports can be listed and downloaded even when only their owner can read the files. A client holding the event stream open when an autostarted run ends with its report saved keeps Goose and its port up ([Waiting after an autostarted run](#waiting-after-an-autostarted-run)).

#### Keeping the token private

To keep the token off the command line, read it in the load test's code (from an environment variable, for example) and set it as a default. On Linux a process's environment, unlike its arguments, is readable only by its own user and root. Make it long and random, such as `openssl rand -hex 32`, since Goose doesn't limit wrong guesses. An empty token counts as none, so the code below refuses one:

```rust
let token = std::env::var("DASHBOARD_TOKEN")
    .ok()
    .filter(|token| !token.is_empty())
    .expect("set DASHBOARD_TOKEN to a non-empty value");
GooseAttack::initialize()?
    .set_default(GooseDefault::DashboardAuthToken, token.as_str())?
    // register scenarios and run as before
```

Anyone who can read the source or the binary can read a token written into the source. A token can also turn up:

- in a reverse proxy's access log ([SSH tunnels](#ssh-tunnels));
- in a browser tab duplicated or restored from the one that opened the dashboard ([Browser bootstrap with `?token=`](#browser-bootstrap-with-token));
- in `curl` command lines ([curl examples](#curl-examples));
- in your shell history, after `export DASHBOARD_TOKEN=...`. In bash or zsh, `read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN` sets it without echoing it or writing it there.

The token goes to whatever holds the dashboard's port:

- If another user holds the port when Goose binds ([Enabling the dashboard](#enabling-the-dashboard) says when that is), Goose exits with the bind error in [Related flags](#related-flags), and a tunnel or a reloaded tab then sends your token to the other process.
- A tab left open after Goose is killed or crashes keeps retrying with the token until something answers. Close the tab once Goose is gone.

Send the token only while your own Goose is up, as its `listening on` line shows. Under `-q`, which hides it, check that `lsof -iTCP:5118 -sTCP:LISTEN` shows your Goose process.

### Security notes (summary)

- Every response sends `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`, so the page doesn't pass its `?token=` URL on as a `Referer`.
- The page renders metric fields with `textContent` only (no `innerHTML`), and the server sends a strict Content-Security-Policy with no `'unsafe-inline'` scripts and `frame-ancestors 'none'`.
- There are no session cookies and no CORS headers, so classic cross-origin cookie CSRF doesn't apply. Under `--dashboard-no-auth` the control routes check `Origin` and `Content-Type` instead ([Control without a token](#control-without-a-token)).
- A client gets 10 seconds to send a request's headers, on a new connection and between requests on a keep-alive one, and is closed if it sends nothing. Event streams aren't cut, since the timeout covers headers only. The connection cap ([Related flags](#related-flags)) is shared by every client, token or not, so a client that keeps that many connections open, with a request on each at least every 10 seconds, keeps everyone else out until it stops.

### What Goose logs

When the [main loop](#endpoints) answers a control request, Goose logs it at info as `[dashboard]: control <name> ok` or `rejected` with the phase; a request the main loop doesn't answer is logged at warn (`control admission full`, `control timed out`, `control unavailable`).

Goose logs a missing or wrong token on a control POST (`report` and `report/now` included) or a Delete at warn, and each completed Delete at info, or at warn when it fails on disk (a Delete refused with 404 or 409 isn't logged). It doesn't log a missing or wrong token on a metric route or a report download, or any other refusal before the main loop: a malformed body, an out of range value, a body over 16 KiB, or a failed cross-origin or `Host` check. Connections closed at the cap are logged only at debug. No line at info or warn includes the client's address, and Goose never logs the token; a Controller's `config` and `config-json` show a set token as `(set)`.

Info lines go to the console, where `-q` hides them. The `--goose-log` (`-G`) file holds only warn and error lines unless `-g` adds info, so for a record of control requests and Deletes on disk, run with `-G` and `-g`.

## SSH tunnels

Keep Goose on loopback and tunnel to it from your laptop:

```bash
ssh -L 5118:127.0.0.1:5118 loadgen.example.com
```

Then open `http://127.0.0.1:5118/` locally, adding `?token=<token>` when a token is set.

Goose doesn't terminate TLS. For encryption in transit, use an SSH tunnel or a reverse proxy.

Behind a reverse proxy, keep Goose on loopback so its plain http port isn't reachable directly, use a token, and have the proxy pass a loopback `Host` (nginx `proxy_pass http://127.0.0.1:5118` does by default). Its access log can record `?token=`, so keep that log private or don't log query strings. Under `--dashboard-no-auth`, control works behind a proxy only in the cases [Control without a token](#control-without-a-token) lists.

## Control

### Controlling a load test from the dashboard

Control with a token:

```bash
read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN

cargo run --release --features dashboard --example simple -- \
  --dashboard \
  --dashboard-control \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 50 -t 10m
```

Open `http://127.0.0.1:5118/?token=<token>`. The Control card then has **Start**, **Stop**, **Target**, **Edit settings** and, in Idle, **Quit** ([The Control card](#the-control-card)). Without a token, see [Control without a token](#control-without-a-token).

#### `--no-autostart` recipe

`--no-autostart` keeps Goose in Idle until something starts the test. Goose refuses it unless a Controller is on (telnet is by default, WebSocket only with `--websocket`) or dashboard control is on. With dashboard control off, it also refuses when every enabled Controller failed to listen on its port.

```bash
read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN

cargo run --release --features dashboard --example simple -- \
  --no-autostart \
  --no-telnet \
  --dashboard \
  --dashboard-control \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 50
```

Then press **Start**, or send `POST /api/v1/control/start`. When the run ends, Goose goes back to Idle and waits for another Start; press **Quit** or send `POST /api/v1/control/quit` (or a Controller `shutdown`) to exit.

On a shared machine, set the token in code instead ([Keeping the token private](#keeping-the-token-private)).

### Endpoints

The main loop is the Goose task that runs the load test's phases and applies control commands.

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `POST` | `/api/v1/control/start` | empty, `{}`, or run settings | Start the test ([Start and Stop semantics](#start-and-stop-semantics)). With run settings, apply them and start in one step |
| `POST` | `/api/v1/control/settings` | run settings | Apply run settings without starting ([Run settings](#run-settings)) |
| `POST` | `/api/v1/control/stop` | empty or `{}` | Stop all users at once |
| `POST` | `/api/v1/control/users` | `{"users": N}` | Set the target user count, an integer from 1 to 100000 ([Users on a running test](#users-on-a-running-test)) |
| `POST` | `/api/v1/control/quit` | empty or `{}` | Shut Goose down ([Start and Stop semantics](#start-and-stop-semantics)) |
| `POST` | `/api/v1/control/report` | `{"save": B, "graph_data": B}`, one or both, each a boolean | Set Save report and Include graph data ([Saving the report](#saving-the-report)) |
| `POST` | `/api/v1/control/report/now` | empty or `{}` | Save the report now. Only while a run is in progress, otherwise `invalid_phase` |
| `DELETE` | `/api/v1/reports/{name}` | none | Delete a folder's reports and logs ([Report endpoints](#report-endpoints)) |

Send the token as `Authorization: Bearer <token>`; a `?token=` query gets 401, even beside a valid header.

A control request gets one of these:

| Status | Body | When |
|--------|------|------|
| 200 | JSON, `"ok": true` | The main loop applied the command |
| 200 | JSON, `"ok": false`, with `error` from the first table below or a run settings refusal ([Refusals](#refusals)) | The main loop refused the command |
| 204 | Empty | A Delete succeeded |
| 400 | JSON, `"ok": false`, with `error` `bad_request` (a malformed body), `invalid_users` (a `/control/users` count out of bounds), or a run settings refusal ([Refusals](#refusals)) | Refused before the main loop |
| 401 | Empty | The token is missing or wrong, or sent as `?token=` |
| 403 | `{ "ok": false, "reason": "cross_origin" }` | Under `--dashboard-no-auth`, a cross-origin request ([Control without a token](#control-without-a-token)) |
| 403 | `{ "ok": false, "reason": "host_not_allowed" }` off loopback under `--dashboard-no-auth`; plain text `host not allowed` on a loopback bind | A `Host` Goose doesn't take ([Loopback default and auth policy](#loopback-default-and-auth-policy)) |
| 404 | Empty | Control is off, so the route isn't registered (or, for a Delete, a folder the runs panel doesn't list) |
| 405 | Empty | A method the route doesn't take, such as `GET` on a control route |
| 409 | Plain text reason | A Delete Goose refuses ([Report endpoints](#report-endpoints)) |
| 413 | Plain text | A POST body over 16 KiB, checked before the token |
| 500 | Plain text `can't delete the report` | A Delete that fails on disk |
| 503 | JSON, `"ok": false`, with `error` from the second table below | The main loop didn't answer |

For a POST, only a 200 with `ok: true` is success. A 200 body has `field` only when one setting is at fault. A 400 or 503 body has `phase`, `active_users` and `target_users` set to `null`, and `field` set to `null` unless a run settings refusal sets it.

When the main loop refuses a command, `error` is one of these:

| `error` | When |
|---------|------|
| `invalid_phase` | The phase can't take the command |
| `prepare_failed` | A Start can't prepare the load test |
| `conflicting_settings` | The command conflicts with other settings, such as 1 user while an increase time is set |
| `internal` | Applying the command failed inside Goose, which then ends the load test with that error |
| `invalid_report` | Save report turned on under `--no-metrics` or during a run with no folder, or Save report now under `--no-metrics` |
| `report_busy` | Save report now is still writing the report it was last asked for |
| `report_failed` | Save report now can't start writing the report, as during a run with no folder or when its folder can't be written to; `message` has the reason |

Run settings have refusals of their own, `no_host` among them ([Refusals](#refusals)). A report write that fails after an `ok: true` shows up in the snapshot's `report.error`.

When the main loop doesn't answer, `error` is one of these:

| `error` | When | Client action |
|---------|------|---------------|
| `busy` | Four control requests are already waiting for the main loop | Wait and retry |
| `unavailable` | The main loop no longer takes control requests, for example because the load test is exiting | Don't retry |
| `timeout` | The main loop didn't answer within 15 seconds | Read `phase` and `target_users` from a snapshot before retrying. A command still queued at the timeout is skipped, but one the main loop had started may still apply |

A Start is refused with `prepare_failed` and `"field": "host"` when a scenario has no host from `--host`, the scenario itself or the default host. It's also refused that way when a `test_start` or `test_stop` transaction is registered and neither `--host` nor the default host is set, since those transactions run with no scenario host. An autostarted run with the same configuration fails at launch instead.

#### curl examples

The examples below put the token on `curl`'s command line, which `ps` shows to every user. On a shared machine, put the header in a file only you can read and pass it with `-H @<file>` (curl 7.55 or later). Under `--dashboard-no-auth`, drop `-H "$AUTH"`, and send `-H 'Content-Type: application/json' -d '{}'` on every control POST that has no other body (a Delete takes the header alone); GET routes need neither ([Control without a token](#control-without-a-token)).

Each command is a separate example, run against a `--no-autostart` Goose ([`--no-autostart` recipe](#--no-autostart-recipe)). Add `-w '\n%{http_code}\n'` to see the status. In CI, set `DASHBOARD_TOKEN` from the job's secrets instead of `read -rs`. A Stop's `ok: true` means stopping began; wait until a [snapshot](#snapshot-data) reads `"phase": "idle"` before a Start or a Quit.

```bash
read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN
AUTH="Authorization: Bearer $DASHBOARD_TOKEN"
URL=http://127.0.0.1:5118/api/v1

# Start
curl -sS -X POST -H "$AUTH" "$URL/control/start"

# Stop
curl -sS -X POST -H "$AUTH" "$URL/control/stop"

# Start against another host at 20 users per second, holding 10 minutes
curl -sS -H "$AUTH" -H 'Content-Type: application/json' -d '{"host":"https://test.example.com","increase_rate":20,"run_time_secs":600}' "$URL/control/start"

# Change a running users ramp's hold to 15 minutes
curl -sS -H "$AUTH" -H 'Content-Type: application/json' -d '{"run_time_secs":900}' "$URL/control/settings"

# Set the user count
curl -sS -H "$AUTH" -H 'Content-Type: application/json' -d '{"users":100}' "$URL/control/users"

# Turn Save report off
curl -sS -H "$AUTH" -H 'Content-Type: application/json' -d '{"save": false}' "$URL/control/report"

# Save the report now (while a run is in progress)
curl -sS -X POST -H "$AUTH" "$URL/control/report/now"

# Quit (from Idle)
curl -sS -X POST -H "$AUTH" "$URL/control/quit"
```

A Start or a Stop answers:

```json
{
  "ok": true,
  "command": "start",
  "message": "load test started",
  "phase": "increase",
  "active_users": 0,
  "target_users": 50
}
```

```json
{
  "ok": true,
  "command": "stop",
  "message": "load test stopped",
  "phase": "decrease",
  "active_users": 50,
  "target_users": 0
}
```

## Start, stop, users and run settings

A users ramp is a run set by `users` and the ramp settings, with no test plan.

### Start and Stop semantics

Start is accepted only in Idle. A successful Start means the phase has entered Increase. Goose replies as soon as the phase changes, then runs `reset_run_state` (and with it any `test_start` transaction) on the main loop, so a long `test_start` doesn't hold up the 200.

Stop is accepted only in Increase and Maintain. In any other phase, including a configured ramp down or a Users decrease, it answers `invalid_phase`. To stop all users during a ramp down, use a Controller's `shutdown` or Ctrl-C; both also exit Goose.

A successful Stop moves to phase `decrease` and stops all users at once. The decrease rate and time don't apply. When the users have stopped, Goose writes the run's reports as at the end of any run and returns to Idle. Until then the phase is `decrease` with `stopping` true, and Users is refused (`invalid_phase` from the dashboard, false from a Controller).

Quit is accepted only in Idle; in any other phase it answers `invalid_phase`. Stop the test first, or send a Controller `shutdown`.

### Users on a running test

When the user count changes mid run (**Target**, a Controller's `users`, or `POST /api/v1/control/users`), Goose regenerates the steps of a users ramp still ahead for the new count. What happens to the hold (the run time after the ramp up) and the ramp down depends on when the change arrives:

| When Users arrives | What happens |
|--------------------|--------------|
| During the ramp up | Ramp to the new count, hold it for the full run time, then ramp down for the new count. A count equal to the active users keeps that count with no end and no ramp down |
| During the hold | Ramp to the new count; the hold keeps its end. If the ramp ends after the hold's end, or the new count equals the active users, the run keeps that count with no end and no ramp down (`stage` still reads `hold`) |
| During the ramp down | The ramp down is abandoned, and the run holds the new count until stopped. `run_time_secs` is unchanged and `stage` reads `hold_until_stopped` |
| While holding until stopped | Ramp to the new count, and keep holding |

Goose adds users at the increase rate, and removes them at the decrease rate or, without one, the increase rate. With no rate to use, it goes at 1 user per second. Increase and decrease times don't apply to these changes.

The rates can change on a running users ramp too ([Refusals](#refusals) lists what's refused):

| Sent | What happens |
|------|--------------|
| `decrease_rate` | The ramp down still ahead is regenerated at the new rate. On a run with a run time and no ramp down, where all users would stop at once when the hold ends, it adds a ramp down at that rate. On a run holding until stopped, it only sets the rate for the next Users decrease, for the ramp down if a run time is set later, and for the next Start |
| `decrease_rate: null` | Removes a ramp down this run added, so all users stop at once again. Refused with `conflicting_settings` while a ramp down the run started with is still ahead |
| `increase_rate` | Used for the next Users increase. A ramp in progress keeps its pace |

While a test plan runs, Users is refused with `invalid_phase` and `"field": "users"`, because the plan sets users. Edit the plan, or stop the test and set users, which switches to a users ramp.

### Run settings

The page's **Edit settings** form sends these settings ([The Control card](#the-control-card)). A Start with a body applies them and starts in one step, so no other client can change a setting in between. Goose applies a body all or nothing: a refusal (including a Start's `prepare_failed`) leaves every setting, the test plan, the running test and `target_users` as they were.

#### Body

```json
{
  "host": "https://staging.example.com",
  "users": 100,
  "increase_rate": 10,
  "increase_time_secs": null,
  "decrease_rate": 5,
  "decrease_time_secs": null,
  "run_time_secs": 600,
  "test_plan": null
}
```

- A field left out stays as it is, and `null` clears it back to Goose's default (`users` can't be cleared, and `test_plan: null` needs `users`; [Refusals](#refusals)). Clearing `host` goes back to each scenario's host, or the default host. An unknown field is refused.
- `test_plan` switches to a test plan and clears users and the ramp settings (the rates, the times and the run time). `users` switches back and clears the test plan.
- In Idle, a rate and its own time replace each other: a rate alone clears its time, and a time alone clears its rate.

Values have the Controllers' bounds:

| Field | Bound |
|-------|-------|
| `host` | An `http` or `https` url with no user name or password |
| `users` | 1 to 100000 |
| `increase_rate`, `decrease_rate` | 0.01 to 100000 users per second |
| `increase_time_secs`, `decrease_time_secs`, `run_time_secs` | Whole seconds from 1 to 30 days (`0` is refused; send `null` to clear) |
| `test_plan` | The `--test-plan` format, at most 100 steps of up to 30 days each, 4096 bytes, and 1 to 100000 users in total ([Controller commands](telnet.md#controller-commands)) |

#### Refusals

Goose checks these in order and answers the first that applies:

| Status | `error` | When |
|--------|---------|------|
| 400 | `bad_request` | The body isn't JSON, isn't an object, or has a field Goose doesn't know |
| 400 | `invalid_value` | A value of the wrong type or out of bounds, or `users: null` |
| 400 | `conflicting_settings` | `test_plan` with `users` or a ramp setting (clearing a ramp setting beside it with `null` is fine) |
| 400 | `conflicting_settings` | `test_plan: null` without `users` |
| 400 | `conflicting_settings` | A rate and its own time both set to values in one body |
| 200 | `invalid_phase` | A setting set or cleared in a stage that can't change it ([By stage](#by-stage)); `users` in a body on a running test (use `/api/v1/control/users`) |
| 200 | `conflicting_settings` | A ramp setting without `users` while a test plan is configured (`"field": "users"`) |
| 200 | `conflicting_settings` | A rate on a running test while its own time is set (`field` is the rate) |
| 200 | `conflicting_settings` | On a running test, clearing the decrease rate while a ramp down the run started with is still ahead, or clearing the run time while a ramp down is configured |
| 200 | `conflicting_settings` | Under `--iterations`: a run time, decrease rate, decrease time or test plan |
| 200 | `conflicting_settings` | Under `--no-reset-metrics`: a test plan |
| 200 | `conflicting_settings` | In Idle: a ramp time with fewer than 2 users |
| 200 | `no_host` | `host: null` would leave a scenario, or a `test_start` or `test_stop` transaction, with no host |
| 200 | `prepare_failed` | A Start that can't prepare the load test, as without settings |

A refusal puts the setting at fault in `field`: `host`, `users`, `increase_rate`, `increase_time`, `decrease_rate`, `decrease_time`, `run_time` or `test_plan` (the times without `_secs`). A Start refused for its phase, and a `prepare_failed` not caused by a missing host, have no `field`.

The reply to `/control/settings`, or to a Start with a body, carries a `settings` object in the snapshot's format, whether the main loop accepted the new values or refused them. A 400 or 503 carries none; read the settings from the next snapshot.

#### By stage

The columns follow `settings.stage`: `ramp_up`, `hold` and `hold_until_stopped` are a running users ramp, `test_plan` is a running test plan, `ramping_down` is the configured ramp down, and `stopping` is a Stop, `shutdown` or Ctrl-C.

| Setting | Idle | Running, users ramp | Running, test plan | Ramping down | Stopping |
|---------|------|---------------------|--------------------|--------------|----------|
| `host` | yes | no | no | no | no |
| `users` (`/api/v1/control/users`; in a settings body, Idle only) | yes | yes | no | yes, abandons the ramp down and holds | no |
| `increase_rate` | yes | yes ([Users on a running test](#users-on-a-running-test)) | no | no | no |
| `decrease_rate` | yes | yes ([Users on a running test](#users-on-a-running-test)) | no | no | no |
| `increase_time_secs`, `decrease_time_secs` | yes | no | no | no | no |
| `run_time_secs` | yes | yes ([Run time on a running test](#run-time-on-a-running-test)) | no, the plan sets how long it runs | no | no |
| `test_plan` | yes | yes, replaces the ramp | yes, restarts from its first step, ramping from the current user count | yes, replaces the ramp down | no |

### Run time on a running test

An accepted run time goes into the configuration, which `settings.run_time_secs` and the next Start read, and it changes the current run:

| When the run time changes | What happens |
|---------------------------|--------------|
| During the ramp up, before the hold | The hold ahead takes the new run time. With no run time before, a hold is added, then the run ramps down as configured, or stops all users at once |
| During the hold | The hold ends at its start plus the new run time; if that has passed, the ramp down starts at once. If a ramp from a Users change is still running at the new end, the run keeps its count with no end and no ramp down (`stage` still reads `hold`) |
| While holding until stopped | The run holds from now for the new run time, then ramps down as configured, or stops all users at once |

Clearing it (`null`, or `runtime 0` from a Controller) on a running test holds until stopped, and is refused with `conflicting_settings` while a ramp down is configured. The telnet and WebSocket Controllers' `runtime` follows the same rules.

### Run settings in the snapshot

Each snapshot carries `settings`: what the next Start runs, and where the running test is. Every tab sees a change within about a second.

```json
"settings": {
  "host": "https://example.com",
  "hosts": ["https://example.com/"],
  "users": 100,
  "increase_rate": 10.0,
  "increase_time_secs": null,
  "decrease_rate": null,
  "decrease_time_secs": null,
  "run_time_secs": 600,
  "test_plan": null,
  "hold_ends_in_secs": 412,
  "stage": "hold"
}
```

Names are the command line's flags in snake case, durations are whole seconds with a `_secs` suffix, and `null` means unset.

| Field | Meaning |
|-------|---------|
| `host` | The `--host` override, or `null` |
| `hosts` | The hosts the next Start will load test, sorted and deduplicated. Each scenario adds `--host`, else its own host, else the default host, or nothing when it has none. The form matches the top level `hosts` (`http://localhost/`, with the slash), which still means the current or last run's hosts |
| `users` | The configured user count, `null` only with a test plan. (`target_users` is the count the current step ramps toward.) |
| `increase_rate`, `decrease_rate` | Users per second |
| `increase_time_secs`, `decrease_time_secs` | The ramp up or ramp down as a time |
| `run_time_secs` | The hold at full users after the ramp up, not the total run time |
| `test_plan` | The test plan as `--test-plan` takes it, in canonical form with each step's duration in whole seconds, so `10,1h30m` reads `10,5400` |
| `hold_ends_in_secs` | Seconds until the hold ends, once it has a fixed end. `0` while a ramp from a Users change runs past the hold's end, and still `0` once the run keeps that count with no end. `null` at any other time |
| `stage` | Where the run is: `idle`, `ramp_up`, `hold`, `hold_until_stopped`, `ramping_down` or `stopping` for a users ramp; `idle`, `test_plan` or `stopping` for a test plan ([Users on a running test](#users-on-a-running-test) covers a Users change that runs past the hold's end) |

`phase` alone can't tell the stages apart: `decrease` with `stopping` false is a Users decrease, the configured ramp down, or a test plan step to fewer users.

## The page in detail

### What the UI shows

[The tour](dashboard.md) walks through each card; this section has the rules behind them.

The phase badge is gray in `idle`, blue in `increase`, green in `maintain`, orange in `decrease` and red in `shutdown`. The connection dot is green on SSE, yellow when the page has fallen back to polling (the event stream was refused, as past `--dashboard-max-clients`, or kept failing), red when disconnected and gray once Goose has exited.

The header's hosts are the current or last run's. In Idle they read `Last run: …` when the next Start would load test other hosts. With control off, the header also shows the run settings summary ([The Control card](#the-control-card)). While a test runs the summary leaves out the hosts; in Idle it shows the next Start's hosts only when they differ from the last run's. The subtitle reads `Live dashboard · control` with control and a token, `Live dashboard · control, no token` with control under `--dashboard-no-auth`, and `Live dashboard · read-only` without control, and stays `Live dashboard` when `GET /api/v1/health` fails. The footer shows the address each Controller listens on, as `GET /api/v1/health` reports it, or says none is running.

The Users tile shows the active users, then the target in smaller type. RPS shows whole requests a second from 100 up and two decimals below; hover it for the two decimals the tables show.

Every figure except the Users tile, the charts and the heatmap covers the whole run, from the start of the test or the last metrics reset. The tiles read `p95 ms (whole run)` and `Avg ms (whole run)`, the totals card is titled **Whole run**, and hovering a table's `avg ms`, `min ms`, `p50`, `p95`, `p99`, `max ms` or `status codes` heading shows `Whole run`. Goose measures response times to the microsecond; the tiles, the **Whole run** card and the tables show times and averages under 10 ms to the hundredth of a millisecond and from 10 ms in whole milliseconds, rounded half up, so 0.418 ms reads `0.42` and 37.6 ms reads `38` (the heatmap truncates instead; [The response time heatmap](#the-response-time-heatmap)).

The line charts and the heatmap keep the last 300 seconds. The requests and failures chart, the average latency chart and their data tables end at the last complete second. Chart colors follow the system's light or dark theme, and switch with it.

The Scenarios and Transactions tables list every registered scenario and transaction, so a scenario excluded by `--scenarios` or assigned no users stays at 0 runs. Goose cuts the other tables before sending them:

| Table | Rows sent | Set when rows were cut |
|-------|-----------|------------------------|
| Requests | The 100 with the most requests | `flags.requests_truncated` |
| Errors | The 50 with the most occurrences | `flags.errors_truncated` |
| Transaction errors | The 50 with the most occurrences | `flags.transaction_errors_truncated` |

Transaction errors counts each error a transaction returned that isn't a request error, by transaction (numbered from 1), scenario, name and error. Transactions and Requests show failures per second and minimum and maximum response times. Requests also lists each request's status codes, most frequent first: the top three and how many more, such as `200 9,812 · 503 44 · 500 40 · +2`, with every code on hover. Under `--no-status-codes` the column is empty.

A transaction's time covers its whole function, so it includes `--throttle-requests` delays and any sleep inside it. A scenario's time includes the wait after each transaction.

When `--co-mitigation` has recorded at least one event, the KPI strip, the **Whole run** card and the Requests table show adjusted response times, which include synthetic times that mitigation records for requests a stalled user couldn't send. The tiles read `p95 ms (whole run, adjusted)` and `Avg ms (whole run, adjusted)`, and above the Requests table a note reads `Response times include the synthetic times coordinated omission mitigation adds. Hover a value to see the measured one.` The **Whole run** card shows both. The maximum is adjusted too; the minimum is always measured. As in the printed metrics and the reports, the adjusted figures include every request, with measured times for requests that got no synthetic times.

The report bar and the runs panel are in [Saving the report](#saving-the-report).

### The Control card

With control off (`health.control_enabled` false), the Control card shows [the read-only card](#the-read-only-card). With control on, it shows its controls. When the page doesn't have the token and Goose isn't under `--dashboard-no-auth`, none of them act: **Target** and the settings form show `Enter the control token to change settings`, and a banner reads:

```text
Open this dashboard as http://host:port/?token=… (token required for control).
```

The same banner shows after a 401 on a start, stop, users, quit or settings request.

**Start** is enabled only in Idle and **Stop** only in Increase and Maintain ([Start and Stop semantics](#start-and-stop-semantics)). **Quit** shows only in Idle; after it the status line reads `Goose is shutting down.`

The summary line above the buttons shows what the next Start runs, such as `https://example.com/ · 100 users at 10/s · hold 10 min · then 10/s down`. While a test runs it says where the run is, worded from `settings.stage`: `ramping to 100 users at 10/s`, `ramp down in 6 min 52 s` during the hold, `holding until stopped`, `ramping down at 10/s`, `stopping`, or for a test plan `test plan: 3 steps, peak 100 users, 1 h 1 min total`. It isn't a live region, so screen readers don't announce the countdown.

**Target** takes a user count: type one and press **Apply**, or press **−** or **+**, which change it by the **step** value next to them (10 by default) and apply at once. Active users is a read-only label from the latest snapshot.

In Idle the field shows the users the next Start runs; with a test plan it's empty, and applying a count replaces the plan with a users ramp. While running it shows the count the run ramps toward.

The field follows the server until you edit it. An edit holds until its Apply succeeds and a snapshot reports it, or until another client changes the count first; the field then shows the new count and the status line says it changed elsewhere. An unapplied edit is discarded, with one note in the status line, when the run starts stopping or a test plan starts. While `stopping` is set or a test plan runs, or the page doesn't have the token, the field and step are read only, with the reason under them.

**Edit settings** opens a form with Host, Ramp up and Ramp down (each users per second, or a total time in s, min or h) and Run time. It starts open in Idle and closed while running, and once you open or close it, the browser remembers your choice. Times show in the largest unit that divides them evenly, so `120 s` reads `2 min`.

Fields follow the server until you edit them, as Target does. Edits stay pending: the form reads `Edit settings (N unapplied)`, **Apply settings** sends them, **Discard edits** drops them, and in Idle **Start** reads **Start with changes** and sends them with the start. An unapplied Target count is an edit too: on its own it turns **Start** into **Start with changes**, and N includes it. An emptied field goes out as `null`, an untouched one is left out, and a number the browser can't read is refused on the page and doesn't go out as `null`. While other edits are pending in Idle, the Target count goes with them: Target's **Apply** is off, and **−** and **+** only change the field.

The first host change in a page load asks for confirmation: `Load will go to …` appears under Host, with **Confirm** and **Cancel** buttons. While it's open, editing Host changes the destination it shows. It closes without sending if the edits stop changing the host, or if the button that opened it (**Apply settings** or **Start with changes**) can no longer act.

While a users ramp runs, a ramp set by rate takes only a new rate, a ramp set by time is read only (`Set by time: 5 min; change it when idle`), and the run time stays editable, with `held 3 min 8 s of 10 min` under it during the hold. Fields the stage can't change are read only, with the reason under them, and an edit the stage can no longer take is discarded, with one note in the status line. When a refusal gives a `field`, the form opens, marks the field and moves focus to it. A test plan can't be set here; send `test_plan` instead ([Run settings](#run-settings)).

The status line shows the server's `message`, in red for a refusal; a 503 says control is busy or timed out, or shows the server's message. A 403 from the cross-origin guard reads `Refused: the request's Origin doesn't match this dashboard's address (another site, or a proxy)`, and one from the `Host` check reads `Refused: open the dashboard by its IP address or its --dashboard-host name`. [Control without a token](#control-without-a-token) lists the proxy setups that work.

### The read-only card

Without `--dashboard-control` the Control card shows this in place of its controls:

```text
Read-only: this dashboard can't start, stop or change the test.
To control it from here, restart Goose with --dashboard-control --dashboard-no-auth (no token), or with --dashboard-control --dashboard-auth-token <token> and open http://127.0.0.1:5118/?token=<token>.
Control this test via the telnet controller listening on 127.0.0.1:5116.
```

The URL in the second sentence uses the address the page was opened at. When the page has a token, or the dashboard runs with `--dashboard-no-auth`, the second sentence reads `To control it from here, restart Goose with --dashboard-control added.` The last sentence lists whichever of the telnet and WebSocket Controllers are listening, and is left out when neither is. Screen readers get the notice as a note named `Read-only dashboard`; nothing in it takes focus. When `GET /api/v1/health` fails, the card reads `Couldn't check whether control is on (/api/v1/health failed). Reload to retry.` instead.

### The response time heatmap

The **Response times by second** card is a heatmap. Each column holds the requests that started in that second. While a test runs, the heatmap doesn't draw the two newest seconds. The rows are 100 microseconds wide under 1 ms, 1 ms up to 64 ms, 2 ms up to 100 ms, 10 ms up to 500 ms, 50 ms up to 1 s, 100 ms up to 5 s and 1 s up to 61 s: 237 rows. The row from 100 ms holds 100 to 109 ms, and the row from 1.4 s holds 1,400 to 1,499 ms. Each row is made of whole buckets of the histogram Goose keeps a run's times in. A bucket that crosses a row's edge is drawn in the row where it ends, so a time just under a row's upper edge can land in the row above. Past 61 seconds (possible with a longer `--timeout`) each row is a single bucket, 0.8% to 1.6% as wide as the times it holds.

Each row is as tall as the range of times it holds on the logarithmic axis, which is marked at 0.1, 0.2 and 0.5 ms, 1, 2, 5, 10, 20, 50, 100, 200 and 500 ms, and 1, 2, 5, 10, 20 and 60 s, as far as the data reaches. Seconds along the bottom are labelled at round steps from the start of the test (1, 2, 5, 10, 15, 30, 60, 120 or 300 seconds, the smallest that gives at most eight labels), so a label stays on its second as the window slides.

Cell color runs on a logarithmic scale from the sparsest cell drawn to the busiest. The legend shows those two counts at its ends and is marked `(log scale)`. A cell of one request still stands out; a second with no requests is empty. The tooltip shows the second, the row's range, how many requests the cell holds and their share of the second, then that second's p50, p95 and p99. A row under 1 ms reads as its range, such as `0.9 to 1 ms`.

The p50 line is solid, p95 dashed and p99 dotted, in one color and width, each stroke shown in the legend. Each is labelled at the right edge with its newest value, such as `p95: 81 ms`, and lines with the same value share a label (`p95, p99: 4 ms`). A second's percentiles come from its cells' counts by the rule Goose uses for whole run figures: the rank is the second's request count times the percentile (0.95 for p95), rounded, and the value is the first row, from the fastest, where the running count reaches that rank. The point sits at the middle of that row, and a line breaks at a second with no requests. The labels, the tooltip, the chart summary and the data table truncate the value, to a tenth of a millisecond under 1 ms and to whole milliseconds from 1 ms, so it always falls inside its row.

The select at the top of the card shows one request alone, and **All requests** goes back. Goose keeps per second times for the first 100 request names it records a response time for, counting again from each metrics reset. The select lists those of them that are among the Requests table's rows. A name recorded after the first 100 counts only toward **All requests**. The select is sorted by method, then name, and isn't rebuilt while it has focus, so a request that appears then is added when focus leaves.

When coordinated omission mitigation has recorded an event, **Adjusted** (the default, matching the KPI strip and the tables) counts the synthetic times that mitigation adds, and so do the lines; **Measured** leaves them out. Synthetic times belong to the seconds of the stall, which Goose only learns of when the stall ends, so cells already drawn can change.

Under `--no-percentiles-over-time` the card shows the average latency chart instead.

## Snapshot data

`GET /api/v1/snapshot` returns the current state. `phase` is `idle`, `increase`, `maintain`, `decrease` or `shutdown`; `stopping` is true while a Stop, `shutdown` or Ctrl-C finishes; `active_users` and `target_users` are the counts. With `AUTH` and `URL` set as in [curl examples](#curl-examples):

```bash
curl -sS -H "$AUTH" "$URL/snapshot"
```

It returns 503 with an empty body when no snapshot arrives within 5 seconds (the build itself gets 4; see `snapshot_timeouts` in [Health endpoint exception](#health-endpoint-exception)). That happens while the main loop or the metrics processor is busy (building the users of a large `users` change, say) and while the dashboard shuts down. Retry after a short wait.

### Routes and top level fields

In 0.19 `/api/v1` serves `GET /api/v1/health`, `GET /api/v1/snapshot`, `GET /api/v1/events`, `GET /api/v1/series`, `GET /api/v1/reports`, `GET /api/v1/reports/{name}/{file}` and `GET /api/v1/reports/{name}/compare.md`. With `--dashboard-control` it adds the control routes in [Endpoints](#endpoints).

A snapshot's top level fields are `version`, `generated_at`, `goose_version`, `phase`, `stopping`, `duration_secs`, `active_users`, `maximum_users`, `target_users`, `total_users`, `hosts`, `aggregate`, `requests`, `errors`, `transaction_errors`, `scenarios`, `transactions`, `series`, `flags`, `report` and `settings`. [Run settings in the snapshot](#run-settings-in-the-snapshot) covers `settings`, and [Heatmap data](#heatmap-data) covers `series.heatmap`.

### Heatmap data

The snapshot's `series.heatmap` carries the heatmap, or `null` under `--no-percentiles-over-time`:

| Field | Meaning |
|-------|---------|
| `generation` | Which set of histograms the columns come from. It changes when they are cleared, by a metrics reset or a new test, and a client then starts over |
| `rev` | The newest change the columns include |
| `since` | The `rev` these columns follow on from, or `0` for every column of the window |
| `start_second`, `end_second` | The window, in seconds from the start of the test. Columns before `start_second` are dropped |
| `hold_from` | Seconds from this one on are still filling and not drawn |
| `buckets_us` | `[time, lowest, highest]` in microseconds for each row index: the time the row stands for (its middle), then the lowest and highest time the row holds. A longer table replaces a shorter one; entries never change |
| `columns` | The columns that changed: `{ "second", "measured": [index, count, ...], "synthetic": [index, count, ...] }`, each non empty row as its index in `buckets_us` and a count, `synthetic` empty unless mitigation has recorded an event |

Each SSE snapshot carries only the columns changed since the previous one, the same for every client. A client holding another `rev` (after a reconnect, for example) fetches what it lacks with `GET /api/v1/snapshot?heatmap_generation=<generation>&heatmap_since=<rev>`; without those parameters it gets every column of the window. The bundled page does this itself.

Size depends on how widely response times spread. With 300 users at about 26 requests a second over 3 request names, each SSE snapshot's heatmap was about 7 KB (most of it `buckets_us`) and a whole 92 second window 12.4 KB. When times spread over all 237 rows, the columns and a whole window grow many times larger. Each SSE snapshot grows far less, since it carries only the changed columns and `buckets_us` is still most of it.

### Request heatmap endpoint

`GET /api/v1/series?method=<method>&name=<request name>` returns one request's heatmap, over the same window and in the same shape as `series.heatmap`. The card fetches it when a request is chosen. `method` is the method as the Requests table shows it (`GET`, `POST`, ...) and `name` the request name; URL encode both. Add `heatmap_generation` and `heatmap_since` for only the columns changed since the heatmap you hold.

| Status | Body |
|--------|------|
| `200` | `{ "method", "name", "heatmap": {...} }`, the heatmap in the shape of the snapshot's `series.heatmap` |
| `400` | `{"error":"bad_request"}`: `method` or `name` is missing, or the two together are longer than 2,048 bytes |
| `401` | Empty: the token is missing or wrong |
| `404` | `{"error":"not_found"}`: the request has no heatmap of its own (not one of the first 100 names, or `--no-percentiles-over-time`) |
| `503` | `{"error":"busy"}` with four series requests already in flight, `{"error":"timeout"}` when no answer arrives within 15 seconds, or `{"error":"unavailable"}` when Goose is no longer taking dashboard requests (shutting down) |

A 200 answer is cached for one second for each request and each `heatmap_generation` and `heatmap_since` pair, so several viewers polling one request cost about one export a second.

```bash
curl -sS -H "$AUTH" "$URL/series?method=GET&name=%2F"
```

### Metrics disabled

Under `--no-metrics` the dashboard still serves the page and the health endpoint. Snapshots report `flags.metrics_disabled: true`, with empty request, error, scenario and transaction tables, zero totals and empty series. The Requests, Scenarios and Transactions tables say metrics are disabled.

`--no-transaction-metrics` sets `flags.transaction_metrics_disabled: true` and empties `transactions`; `--no-scenario-metrics` sets `flags.scenario_metrics_disabled: true` and empties `scenarios`. Each table then says its metrics are disabled. Both flags are also true under `--no-metrics`.

## Health endpoint exception

`GET /api/v1/health` needs no token, so a monitor can check the dashboard is up:

```json
{
  "ok": true,
  "version": "<goose package version>",
  "last_build_ms": 2,
  "last_snapshot_ms": 180,
  "snapshot_timeouts": 0,
  "build_count": 42,
  "active_sse_clients": 1,
  "control_enabled": false,
  "auth_required": true,
  "loopback": true,
  "controllers": { "telnet": "127.0.0.1:5116", "websocket": null }
}
```

| Field | Meaning |
|-------|---------|
| `ok` | Always `true` when the handler runs |
| `version` | The Goose crate's package version (`CARGO_PKG_VERSION`) |
| `last_build_ms` | Milliseconds the metrics processor spent on the last published snapshot, draining pending metrics and building it (`0` if none yet) |
| `last_snapshot_ms` | Round trip of the last published snapshot in milliseconds, from the dashboard's request to the published JSON (`0` if none yet) |
| `snapshot_timeouts` | Snapshot builds that didn't finish within 4 seconds, since the process started. A late answer is dropped |
| `build_count` | Successful snapshot builds since the process started |
| `active_sse_clients` | Clients currently holding a slot on the event stream |
| `control_enabled` | Whether the `POST /api/v1/control/*` routes and Delete are registered (`--dashboard-control`) |
| `auth_required` | `false` only under `--dashboard-no-auth`, `true` otherwise. It doesn't say whether metrics need a token: a loopback dashboard with no token reads `true` and serves its metrics to anyone on the machine |
| `loopback` | Whether the dashboard is bound to a loopback address. With this and `auth_required` both false, the page shows the no-token banner ([Off loopback](#off-loopback)) |
| `controllers` | The `host:port` the telnet and WebSocket Controllers listen on, after defaults (`127.0.0.1:5116` and `127.0.0.1:5117` unless set). `null` for a Controller that's off (telnet with `--no-telnet`, WebSocket unless `--websocket` turns it on) or couldn't listen on its port. Both Controllers take no authentication; the field reveals only where they listen |

The health response has no rates, hosts under test, request names or error strings.

`last_snapshot_ms` minus `last_build_ms` is the wait on the main loop and the metrics processor's queue, plus serializing the JSON. The main loop answers the dashboard between steps and sleeps up to 500 ms between them, so that difference can reach about 500 ms; a large `last_build_ms` points at the metrics processor. Goose logs both durations at `debug`, with a `[dashboard]` prefix, when a build completes, and logs a late answer to a timed out request with its round trip.

## Saving the report

[Output files](../getting-started/output-files.md) has the rules for every file Goose writes; the dashboard adds rules 9 to 13.

The report bar's **Save report** switch sets whether this run's `report.html` and `report.json` are written to the run folder, `goose-runs/<YYYYMMDD-HHMMSS>` unless `--output-dir` gives one. It starts on with `--dashboard`, except under `--no-metrics`, which keeps no report. A run without `--dashboard` saves them to its run folder only with a bare `--report-file` (one given without a path); such a run shows up in the runs panel too. The bar also lists the `--report-file` paths, which are always written.

The switches, Save report now and Delete need control ([Related flags](#related-flags)); with it, both switches can change at any time. With no other report to write, turning Save report off keeps memory flat ([GraphData memory cost](#graphdata-memory-cost)). Turn it on mid run and the report has graphs from then on, plus the recent window, and says when its graphs start and why.

**Save report now** writes the report as it stands into the run folder, marked `in_progress`, while the run continues; the report at the end of the run replaces it, even with Save report off. **Include graph data** adds the per second graph data to the JSON report as `--report-graph-data` does.

Without control the switches are read only, so every `--dashboard` run writes its reports and keeps its graph data for the whole run, except under `--no-metrics` or when its folder can't be written to (below).

When a run ends, a banner reads `Report saved in <folder>. Download HTML, JSON or Markdown.` with a button for each format, or `Couldn't save the report: <reason>`. If the folder's name is one the report routes don't take ([Report endpoints](#report-endpoints)), it reads `Report saved in <folder>.` with no buttons. If Save report now fails during the run, the reason shows at the foot of the runs panel.

When `--dashboard` is the only thing asking for a run folder (no output flag is given bare, and there's no `--output-dir`), a folder Goose can't write to doesn't stop the run. Goose logs `<reason>. This run's report is not saved.` at warn before any load and runs with Save report off; a banner reading `Save report is off for this run. <reason>` stays at the top of the page for the whole run, and the switch shows the reason on hover.

### The runs panel

The runs panel looks in the runs folder: the parent of the `--output-dir` folder, or `goose-runs` by default. It lists each folder there with a readable `report.json` or `report.json.gz` and a name the report routes take ([Report endpoints](#report-endpoints)), including runs made without the dashboard. The panel works in Idle too.

Rows are newest first by start time, or by when the report was written if it has none. Each row shows the folder's name, when the run started (in the viewer's local time), its notes, how long it ran, its users, requests and failures, how it ended (`Finished`, `Stopped`, `Ctrl-C`, `Canceled: <reason>` or `Users exited`) and the test. A report still marked in progress shows as `Running` for the run in progress and `Incomplete` for any other (a run that ended without writing its final report). The panel shows 50 rows; **Show all** lists the rest.

The panel reloads when the page opens, when a run starts, after a Delete, and when Goose writes a report into a run folder (at the end of a run and on Save report now; the snapshot's `report.writes` counts these).

Each row has HTML, Markdown and JSON downloads ([Report endpoints](#report-endpoints)). An HTML report rendered from JSON without graph data says the graphs weren't recorded. Downloads send the token in an `Authorization` header, not in the URL. They always save as files.

With `--dashboard-control`, each row also has a **Delete** button. It asks `Delete the reports and logs in <folder>?`, then deletes what `DELETE /api/v1/reports/{name}` deletes, with the same refusals.

Check **Compare** on exactly two rows and **Download comparison** downloads a Markdown report of the newer run against the older, every number showing its change as with [`--baseline-file`](../getting-started/common.md#comparing-against-a-baseline). A run with no requests can't be checked. If the two runs are from different tests, the panel warns `These runs are from different tests, so the comparison may not mean much.`

### Report endpoints

The token rules are in [What requires a token](#what-requires-a-token). Every route reads the disk on request. Goose caches what it parsed of each JSON report in memory, keyed by path, size and modification time, and writes no index.

| Endpoint | Returns |
|----------|---------|
| `GET /api/v1/reports` | `{ "dir": "goose-runs", "reports": [ ... ] }`, newest first, or `500` with plain text `can't list the reports` when the runs folder can't be read. Each entry has the folder's `name`; the run's record from its JSON report (`notes`, `goose_version`, `test`, `started`, `ended`, `ended_by`, `canceled_reason`, `in_progress` and `graphs_start`, where `notes`, `canceled_reason` and `graphs_start` are left out when unset); and `running`, `duration_secs`, `max_users`, `requests` and `failed_requests` |
| `GET /api/v1/reports/{name}/{file}` | `report.html`, `report.md` or `report.json` of folder `{name}`, as an attachment named `goose-<test>-<name>.<ext>`. A format the folder lacks is rendered from its JSON report, or the route answers `422` with a one line reason when it can't be. A report stored gzipped is sent gzip encoded to a client that accepts gzip, and decompressed for any other |
| `GET /api/v1/reports/{name}/compare.md?baseline={name}` | Report `{name}` compared to the baseline, as Markdown named `goose-<test>-<name>-vs-<baseline>.md`, starting with a line that lists both runs and when each started. `422` with a one line reason when a JSON report can't be read or is inconsistent; a run with no requests compares as zeros. `404` without `baseline` |
| `DELETE /api/v1/reports/{name}` | `204` once these are deleted from the folder, plain or gzipped: `report.html`, `report.md`, `report.json`, `goose.log`, the request, transaction, scenario, error and debug logs, under the file names Goose gives them, and any temporary report file a stopped process left; then the folder, if that leaves it empty. `409` with a plain text reason for the folder of the run in progress once it holds a report (`The run in progress writes to this folder.`), the folder a bare `--goose-log` is open in (`Goose is writing its log to this folder.`), or a folder that holds `.goose-run` (`Another Goose process is writing to this folder.`) |

Goose keeps a bare `--goose-log` open in the last run's folder until the next run starts, and keeps `.goose-run` in its run folder for the whole run and for as long as that log is open there. A killed process leaves `.goose-run` behind; remove it by hand to free the folder.

A name is an ASCII letter or digit followed by any mix of ASCII letters, digits, `.`, `_` and `-`. Any other name, a folder that isn't one level down, a symlink, or a folder without a regular JSON report gets a plain `404`. Goose writes the JSON report last, through a temporary file and a rename, so the routes never read a half written JSON report. Reports and comparisons are also sent with `Content-Disposition: attachment` and `Content-Security-Policy: sandbox`, so a report opened anyway runs no script with the dashboard's origin.

With `AUTH` and `URL` set as in [curl examples](#curl-examples):

```bash
curl -sS -H "$AUTH" "$URL/reports"
curl -sS -OJ -H "$AUTH" "$URL/reports/nightly/report.html"
curl -sS -OJ -H "$AUTH" "$URL/reports/nightly-2/compare.md?baseline=nightly"
```

### Waiting after an autostarted run

When an autostarted `--dashboard` run (one without `--no-autostart`) ends on its own, Goose exits unless the run's report was written to its folder (Save report on, or a Save report now during the run) and a client is connected to the event stream; then it stays up so the report can still be downloaded. With `--dashboard-control` it prints:

```text
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Quit in the dashboard or Ctrl-C to exit.
```

and without it:

```text
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Ctrl-C to exit.
```

The phase goes back to `idle` and the test doesn't restart on its own; a Start from the dashboard (with `--dashboard-control`) or a Controller runs another. Goose exits on Quit, Ctrl-C or a Controller `shutdown`; there's no exit timer, so closing the tab doesn't end the wait. Until another run starts, nothing more is written or printed, and `test_stop` doesn't run again. Any client on the event stream when the run ends counts, even a script tailing it.

A run stopped from the dashboard or a Controller prints no `Run finished` line and stays in Idle, as after any Stop. A run canceled with Ctrl-C or aborted by a `--threshold` exits with no wait.

## GraphData memory cost

`--dashboard`, `--report-file` and `--pdf-print-html` each turn on the per second GraphData recording that the HTML report's graphs use; without any of them nothing is recorded. It costs memory even when no page is open.

While a report will be written at the end of the run (to a `--report-file` or `--pdf-print-html` path, or by [Save report](#saving-the-report)), Goose keeps every second of the run: requests, errors, users and average latency. Memory grows with unique request names × run length, the same as for HTML report graphs. Otherwise the dashboard keeps twice its 300 second chart window (10 minutes) and 300 seconds of heatmap, and memory stays flat.

Goose also keeps one response time histogram per second for the last 300 seconds, for the response time heatmap: one for all requests and one for each of the first 100 request names, and the same again for synthetic times while coordinated omission mitigation is on. That doesn't grow with run length. Its size depends on how many request names and distinct response times there are: about 0.2 MB for all requests and 10 MB for 100 names with typical response times, up to 30 MB in the worst case at the default 60 second timeout, and more with a longer `--timeout`. In a test of 1,000 users at about 21,800 requests a second over 25 request names, it added about 3.1 MiB of peak memory and no measurable CPU per request.

Each user sends its metrics in batches, every 100 requests or 250 ms. A batch holds each request name's response times for two seconds, the one it's filling and the next. A request from any other second sends the batch early. While a report will be written, each second is also added once to whole run columns for the HTML report's heatmap, up to 600 for all requests and 60 for each of those 100 names; the columns merge in pairs as they fill, so they stop growing too. `--no-percentiles-over-time` turns off the per second histograms and these whole run columns, along with their small per request cost; the other graph data is still recorded.

With no client on the event stream and no `GET /api/v1/snapshot` in the last two seconds (`/api/v1/health` doesn't count), the dashboard builds no snapshots, so the metrics processor does no extra work beyond recording GraphData. With clients, builds are coalesced to about one a second for all of them.

Each snapshot build runs on the metrics processor task, and the main loop doesn't wait for it. It drains pending metrics, exports the series window, and builds percentile maps for the aggregate, up to 100 request rows, and each scenario and transaction, so its cost scales with unique request names. With tens of thousands of them, use fewer request names or a shorter test, or leave the dashboard off.

## Stability

### Building it in

In 0.19 and 0.20 the dashboard is opt in ([Enabling the dashboard](#enabling-the-dashboard)). The plan is to make it a default feature in 1.0; feedback on 0.19 and 0.20 will decide.

### `/api/v1` in 0.19 and 0.20

Changes are additive where possible (new fields, endpoints and error codes) under snapshot `version: 1`, and the CHANGELOG lists every change to `/api/v1`. A change that isn't additive, such as a field or endpoint removed, renamed or given a different meaning, raises the snapshot `version`, and the CHANGELOG entry flags the new version. 0.20 brings back distributed load testing (Gaggle) and may make such changes so the API can describe more than one worker.

### `/api/v1` from 1.0

Semver covers it. Within 1.x no field, endpoint or error code is removed, renamed or given a different meaning; new ones may be added.

### What the contract is

The contract is the JSON served under `/api/v1` and the snapshot `version`, a top level field in every snapshot. [Routes and top level fields](#routes-and-top-level-fields) lists what 0.19 serves. The Rust types behind it are internal and not part of the public crate API. In any release, a client must ignore fields it doesn't know and must not fail on an error code it doesn't know. The bundled client is built for the one `version` that `snapshot.d.ts` declares (`SNAPSHOT_VERSION` in the Rust module) and refuses any other with the banner `This page reads snapshot version 1 but the server sent <version>. Reload the page.`; a client of your own should check `version` the same way.

## Building the dashboard UI (TypeScript)

The page's sources live under `src/dashboard/static/`. Edit the TypeScript modules in `src/dashboard/static/src/`, never the bundled `app.js`, which the next `npm run build` overwrites. Paths in the table are relative to `src/dashboard/static/`.

| File | Role |
|------|------|
| `src/main.ts` | Entry point: renders each snapshot and wires the modules together |
| `src/connection.ts` | Token handling, SSE stream, poll fallback, snapshot fetch and version check |
| `src/control.ts` | The Control card, its POSTs and the phase badge |
| `src/settings-form.ts` | The Edit settings form (Host, Ramp up, Ramp down and Run time) and its pending edits |
| `src/run-settings.ts` | The run settings summary line, worded from `settings.stage` |
| `src/follow.ts` | Fields that follow the server unless being edited (Target and the settings form) |
| `src/charts.ts` | Chart.js setup and the line charts: requests and failures, active users, and average latency |
| `src/chart-data.ts` | Each line chart's one sentence summary and its data table |
| `src/heatmap.ts` | The response time heatmap: data shaping and drawing |
| `src/heatmap-card.ts` | The Response times by second card: its request select, the Adjusted and Measured toggle, and the average latency chart when percentiles over time are off |
| `src/coadjust.ts` | Which response time to show: coordinated omission adjusted or measured |
| `src/runs.ts` | The report bar, Save report now, the report banners and the runs panel |
| `src/tables.ts` | The shared sortable table (sorting, filtering, empty states) and the scenarios, transactions, requests, errors and transaction errors tables |
| `src/format.ts` | Number and duration formatting, and the small DOM builders the summary and tables share |
| `src/status.ts` | Banner and connection indicator |
| `src/dom.ts` | Element lookup that checks each element's type |
| `snapshot.d.ts` | Generated types of the snapshot JSON, from the Rust structs in the repository's `src/metrics/dashboard_snapshot.rs` |
| `idle-snapshot.mjs` | A complete idle snapshot for the client tests, type checked against `snapshot.d.ts` by `npm run check` |
| `tsconfig.json`, `package.json` | TypeScript type check and esbuild bundle config |
| `*.test.mjs` | Client tests: `token`, `tables`, `runs`, `control`, `settings`, `coadjust`, `format`, `heatmap`, `a11y`, `chrome` and `monitor`. `npm test` first builds `coadjust.ts`, `format.ts`, `heatmap.ts` and `charts.ts` into `test-build/` for the tests that load one module on its own |
| `test-harness.mjs` | The jsdom page loader and stub server the client tests share |
| `app.js` | Bundled output (the modules plus Chart.js), embedded by the Rust server with `include_str!` |
| `index.html`, `app.css` | Shell and styles (not generated) |

`npm run build` bundles `src/main.ts`, the modules it imports and Chart.js into one minified `app.js` with [esbuild](https://esbuild.github.io/). Chart.js comes from npm at the version pinned in `package.json`, with its own TypeScript types, and its license comment stays in the bundle.

Cargo builds don't run npm: the committed `app.js` is what gets embedded, so `cargo build` works without Node.js. After changing a module in `src/dashboard/static/src/` (or the type declarations it uses), rebuild `app.js` and commit it with the sources. `node_modules/` is gitignored; `package.json` and `package-lock.json` are committed.

### Prerequisites

You need [Node.js](https://nodejs.org/) with npm, and network access once to install the dev dependencies (esbuild, Chart.js, TypeScript, and jsdom for the client tests).

### Rebuild

From the repository root:

```bash
cd src/dashboard/static
npm ci
npm run build
```

Then rebuild Goose so it embeds the new script:

```bash
cargo build --features dashboard
```

### Snapshot types

`snapshot.d.ts` declares `DashboardSnapshot`, every struct it holds, and `RequestSeries` (the `/api/v1/series` body). [`ts-rs`](https://crates.io/crates/ts-rs) generates it from the Rust structs, so `tsc` rejects a read of a field the structs don't have. Every `u64` is declared as `number`, which is what `JSON.parse` returns. `tsconfig.json` sets `skipLibCheck` to false, so `tsc` checks the generated file as well.

To add a field: add it to the struct in `src/metrics/dashboard_snapshot.rs` (a new struct also needs `#[cfg_attr(test, derive(ts_rs::TS))]`), regenerate `snapshot.d.ts`, add the field to `idle-snapshot.mjs` (until you do, `npm run check` fails), rebuild `app.js` if a TypeScript module reads it, and list the field in the CHANGELOG ([Stability](#stability)). From the repository root, after `npm ci` ([Rebuild](#rebuild)):

```bash
GOOSE_UPDATE_DASHBOARD_TYPES=1 cargo test --lib dashboard_types_match_rust
cd src/dashboard/static
npm run check
```

Without the environment variable, `dashboard_types_match_rust` compares the committed file with the structs and fails on any difference, even a reordered field, so a struct change without a regenerated `snapshot.d.ts` fails `cargo test` in CI. A second test fails on any serde attribute other than `serde(skip)` (which `ts-rs` honours) and on a hand-written `Serialize` in the snapshot module, since `ts-rs` can't see either.

`ts-rs` is a dev dependency used only by unit tests. It's pinned to an exact version in `Cargo.toml`, since the committed file is compared byte for byte with its output and Goose commits no `Cargo.lock`. Dependabot opens a pull request for each new `ts-rs` release; if `dashboard_types_match_rust` fails on it, regenerate the file on that branch as above and review the diff.

### Type check

esbuild strips the TypeScript types without checking them, so `npm run build` succeeds on code that doesn't type check. `npm run check` runs `tsc` over the modules and `snapshot.d.ts` (`tsconfig.json` sets `noEmit`, so `tsc` writes nothing), then checks `idle-snapshot.mjs` against the snapshot types. From the repository root:

```bash
cd src/dashboard/static
npm run check
```

### Client tests

Most tests load the bundled `app.js` into a jsdom page with a stubbed `fetch`. Build `app.js` first. From the repository root:

```bash
cd src/dashboard/static
npm run build
npm test
```

CI runs `npm ci` and `npm run check`, rebuilds `app.js` with `npm run build` (failing if the committed file differs from the bundle), and runs `npm test`.
