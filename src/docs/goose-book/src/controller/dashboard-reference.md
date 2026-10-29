# Dashboard Reference

This page holds every flag, rule and endpoint of the [Live Dashboard](dashboard.md), and the snapshot fields a client most needs. Start there for what the dashboard shows and how to use it.

> The dashboard is an opt in crate feature: build Goose with `--features dashboard`. [Stability](#stability) says what the dashboard and its `/api/v1` endpoints promise from one release to the next.

Goose can optionally serve a **live web dashboard** while a load test runs. The dashboard streams compact metric snapshots (including trailing RPS, users, and latency series) over Server-Sent Events (SSE), with a short poll fallback if SSE is unavailable.

## Enabling the dashboard

The dashboard is off by default. Compile Goose with the `dashboard` crate feature, then enable observation with `--dashboard`:

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

The dashboard port opens when the Controllers' ports do, once Goose has built a client for every user, which takes longer with more users and fewer cores. With `--no-autostart` users are built on Start, so the port opens at once.

### Related flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--dashboard` | off | Enable the live dashboard HTTP server (observe) |
| `--dashboard-control` | off | Enable the control endpoints and UI: Start, Stop, users, the run settings (which can point the load at any host), Quit, the report switches, Save report now and Delete of saved reports and logs (requires `--dashboard`; does **not** auto-enable the dashboard) |
| `--dashboard-host HOST` | `127.0.0.1` | Bind address. A loopback address needs no token for metrics; any other needs a token or `--dashboard-no-auth`; see [Bind and token rules](#bind-and-token-rules) |
| `--dashboard-port PORT` | `5118` | Bind port. `0` picks a free port, which the startup log names: `[dashboard]: listening on http://127.0.0.1:60096 (read-only)`. |
| `--dashboard-auth-token TOKEN` | empty | Shared secret for metric APIs when configured; required when `--dashboard-control` is set (even on loopback), and for any bind off loopback, unless `--dashboard-no-auth` is. Visible in process listings (`ps`) like any CLI flag; set it in the load test's code with `GooseDefault::DashboardAuthToken` to keep it out of them. |
| `--dashboard-no-auth` | off | Allow dashboard access and control without a token. Anyone who can reach the port can watch the test and download saved reports, and with `--dashboard-control` start, stop and point the load at any host, and delete saved reports and logs; Goose warns at startup with control on loopback, and on anything but loopback. Needs `--dashboard`; refused with `--dashboard-auth-token`. See [Control without a token](#control-without-a-token). |
| `--dashboard-max-clients N` | `32` | Max concurrent SSE clients (`GET /api/v1/events`); further clients receive HTTP 503. `0` is refused at startup (`must be at least 1`). The server also holds at most twice this many connections plus 16 (80 by default); a connection beyond that is closed as soon as it is accepted. |
| `--no-percentiles-over-time` | off | Turn off the response time heatmap and the histograms behind it; the card shows the average latency chart instead. It also turns off the HTML report's response time heatmap, and works without `--dashboard`. See [The response time heatmap](#the-response-time-heatmap). |

Defaults can also be set programmatically with `GooseDefault::Dashboard`, `GooseDefault::DashboardControl`, `GooseDefault::DashboardNoAuth`, `GooseDefault::DashboardHost`, `GooseDefault::DashboardPort`, `GooseDefault::DashboardAuthToken`, `GooseDefault::DashboardMaxClients`, and `GooseDefault::NoPercentilesOverTime`.

> **Feature flag:** the HTTP server is compiled behind the opt-in `dashboard` crate feature (not in default features, same pattern as `pdf-reports`). Enable it with `--features dashboard`. Builds without that feature reject `--dashboard` at startup with a clear rebuild hint.

The server lives as long as the load test, with one exception: see [Waiting after an autostarted run](#waiting-after-an-autostarted-run). When `GooseAttack::execute()` returns, whether the run finished or failed, connected browsers receive `event: closed`, open connections get up to one second to finish, and the listener and every connection are closed before `execute()` returns, so a program can run another `GooseAttack` on the same port right away.

### Observe vs control at a glance

| Capability | Live Dashboard (observe) | Live Dashboard + control | Controllers (telnet / WebSocket) |
|------------|--------------------------|--------------------------|----------------------------------|
| Live rates, percentiles, errors | Yes | Yes | `metrics` / `metrics-json` |
| Trailing RPS / users / latency charts | Yes | Yes | No |
| Start / stop / set users | **No** | Yes | Yes |
| Host / ramps / run time | No | Yes | Yes |
| Test plan | No | Through `POST /api/v1/control/settings` or `POST /api/v1/control/start` only | Yes (`test-plan`) |
| `config` / `metrics` | No | No | Yes |
| Process shutdown | No | Quit, from Idle only | Yes (`shutdown`, any phase) |
| Default bind | `127.0.0.1:5118` | `127.0.0.1:5118` | `127.0.0.1:5116` / `:5117`, WebSocket only with `--websocket` |
| Auth | Token on metric APIs when configured | Token for control unless `--dashboard-no-auth`; metrics follow observe rules | None (see Controller docs) |

When control is off, `POST /api/v1/control/*` returns **404** (routes are not registered). Prefer Controllers for `config`, `metrics` and shutting down a running test; use dashboard control for Start, Stop, users and the run settings from a browser.

## Bind and token rules

### On a shared machine

Every user of the machine running Goose can reach what Goose serves on loopback. This is what each of them can do, and how to close it:

| Who | What they can reach | To close it |
|-----|---------------------|-------------|
| Any local process | The telnet Controller, on by default on `127.0.0.1:5116` with no authentication: start, stop, change users, the host, the ramps, the run time and the test plan, read `config` and `metrics`, and shut Goose down | `--no-telnet` |
| Any local process, or any page in a local browser | The WebSocket Controller, when `--websocket` turns it on; it takes no authentication and accepts a connection from any web page | Leave `--websocket` off |
| Any local user, with no token set | The dashboard's metrics (request names, hosts, error strings, rates) and the run's details (its notes, its `--report-file` paths and its run settings: host, ramps, run time and test plan): snapshots, the event stream, the per request heatmap, and every saved report in the runs folder, to list and download. By holding the event stream open when an autostarted run ends, they also keep Goose and its port up until it is told to quit ([Waiting after an autostarted run](#waiting-after-an-autostarted-run)) | A token, set in code as below |
| Any local user, under `--dashboard-no-auth` with `--dashboard-control` | Everything in the row above, and Start, Stop, users, the run settings (including the host the load goes to), Quit, the report switches, Save report now, and Delete of saved reports and logs | A token in place of `--dashboard-no-auth` |
| Any local user, with a token set | The page's HTML and static files, and [`GET /api/v1/health`](#health-endpoint-exception): among other fields, the Goose version, whether control is on, the Controllers' addresses and how many pages are watching. They can also fill the connection cap, which every client shares, token or not: a user who keeps `2 × --dashboard-max-clients + 16` connections open, sending a request on each at least every 10 seconds, keeps your browser out until they stop | The metrics are already closed. No setting closes the lockout: run where you trust the other users, or behind a firewall when the bind is off loopback |
| Any local user, through the file system | The run folders under `goose-runs` (reports and logs), which Goose writes as any file, with the process's umask, so a usual umask lets every local user read them | Run Goose under `umask 077`, or keep the runs under a folder only you can read, such as `--output-dir ~/private/nightly` with `~/private` at mode 700: a second run with the same `--output-dir`, once that folder holds files, writes a new folder beside it, `nightly-2` |
| Any local user, with the token on the command line | The token itself, in process listings (`ps`) | Set it in the load test's code with `GooseDefault::DashboardAuthToken`, as below |

With the dashboard off loopback, "local" in the dashboard rows of this table becomes everyone who can reach its port (the telnet and WebSocket Controllers keep their own binds), and the token and the metrics cross the network as unencrypted http.

To keep the token off the command line, read it in the load test's code, for example from an environment variable, and set it as a default. Make the token long and random, such as the output of `openssl rand -hex 32`: Goose does not limit wrong guesses on any route, and logs them only on the control routes and Delete. An empty token counts as none, so refuse one, as this does:

```rust
let token = std::env::var("DASHBOARD_TOKEN")
    .ok()
    .filter(|token| !token.is_empty())
    .expect("set DASHBOARD_TOKEN to a non-empty value");
GooseAttack::initialize()?
    .set_default(GooseDefault::DashboardAuthToken, token.as_str())?
    // register scenarios and run as before
```

A token written into the source instead is readable by anyone who can read the source or the binary. The token can also turn up in other places: in a reverse proxy's access log, as `?token=` on the first page load (`/?token=`) and on the page's metric URLs (see [Security notes](#security-notes-summary)); in a browser tab duplicated or restored from the one that opened the dashboard (see [Browser bootstrap with `?token=`](#browser-bootstrap-with-token)); and in `curl` command lines, which `ps` shows like any other (see [curl examples](#curl-examples)). An `export DASHBOARD_TOKEN=...` typed at a shell also lands in its history; `read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN` in bash or zsh sets it without echoing it or writing it there. On Linux a process's environment, unlike its arguments, is readable only by its own user and root.

The token also goes to whatever listens on the dashboard's port. If another user holds that port when Goose binds it, after building its users or at once with `--no-autostart`, Goose logs `[dashboard]: failed to bind ...: ... (is the port already in use?)` and exits, and the other process keeps the port: a tunnel or a reloaded tab then sends your token to it. So does a tab left open after Goose stops without a clean exit (killed or crashed): the page keeps retrying the event stream and polling, with the token, until it gets through. Send the token only while your own Goose is up, as its `listening on` line shows. That is an info line, which `-q` hides; under `-q`, check that `lsof -iTCP:5118 -sTCP:LISTEN` names your own Goose process before you send the token. Close the dashboard tab once Goose is gone, and pick another port with `--dashboard-port` when the default is taken.

So on a shared machine: keep the dashboard on loopback, set a token in the load test's code, add `--no-telnet` unless you need the telnet Controller, leave `--websocket` off, run under `umask 077` or keep the run folders under a folder only you can read, reach the dashboard from your own machine over an [SSH tunnel](#ssh-tunnels), and send the token only once your own Goose holds the port, as above.

### Loopback default and auth policy

By default the dashboard binds to **loopback only** (`127.0.0.1`). That is intentional: metrics snapshots include request names, error strings, hostnames and rates, and the run's notes, `--report-file` paths and run settings; none of it should be world-readable.

Goose treats a bind host as loopback when it is:

- `localhost` (case-insensitive, no DNS lookup)
- any IPv4 address in `127.0.0.0/8` (for example `127.0.0.1`, `127.0.0.2`)
- IPv6 loopback `::1` (with or without brackets: `[::1]`)

**Non-loopback** binds, including unspecified addresses that listen on all interfaces (`0.0.0.0`, `::`) and LAN addresses and hostnames, **require** a token at startup (`--dashboard-auth-token` or `GooseDefault::DashboardAuthToken`), unless `--dashboard-no-auth` is set. Goose refuses to start, with an error, if neither is given.

If a token **is** configured, it is enforced, even on loopback, on the metric APIs: every metric and report route that [What requires a token](#what-requires-a-token) lists.

On a loopback bind every request must carry a loopback `Host` header (`localhost`, an address in `127.0.0.0/8` or `[::1]`, with any port), whatever the token setting; anything else, a missing `Host` included, gets **403** with the plain text body `host not allowed`. This covers every route, the page, its static files and `/api/v1/health` included. This stops DNS rebinding: a web page that points its own domain at 127.0.0.1 sends its own domain as `Host`, so it cannot read snapshots from a dashboard that has no token. A reverse proxy in front of a loopback bind must pass a loopback `Host` (nginx `proxy_pass http://127.0.0.1:5118` does by default). Whether a bind is loopback is decided by the `--dashboard-host` value as written, by the list above. A hostname that resolves to a loopback address does not count as loopback: this `Host` check does not apply to it, and it needs a token like any other non-loopback bind.

### Security matrix (observe vs control)

| Bind | Observe (`--dashboard`) | Control (`--dashboard-control`) |
|------|-------------------------|----------------------------------|
| Loopback, no token | Metrics OK without auth | **Startup error** (control needs a token or `--dashboard-no-auth`) |
| Loopback, token set | Metrics require token | Control requires token |
| Loopback, `--dashboard-no-auth` | Metrics OK without auth | Control open without a token; cross site requests refused |
| Non-loopback, no token | **Startup error** | **Startup error** |
| Non-loopback, token set | Metrics require token | Control requires token |
| Non-loopback, `--dashboard-no-auth` | Metrics open; startup warning, page banner, Host rule | Control open without a token; startup warning, page banner, Host rule, cross site requests refused |
| Any bind, `--dashboard-no-auth` with `--dashboard-auth-token` | **Startup error** | **Startup error** |

"Metrics" in this table means every route the token guards when one is set: the snapshot, the event stream, the per request heatmap and the saved reports, which can be listed and downloaded. [What requires a token](#what-requires-a-token) lists them. Every loopback bind also checks `Host` on every route, as [Loopback default and auth policy](#loopback-default-and-auth-policy) explains; the "Host rule" in the table is the check off loopback under `--dashboard-no-auth`, in [Off loopback](#off-loopback), and "cross site requests refused" is the check in [Control without a token](#control-without-a-token).

Additional control rules:

- `--dashboard-control` without `--dashboard` → **startup hard-fail** (control does not auto-enable the dashboard). The same goes for `--dashboard-no-auth` without `--dashboard`.
- Without `--dashboard-no-auth`, control endpoints always require a non-empty `--dashboard-auth-token`, even on loopback, so other local processes cannot POST start/stop/users without the secret. The telnet Controller, on by default, takes the same commands from any local process with no authentication; see [On a shared machine](#on-a-shared-machine).
- Startup log shows capability at a glance: `(read-only)`, `(read-only, no token)`, `(control enabled)` or `(control enabled, no token)`; see [Startup lines](#startup-lines).

### Startup lines

Goose says at startup what the dashboard allows and how to get control. Read-only:

```text
[dashboard]: listening on http://127.0.0.1:5118 (read-only)
[dashboard]: read-only; for control add --dashboard-control and either --dashboard-no-auth (no token) or --dashboard-auth-token <token>
```

With a token already set, the second line reads `read-only; for control add --dashboard-control`, as `--dashboard-no-auth` would then be refused.

Control with a token, which is never printed; `<token>` is the literal placeholder:

```text
[dashboard]: listening on http://127.0.0.1:5118 (control enabled)
[dashboard]: open http://127.0.0.1:5118/?token=<token> to control the test
```

On an unspecified bind, `--dashboard-host 0.0.0.0` or `::`, the listening line names the bind address, and the line to open names `127.0.0.1` or `[::1]`, which a browser on the same machine can open: `open http://127.0.0.1:5118/?token=<token> to control the test`. From another machine, use this machine's address instead.

Control with `--dashboard-no-auth` on loopback, the second line at warn level:

```text
[dashboard]: listening on http://127.0.0.1:5118 (control enabled, no token)
[dashboard]: --dashboard-no-auth: anyone on this machine can start, stop and redirect the test
```

Read-only with `--dashboard-no-auth`, the listening line reads `(read-only, no token)` and the second line `read-only; for control add --dashboard-control`. Off loopback, `--dashboard-no-auth` adds the warning in [Control without a token](#control-without-a-token).

### Control without a token

`--dashboard-no-auth` lets the dashboard start, stop, quit and change users, the run settings and the report switches, save the report now, and delete saved reports and logs, with no token and no `?token=` URL:

```bash
cargo run --release --features dashboard --example simple -- \
  --dashboard --dashboard-control --dashboard-no-auth \
  -H https://staging.example.com \
  -u 50 -t 10m
```

Open `http://127.0.0.1:5118/`. The header reads `Live dashboard · control, no token`, and the Control card works as with a token. On loopback anyone on the machine can use it, as with a read-only dashboard with no token; with control on, Goose warns at startup.

`--dashboard-no-auth` can't be given with `--dashboard-auth-token`, and needs `--dashboard`. Without `--dashboard-control` it keeps the dashboard read-only and lifts the token rule for a non-loopback bind.

With no token, nothing but the browser's own rules keeps another web page from sending the dashboard a request, so the control routes (`POST /api/v1/control/*` and `DELETE /api/v1/reports/{name}`) refuse what another site's page can send. A request with an `Origin` other than `http://` followed by its own `Host`, or without `Content-Type: application/json`, gets **403** with `{ "ok": false, "reason": "cross_origin" }`. A form, a link and a plain `fetch` from another site can't send that content type without the browser first asking the server, and Goose sends no CORS headers, so the browser never lets them. The dashboard page sends `Content-Type: application/json` on every control request, with `{}` as the body of a POST with nothing else to send; its Delete sends the header with no body. Behind a reverse proxy that serves the page over https, or that rewrites `Host`, the page's `Origin` no longer matches, and every control request is refused this way.

Under `--dashboard-no-auth`, control works through a proxy only when the page's `Origin` still matches the `Host` Goose sees. Off loopback Goose takes any IP address as `Host`, so a proxy reached by an IP address only has to serve plain http and forward `Host` unchanged. A proxy reached by a name takes three things:

- the proxy serves plain http, under a name that resolves to the machine Goose runs on;
- it forwards that `Host` unchanged;
- Goose runs with `--dashboard-host` set to that name, and binds an address it resolves to.

Such a proxy adds no encryption. For https at the proxy, use a token instead. Behind a proxy to a loopback bind, use a token too: that bind takes only a loopback `Host`, which never matches the page's `Origin` once the proxy is reached by any other name.

`curl` and scripts send no `Origin`, so they need only the content type:

```bash
curl -X POST -H 'Content-Type: application/json' -d '{}' \
  http://127.0.0.1:5118/api/v1/control/start
```

#### Off loopback

On a non-loopback bind, such as `--dashboard-host 0.0.0.0`, anyone who can reach the port can watch the test and, with `--dashboard-control`, start, stop and point the load at any host. Goose starts, and writes this warning straight to standard error, where neither `-q` nor the log level hides it, and to the Goose log at warn level:

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

Without `--dashboard-control` the second sentence reads `Anyone who can reach this port can watch the test.` The page shows the first two sentences in a warning banner that names the address the page was opened at and stays for as long as the page is open.

The safer ways are an [SSH tunnel](#ssh-tunnels) to a loopback bind, or a token.

Under `--dashboard-no-auth`, off loopback the server also checks `Host` on every route: it takes an IP address (`10.0.0.5:5118`, `[fe80::1]:5118`), `localhost`, or the `--dashboard-host` value, with any port, and refuses any other name with **403** and `{ "ok": false, "reason": "host_not_allowed" }`. A page that points its own domain at the load generator (DNS rebinding) sends that domain as both `Host` and `Origin`, so the `Origin` rule alone would let it through. To browse to the dashboard by a DNS name, set `--dashboard-host` to that name; Goose then binds an address that name resolves to.

### What requires a token

| Path | Auth when token configured | Notes |
|------|----------------------------|-------|
| `GET /` (the page's HTML shell; the page is a single page app, SPA) | **Public** | |
| `GET /static/app.js`, `GET /static/app.css` | **Public** | |
| `GET /api/v1/health` | **Public** | Liveness and timing counters; includes `control_enabled` and the Controller addresses (no load-test metrics); see [Health endpoint exception](#health-endpoint-exception) |
| `GET /api/v1/snapshot` | **Required** | |
| `GET /api/v1/events` (SSE) | **Required** | |
| `GET /api/v1/series` | **Required** | One request's response time heatmap; see [Request heatmap endpoint](#request-heatmap-endpoint) |
| `GET /api/v1/reports` and `/api/v1/reports/*` | **Required** | The runs panel's reports; see [Report endpoints](#report-endpoints) |
| `POST /api/v1/control/*` | **Required** when control is enabled, unless `--dashboard-no-auth` | Even on loopback; **404** when control is off |
| `DELETE /api/v1/reports/{name}` | **Required** when control is enabled, unless `--dashboard-no-auth` | Control token, as for `POST /api/v1/control/*`; **404** when control is off |

Under `--dashboard-no-auth` no token is configured, so no route needs one: the metric and report routes are open as on a loopback bind with no token, and the control routes take a request without a token and refuse a cross site one instead; see [Control without a token](#control-without-a-token).

The shell and static assets stay public because classic `<script src>` loads cannot forward a document `?token=` query (or an in-memory secret). Sensitive data lives only in the snapshot, event stream, series and report APIs; mutation lives only on the control routes (the control POSTs and `DELETE /api/v1/reports/{name}`).

#### Browser bootstrap with `?token=`

Browsers' `EventSource` cannot set `Authorization` headers. For remote or token-protected dashboards, open:

```text
http://{host}:{port}/?token=SECRET
```

The embedded UI reads `token` from the URL on load, strips it from the address bar with `history.replaceState`, and attaches `?token=` to `/api/v1/snapshot`, `/api/v1/events` and `/api/v1/series` requests.

The UI keeps the token in the tab's `sessionStorage`, so reloading the page keeps it. A new tab or window opened on the bare URL does not get it and needs `?token=` again, and the token goes away with the tab. A tab duplicated from the browser menu gets a copy, since browsers copy `sessionStorage` when duplicating a tab, and a browser that restores closed tabs may restore it with the tab. It is never written to `localStorage`. A token in the URL replaces a stored one. When the server answers **401** to a snapshot, event stream or series request, or to a start, stop, users, quit or settings request, the UI removes the stored token (a 401 on the report switches, Save report now, a Delete or a report route only shows the error), so a stale token (for example after Goose restarts with a new one) is not sent again on the next reload.

Control POSTs from the SPA use **`Authorization: Bearer` only** (never `?token=` on control routes), to avoid putting the secret on mutating URLs that may appear in proxy logs.

If the shell loads without a token when one is required, the page still renders, but metric calls return **401** and the UI shows:

```text
Open this dashboard as http://host:port/?token=… (token required for metrics).
```

Non-browser clients may use either for **metrics**:

```bash
curl -H "Authorization: Bearer SECRET" http://host:5118/api/v1/snapshot
# or
curl "http://host:5118/api/v1/snapshot?token=SECRET"
```

`GET /api/v1/snapshot` returns **503** with an empty body when no snapshot arrives within five seconds, which happens while the main loop (the Goose task that runs the load test's phases) or the metrics processor is busy (for example building the users of a large `users` change), or when the dashboard is shutting down. Retry after a short wait.

For **control**, the server accepts **`Authorization: Bearer` only** (query `?token=` is rejected on control routes):

```bash
curl -H "Authorization: Bearer SECRET" \
  -H "Content-Type: application/json" \
  -d '{}' \
  -X POST http://host:5118/api/v1/control/start
```

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

From another machine, open `http://<this machine's address>:5118/?token=<token>`, with the token's value in place of `<token>`. On a shared machine, set the token in the load test's code instead of on the command line, where `ps` shows it to every user; see [On a shared machine](#on-a-shared-machine). Goose serves plain http, so the token and the metrics cross the network unencrypted; [SSH tunnels](#ssh-tunnels) avoid that.

### Security notes (summary)

- **Default off**, **default loopback** bind; control is a **separate** opt-in (`--dashboard-control`).
- Non-loopback without a configured token → **startup hard-fail**, unless `--dashboard-no-auth` is set.
- Control without a token → **startup hard-fail**, even on loopback, unless `--dashboard-no-auth` is set.
- Token protects **metric APIs** when configured; control POSTs require the token when control is on, unless `--dashboard-no-auth` is set.
- `--dashboard-no-auth` with `--dashboard-auth-token` → **startup hard-fail**. Under `--dashboard-no-auth` the control routes refuse a cross site request (403 `cross_origin`), and off loopback Goose warns at startup, the page shows a warning banner, and only an IP address, `localhost` or the `--dashboard-host` name is taken as `Host` (403 `host_not_allowed`).
- Shell/static/health stay public and contain no load-test metrics.
- Prefer `Authorization: Bearer` for scripts; browsers use `?token=` for metrics (EventSource limits) and **Bearer only** for control POSTs (with a token set, the server rejects query tokens on `/api/v1/control/*`).
- Query tokens on the first page load and on metric URLs can appear in reverse-proxy access logs; prefer an SSH tunnel, or a reverse proxy whose access log only you can read or that does not log query strings, when that matters. Every response sends `Referrer-Policy: no-referrer`, so the page does not pass its `?token=` URL on as a `Referer`.
- `--dashboard-auth-token` is visible in process listings (`ps`) like any CLI argument; on a shared machine, set the token in code instead, as [On a shared machine](#on-a-shared-machine) shows.
- Goose never logs the token value; the startup lines name the `<token>` placeholder, never the secret. See [What Goose logs](#what-goose-logs).
- The UI renders metric fields with `textContent` only (no `innerHTML`) and serves a strict Content-Security-Policy without `'unsafe-inline'` scripts.
- No session cookies and no CORS headers; classic cross-site cookie CSRF does not apply. With no token, under `--dashboard-no-auth`, the control routes check `Origin` and `Content-Type` instead.
- Every response sends `Cache-Control: no-store` (the first page load carries `?token=` and the APIs carry metrics) and `X-Content-Type-Options: nosniff`.
- On a loopback bind, requests whose `Host` is not loopback get 403 (DNS rebinding).
- A client gets 10 seconds to send a request's headers, on a new connection and between requests on a keep-alive one, and the server holds at most `2 × --dashboard-max-clients + 16` connections, so a client that sends nothing is closed after 10 seconds and the number of open connections is bounded. The cap is shared by every client, with a token or without one, so a client that holds that many connections open keeps every other client out; Goose logs each connection it closes this way only at debug level. Long-lived SSE streams are not cut: the timeout covers headers only.

### What Goose logs

The main loop is the Goose task that runs the load test's phases and applies control commands. Goose logs at info each control request the main loop answers, as `[dashboard]: control <name> ok` or `rejected`, with the phase, and logs at warn each one it can't answer (`control admission full`, `control timed out`, `control unavailable`). Goose does not log a control request it refuses before the main loop sees it: a malformed body, an out of range value, a body over 16 KiB, or one that fails the cross site or `Host` checks (see [Control without a token](#control-without-a-token) and [Loopback default and auth policy](#loopback-default-and-auth-policy)). A missing or wrong token on a control POST (`report` and `report/now` among them) or on a Delete is logged at warn, and each deleted report at info. A missing or wrong token on a metric route or a report download is not logged, and no line at info or warn names the client's address. Goose never logs the token value: the startup lines name the `<token>` placeholder, and a Controller's `config` and `config-json` show a set token as `(set)`.

Info lines go to the console, where `-q` hides them. Without `--goose-log` (`-G`) there is no log file, and the file `-G` writes holds warn and error lines only unless `-g` adds info. For a record of control requests and Deletes on disk, run with `-G` and `-g`.

## SSH tunnels

Prefer keeping Goose on loopback and tunneling from your laptop. On a shared machine, open the tunnel only once your own Goose holds the port ([On a shared machine](#on-a-shared-machine) says how to check):

```bash
ssh -L 5118:127.0.0.1:5118 loadgen.example.com
# open http://127.0.0.1:5118/ locally
```

With a loopback bind and no auth token (observe-only, or control under `--dashboard-no-auth`), no `?token=` is required over the tunnel. If you set a token (required when control is enabled without `--dashboard-no-auth`), pass it in the browser URL; see [Browser bootstrap with `?token=`](#browser-bootstrap-with-token). On a shared machine a loopback bind with no token is open to every local user, so set a token there as well.

TLS termination is not provided by Goose; use an SSH tunnel or reverse proxy when you need encryption in transit. A reverse proxy must pass a loopback `Host` to a loopback bind ([Loopback default and auth policy](#loopback-default-and-auth-policy)); under `--dashboard-no-auth` one that serves https or rewrites `Host` gets every control request refused ([Control without a token](#control-without-a-token)); and its access log records the `?token=` on the first page load and on metric URLs ([Security notes](#security-notes-summary)).

## Control endpoints

### Controlling a load test from the dashboard

Control is **off by default**. Enable it with `--dashboard-control` and a token, as here, or with `--dashboard-no-auth` instead of the token; see [Control without a token](#control-without-a-token):

```bash
read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN

cargo run --release --features dashboard --example simple -- \
  --dashboard \
  --dashboard-control \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 50 -t 10m
```

Open `http://127.0.0.1:5118/?token=<token>`, with the token's value in place of `<token>`; on a shared machine, set the token in code, as [On a shared machine](#on-a-shared-machine) says. The header shows `Live dashboard · control`, and a control panel appears with **Start**, **Stop**, a target-users input (`Apply`, `−` / `+` step buttons), **Edit settings**, which opens a form for the host, the ramps and the run time, and, in Idle, **Quit**.

#### `--no-autostart` recipe

`--no-autostart` keeps Goose in **Idle** until something starts the test. It is allowed when any Controller is enabled (the telnet Controller is on by default, the WebSocket Controller only with `--websocket`) **or** when `--dashboard-control` is set (with dashboard, and a token or `--dashboard-no-auth`):

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

Then Start from the UI or via `POST /api/v1/control/start`. On a shared machine, set the token in code instead, as [On a shared machine](#on-a-shared-machine) says, so `ps` does not show it. Without Controllers **and** without dashboard control, `--no-autostart` is a startup hard-fail (there would be no way to leave Idle). The same goes, with dashboard control off, when the Controllers are enabled but none could listen on its port.

### Endpoints

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `POST` | `/api/v1/control/start` | empty, `{}`, or run settings | Start an idle load test → enters **Increase**; with run settings, apply them and start in one step, all or none |
| `POST` | `/api/v1/control/settings` | run settings | Apply run settings without starting, all or none; see [Run settings](#run-settings) |
| `POST` | `/api/v1/control/stop` | empty or `{}` | Stop all users at once → enters **Decrease** (eventual Idle); accepted only in **Increase** and **Maintain**, else `"ok": false` with `invalid_phase` |
| `POST` | `/api/v1/control/users` | `{"users": N}` | Set absolute target user count (`N` an integer from 1 to 100000) |
| `POST` | `/api/v1/control/quit` | empty or `{}` | Shut Goose down; accepted only in **Idle** |
| `POST` | `/api/v1/control/report` | `{"save": B, "graph_data": B}`, either or both, each `B` a boolean | Set Save report and Include graph data; see [Saving the report](#saving-the-report) |
| `POST` | `/api/v1/control/report/now` | empty or `{}` | Save report now; accepted only while a run is in progress, else `invalid_phase` (`invalid_report` under `--no-metrics`) |
| `DELETE` | `/api/v1/reports/{name}` | none | Delete a folder's reports and logs; see [Report endpoints](#report-endpoints) |

Auth: `Authorization: Bearer <token>` only (query `?token=` → **401**). Missing/wrong token → **401**. Control disabled → **404**. Under `--dashboard-no-auth` no token is needed, and a cross site request gets **403**; see [Control without a token](#control-without-a-token).

#### curl examples

These pass the token on `curl`'s command line, which `ps` shows to every user. On a shared machine, put the header in a file only you can read and pass it with `-H @<file>` (curl 7.55 or later) instead.

```bash
read -rs DASHBOARD_TOKEN && export DASHBOARD_TOKEN
BASE="http://127.0.0.1:5118"

# Start (success means the phase entered Increase, not that test_start finished)
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}' \
  "$BASE/api/v1/control/start"

# Stop (success means the cancel began: the phase is decrease, not idle)
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}' \
  "$BASE/api/v1/control/stop"

# Start against another host at 20 users per second, holding 10 minutes
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"host":"https://staging.example.com","increase_rate":20,"run_time_secs":600}' \
  "$BASE/api/v1/control/start"

# Hold the running test 5 more minutes than it started with
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"run_time_secs":900}' \
  "$BASE/api/v1/control/settings"

# Set absolute user count
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"users":100}' \
  "$BASE/api/v1/control/users"

# Quit (only while idle: Goose shuts down)
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}' \
  "$BASE/api/v1/control/quit"
```

Example success bodies:

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

Logical rejections return **HTTP 200** with `"ok": false` and a stable `error` code: `invalid_phase` when the phase cannot take the command, `prepare_failed` when a Start cannot prepare the load test, `conflicting_settings` when the command conflicts with other settings, such as `users 1` while an increase time is set, `no_host` when clearing the host would leave something with no host, and `internal` when applying the command fails inside Goose, which then ends the load test with that error. The report control routes add three: `invalid_report` when Save report is turned on under `--no-metrics` or during a run with no folder, and for Save report now under `--no-metrics`; `report_busy` while Save report now is still writing the report it was last asked for; and `report_failed` when Save report now can't start writing the report, as during a run with no folder or when its folder can't be written to, with the reason in `message`. A write that fails after an `ok: true` answer shows in the snapshot's `report.error`. When one setting is at fault the body also names it in `field` (for example `"field": "users"`, or `"field": "host"` for a Start with no host); `field` is left out otherwise. A malformed body returns **400** with `error` set to `bad_request`, and a `users` value that is not an integer from 1 to 100000 returns **400** with `invalid_users`; those two 400 bodies, and every 503 body below, have `"field": null`. A run settings body has 400 refusals of its own, which name the field; see [Run settings](#run-settings). A body larger than 16 KiB gets **413** with a plain text body, before the token is checked.

A Start is refused with `prepare_failed` and `"field": "host"` before it replies when some scenario has no host from `--host`, the scenario itself or the default host, and also when a `test_start` or `test_stop` transaction is registered and neither `--host` nor the default host is set, since those run with no scenario host. An autostarted run with the same configuration fails at launch instead.

When the main loop does not answer, a control POST returns **503** with `"ok": false`, an `error` code from this table, and `phase`, `active_users`, `target_users` and `field` set to `null`:

| `error` | When | Client action |
|---------|------|---------------|
| `busy` | Four control requests are already waiting for the main loop | Wait and retry |
| `unavailable` | The main loop no longer takes control requests, for example because the load test is exiting | Do not retry |
| `timeout` | The main loop did not answer within 15 seconds | Re-read `phase` and `target_users` from a snapshot before retrying: a command still queued at the timeout is skipped, but one the main loop had started may still apply |

## Start, stop, users and run settings

### Start and Stop semantics

**Start success means the test entered Increase**, not that `test_start` transactions finished. Goose replies as soon as the phase transitions (same timing as Controllers), then runs `reset_run_state` / user `test_start` on the main loop. A long `test_start` does not block the HTTP 200.

**Stop is accepted only in Increase and Maintain.** In any other phase, a planned ramp down or a users decrease among them, it replies `"ok": false` with `invalid_phase`; to end a ramp down early, use a Controller's `shutdown` or Ctrl-C, both of which also exit Goose.

**Stop is not instantaneous idle.** A successful Stop begins a **cancel** through `AttackPhase::Decrease` (Canceling), which stops all users at once, whatever the decrease rate or time: those shape a users ramp's own ramp down, never a Stop. The process returns to **Idle** only after decrease finishes. While a Stop's cancel runs (phase `decrease` with `stopping` true):

- **Stop** is disabled (a Stop is already under way)
- **Start** stays disabled until phase is `idle` again
- **Users** is refused until phase is `idle` again (the dashboard returns `"ok": false` with `invalid_phase`, a Controller replies false), so Users cannot turn a Stop back into a running load test

The snapshot's `stopping` field is `true` from the Stop until `idle`; a Controller `shutdown` and Ctrl-C set it the same way. A users ramp's own ramp down is also phase `decrease`, but `stopping` stays `false` and **Users** is accepted there: the ramp down is abandoned and the run holds the new count until stopped. While a test plan runs, **Users** is refused with `invalid_phase` and `"field": "users"`, as the plan sets users: edit the plan, or stop and set users to switch to a users ramp.

### Users on a running test

A users ramp keeps its shape when the user count changes mid run (Target, the `users` command, or `POST /api/v1/control/users`): Goose takes the new count and keeps the hold (the run time after the ramp up) and the ramp down, and regenerates the steps still ahead from them.

| When Users arrives | What happens |
|--------------------|--------------|
| During the ramp up | Ramp to the new count, then hold for the full run time, then ramp down for the new count; a new count that is the one the run already has holds that count until stopped |
| During the hold | Ramp to the new count; the hold keeps its end. If the ramp outlasts it, or the new count is the one the run already has, the run holds that count until stopped, with no ramp down |
| During the ramp down | The ramp down is abandoned, and the run holds the new count until stopped |
| While holding until stopped | Ramp to the new count, and keep holding |

Adding users ramps at the increase rate; removing them at the decrease rate, else the increase rate; 1 per second for an increase with no increase rate, and for a decrease with neither rate. Increase and decrease times are not used for these changes. A decrease rate sent while a users ramp runs regenerates the ramp down still ahead at the new rate; on a run with a run time and no ramp down, where all users would stop at once when the hold ends, it adds a ramp down at that rate; on a run holding until stopped it only sets the rate for the next Users decrease, a later run time, and the next Start. Clearing the decrease rate (`null`) on a running test removes a ramp down this run added, so all users stop at once again, and is refused with `conflicting_settings` while a ramp down the run started with is still ahead. An increase rate sent while running is for the next Users increase, and never re-paces a ramp in progress. While running, either rate is refused with `conflicting_settings` when its own time is set, since a time changes only in Idle, and both are refused during the ramp down, while stopping, and while a test plan runs.

### Run settings

`POST /api/v1/control/settings` applies run settings, and `POST /api/v1/control/start` applies them and starts in one step, so no other client can change a setting in between. Both take the token (the one `--dashboard-auth-token` or `GooseDefault::DashboardAuthToken` sets; there is no second token for control) and the 16 KiB body limit, and go to the main loop as one command, checked as a whole: a refusal, a Start's `prepare_failed` among them, leaves every setting, the test plan, the running test and `target_users` as they were.

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

- A field left out is left alone, and `null` clears it back to Goose's default. Clearing `host` goes back to each scenario's host, or the default host. A field that is not one of these is refused.
- Values are bounded as the Controllers bound them: `host` an `http` or `https` url with no user name or password; `users` 1 to 100000; the rates 0.01 to 100000 users per second; the times whole seconds from 1 to 30 days (`0` is refused: send `null` to clear); `test_plan` in the `--test-plan` format, within the bounds of the [Controller commands](telnet.md#controller-commands): at most 100 steps of up to 30 days each, in at most 4096 bytes, starting 1 to 100000 users in total.
- `test_plan` switches to a test plan and clears users, the ramp settings and the run time; `users` switches back and clears the test plan. `test_plan: null` is accepted only with `users`. While a test plan is configured, a ramp setting without `users` is refused with `"field": "users"`.
- In Idle, a rate and its own time each replace the other, so a rate alone clears its time, and a time alone clears its rate; on a running test a rate is refused while its own time is set. The two cannot both be set in one body. A test plan cannot be sent with `users` or a ramp setting, though clearing a ramp setting beside it with `null` is accepted.
- `users: null` is refused. On a running test `users` is refused in a settings body: change users on its own with `/api/v1/control/users`.

Refusals are checked in this order, and the first one is answered:

| Status | `error` | When |
|--------|---------|------|
| 400 | `bad_request` | The body is not JSON, not an object, or has a field Goose does not know |
| 400 | `invalid_value` | A value of the wrong type or out of bounds, and `users: null` |
| 400 | `conflicting_settings` | Settings that never go together, whatever the state |
| 200 | `invalid_phase` | A setting the phase cannot change, set or cleared |
| 200 | `conflicting_settings` | A ramp setting while a test plan is configured (`"field": "users"`); a rate on a running test while its own time is set; clearing the decrease rate or the run time on a running test where that is refused; the command line's rules (run time, a decrease rate or time, and a test plan under `--iterations`, a test plan under `--no-reset-metrics`, and in Idle a time with fewer than 2 users) |
| 200 | `no_host` | `host: null` would leave a scenario, or a `test_start` or `test_stop` transaction, with no host |
| 200 | `prepare_failed` | A Start that cannot prepare the load test, as without settings |

When one setting is at fault, the refusal names it in `field` (`host`, `users`, `increase_rate`, `increase_time`, `decrease_rate`, `decrease_time`, `run_time` or `test_plan`); a `bad_request` has `"field": null`, and a Start refused for its phase, or a `prepare_failed` not caused by a missing host, has no `field`. A reply to new settings, sent to `settings` or with a Start, carries `settings` as the snapshot does, whether accepted or refused by the main loop, so the page that sent them has the settings before the next snapshot; after a 503 `timeout`, read them from the next snapshot.

Which settings each phase takes, by `settings.stage`: `ramp_up`, `hold` and `hold_until_stopped` are a running users ramp, `test_plan` a running test plan, `ramping_down` the configured ramp down, and `stopping` a Stop, `shutdown` or Ctrl-C:

| Setting | Idle | Running, users ramp | Running, test plan | Ramping down | Stopping |
|---------|------|---------------------|--------------------|--------------|----------|
| `host` | yes | no | no | no | no |
| `users` (`/api/v1/control/users`) | yes | yes | no | yes, abandons the ramp down and holds | no |
| `increase_rate` | yes | yes, for the next Users increase; refused while the increase time is set | no | no | no |
| `decrease_rate` | yes | yes, regenerates the ramp down ahead, or only sets the rate on a run holding until stopped; refused while the decrease time is set | no | no | no |
| `increase_time_secs`, `decrease_time_secs` | yes | no | no | no | no |
| `run_time_secs` | yes | yes | no, the plan sets how long it runs | no | no |
| `test_plan` | yes | yes, replaces the ramp | yes, restarts from its first step | yes, replaces the ramp down | no |

A test plan sent to a running test starts from its first step, ramping from the current user count.

### Run time on a running test

The run time is the hold at full users after the ramp up, on a running test as in Idle. A new value is always written to the configuration, which `settings.run_time_secs` and the next Start read, and changes this run:

| When the run time changes | What happens |
|---------------------------|--------------|
| During the ramp up, before the hold | The hold ahead takes the new run time; with no run time before, a hold is added, then the run ramps down as configured, or stops all users at once |
| During the hold | The hold ends at its start plus the new run time; if that has passed, the ramp down starts at once. If the new run time puts the end of the hold earlier than the end of the ramp from a users change made during the hold, the run holds until stopped |
| While holding until stopped | The run holds from now for the new run time, then ramps down as configured, or stops all users at once |

Clearing it (`null`, or `runtime 0` from a Controller) on a running test holds until stopped, and is refused with `conflicting_settings` while a ramp down is configured. The run time is refused during the ramp down, while stopping and while a test plan runs. Telnet and WebSocket take the same rules for `runtime`.

### Run settings in the snapshot

Every snapshot carries `settings`: what the next Start runs, and where the running test is. It reaches every tab within a tick of a change from any front end.

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

Names are the command line's flags in snake case, durations are whole seconds with a `_secs` suffix, and `null` means unset, so Goose's default applies.

| Field | Meaning |
|-------|---------|
| `host` | The `--host` override, or `null` |
| `hosts` | The hosts the next Start will load test: per scenario, `--host`, else the scenario's host, else the default host, sorted and deduplicated, written as the top level `hosts` writes them (`http://localhost/`, with the slash). A scenario with no host from any source adds nothing, so it is right in Idle, after a host change, and under `--no-autostart` with no host yet. The top level `hosts` keeps meaning the current or last run's hosts |
| `users` | The configured user count, `null` only with a test plan. `target_users` is different: the count the current step ramps toward |
| `increase_rate`, `decrease_rate` | Users per second |
| `increase_time_secs`, `decrease_time_secs` | The ramp up or ramp down as a time |
| `run_time_secs` | The hold at full users after the ramp up, not the total run time |
| `test_plan` | The test plan as `--test-plan` takes it, in canonical form: each step's duration in whole seconds, so `10,1h30m` reads `10,5400` |
| `hold_ends_in_secs` | Seconds until the hold ends once it has a fixed end, `0` while a ramp runs past the hold's end, and still `0` once a users ramp outlasts the hold and the run holds until stopped (see [Users on a running test](#users-on-a-running-test)); `null` at any other time |
| `stage` | Where the run is: `idle`, `ramp_up`, `hold`, `hold_until_stopped`, `ramping_down` or `stopping` for a users ramp; `idle`, `test_plan` or `stopping` for a test plan. When a users ramp outlasts the hold and the run holds until stopped, `stage` still reads `hold` |

`phase` alone cannot say this: `decrease` with `stopping` false is a Users decrease, the configured ramp down, or a test plan step to fewer users. After Users during a ramp down, `run_time_secs` is unchanged while `stage` reads `hold_until_stopped`.

## Health endpoint exception

`GET /api/v1/health` is intentionally **unauthenticated** so external monitors can check that the dashboard process is up without holding the metrics secret. Example response:

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
| `version` | Goose crate package version (`CARGO_PKG_VERSION`) |
| `last_build_ms` | Time the metrics processor spent on the last published snapshot, draining pending metrics and building it, in milliseconds (`0` if none yet) |
| `last_snapshot_ms` | Round trip of the last published snapshot in milliseconds, from the dashboard's request to the published JSON (`0` if none yet) |
| `snapshot_timeouts` | Snapshot requests not answered within 4 seconds since process start; a late answer is dropped, never published |
| `build_count` | Successful hub snapshot builds since process start |
| `active_sse_clients` | Concurrent SSE clients currently holding a slot |
| `control_enabled` | Whether the `POST /api/v1/control/*` routes and Delete are registered (`--dashboard-control`) |
| `auth_required` | Whether control needs a token: `false` only under `--dashboard-no-auth`, when the page sends no token; `true` otherwise. It does not say whether metrics need one: a loopback dashboard with no token reads `true` and serves its metrics to anyone on the machine |
| `loopback` | Whether the dashboard is bound to a loopback address; with `auth_required` false and this false, the page shows the no token warning |
| `controllers` | The `host:port` the telnet and WebSocket Controllers listen on, after defaults (`127.0.0.1:5116` and `127.0.0.1:5117` unless set), or `null` for one turned off, the telnet Controller with `--no-telnet` and the WebSocket Controller unless `--websocket` turns it on, or one that couldn't listen on its port; the page footer names them |

These counters are ops-safe (timing and client counts only). The response does **not** include rates, hosts under test, request names, or error strings. `controllers` names this process's own listeners, which take no authentication; it reveals only where they listen. `last_snapshot_ms` minus `last_build_ms` is time spent waiting on the main loop and the metrics processor's queue, plus serializing the JSON. The main loop answers the dashboard between steps and sleeps up to 500 ms between them, so this difference can reach about 500 ms by design; a large `last_build_ms` is the one that points at the metrics processor. Both durations are also logged at `debug` with a `[dashboard]` prefix when a build completes, and a late answer to a timed out request is logged with its round trip.

## The page in detail

### What the UI shows

- **Phase badge**: `idle` (gray), `increase` (blue), `maintain` (green), `decrease` (orange), `shutdown` (red)
- **Connection indicator**: green for SSE, yellow for poll fallback, red when disconnected, gray once Goose has exited
- **KPI strip**: users, RPS, fail %, and the whole run p95 and average latency. The tiles share the row, and each value is sized to its tile, so it stays on one line. The users tile shows the active users, then the target, smaller. RPS shows whole requests a second at 100 and above, and two decimals below 100; hover the tile for the two decimals the tables show.
- **Charts**: trailing series window (300 seconds) for RPS and failures per second and active users, and the response time heatmap; see [The response time heatmap](#the-response-time-heatmap). The RPS and failures chart and the average latency chart, and their data tables, end at the last complete second: the newest second is still filling, so it is not drawn. Their colors follow the system's light or dark theme, and change with it while the page is open.
- **Whole run** card: requests, failures, RPS, failures per second, fail %, average, p50, p95 and p99, and minimum and maximum response times
- **Sortable tables**: Scenarios, Transactions, Requests, Errors and Transaction errors. The scenario and transaction tables list every registered scenario and transaction and are never truncated; the request, error and transaction error tables show the top rows, 100 requests ranked by count and 50 of each kind of error ranked by occurrences (truncated server-side for large runs). The Transaction errors table counts each error a transaction returned that is not a request error, by transaction (numbered from 1), scenario, name and error. The Transactions and Requests tables show failures per second and minimum and maximum response times, and the Requests table each request's status codes, most frequent first: the top three and how many more, such as `200 9,812 · 503 44 · 500 40 · +2`, with every code on hover (empty with `--no-status-codes`)
- **Hosts** in the header: the current or last run's hosts. In Idle, when the next Start would load test other hosts, they read `Last run: …`.
- **Run settings** in the header, with control off: the same summary line the control panel shows, leaving out the hosts while running, and including the next Start's hosts in Idle when they differ from the last run's; see [Run settings in the snapshot](#run-settings-in-the-snapshot)
- **Control panel**: with `--dashboard-control`, the run settings summary, Start / Stop / Quit / target users; see [Controlling a load test from the dashboard](#controlling-a-load-test-from-the-dashboard). Without it, the card says the dashboard is read-only and how to get control; see [The read-only card](#the-read-only-card)
- **Notes** in the header: the run's `--notes`
- **Report bar** at the top of the page: the **Save report** and **Include graph data** switches, **Save report now** with control on, and the `--report-file` paths; see [Saving the report](#saving-the-report)
- **Runs panel**: the reports in the folder that holds the run folders, with downloads and comparisons, and with `--dashboard-control` a Delete button for each

Every figure in the KPI strip except the users, and every figure in the **Whole run** card and the tables, covers the whole run, since the test started or since metrics were last reset; the p95 and average tiles and the **Whole run** card say so in their labels, and hovering a table's `avg ms`, `min ms`, `p50`, `p95`, `p99`, `max ms` or `status codes` heading says "Whole run". Only the charts and the heatmap are by second. Goose measures response times to the microsecond. It shows times under 10 ms to the hundredth of a millisecond and from 10 ms in whole milliseconds, rounded half up, so a 0.418 ms response reads `0.42` and a 37.6 ms one `38`. Averages follow the same rule.

Transaction times cover the whole transaction function, so they include `--throttle-requests` delays and any sleep inside it, and scenario times include the wait after each transaction. Every registered scenario and transaction has a row, so a scenario excluded by `--scenarios`, or one no user was assigned to, stays at 0 runs.

When control is off there are **no control buttons**. Use the Controllers (or enable dashboard control) to change the running test. The page footer names the address each Controller listens on, as `GET /api/v1/health` reports it, or says none is running.

When coordinated omission mitigation is on (`--co-mitigation`) and has recorded at least one event, the KPI strip, the **Whole run** card and the requests table show adjusted response times, which include the synthetic times mitigation adds for requests a stalled user could not send. The KPI tiles and the **Whole run** card label them "adjusted", the requests table says so in a note above it, and hovering a value shows the measured one. The **Whole run** card shows both. The maximum is adjusted too; the minimum has no adjusted value and is always the measured one. The adjusted aggregate includes every request, using measured times for requests that never needed backfill, as the printed metrics and the reports do.

### UI walkthrough (control panel)

When `health.control_enabled` is true and the SPA has a token, or `health.auth_required` is false:

1. **Phase badge** still shows `idle` / `increase` / `maintain` / `decrease` / `shutdown`.
2. **Start** is enabled only in `idle`.
3. **Stop** is enabled in `increase` and `maintain` only.
4. **Summary line**: one line above the buttons says what the next Start runs, for example `https://example.com/ · 100 users at 10/s · hold 10 min · then 10/s down`, and while running where the run is: `ramping to 100 users at 10/s`, `ramp down in 6 min 52 s` during the hold, `holding until stopped`, `ramping down at 10/s`, `stopping`, or for a test plan `test plan: 3 steps, peak 100 users, 1 h 1 min total`. Its wording comes from `settings.stage`. It is not a live region, so the countdown is not announced.
5. **Target users**: enter an absolute count and **Apply**, or use **−** / **+** with a configurable step (default 10). Active user count is a read-only label from the latest snapshot. In Idle the field shows the configured users the next Start runs (empty with a test plan, where applying a count replaces the plan with a users ramp); while running, the count the run ramps toward. It follows the server unless you are editing it: an edit holds until its Apply succeeds and a snapshot reports the value, or until another client changes the count, when the field shows the new count and the status line says it changed elsewhere. **−** and **+** step from what the field shows. An unapplied edit is discarded, and the status line says so once, when the run starts stopping or a test plan starts. The field and step are read only, never disabled, with the reason under them, while `stopping` is set, while a test plan runs, and without a token.
6. **Edit settings** opens a form under the summary line for **Host**, **Ramp up** and **Ramp down** (each a number with users per second, or a total time in s, min or h), and **Run time**. It starts open in Idle and closed while running, and this browser remembers whether you opened or closed it, which then decides how it starts. Each field follows the server unless you are editing it, as Target does. An unapplied edit is pending: the form reads `Edit settings (N unapplied)`, **Apply settings** sends the pending edits, **Discard edits** drops them, and in Idle **Start** reads **Start with changes** and sends them with the start, the Target count among them. An emptied field goes out as `null`, and a field you did not touch is left out. A number the browser cannot read is refused on the page, naming the field, rather than sent as a clear. While edits are pending in Idle, Target's own Apply waits, and **−** / **+** only change the field. The first change of host in a page load asks first: `Load will go to …` under Host, with Confirm and Cancel, before anything is sent. Editing Host while the confirmation is open changes the destination it names; the confirmation closes, sending nothing, once the edits no longer change the host, or once the button that opened it can no longer act. A time shows in the largest unit that divides it evenly, so an applied `120 s` reads `2 min`, the same setting. While a users ramp runs, a ramp set by rate changes only its rate, a ramp set by time is read only (`Set by time: 5 min; change it when idle`), and the run time stays live, with `held 3 min 8 s of 10 min` under it during the hold. Fields the stage cannot change are read only, with the reason under them, and an edit the stage can no longer take is discarded, the status line saying so once. On a refusal that names a setting, the form opens, marks the field and moves focus to it. A test plan is set from a Controller or through `/api/v1/control/settings` or `/api/v1/control/start`.
7. The status line shows the server's `message`, in red for a refusal; a 503 reads that control is busy or timed out, or gives the server's message; a 401 also puts a banner at the top of the page saying to reopen with `?token=`. A 403 from the cross site guard reads `Refused: the request's Origin doesn't match this dashboard's address (another site, or a proxy)`, and one from the Host rule `Refused: open the dashboard by its IP address or its --dashboard-host name`.
8. **Quit** shows only in `idle`, and shuts Goose down; the status line then reads `Goose is shutting down.`

Quit is refused outside `idle` (`"ok": false` with `invalid_phase`), so stop a running test first. A Controller `shutdown` command exits the Goose process from any phase.

The header subtitle says which dashboard it is: `Live dashboard · control` with a token, `Live dashboard · control, no token` under `--dashboard-no-auth`, and `Live dashboard · read-only` without control; it stays `Live dashboard` when `GET /api/v1/health` fails.

### The read-only card

Without `--dashboard-control` the Control card stays, and says in place of its controls that the dashboard is read-only and how to get control:

```text
Read-only: this dashboard can't start, stop or change the test.
To control it from here, restart Goose with --dashboard-control --dashboard-no-auth (no token), or with --dashboard-control --dashboard-auth-token <token> and open http://127.0.0.1:5118/?token=<token>.
Control this test via the telnet controller listening on 127.0.0.1:5116.
```

The address is the one the page was opened at. When the page has a token, or the dashboard runs with `--dashboard-no-auth`, only control is missing, and the second sentence reads `To control it from here, restart Goose with --dashboard-control added.` The last sentence names the telnet and WebSocket Controllers that listen, and is left out when neither does. The notice is a note, named `Read-only dashboard` for screen readers, and holds nothing that takes focus. When `GET /api/v1/health` fails, the card says `Couldn't check whether control is on (/api/v1/health failed). Reload to retry.` instead.

### The response time heatmap

The response time card shows how long requests took, second by second, over the trailing window (300 seconds). Each column is one second, the oldest on the left. Each row is a range of response times: 100 microseconds each under a millisecond, 1 ms each up to 64 ms, 2 ms up to 100 ms, 10 ms up to 500 ms, 50 ms up to 1 second, 100 ms up to 5 seconds and 1 second up to 61 seconds, 237 rows in all. The bottom row holds responses under 100 microseconds, the row from 100 ms holds 100 to 109 ms, and the row from 1.4 s holds 1,400 to 1,499 ms. Past 61 seconds, which a longer `--timeout` allows, each row is one of the buckets Goose keeps a run's times in, 0.8% to 1.6% as wide as the times it holds. Each row takes whole buckets, and a bucket that crosses a row's edge is drawn in the row where it ends, so a time just under a row's upper edge may be drawn in the row above it. The rows sit on a logarithmic latency axis, so each row is as tall as the range of times it holds, and the axis is marked at 0.1, 0.2 and 0.5 ms, 1, 2, 5, 10, 20, 50, 100, 200 and 500 ms and 1, 2, 5, 10, 20 and 60 s, as far as the data reaches. Seconds are labelled along the bottom at round steps of 1, 2, 5, 10, 15, 30, 60, 120 or 300 seconds from the start of the test, the smallest that gives at most eight labels, so a label stays on its second as the window slides.

The color of a cell shows how many requests in that second took that long, on a logarithmic scale running from the sparsest cell drawn to the busiest: the legend under the chart shows the sparsest cell's count at its left end and the busiest's at its right. In the light theme more requests are darker, in the dark theme brighter, and the legend marks the scale `(log scale)`. Even the sparsest cell stands out from the card, so a cell of one request is visible. A second with no requests is empty. Hover a cell for its second, its row's range of times, how many requests it holds and their share of the second's requests, followed by that second's p50, p95 and p99. A row under a millisecond reads as its range, such as `0.9 to 1 ms`.

Three lines cross the heatmap: each second's p50, p95 and p99, in one color and one width, p50 solid, p95 dashed and p99 dotted. Each is labelled at the right edge with its newest value, such as `p95: 81 ms`, or one label for lines with the same newest value, such as `p95, p99: 4 ms`, and the legend shows a sample of each stroke. A second's percentile is worked out from the same counts its cells show, by the rule Goose uses for its whole run figures: the rank is the second's request count times the percentile, rounded, and the percentile is the time of the first row, from the fastest, at which the running count of requests reaches that rank. A heatmap percentile is the middle of its row, where its point sits. A line breaks at a second with no requests. The labels, the tooltip, the summary and the data table show it truncated, to the tenth of a millisecond under 1 ms and in whole milliseconds from 1 ms, so the value shown always falls inside its row. In the **Adjusted** view the lines include the synthetic times, as the cells do.

Each column covers only the requests that started in that second. The KPI strip, the **Whole run** card and the tables cover the whole run instead, since the test started or since metrics were last reset, and say so. While a test runs, the two newest seconds are not drawn yet, because their slowest requests are still arriving.

Choose a request in the select at the top of the card to see that request alone, and **All requests** to go back. Response times are kept by second for each of the first 100 request names Goose records a response time for, in the order it first records one, counting again from each metrics reset. Those are the names in the select, as far as they are among the rows the Requests table receives; a name seen later counts toward all requests but is not in the select. The select lists them by method, then name, whatever the Requests table's sort, and is not rebuilt while it has focus: a request that appears then is added when focus leaves it.

When coordinated omission mitigation is on and has recorded an event, two buttons choose what the heatmap counts: **Adjusted**, the default, as in the KPI strip and the tables, adds the synthetic times mitigation adds for requests a stalled user could not send, and **Measured** leaves them out. Synthetic times belong to the seconds of the stall, which Goose only learns about when it ends, so cells in seconds already drawn can change.

`--no-percentiles-over-time` turns the heatmap and the histograms behind it off, and the card shows the average latency chart instead. The same flag turns off the HTML report's heatmap.

## Snapshot data

### Heatmap data

The snapshot's `series.heatmap` carries the heatmap, or `null` when percentiles over time are off:

| Field | Meaning |
|-------|---------|
| `generation` | Which set of histograms the columns come from. It changes when they are cleared, by a metrics reset or a new test, and a client then starts over. |
| `rev` | The newest change the columns include. |
| `since` | The `rev` these columns follow on from, or `0` for every column of the window. |
| `start_second`, `end_second` | The window, in seconds from the start of the test. Columns before `start_second` are dropped. |
| `hold_from` | Seconds from this one on are still filling and not drawn. |
| `buckets_us` | `[time, lowest, highest]` in microseconds for each row index: the time the row stands for, which is its middle, then the lowest and highest time the row holds. A longer table replaces a shorter one; entries never change. |
| `columns` | The columns that changed: `{ "second", "measured": [index, count, ...], "synthetic": [index, count, ...] }`, each non empty bucket as an index and a count, `synthetic` empty unless mitigation has recorded an event. |

Only the columns that changed are sent. Every SSE client gets the same snapshot, whose heatmap holds the columns changed since the previous one; a client holding another `rev`, for example after it reconnects, asks `GET /api/v1/snapshot?heatmap_generation=<generation>&heatmap_since=<rev>` for the columns it lacks. Without those parameters `GET /api/v1/snapshot` returns every column of the window. The dashboard does all of this itself.

How big this is depends on how widely response times spread. In a test of 300 users at about 26 requests a second over 3 request names, each SSE snapshot's heatmap took about 6.7 KB (7.1 KB at most), 5.4 KB of it the row table `buckets_us`, and a whole window of 92 seconds took 12.4 KB. Times spread over every row are the worst case: then each column holds an index and a count for up to 237 rows (more past 61 seconds with a longer `--timeout`) instead of a few. In that worst case the columns in each snapshot grow many times over, but the snapshot as a whole grows far less, as the row table it always carries stays most of it; a whole window, which is all columns, grows many times over.

### Request heatmap endpoint

`GET /api/v1/series?method=<method>&name=<request name>` returns one request's heatmap over the same window, as the card uses it when a request is chosen, in the same shape: `{ "method", "name", "heatmap" }`. `method` is the method as the Requests table shows it (`GET`, `POST`, ...) and `name` is the request name; URL encode both. Add `heatmap_generation` and `heatmap_since` to get only the columns changed since the heatmap you hold. It needs the token when one is configured, as `GET /api/v1/snapshot` does.

| Status | Body |
|--------|------|
| `200` | `{ "method", "name", "heatmap": {...} }`, the heatmap in the shape of the snapshot's `series.heatmap` |
| `400` | `{"error":"bad_request"}`: `method` or `name` is missing, or the two together are longer than 2,048 bytes |
| `401` | Empty: the token is missing or wrong, as for `GET /api/v1/snapshot` |
| `404` | `{"error":"not_found"}`: the request has no heatmap of its own (not one of the first 100 names, or `--no-percentiles-over-time`) |
| `503` | `{"error":"busy"}` with four series requests already in flight, `{"error":"timeout"}` when no answer arrives within 15 seconds, or `{"error":"unavailable"}` when Goose is no longer taking dashboard requests (shutting down) |

Answers are cached for one second for each request and heatmap held, so several viewers polling one request cost about one export a second.

```bash
curl -s -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  "http://127.0.0.1:5118/api/v1/series?method=GET&name=%2F"
```

### Metrics disabled

If Goose is started with `--no-metrics`, the dashboard still serves the shell and health endpoint, but snapshots report `flags.metrics_disabled: true` with empty request/error tables and empty series. The UI surfaces an honest empty state rather than fabricating data.

Scenario and transaction metrics can also be turned off on their own:

- `--no-transaction-metrics` sets `flags.transaction_metrics_disabled: true`; `transactions` is empty and the Transactions table says transaction metrics are disabled.
- `--no-scenario-metrics` sets `flags.scenario_metrics_disabled: true`; `scenarios` is empty and the Scenarios table says scenario metrics are disabled.

Both flags are also true under `--no-metrics`, where both tables say metrics are disabled.

## Saving the report

[Output files](../getting-started/output-files.md) gives the rules for every file Goose writes. The dashboard adds rules 9 to 13.

The report bar at the top of the page shows the **Save report** switch: whether this run's `report.html` and `report.json` are written to its folder, `goose-runs/<YYYYMMDD-HHMMSS>` unless `--output-dir` names one. It starts on with `--dashboard`, except under `--no-metrics`. Without `--dashboard`, it starts on with a bare `--report-file`, which writes into the run's folder, so such a console run is listed in the runs panel too; `--report-file` paths alone do not turn it on. The bar lists the `--report-file` paths, which are always written.

Save report can be turned on or off at any time. While it is on, Goose keeps the run's graph data and heatmap for the whole run. While it is off and no `--report-file` path or `--pdf-print-html` is given, Goose keeps only the dashboard's last 10 minutes of graphs and 5 minutes of heatmap, so memory stays flat on a long soak test. Turning it on during a run keeps graphs from then on, with that window before it, and the report says when its graphs start and why.

**Save report now** writes the run's report as it stands into its folder while the run continues, marked `in_progress`. The report at the end of the run replaces it, also when Save report is off. The second switch, **Include graph data**, adds the per second graph data to the JSON report, as `--report-graph-data` does, and can also be changed at any time.

Save report now, Delete, and changing either switch need `--dashboard-control` and the token, or `--dashboard-no-auth`, as Start and Stop do. Without control the switches are shown read only, so Save report stays on and every `--dashboard` run, except under `--no-metrics` or with a folder it can't write to, writes `report.html` and `report.json` into its folder. A 60 minute run of `examples/simple` without control, with 300 users at about 28 requests a second over 2 request names, grew by about 0.8 MiB an hour. With `--dashboard-control` you can turn Save report off to keep memory flat. `--no-metrics` keeps no report, so Save report stays off.

When a run ends, a banner reads `Report saved in <folder>. Download HTML, JSON or Markdown.`, naming the run's folder, with a button for each report, or says why the report couldn't be saved. For a folder whose name the report routes don't take (see [Report endpoints](#report-endpoints)), it reads `Report saved in <folder>.` with no buttons. A Save report now that fails while the run goes on is shown at the foot of the runs panel, with the reason.

When only `--dashboard` asks for the run's folder, with no output flag given without a path and no `--output-dir`, a folder Goose can't write to does not stop the run. Goose prints a warning before any load, and runs with Save report off. A banner at the top of the page names the folder and the reason, and stays for the whole run. The Save report switch says why when you hover over it.

### The runs panel

The **Runs** panel looks in the folder that holds each run's folder: the parent of the `--output-dir` folder, or `goose-runs` by default. It lists each folder there that holds a `report.json` or `report.json.gz` it can read; a folder whose JSON report can't be read is left out. It lists only folders with a name the report routes take; see [Report endpoints](#report-endpoints). It lists them while Goose is Idle too, before any run has a folder, and it lists folders written by runs without the dashboard. Rows are sorted newest first, by start time, or by when the report was written if it records no start time. Each row shows the folder's name, when the run started (in the viewer's local time), its notes, how long it ran, its users, requests and failures, how it ended (`Finished`, `Stopped`, `Ctrl-C`, `Canceled: <reason>` or `Users exited`), and the test. A report still marked in progress shows as `Running` for the run in progress and as `Incomplete` for any other: a run that ended without its final report. The first 50 rows are shown; `Show all` lists the rest. The list is read when the page opens, when a run starts, when Goose writes a report into a run's folder, and after a Delete. Goose writes a report there at the end of a run and on Save report now, and the snapshot's `report.writes` counts those writes.

Each row offers HTML, Markdown and JSON. A format the folder lacks is rendered from its JSON report; without graph data in it, the rendered HTML report says the graphs were not recorded.

With `--dashboard-control`, each row also has a **Delete** button. It asks `Delete the reports and logs in <folder>?`, then deletes the folder's reports and the logs Goose names there, and the folder if that leaves it empty. The folder of the run in progress can't be deleted. Nor can the folder a bare `--goose-log` is open in: Goose keeps that log open in the last run's folder until the next run starts. Nor can a folder that holds `.goose-run`. A Goose process keeps that file in its run's folder while the run lasts, and after the run while a bare `--goose-log` is open there, so a dashboard never deletes what another process is writing. A process that is killed leaves the file behind; remove it by hand to free the folder.

Check **Compare** on exactly two rows and `Download comparison` downloads a Markdown report of the newer run with the older as its baseline: every number shows its change, as with [`--baseline-file`](../getting-started/common.md#comparing-against-a-baseline). Runs with no requests can't be compared. When the two runs are from different tests the panel warns that the comparison may not mean much.

Downloads are fetched with the token in an `Authorization` header and saved from the response, so the token never goes into a URL. Reports are always downloaded, never shown in the browser.

### Report endpoints

The `GET` routes need the token when one is configured, as `GET /api/v1/snapshot` does. `DELETE` is registered only with `--dashboard-control` and needs the token as `Authorization: Bearer` (none under `--dashboard-no-auth`), as the control POSTs do. Each reads the disk only on request. Goose keeps what it parsed of each JSON report in memory, keyed by path, size and modification time, and writes no index.

| Endpoint | Returns |
|----------|---------|
| `GET /api/v1/reports` | `{ "dir": "goose-runs", "reports": [ ... ] }`: for each folder holding a regular `report.json` or `report.json.gz` that can be read, its `name`, the run's record from its JSON report (`notes`, `goose_version`, `test`, `started`, `ended`, `ended_by`, `canceled_reason`, `in_progress` and `graphs_start`, with `notes`, `canceled_reason` and `graphs_start` left out when unset), `running`, `duration_secs`, `max_users`, `requests` and `failed_requests`, newest first |
| `GET /api/v1/reports/{name}/{file}` | `report.html`, `report.md` or `report.json` of folder `{name}`, as an attachment named `goose-<test>-<name>.<ext>`; a format the folder lacks is rendered from its JSON report, or answers **422** with a one line reason when it can't be |
| `GET /api/v1/reports/{name}/compare.md?baseline={name}` | Report `{name}` compared to the baseline, as Markdown, named `goose-<test>-<name>-vs-<baseline>.md`, starting with a line naming both and when each started. `422` with a one line reason when a JSON report can't be read or is inconsistent; a run with no requests compares as zeros |
| `DELETE /api/v1/reports/{name}` | `204` once the folder's `report.html`, `report.md`, `report.json`, `goose.log` and the request, transaction, scenario, error and debug logs Goose names there, each plain or gzipped, and any temporary report file a stopped process left, are deleted, then the folder if that leaves it empty; `409` for the folder of the run in progress once it holds a report, the folder a bare `--goose-log` is open in, or a folder that holds `.goose-run`; `404` for a folder the panel does not list |

A name is an ASCII letter or digit, then any mix of ASCII letters, digits, `.`, `_` and `-`. A folder with any other name is not listed. A name of any other form, a folder that is not one level down, a symlink, or a folder without a regular JSON report gets a plain `404`. A folder whose JSON report is not yet on disk is not listed: Goose writes the JSON report last, through a temporary file and a rename. Reports and comparisons are sent with `Content-Disposition: attachment`, `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` and `Content-Security-Policy: sandbox`, so a report opened anyway runs no script with the dashboard's origin.

```bash
curl -s -H "Authorization: Bearer $DASHBOARD_TOKEN" http://127.0.0.1:5118/api/v1/reports
curl -s -OJ -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  http://127.0.0.1:5118/api/v1/reports/nightly/report.html
curl -s -OJ -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  "http://127.0.0.1:5118/api/v1/reports/nightly-2/compare.md?baseline=nightly"
curl -s -X POST -H "Authorization: Bearer $DASHBOARD_TOKEN" -H 'Content-Type: application/json' \
  -d '{"save": false}' http://127.0.0.1:5118/api/v1/control/report
```

With no token, which only a loopback bind or `--dashboard-no-auth` allows, every user on this machine (and off loopback, everyone who can reach the port) can list and download every report in that folder while the dashboard is up, even if the files themselves are readable only by their owner. On a shared machine, set a token in the load test's code, as [On a shared machine](#on-a-shared-machine) shows.

### Waiting after an autostarted run

When an autostarted `--dashboard` run (one without `--no-autostart`) ends, Goose stays up instead of exiting if Save report wrote its report and any client is connected to the event stream, so the report can still be downloaded. With `--dashboard-control` it prints:

```text
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Quit in the dashboard or Ctrl-C to exit.
```

and without it:

```text
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Ctrl-C to exit.
```

The phase goes back to `idle`, and the test does not start again. Goose exits on Quit in the dashboard, on Ctrl-C, or on a Controller `shutdown`; there is no exit timer, so closing the tab does not end the wait. Nothing is written or printed again, and `test_stop` does not run again. Any client on `/api/v1/events` counts, not only browser tabs: a script tailing the event stream when the run ends keeps Goose up until Goose is told to quit. With no client connected when the run ends, Goose exits at once, as it does without the dashboard. A run stopped from the dashboard or a Controller prints no `Run finished` line; Goose stays up in Idle, as after any Stop. A run canceled with Ctrl-C or aborted by a `--threshold` exits with no wait.

## GraphData memory cost

Enabling `--dashboard` turns on the same per second **GraphData** series collection used for HTML report graphs (also enabled by `--report-file` and `--pdf-print-html`). Goose keeps every second of the whole run while a report will be written at the end of it: to a `--report-file` or `--pdf-print-html` path, or by [Save report](#saving-the-report), which is on by default with `--dashboard`. With Save report off and no `--report-file` or `--pdf-print-html` path, the dashboard keeps only twice its chart window (10 minutes). Without `--dashboard`, `--report-file` or `--pdf-print-html` nothing is recorded. The recording costs memory even when no browser is connected:

- Per-second counters for requests, errors, users, and average latency are kept for the whole run while a report will be written, and for the last 10 minutes otherwise.
- While a report will be written, memory grows with unique request names × run length, the same cost as generating HTML graphs; otherwise it stays flat.
- Request, error and transaction error tables truncate to the top rows (`flags.requests_truncated` / `flags.errors_truncated` / `flags.transaction_errors_truncated` when capped); series charts use a fixed trailing window (5 minutes).

With no SSE client connected and no `GET /api/v1/snapshot` in the last two seconds (`/api/v1/health` does not count), the dashboard issues **no** snapshot builds (no extra metrics-processor work beyond GraphData recording). When clients are connected, snapshots are coalesced to about **1 Hz** for all viewers. Concurrent SSE clients are capped (default **32**, tunable with `--dashboard-max-clients`); further clients receive HTTP 503.

Each snapshot build (while clients are connected or polling) runs on the **metrics processor** task: it drains pending metrics, exports the trailing series window, and builds percentile maps for the aggregate, up to 100 request rows, and one row per scenario and transaction, a number fixed by the load test's code. That cost scales with unique request names and the timing histograms Goose already maintains; the attack main loop does **not** await the build. Under extreme cardinality, prefer fewer unique request names, a shorter test, or disable the dashboard if you need absolute minimal overhead (same trade-off as `--report-file`).

With `--dashboard`, and also for a `--report-file` or `--pdf-print-html`, Goose keeps one response time histogram per second for the last 300 seconds, for all requests together and for each of the first 100 request names, plus the same for synthetic times while coordinated omission mitigation is on. It does not grow with run length. Its size depends on how many request names there are and how many distinct response times occur: about 0.2 MB for all requests and 10 MB for 100 names with typical response times, and up to 30 MB in the worst case at the default 60 second timeout, more with a longer `--timeout`. Filling it costs little. In a test of 1,000 users at about 21,800 requests a second over 25 request names, the heatmap added no measurable CPU or instructions per request: the difference from the dashboard without it was within the runs' spread of about 1%. It added 3.1 MiB of peak memory. Each user's batch of metrics is still sent every 100 requests or 250 ms, as without the heatmap; a batch keeps each request name's response times for the second it is filling and the next one, and a request from any other second sends it early. For the HTML report's heatmap, while a report will be written, each second is also added once to whole run columns: up to 600 for all requests and 60 for each of those 100 names. The columns merge in pairs as they fill, so they too stop growing. `--no-percentiles-over-time` turns all of it off, along with the small per request cost of filling it.

For extreme runs with tens of thousands of unique request names, expect higher GraphData memory (same as `--report-file`). Disable the dashboard for the least overhead, just as you can turn off the telnet Controller with `--no-telnet` (the WebSocket Controller is off unless `--websocket` turns it on).

## Relationship to Controllers

| Need | Prefer |
|------|--------|
| Watch live charts and tables | Live Dashboard (`--dashboard`) |
| Start / Stop / set users, host, ramps and run time from a browser | Dashboard control (`--dashboard-control` + token, or `--dashboard-no-auth`) |
| Test plan from a script | Dashboard control (`/api/v1/control/settings` or `/api/v1/control/start`) or the Controllers |
| Full command set, `config` and `metrics` | [Telnet](telnet.md) / [WebSocket](websocket.md) Controllers |
| Shut Goose down from Idle | Dashboard control (Quit) or the Controllers (`shutdown`) |
| Shut down a running test | Controllers (`shutdown`) |

Dashboard control reuses the same phase-transition logic as Controllers (shared helpers), and Controllers remain available alongside the dashboard.

For full Controller documentation, see [Controlling A Running Goose Load Test](overview.md).

## Stability

### Building it in

In 0.19 and 0.20 the dashboard is an opt in crate feature: build with `--features dashboard`. The goal is to make it a default feature in 1.0, a decision to be made on the feedback from 0.19 and 0.20.

### `/api/v1` in 0.19 and 0.20

Changes are additive where possible: new fields, new endpoints and new error codes, under snapshot `version: 1`. A client must ignore fields it does not know and must not fail on an error code it does not know. Every change to `/api/v1` is listed in the CHANGELOG. A change that is not additive, such as a field or endpoint removed, renamed or given a different meaning, raises the snapshot `version`, and the CHANGELOG says so. 0.20 brings back distributed load testing (Gaggle) and may make such changes so the API can describe more than one worker.

### `/api/v1` from 1.0

Semver covers it: within 1.x no field, endpoint or error code is removed, renamed or given a different meaning. New ones may be added, so a client must ignore fields it does not know and must not fail on an error code it does not know.

### The routes and the snapshot

In 0.19 `/api/v1` serves `GET /api/v1/health`, `GET /api/v1/snapshot`, `GET /api/v1/events`, `GET /api/v1/series`, `GET /api/v1/reports`, `GET /api/v1/reports/{name}/{file}` and `GET /api/v1/reports/{name}/compare.md`, and with `--dashboard-control` `POST /api/v1/control/start`, `settings`, `stop`, `users`, `quit`, `report` and `report/now`, and `DELETE /api/v1/reports/{name}`. A snapshot's top level fields are `version`, `generated_at`, `goose_version`, `phase`, `stopping`, `duration_secs`, `active_users`, `maximum_users`, `target_users`, `total_users`, `hosts`, `aggregate`, `requests`, `errors`, `transaction_errors`, `scenarios`, `transactions`, `series`, `flags`, `report` and `settings`. The SSE `closed` event's data is `1`.

### What the contract is

The contract is the JSON served under `/api/v1` and the snapshot `version`, a top level field in every snapshot. The Rust types behind it are internal and are not part of the public crate API. The bundled client is built for one `version` and refuses any other, and a client of your own should check `version` the same way.

## Building the dashboard UI (TypeScript)

The browser SPA lives under `src/dashboard/static/`. **Edit the TypeScript modules in `src/`, not the bundled JavaScript:**

| File | Role |
|------|------|
| `src/main.ts` | Entry point: renders each snapshot and wires the modules together |
| `src/connection.ts` | Access token handling, SSE stream, poll fallback, snapshot fetch and version check |
| `src/control.ts` | Control panel, its POSTs, the phase badge and the run settings summary |
| `src/settings-form.ts` | The Edit settings form: Host, Ramp up, Ramp down and Run time, and their pending edits |
| `src/run-settings.ts` | The run settings summary line, worded from `settings.stage` |
| `src/follow.ts` | Fields that follow the server unless being edited (Target and the settings form) |
| `src/charts.ts` | Chart.js setup and the line charts: RPS and failures, active users, and average latency |
| `src/chart-data.ts` | Each line chart's one sentence summary and its data table |
| `src/heatmap.ts` | The response time heatmap: data shaping and drawing |
| `src/heatmap-card.ts` | The response time card: its request select, the Adjusted and Measured toggle, and the average latency chart when percentiles over time are off |
| `src/coadjust.ts` | Which response time to show: coordinated omission adjusted or measured |
| `src/runs.ts` | The report bar, Save report now, the report banners and the runs panel |
| `src/tables.ts` | The shared sortable table (sorting, filtering, empty states) and the scenarios, transactions, requests, errors and transaction errors tables |
| `src/format.ts` | Number and duration formatting, and the small DOM builders the summary and tables share |
| `src/status.ts` | Banner and connection indicator |
| `src/dom.ts` | Element lookup that checks each element's type |
| `snapshot.d.ts` | **Generated** types of the snapshot JSON, from the Rust structs in `src/metrics/dashboard_snapshot.rs` |
| `idle-snapshot.mjs` | A complete idle snapshot for the client tests, type checked against `snapshot.d.ts` by `npm run check` |
| `tsconfig.json` / `package.json` | TypeScript type check and esbuild bundle config |
| `*.test.mjs` | Client tests, run with `npm test`: `token`, `tables`, `runs`, `control`, `settings`, `coadjust`, `format`, `heatmap`, `a11y`, `chrome` and `monitor`. Most run in jsdom against the bundled `app.js`; `npm test` first builds `coadjust.ts`, `format.ts`, `heatmap.ts` and `charts.ts` into `test-build/` for the tests that load a module on its own |
| `test-harness.mjs` | The jsdom page loader and stub server the client tests share |
| `app.js` | **Bundled output** (the modules plus Chart.js), embedded by the Rust server via `include_str!` |
| `index.html`, `app.css` | Shell and styles (not generated) |

`npm run build` bundles `src/main.ts`, the modules it imports and Chart.js into one minified `app.js` with [esbuild](https://esbuild.github.io/). Chart.js comes from npm at a pinned version (see `package.json`), with its own TypeScript types, so the chart configuration is type checked; its license comment is kept in the bundle.

Cargo builds do **not** run npm. The committed `app.js` is what gets embedded, so a normal `cargo build` works without Node.js. After changing a module in `src/` (or the related type declarations), regenerate `app.js` and commit it with the sources.

### Prerequisites

- [Node.js](https://nodejs.org/) (with npm)
- Network access once to install the dev dependencies (esbuild, Chart.js, TypeScript, and jsdom for the client tests)

### Rebuild

From the repository root:

```bash
cd src/dashboard/static
npm ci
npm run build
```

That runs esbuild and overwrites `app.js` in the same directory. Then rebuild Goose so the new script is embedded:

```bash
cargo build --features dashboard
# or exercise the dashboard:
cargo run --release --features dashboard --example simple -- --dashboard ...
```

### Snapshot types

`snapshot.d.ts` declares `DashboardSnapshot` and every struct it holds, and `RequestSeries`, the `/api/v1/series` body, generated by [`ts-rs`](https://crates.io/crates/ts-rs) from the Rust structs, so `tsc` rejects a read of a field the structs do not have. Every `u64` is declared as `number`, which is what `JSON.parse` returns. After changing a snapshot struct, regenerate the file, then type check the client (and run `npm run build` if a module in `src/` changed). `tsconfig.json` sets `skipLibCheck` to false so `tsc` also checks the generated file:

```bash
GOOSE_UPDATE_DASHBOARD_TYPES=1 cargo test --lib dashboard_types_match_rust
cd src/dashboard/static
npm run check
```

`ts-rs` is a dev dependency used only by unit tests, so crates that depend on Goose never pull it in. Without the environment variable, `dashboard_types_match_rust` compares the committed file with the structs and fails when they differ (a reordered field counts as a difference), so a struct change without a regenerated `snapshot.d.ts` fails `cargo test` in CI, and a client that reads a removed or renamed field then fails `npm run check`. A second test fails on any serde attribute other than `serde(skip)`, which `ts-rs` honours, or on a hand-written `Serialize` in the snapshot module, because `ts-rs` cannot see either, so the JSON could change without the types changing.

`ts-rs` is pinned to an exact version in `Cargo.toml`. The committed file is compared byte for byte with its output, and Goose commits no `Cargo.lock`, so a new release that formats differently would otherwise fail CI with no change in Goose. Dependabot opens a pull request when a new `ts-rs` is released; if `dashboard_types_match_rust` fails on it, regenerate the file on that branch as above and review the diff.

The client accepts only the snapshot `version` named in `snapshot.d.ts` (`SNAPSHOT_VERSION` in the Rust module) and shows an error banner for any other.

### Type check

esbuild strips the TypeScript types without checking them, so `npm run build` succeeds on code that does not type check. `npm run check` is the type check: it runs `tsc` over the modules and `snapshot.d.ts` (`tsconfig.json` sets `noEmit`, so `tsc` writes nothing), then checks `idle-snapshot.mjs` against the snapshot types.

```bash
cd src/dashboard/static
npm run check
```

### Client tests

```bash
cd src/dashboard/static
npm run build
npm test
```

Most tests load the bundled `app.js` into a jsdom page with a stubbed `fetch`, so rebuild it first.

### Notes

- Do not hand-edit `app.js`; changes there will be lost on the next `npm run build`.
- `node_modules/` is gitignored; only `package.json` / `package-lock.json` are committed so installs stay reproducible.
- Chart.js is bundled into `app.js`; the server does not serve it as a separate file.
- CI runs `npm run check`, rebuilds `app.js` with `npm ci && npm run build` (failing if the committed file differs from the esbuild bundle), and runs `npm test`.
