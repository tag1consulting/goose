# Live Dashboard

> The dashboard is an opt in crate feature: build Goose with `--features dashboard`. [Stability](#stability) says what the dashboard and its `/api/v1` endpoints promise from one release to the next.

Goose can optionally serve a **live web dashboard** while a load test runs. The dashboard streams compact metric snapshots (including trailing RPS, users, and latency series) over Server-Sent Events (SSE), with a short poll fallback if SSE is unavailable.

By default the dashboard is **observe-only**: start, stop, user counts, and other control remain on the [Telnet](telnet.md) and [WebSocket](websocket.md) Controllers. With an explicit opt-in (`--dashboard-control` plus a required auth token), the same browser can also **Start**, **Stop**, **set the target user count**, and edit the host, the ramps and the run time. Controllers remain the power-user path for test plan changes, `config` and `metrics`, and process **shutdown** (not available from the dashboard).

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

The dashboard port opens when the Controllers' ports do, once Goose has built a client for every user: about 3.5 seconds after launch with 1,000 users on two cores. With `--no-autostart` users are built on Start, so the port opens at once.

### Related flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--dashboard` | off | Enable the live dashboard HTTP server (observe) |
| `--dashboard-control` | off | Enable Start/Stop/Users control endpoints and UI (requires `--dashboard`; does **not** auto-enable the dashboard) |
| `--dashboard-host HOST` | `127.0.0.1` | Bind address |
| `--dashboard-port PORT` | `5118` | Bind port. CLI value `0` means “unset” and is rewritten to `5118` when the dashboard is enabled (not an ephemeral OS port). |
| `--dashboard-auth-token TOKEN` | empty | Shared secret for metric APIs when configured; **always required** when `--dashboard-control` is set (even on loopback). Visible in process listings (`ps`) like any CLI flag. |
| `--dashboard-max-clients COUNT` | `32` | Max concurrent SSE clients (`GET /api/v1/events`); further clients receive HTTP 503. CLI value `0` means “unset” and becomes `32`. The server also holds at most twice this many connections plus 16 (80 by default); a connection beyond that is closed as soon as it is accepted. |
| `--no-percentiles-over-time` | off | Turn off the response time heatmap and the histograms behind it; the card shows the average latency chart instead. It also turns off the HTML report's response time heatmap, and works without `--dashboard`. See [The response time heatmap](#the-response-time-heatmap). |

Defaults can also be set programmatically with `GooseDefault::Dashboard`, `GooseDefault::DashboardControl`, `GooseDefault::DashboardHost`, `GooseDefault::DashboardPort`, `GooseDefault::DashboardAuthToken`, `GooseDefault::DashboardMaxClients`, and `GooseDefault::NoPercentilesOverTime`.

> **Feature flag:** the HTTP server is compiled behind the opt-in `dashboard` crate feature (not in default features, same pattern as `pdf-reports`). Enable it with `--features dashboard`. Builds without that feature reject `--dashboard` at startup with a clear rebuild hint.

The server lives as long as the load test, with one exception: see [Waiting after an autostarted run](#waiting-after-an-autostarted-run). When `GooseAttack::execute()` returns, whether the run finished or failed, connected browsers receive `event: closed`, open connections get up to one second to finish, and the listener and every connection are closed before `execute()` returns, so a program can run another `GooseAttack` on the same port right away.

### Observe vs control at a glance

| Capability | Live Dashboard (observe) | Live Dashboard + control | Controllers (telnet / WebSocket) |
|------------|--------------------------|--------------------------|----------------------------------|
| Live rates, percentiles, errors | Yes | Yes | `metrics` / `metrics-json` |
| Trailing RPS / users / latency charts | Yes | Yes | No |
| Start / stop / set users | **No** | Yes | Yes |
| Host / test plan / rates | No | No | Yes |
| Process shutdown | No | No | Yes (`shutdown`) |
| Default bind | `127.0.0.1:5118` | `127.0.0.1:5118` | `0.0.0.0:5116` / `:5117` |
| Auth | Token on metric APIs when configured | Token **always** for control; metrics follow observe rules | None (see Controller docs) |

When control is off, `POST /api/v1/control/*` returns **404** (routes are not registered). Prefer Controllers when you need advanced commands; use dashboard control for common browser-side Start/Stop/Users.

## Loopback default and auth policy

By default the dashboard binds to **loopback only** (`127.0.0.1`). That is intentional: metrics snapshots include request names, error strings, hostnames, and rates that should not be world-readable.

Goose treats a bind host as loopback when it is:

- `localhost` (case-insensitive, no DNS lookup)
- any IPv4 address in `127.0.0.0/8` (for example `127.0.0.1`, `127.0.0.2`)
- IPv6 loopback `::1` (with or without brackets: `[::1]`)

**Non-loopback** binds — including unspecified addresses that listen on all interfaces (`0.0.0.0`, `::`) and LAN/hostnames — **require** `--dashboard-auth-token` at startup. Goose hard-fails with a clear error if the token is missing.

If a token **is** configured, it is enforced on metric APIs even on loopback.

On a loopback bind every request must carry a loopback `Host` header (`localhost`, an address in `127.0.0.0/8` or `[::1]`, with any port), whatever the token setting; anything else, a missing `Host` included, gets **403**. This stops DNS rebinding: a web page that points its own domain at 127.0.0.1 sends its own domain as `Host`, so it cannot read snapshots from a dashboard that has no token. A reverse proxy in front of a loopback bind must pass a loopback `Host` (nginx `proxy_pass http://127.0.0.1:5118` does by default). Loopback here means the configured `--dashboard-host` name, as in the list above, so a hostname that resolves to a loopback address is not checked; it needs a token like any other non-loopback bind.

### Security matrix (observe vs control)

| Bind | Observe (`--dashboard`) | Control (`--dashboard-control`) |
|------|-------------------------|----------------------------------|
| Loopback, no token | Metrics OK without auth | **Startup error** (token always required with control) |
| Loopback, token set | Metrics require token | Control requires token |
| Non-loopback, no token | **Startup error** | **Startup error** |
| Non-loopback, token set | Metrics require token | Control requires token |

Additional control rules:

- `--dashboard-control` without `--dashboard` → **startup hard-fail** (control does not auto-enable the dashboard).
- Control endpoints always require a non-empty `--dashboard-auth-token`, even on loopback, so other local processes cannot POST start/stop/users without the secret.
- Startup log shows capability at a glance: `(read-only)` vs `(control enabled)`.

## What requires a token

| Path | Auth when token configured | Notes |
|------|----------------------------|-------|
| `GET /` (SPA shell) | **Public** | |
| `GET /static/*` | **Public** | |
| `GET /api/v1/health` | **Public** | Liveness + ops counters; includes `control_enabled` and the Controller addresses (no load-test metrics) |
| `GET /api/v1/snapshot` | **Required** | |
| `GET /api/v1/events` (SSE) | **Required** | |
| `GET /api/v1/series` | **Required** | One request's response time heatmap; see [Request heatmap endpoint](#request-heatmap-endpoint) |
| `POST /api/v1/control/*` | **Always required** when control is enabled | Even on loopback; **404** when control is off |
| `DELETE /api/v1/runs/{id}` | **Always required** when control is enabled | Control token, as for `POST /api/v1/control/*`; **404** when control is off |

The shell and static assets stay public because classic `<script src>` loads cannot forward a document `?token=` query (or an in-memory secret). Sensitive data lives only in the snapshot and saved run APIs; mutation lives only on the control routes (the control POSTs and `DELETE /api/v1/runs/{id}`).

### Browser bootstrap with `?token=`

Browsers' `EventSource` cannot set `Authorization` headers. For remote or token-protected dashboards, open:

```text
http://{host}:{port}/?token=SECRET
```

The embedded UI reads `token` from the URL on load, strips it from the address bar with `history.replaceState`, and attaches `?token=` to `/api/v1/snapshot` and `/api/v1/events` requests.

The UI keeps the token in the tab's `sessionStorage`, so reloading the page keeps it. A new tab or window opened on the bare URL does not get it and needs `?token=` again, and the token goes away with the tab. A tab duplicated from the browser menu gets a copy, since browsers copy `sessionStorage` when duplicating a tab, and a browser that restores closed tabs may restore it with the tab. It is never written to `localStorage`. A token in the URL replaces a stored one. When the server answers **401** to a metric or control request, the UI removes the stored token, so a stale token (for example after Goose restarts with a new one) is not sent again on the next reload.

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

`GET /api/v1/snapshot` returns **503** with an empty body when no snapshot arrives within five seconds, which happens while the main loop or the metrics processor is busy (for example building the users of a large `users` change), or when the dashboard is shutting down. Retry after a short wait.

For **control**, the server accepts **`Authorization: Bearer` only** (query `?token=` is rejected on control routes):

```bash
curl -H "Authorization: Bearer SECRET" \
  -H "Content-Type: application/json" \
  -d '{}' \
  -X POST http://host:5118/api/v1/control/start
```

### Example: bind on all interfaces

```bash
export DASHBOARD_TOKEN="replace-me"

cargo run --release --features dashboard --example simple -- \
  --dashboard \
  --dashboard-host 0.0.0.0 \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 200 -t 1h
```

Open `http://ci-host:5118/?token=$DASHBOARD_TOKEN`.

## Controlling a load test from the dashboard

Control is **off by default**. Enable it with `--dashboard-control` (and a token):

```bash
export DASHBOARD_TOKEN="replace-me"

cargo run --release --features dashboard --example simple -- \
  --dashboard \
  --dashboard-control \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 50 -t 10m
```

Open `http://127.0.0.1:5118/?token=$DASHBOARD_TOKEN`. The header shows `Live dashboard · control`, and a control panel appears with **Start**, **Stop**, a target-users input (`Apply`, `−` / `+` step buttons), and **Edit settings**, a form for the host, the ramps and the run time.

### `--no-autostart` recipe

`--no-autostart` keeps Goose in **Idle** until something starts the test. It is allowed when any Controller is enabled **or** when `--dashboard-control` is set (with dashboard + token):

```bash
export DASHBOARD_TOKEN="replace-me"

cargo run --release --features dashboard --example simple -- \
  --no-autostart \
  --no-telnet --no-websocket \
  --dashboard \
  --dashboard-control \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 50
```

Then Start from the UI or via `POST /api/v1/control/start`. Without Controllers **and** without dashboard control, `--no-autostart` is a startup hard-fail (there would be no way to leave Idle).

### Control endpoints

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `POST` | `/api/v1/control/start` | empty, `{}`, or run settings | Start an idle load test → enters **Increase**; with run settings, apply them and start in one step, all or none |
| `POST` | `/api/v1/control/settings` | run settings | Apply run settings without starting, all or none; see [Run settings](#run-settings) |
| `POST` | `/api/v1/control/stop` | empty or `{}` | Begin cancel → enters **Decrease** (eventual Idle) |
| `POST` | `/api/v1/control/users` | `{"users": N}` | Set absolute target user count (`N` integer, 1–100_000) |
| `POST` | `/api/v1/control/quit` | empty or `{}` | Shut Goose down; accepted only in **Idle** |
| `DELETE` | `/api/v1/runs/{id}` | none | Delete a saved run from disk; see [Saved run endpoints](#saved-run-endpoints) |

Auth: `Authorization: Bearer <token>` only (query `?token=` → **401**). Missing/wrong token → **401**. Control disabled → **404**.

#### curl examples

```bash
export DASHBOARD_TOKEN="replace-me"
BASE="http://127.0.0.1:5118"

# Start (success means phase entered Increase — not that test_start finished)
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}' \
  "$BASE/api/v1/control/start"

# Stop (success means cancel ramp began — phase is decrease, not idle)
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

Logical rejections return **HTTP 200** with `"ok": false` and a stable `error` code: `invalid_phase` when the phase cannot take the command, `prepare_failed` when a Start cannot prepare the load test, `conflicting_settings` when the command conflicts with other settings, such as `users 1` while an increase time is set, and `no_host` when clearing the host would leave something with no host. When one setting is at fault the body also names it in `field` (for example `"field": "users"`, or `"field": "host"` for a Start with no host); `field` is left out otherwise. A malformed body returns **400** with `error` set to `bad_request`, and a `users` value that is not an integer from 1 to 100000 returns **400** with `invalid_users`; a 400 body has `"field": null`. A run settings body has 400 refusals of its own, which name the field; see [Run settings](#run-settings). A body larger than 16 KiB gets **413** with a plain text body, before the token is checked.

A Start is refused with `prepare_failed` and `"field": "host"` before it replies when some scenario has no host from `--host`, the scenario itself or the default host, and also when a `test_start` or `test_stop` transaction is registered and neither `--host` nor the default host is set, since those run with no scenario host. An autostarted run with the same configuration fails at launch instead.

When the main loop does not answer, a control POST returns **503** with `"ok": false`, an `error` code from this table, and `phase`, `active_users` and `target_users` set to `null`:

| `error` | When | Client action |
|---------|------|---------------|
| `busy` | Four authenticated control requests are already waiting for the main loop | Wait and retry |
| `unavailable` | The main loop no longer takes control requests, for example because the load test is exiting | Do not retry |
| `timeout` | The main loop did not answer within 15 seconds | Re-read `phase` and `target_users` from a snapshot before retrying: a command still queued at the timeout is skipped, but one the main loop had started may still apply |

### Start and Stop semantics

**Start success means the test entered Increase**, not that `test_start` transactions finished. Goose replies as soon as the phase transitions (same timing as Controllers), then runs `reset_run_state` / user `test_start` on the main loop. A long `test_start` does not block the HTTP 200.

**Stop is not instantaneous idle.** A successful Stop begins a **cancel** through `AttackPhase::Decrease` (Canceling), which stops all users at once, whatever the decrease rate or time: those shape a users ramp's own ramp down, never a Stop. The process returns to **Idle** only after decrease finishes (and Goose is not configured to shut down after stop). While phase is `decrease`:

- **Stop** is disabled (cannot stop again mid-ramp)
- **Start** stays disabled until phase is `idle` again
- **Users** is refused until phase is `idle` again (the dashboard returns `"ok": false` with `invalid_phase`, a Controller replies false), so Users cannot turn a Stop back into a running load test

The snapshot's `stopping` field is `true` from the Stop until `idle`; a Controller `shutdown` and Ctrl-C set it the same way. A users ramp's own ramp down is also phase `decrease`, but `stopping` stays `false` and **Users** is accepted there: the ramp down is abandoned and the run holds the new count until stopped. While a test plan runs, **Users** is refused with `invalid_phase` and `"field": "users"`, as the plan sets users: edit the plan, or stop and set users to switch to a users ramp.

### Users on a running test

A users ramp keeps its shape when **Users** changes it mid run: Goose keeps the target user count, the hold (the run time after the ramp up) and the ramp down, and generates the steps still ahead again from them.

| When Users arrives | What happens |
|--------------------|--------------|
| During the ramp up | Ramp to the new count, then hold for the full run time, then ramp down for the new count |
| During the hold | Ramp to the new count; the hold keeps its end, and if the ramp outlasts it, the ramp down starts when the ramp finishes |
| During the ramp down | The ramp down is abandoned, and the run holds the new count until stopped |
| While holding until stopped | Ramp to the new count, and keep holding |

Adding users ramps at the increase rate; removing them at the decrease rate, else the increase rate; 1 per second when neither is set. Increase and decrease times are not used for these changes. A decrease rate sent while a users ramp runs regenerates the ramp down still ahead at the new rate; on a run with a run time and no ramp down, where all users would stop at once when the hold ends, it adds a ramp down at that rate; on a run holding until stopped it only sets the rate for the next Users decrease, a later run time, and the next Start. Clearing the decrease rate (`null`) on a running test removes a ramp down this run added, so all users stop at once again, and is refused with `conflicting_settings` when the run started with that ramp down. An increase rate sent while running is for the next Users increase, and never re-paces a ramp in progress. While running, either rate is refused with `conflicting_settings` when its own time is set, since a time changes only in Idle, and both are refused during the ramp down, while stopping, and while a test plan runs.

### Run settings

`POST /api/v1/control/settings` applies run settings, and `POST /api/v1/control/start` applies them and starts in one step, so no other client can change a setting in between. Both take the control token and the 16 KiB body limit, and go to the main loop as one command, checked as a whole: a refusal, a Start's `prepare_failed` among them, leaves every setting, the test plan, the running test and `target_users` as they were.

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

- A field left out is left alone, and `null` clears it back to Goose's default. Clearing `host` goes back to each scenario's host, or the default host. Any other field is refused.
- Values are bounded as the Controllers bound them: `host` an `http` or `https` url with no user name or password; `users` 1 to 100000; the rates 0.01 to 100000 users per second; the times whole seconds from 1 to 30 days (`0` is refused: send `null` to clear); `test_plan` as `--test-plan` takes it.
- `test_plan` switches to a test plan and clears users and the ramp settings; `users` switches back and clears the test plan. `test_plan: null` is accepted only with `users`. While a test plan is configured, a ramp setting without `users` is refused with `"field": "users"`.
- A rate and its own time each replace the other, so a rate alone clears its time, and a time alone clears its rate; the two can not both be set in one body. A test plan can not be sent with `users` or a ramp setting, though clearing one beside it with `null` is accepted.
- `users: null` is refused. On a running test `users` is refused in a settings body: change users on its own with `/api/v1/control/users`.

Refusals are checked in this order, and the first one is answered:

| Status | `error` | When |
|--------|---------|------|
| 400 | `bad_request` | The body is not JSON, not an object, or has a field Goose does not know |
| 400 | `invalid_value` | A value of the wrong type or out of bounds, and `users: null` |
| 400 | `conflicting_settings` | Settings that never go together, whatever the state |
| 200 | `invalid_phase` | A setting the phase can not change, set or cleared |
| 200 | `conflicting_settings` | A ramp setting while a test plan is configured (`"field": "users"`); a rate on a running test while its own time is set; clearing the decrease rate or the run time on a running test where that is refused; the command line's rules (run time and a test plan under `--iterations`, a test plan under `--no-reset-metrics`, and in Idle a time with fewer than 2 users) |
| 200 | `no_host` | `host: null` would leave a scenario, or a `test_start` or `test_stop` transaction, with no host |
| 200 | `prepare_failed` | A Start that can not prepare the load test, as without settings |

Each names the setting at fault in `field` (`host`, `users`, `increase_rate`, `increase_time`, `decrease_rate`, `decrease_time`, `run_time` or `test_plan`); a `bad_request` has `"field": null`. An accepted reply, and a refusal from the main loop, carry `settings` as the snapshot does, so the page that sent them has the settings before the next snapshot; after a 503 `timeout`, read them from the next snapshot.

Which settings each phase takes, by `settings.stage`: `ramp_up`, `hold` and `hold_until_stopped` are a running users ramp, `test_plan` a running test plan, `ramping_down` the configured ramp down, and `stopping` a Stop, `shutdown` or Ctrl-C:

| Setting | Idle | Running, users ramp | Running, test plan | Ramping down | Stopping |
|---------|------|---------------------|--------------------|--------------|----------|
| `host` | yes | no | no | no | no |
| `users` (`/api/v1/control/users`) | yes | yes | no | yes, abandons the ramp down and holds | no |
| `increase_rate` | yes | yes, for the next Users increase; refused while the increase time is set | no | no | no |
| `decrease_rate` | yes | yes, regenerates the ramp down ahead; refused while the decrease time is set | no | no | no |
| `increase_time_secs`, `decrease_time_secs` | yes | no | no | no | no |
| `run_time_secs` | yes | yes | no, the plan sets how long it runs | no | no |
| `test_plan` | yes | yes, replaces the ramp | yes, restarts from its first step | yes, replaces the ramp down | no |

A test plan sent to a running test starts from its first step, ramping from the current user count.

### Run time on a running test

The run time is the hold at full users after the ramp up, on a running test as in Idle. A new value is always written to the configuration, which `settings.run_time_secs` and the next Start read, and changes this run:

| When the run time changes | What happens |
|---------------------------|--------------|
| During the ramp up, before the hold | The hold ahead takes the new run time; with no run time before, a hold is added before the ramp down |
| During the hold | The hold ends at its start plus the new run time; if that has passed, the ramp down starts at once |
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
| `hold_ends_in_secs` | Seconds until the hold ends once it has a fixed end, and `0` while a ramp runs past it; `null` in Idle, during the first ramp up, once the ramp down starts, with no run time, and with a test plan |
| `stage` | Where the run is: `idle`, `ramp_up`, `hold`, `hold_until_stopped`, `ramping_down` or `stopping` for a users ramp; `idle`, `test_plan` or `stopping` for a test plan |

`phase` alone cannot say this: `decrease` with `stopping` false is either a Users decrease or the configured ramp down. After Users during a ramp down, `run_time_secs` is unchanged while `stage` reads `hold_until_stopped`.

### UI walkthrough (control panel)

When `health.control_enabled` is true and the SPA has a token:

1. **Phase badge** still shows `idle` / `increase` / `maintain` / `decrease` / `shutdown`.
2. **Start** is enabled only in `idle`.
3. **Stop** is enabled in `increase` and `maintain` only.
4. **Summary line**: one line above the buttons says what the next Start runs, for example `https://example.com/ · 100 users at 10/s · hold 10 min · then 10/s down`, and while running where the run is: `ramping to 100 users at 10/s`, `ramp down in 6 min 52 s` during the hold, `holding until stopped`, `ramping down at 10/s`, `stopping`, or for a test plan `test plan: 3 steps, peak 100 users, 1 h 1 min total`. Its wording comes from `settings.stage`. It is not a live region, so the countdown is not announced.
5. **Target users**: enter an absolute count and **Apply**, or use **−** / **+** with a configurable step (default 10). Active user count is a read-only label from the latest snapshot. In Idle the field shows the configured users the next Start runs (empty with a test plan, where applying a count replaces the plan with a users ramp); while running, the count the run ramps toward. It follows the server unless you are editing it: an edit holds until its Apply succeeds and a snapshot reports the value, or until another client changes the count, when the field shows the new count and the status line says it changed elsewhere. **−** and **+** step from what the field shows. An unapplied edit is discarded, and the status line says so once, when the run starts stopping or a test plan starts. The field and step are read only, never disabled, with the reason under them, while `stopping` is set, while a test plan runs, and without a token.
6. **Edit settings** opens a form under the summary line for **Host**, **Ramp up** and **Ramp down** (each a number with users per second, or a total time in s, min or h), and **Run time**. It starts open in Idle and closed while running, and remembers whether you opened or closed it. Each field follows the server unless you are editing it, as Target does. An unapplied edit is pending: the form reads `Edit settings (N unapplied)`, **Apply settings** sends the pending edits, **Discard edits** drops them, and in Idle **Start** reads **Start with changes** and sends them with the start, the Target count among them. An emptied field goes out as `null`, and a field you did not touch is left out. A number the browser can not read is refused on the page, naming the field, rather than sent as a clear. While edits are pending in Idle, Target's own Apply waits, and **−** / **+** only change the field. The first change of host in a page load asks first: `Load will go to …` under Host, with Confirm and Cancel, before anything is sent. Editing Host while it asks changes the destination it names, and it closes, sending nothing, once the edits no longer change the host or the button that opened it can no longer act. A time shows in the largest unit that divides it evenly, so an applied `120 s` reads `2 min`, the same setting. While a users ramp runs, a ramp set by rate changes only its rate, a ramp set by time is read only (`Set by time: 5 min; change it when idle`), and the run time stays live, with `held 3 min 8 s of 10 min` under it during the hold. Fields the stage can not change are read only, with the reason under them, and an edit the stage can no longer take is discarded, the status line saying so once. On a refusal that names a setting, the form opens, marks the field and moves focus to it. A test plan is set from a Controller or through `/api/v1/control/settings`.
7. Status line shows server `message` on success, or an error banner on soft failure / 401 / 503.
8. **Quit** shows only in `idle`, and shuts Goose down; the status line then reads `Goose is shutting down.`

Quit is refused outside `idle` (`"ok": false` with `invalid_phase`), so stop a running test first. A Controller `shutdown` command exits the Goose process from any phase.

## SSH tunnels

Prefer keeping Goose on loopback and tunneling from your laptop:

```bash
ssh -L 5118:127.0.0.1:5118 loadgen.example.com
# open http://127.0.0.1:5118/ locally
```

With a loopback bind and no auth token (observe-only), no `?token=` is required over the tunnel. If you set a token (required when control is enabled), pass it in the browser URL as above.

TLS termination is not provided by Goose; use an SSH tunnel or reverse proxy when you need encryption in transit.

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
  "controllers": { "telnet": "0.0.0.0:5116", "websocket": null }
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
| `control_enabled` | Whether the Start/Stop/Users/Quit and Delete routes are registered (`--dashboard-control`) |
| `controllers` | The `host:port` the telnet and WebSocket Controllers are configured to listen on, after defaults (`0.0.0.0:5116` and `0.0.0.0:5117` unless set), or `null` for one turned off with `--no-telnet` or `--no-websocket`; the page footer names them |

These counters are ops-safe (timing and client counts only). The response does **not** include rates, hosts under test, request names, or error strings. `controllers` names this process's own listeners, which take no authentication; it reveals only where they listen. `last_snapshot_ms` minus `last_build_ms` is time spent waiting on the main loop and the metrics processor's queue, plus serializing the JSON. The main loop answers the dashboard between steps and sleeps up to 500 ms between them, so this difference can reach about 500 ms by design; a large `last_build_ms` is the one that points at the metrics processor. Both durations are also logged at `debug` with a `[dashboard]` prefix when a build completes, and a late answer to a timed out request is logged with its round trip.

## What the UI shows

- **Phase badge** — `idle` (gray), `increase` (blue), `maintain` (green), `decrease` (orange), `shutdown` (red)
- **Connection indicator** — green for SSE, yellow for poll fallback, red when disconnected
- **KPI strip**: users, RPS, fail %, and the whole run p95 and average latency. The tiles share the row, and each value is sized to its tile, so it stays on one line. The users tile shows the active users, then the target, smaller. RPS shows whole requests a second from 100 and two decimals below; hover the tile for the two decimals the tables show.
- **Charts**: trailing series window (default 300 seconds) for RPS and failures per second and active users, and the response time heatmap; see [The response time heatmap](#the-response-time-heatmap). Their colours follow the system's light or dark theme, and change with it while the page is open.
- **Whole run** card: requests, failures, RPS, failures per second, fail %, average, p50, p95 and p99, and minimum and maximum response times
- **Sortable tables**: Scenarios, Transactions, Requests and Errors. The scenario and transaction tables list every registered scenario and transaction and are never truncated; the request and error tables show the top rows (truncated server-side for large runs). The Transactions and Requests tables show failures per second and minimum and maximum response times, and the Requests table each request's status codes, most frequent first: the top three and how many more, such as `200 9,812 · 404 3 · +1`, with every code on hover (empty with `--no-status-codes`)
- **Hosts** in the header: the current or last run's hosts. In Idle, when the next Start would load test other hosts, they read `Last run: …`.
- **Run settings** in the header, with control off: the same summary line the control panel shows, leaving out the hosts while running, and including the next Start's hosts in Idle when they differ from the last run's; see [Run settings in the snapshot](#run-settings-in-the-snapshot)
- **Control panel** (only when `--dashboard-control`): the run settings summary, Start / Stop / Quit / target users; see [Controlling a load test from the dashboard](#controlling-a-load-test-from-the-dashboard)
- **Save state** in the header: `Saving to goose-runs`, `Not saving (turned off)`, or `Not saving: can't create goose-runs (reason)`; see [Saved runs](#saved-runs)
- **Saved runs panel**: every saved run, with downloads and comparisons, and with `--dashboard-control` a Delete button for each

Every figure in the KPI strip, the **Whole run** card and the tables covers the whole run, since the test started or since metrics were last reset; the labels say so, and hovering a table's `avg ms`, `min ms`, `p50`, `p95`, `p99`, `max ms` or `status codes` heading says "Whole run". Only the charts and the heatmap are by second. Goose measures response times to the microsecond. It shows times under 10 ms to the hundredth of a millisecond and from 10 ms in whole milliseconds, rounded half up, so a 0.418 ms response reads `0.42` and a 37.6 ms one `38`. Averages follow the same rule.

Transaction times cover the whole transaction function, so they include `--throttle-requests` delays and any sleep inside it, and scenario times include the wait after each transaction. Every registered scenario and transaction has a row, so a scenario excluded by `--scenarios`, or one no user was assigned to, stays at 0 runs.

When control is off there are **no control buttons**. Use the Controllers (or enable dashboard control) to change the running test. The page footer names the address each Controller listens on, as `GET /api/v1/health` reports it, or says none is running.

When coordinated omission mitigation is on (`--co-mitigation`) and has recorded at least one event, the KPI strip, the **Whole run** card and the requests table show adjusted response times, which include the synthetic times mitigation adds for requests a stalled user could not send. They are labelled "adjusted", and hovering a value shows the measured one. The **Whole run** card shows both. The maximum is adjusted too; the minimum has no adjusted value and is always the measured one. The adjusted aggregate includes every request, using measured times for requests that never needed backfill, as the printed metrics and the reports do.

### The response time heatmap

The response time card shows how long requests took, second by second, over the trailing window (300 seconds). Each column is one second, the oldest on the left. Each row is a range of response times: 100 microseconds each under a millisecond, 1 ms each up to 64 ms, 2 ms up to 100 ms, 10 ms up to 500 ms, 50 ms up to 1 second, 100 ms up to 5 seconds and 1 second up to 61 seconds, 237 rows in all; past 61 seconds, possible with a longer `--timeout`, each row is one of the buckets Goose keeps a run's times in, each 0.8% to 1.6% of its time wide. So the 100 ms row holds 100 to 109 ms and the 1.4 s row 1,400 to 1,499 ms; the bottom row holds responses under 100 microseconds. The rows are coarser than the buckets the whole run figures come from, which are never more than 1% off: a time near the edge of a row may be drawn in the row next to it. The rows sit on a logarithmic latency axis, so each row is as tall as the range of times it holds, and the axis is marked at 0.1, 0.2 and 0.5 ms, 1, 2, 5, 10, 20, 50, 100, 200 and 500 ms and 1, 2, 5, 10, 20 and 60 s, as far as the data reaches. Seconds are labelled along the bottom at round steps of 1, 2, 5, 10, 15, 30, 60, 120 or 300 seconds from the start of the test, the smallest that gives at most eight labels, so a label stays on its second as the window slides.

The colour of a cell shows how many requests in that second took that long, on a logarithmic scale running from the sparsest cell drawn to the busiest: the legend under the chart shows the sparsest cell's count at its left end and the busiest's at its right. In the light theme more requests are darker, in the dark theme brighter, and the legend marks the scale `(log scale)`. Even the sparsest cell stands out from the card, so a cell of one request is visible. A second with no requests is empty. Hover a cell for its second, its bucket, how many requests it holds and their share of the second's requests, followed by that second's p50, p95 and p99. A row under a millisecond reads as its range, such as `0.9 to 1 ms`.

Three lines cross the heatmap: each second's p50, p95 and p99, in one color and one width, p50 solid, p95 dashed and p99 dotted. Each is labelled at the right edge with its newest value, such as `p95: 80 ms`, and the legend shows a sample of each stroke. A second's percentile is worked out from the same counts its cells show, by the rule Goose uses for its whole run figures: the rank is the second's request count times the percentile, rounded, and the percentile is the time of the first bucket, from the fastest, at which the running count of requests reaches that rank. A point sits in the middle of its bucket's row, and a line breaks at a second with no requests. A heatmap percentile is the middle of its row, so the labels, the tooltip, the summary and the data table show it at the row's precision, truncated: to the tenth of a millisecond under 1 ms and in whole milliseconds from 1 ms, so it names the row it is in. In the **Adjusted** view the lines include the synthetic times, as the cells do.


Each column covers only the requests that started in that second. The KPI strip, the **Whole run** card and the tables cover the whole run instead, since the test started or since metrics were last reset, and say so. While a test runs, the two newest seconds are not drawn yet, because their slowest requests are still arriving.

Choose a request in the select at the top of the card to see that request alone, and **All requests** to go back. Response times are kept by second for each of the first 100 request names Goose records a response time for, in the order it first records one, counting again from each metrics reset. Those are the names in the select, as far as they are among the rows the Requests table receives; a name seen later counts toward all requests but is not in the select. The select lists them by method, then name, whatever the Requests table's sort, and is not rebuilt while it is open: a request that appears then is added when it closes.


When coordinated omission mitigation is on and has recorded an event, two buttons choose what the heatmap counts: **Adjusted**, the default, as in the KPI strip and the tables, adds the synthetic times mitigation adds for requests a stalled user could not send, and **Measured** leaves them out. Synthetic times belong to the seconds of the stall, which Goose only learns about when it ends, so cells in seconds already drawn can change.

`--no-percentiles-over-time` turns the heatmap and the histograms behind it off, and the card shows the average latency chart instead. The same flag turns off the HTML report's heatmap.

#### Heatmap data

The snapshot's `series.heatmap` carries the heatmap, or `null` when percentiles over time are off:

| Field | Meaning |
|-------|---------|
| `generation` | Which set of histograms the columns come from. It changes when they are cleared, by a metrics reset or a new test, and a client then starts over. |
| `rev` | The newest change the columns include. |
| `since` | The `rev` these columns follow on from, or `0` for every column of the window. |
| `start_second`, `end_second` | The window, in seconds from the start of the test. Columns before `start_second` are dropped. |
| `hold_from` | Seconds from this one on are still filling and not drawn. |
| `buckets_us` | `[time, lowest, highest]` in microseconds for each bucket index: the time the row stands for, its middle, and the range of measured times it holds. A longer table replaces a shorter one; entries never change. |
| `columns` | The columns that changed: `{ "second", "measured": [index, count, ...], "synthetic": [index, count, ...] }`, each non empty bucket as an index and a count, `synthetic` empty unless mitigation has recorded an event. |

Only the columns that changed are sent. Every SSE client gets the same snapshot, whose heatmap holds the columns changed since the previous one; a client holding another `rev`, for example after it reconnects, asks `GET /api/v1/snapshot?heatmap_generation=<generation>&heatmap_since=<rev>` for the columns it lacks. Without those parameters `GET /api/v1/snapshot` returns every column of the window. The dashboard does all of this itself.

How big this is depends on how widely response times spread. In a test of 1,000 users at about 21,700 requests a second over 25 request names, each SSE snapshot's heatmap held two or three columns and took about 5.2 KB (5.9 KB at most), about 3 KB of it the bucket table, and a whole window of 91 seconds took 75 KB. Times spread over every bucket are the worst case: then a second's changes take about 9 KB, 60 seconds of them about 190 KB, and a whole window of 300 seconds about 600 KB.

#### Request heatmap endpoint

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
curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:5118/api/v1/series?method=GET&name=%2F"
```

## Saved runs

Goose [saves every run](../getting-started/common.md#saved-runs) by default, and the dashboard serves the saved runs in its runs directory (`goose-runs`, or `--runs-dir`).

When a run ends, a banner reads `Run 2026-10-09-141203 saved. Download HTML, JSON or Markdown.` with a button for each report, or says why the run couldn't be saved (`Couldn't save this run: <reason>.` when its directory couldn't be created). When Goose exits, the closed banner says where the last run was saved on the machine running Goose.

The **Saved runs** panel lists every complete run, newest first: the run id, when it started (in the viewer's local time), how long it ran, its users, requests and failures, how it ended (`Finished`, `Stopped`, `Ctrl-C`, `Canceled: <reason>`, `Users exited`), the test, and buttons to download each report. The first 50 runs are shown; `Show all` lists the rest. The list is read when the page opens and whenever another run has been saved, never on a timer.

With `--dashboard-control`, each run also has a **Delete** button. It asks `Delete run <id>? This removes its reports from disk.`, then deletes the run directory and lists the runs again. The run being written now can't be deleted.

Check **Compare** on exactly two runs and `Download comparison` downloads a Markdown report of the newer run with the older as its baseline: every number shows its change, as with [`--baseline-file`](../getting-started/common.md#comparing-against-a-baseline). Runs with no requests can't be compared. When the two runs are from different tests the panel warns that the comparison may not mean much.

Downloads are fetched with the token in an `Authorization` header and saved from the response, so the token never goes into a URL. Reports are always downloaded, never shown in the browser.

### Saved run endpoints

The three `GET` routes need the token when one is configured, as `GET /api/v1/snapshot` does. `DELETE` is registered only with `--dashboard-control` and always needs the control token as `Authorization: Bearer`, as the control POSTs do. Each reads the disk on request only.

| Endpoint | Returns |
|----------|---------|
| `GET /api/v1/runs` | `{ "dir": "goose-runs", "total_bytes": n, "runs": [ ... ] }`: the `run.json` of every complete run (a directory named by a run id that holds `run.json`), newest first |
| `GET /api/v1/runs/{id}/{file}` | `report.html`, `report.json` or `report.md` of run `{id}`, as an attachment named `goose-<test>-<id>.<ext>` |
| `GET /api/v1/runs/{id}/compare.md?baseline={id}` | Run `{id}` compared to the baseline run, as Markdown, named `goose-<test>-<id>-vs-<baseline id>.md`, starting with a line naming the run and the baseline and when each started. `422` with a one line reason when a run has no requests, or its `report.json` can't be read or is inconsistent |
| `DELETE /api/v1/runs/{id}` | `204` once the run directory is deleted; the run being written now has no `run.json` until it ends, so it is a `404` like any incomplete run |

An id that is not a run id, any other file, and anything that is not a real directory holding a regular `run.json` and a regular file (a symlink included) is a plain `404`, so a run still being written, or whose save failed, is not served. Reports and comparisons are sent with `Content-Disposition: attachment`, `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` and `Content-Security-Policy: sandbox`, so a report opened anyway runs no script with the dashboard's origin.

```bash
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5118/api/v1/runs
curl -s -OJ -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:5118/api/v1/runs/2026-10-09-141203/report.html
curl -s -OJ -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:5118/api/v1/runs/2026-10-09-151500/compare.md?baseline=2026-10-09-141203"
```

With no token, which only a loopback bind allows, every user on this machine can list and download every saved run while the dashboard is up, even if the files themselves are readable only by their owner. On a shared machine, set --dashboard-auth-token.

The SSE `closed` event carries the save state as its data, the same object as the snapshot's `save`: `{"state":"on","reason":null,"dir":"goose-runs","last_run":"2026-10-09-141203"}`. Earlier 0.19.0-dev builds sent `1`; the dashboard page accepts both.

### Waiting after an autostarted run

When an autostarted `--dashboard` run (one without `--no-autostart`) ends, the run was saved, and any client is connected to the event stream, Goose stays up instead of exiting, so the run can still be downloaded. With `--dashboard-control` it prints:

```
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Quit in the dashboard or Ctrl-C to exit.
```

and without it:

```
Run finished. Download it from the dashboard at http://127.0.0.1:5118. Press Ctrl-C to exit.
```

The phase goes back to `idle`, and the test does not start again. Goose exits on Quit in the dashboard, on Ctrl-C, or on a Controller `shutdown`; there is no exit timer, so closing the tab does not end the wait. Nothing is saved or printed again, and `test_stop` does not run again. Any client on `/api/v1/events` counts, not only browser tabs: a script tailing the event stream when the run ends keeps Goose up until Goose is told to quit. With no client connected when the run ends, Goose exits at once, as before. A run stopped from the dashboard or a Controller, or canceled with Ctrl-C, never waits.

## Stability

### Building it in

In 0.19 and 0.20 the dashboard is an opt in crate feature: build with `--features dashboard`. The goal is to make it a default feature in 1.0, a decision to be made on the feedback from 0.19 and 0.20.

### `/api/v1` in 0.19 and 0.20

Changes are additive where possible: new fields, new endpoints and new error codes, under snapshot `version: 1`. A client must ignore fields it does not know and must not fail on an error code it does not know. Every change to `/api/v1` is listed in the CHANGELOG. A change that is not additive, such as a field or endpoint removed, renamed or given a different meaning, raises the snapshot `version`, and the CHANGELOG says so. 0.20 brings back distributed load testing (Gaggle) and may make such changes so the API can describe more than one worker.

### `/api/v1` from 1.0

Semver covers it: within 1.x no field, endpoint or error code is removed, renamed or given a different meaning. New ones may be added, so a client must ignore fields it does not know and must not fail on an error code it does not know.

### What the contract is

The contract is the JSON served under `/api/v1` and the snapshot `version`, a top level field in every snapshot. The Rust types behind it are internal and are not part of the public crate API. The bundled client is built for one `version` and refuses any other, and a client of your own should check `version` the same way.

## Building the dashboard UI (TypeScript)

The browser SPA lives under `src/dashboard/static/`. **Edit the TypeScript modules in `src/`, not the bundled JavaScript:**

| File | Role |
|------|------|
| `src/main.ts` | Entry point: renders each snapshot and wires the modules together |
| `src/connection.ts` | Access token handling, SSE stream, poll fallback, snapshot fetch and version check |
| `src/control.ts` | Control panel, its POSTs, and the phase badge |
| `src/charts.ts` | Chart.js setup and chart updates |
| `src/tables.ts` | The shared sortable table (sorting, filtering, empty states) and the scenarios, transactions, requests and errors tables |
| `src/format.ts` | Number and duration formatting, and the small DOM builders the summary and tables share |
| `src/status.ts` | Banner and connection indicator |
| `src/dom.ts` | Element lookup that checks each element's type |
| `snapshot.d.ts` | **Generated** types of the snapshot JSON, from the Rust structs in `src/metrics/dashboard_snapshot.rs` |
| `tsconfig.json` / `package.json` | TypeScript type check and esbuild bundle config |
| `token.test.mjs` | Client tests of access token handling, run in jsdom against the bundled `app.js` (`npm test`) |
| `tables.test.mjs` | Client tests of the four tables, run in jsdom against the bundled `app.js` (`npm test`) |
| `test-harness.mjs` | The jsdom page loader and stub server both client test files share |
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
cargo build
# or exercise the dashboard:
cargo run --release --features dashboard --example simple -- --dashboard ...
```

### Snapshot types

`snapshot.d.ts` declares `DashboardSnapshot` and every struct it holds, generated by [`ts-rs`](https://crates.io/crates/ts-rs) from the Rust structs, so `tsc` rejects a read of a field the structs do not have. Every `u64` is declared as `number`, which is what `JSON.parse` returns. After changing a snapshot struct, regenerate the file, then type check the client (and run `npm run build` if a module in `src/` changed). `tsconfig.json` sets `skipLibCheck` to false so `tsc` also checks the generated file:

```bash
GOOSE_UPDATE_DASHBOARD_TYPES=1 cargo test --lib dashboard_types_match_rust
cd src/dashboard/static
npm run check
```

`ts-rs` is a dev dependency used only by unit tests, so crates that depend on Goose never pull it in. Without the environment variable, `dashboard_types_match_rust` compares the committed file with the structs and fails when they differ (a reordered field counts as a difference), so a struct change without a regenerated `snapshot.d.ts` fails `cargo test` in CI, and a client that reads a removed or renamed field then fails `npm run check`. A second test fails on any serde attribute or hand-written `Serialize` in the snapshot module, because `ts-rs` cannot see either, so the JSON could change without the types changing.

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

The tests load the bundled `app.js` into a jsdom page with a stubbed `fetch`, so rebuild it first.

### Notes

- Do not hand-edit `app.js`; changes there will be lost on the next `npm run build`.
- `node_modules/` is gitignored; only `package.json` / `package-lock.json` are committed so installs stay reproducible.
- Chart.js is bundled into `app.js`; the server does not serve it as a separate file.
- CI runs `npm run check`, rebuilds `app.js` with `npm ci && npm run build` (failing if the committed file differs from the esbuild bundle), and runs `npm test`.

## GraphData memory cost

Enabling `--dashboard` turns on the same per-second **GraphData** series collection used for HTML report graphs (also enabled by `--report-file` and by [saving runs](../getting-started/common.md#saved-runs), the default). Every saved run records it and keeps every second of the whole run for the report, with or without `--dashboard`, and so does `--report-file`. `--no-save` restores the earlier behaviour: without `--dashboard` or `--report-file` nothing is recorded, and with `--dashboard` alone the dashboard keeps only twice its chart window (10 minutes). That cost applies for the whole run even when no browser is connected:

- Per-second counters for requests, errors, users, and average latency are retained for the duration of the test.
- Memory scales with unique request names × run length (same class of cost as generating HTML graphs).
- Request and error tables truncate to the top rows (`flags.requests_truncated` / `flags.errors_truncated` when capped); series charts use a fixed trailing window (default 5 minutes).

With no SSE client connected and no `GET /api/v1/snapshot` in the last two seconds (`/api/v1/health` does not count), the dashboard issues **no** snapshot builds (no extra metrics-processor work beyond GraphData recording). When clients are connected, snapshots are coalesced to about **1 Hz** for all viewers. Concurrent SSE clients are capped (default **32**, tunable with `--dashboard-max-clients`); further clients receive HTTP 503.

Each snapshot build (while clients are connected or polling) runs on the **metrics processor** task: it drains pending metrics, exports the trailing series window, and builds percentile maps for the aggregate, up to 100 request rows, and one row per scenario and transaction, a number fixed by the load test's code. That cost scales with unique request names and the timing histograms Goose already maintains; the attack main loop does **not** await the build. Under extreme cardinality, prefer fewer unique request names, a shorter test, or disable the dashboard if you need absolute minimal overhead (same trade-off as `--report-file`).

With `--dashboard`, and also for a saved run or a `--report-file`, Goose keeps one response time histogram per second for the last 300 seconds, for all requests together and for each of the first 100 request names, plus the same for synthetic times while coordinated omission mitigation is on. It does not grow with run length. Its size depends on how many request names there are and how many distinct response times occur: about 0.2 MB for all requests and 10 MB for 100 names with typical response times, and up to 30 MB in the worst case at the default 60 second timeout, more with a longer `--timeout`. Filling it costs little: in a test of 1,000 users at about 21,800 requests a second over 25 request names, the heatmap added no measurable CPU or instructions per request, the difference from the dashboard without it being within the runs' spread of about 1%, and 3.1 MiB of peak memory. Each user's batch of metrics is still sent every 100 requests or 250 ms, as without the heatmap; a batch keeps each request name's response times for the second it is filling and the next one, and a request from any other second sends it early. For the HTML report's heatmap, each second is also added once to whole run columns, up to 600 for all requests and 60 for each of those 100 names, which merge in pairs as they fill, so they too stop growing. `--no-percentiles-over-time` turns all of it off, along with the small per request cost of filling it.

For extreme runs with tens of thousands of unique request names, expect higher GraphData memory (same as `--report-file`). Disable the dashboard if you need absolute minimal overhead, just as you can disable Controllers with `--no-telnet --no-websocket`.

## Metrics disabled

If Goose is started with `--no-metrics`, the dashboard still serves the shell and health endpoint, but snapshots report `flags.metrics_disabled: true` with empty request/error tables and empty series. The UI surfaces an honest empty state rather than fabricating data.

Scenario and transaction metrics can also be turned off on their own:

- `--no-transaction-metrics` sets `flags.transaction_metrics_disabled: true`; `transactions` is empty and the Transactions table says transaction metrics are disabled.
- `--no-scenario-metrics` sets `flags.scenario_metrics_disabled: true`; `scenarios` is empty and the Scenarios table says scenario metrics are disabled.

Both flags are also true under `--no-metrics`, where both tables say metrics are disabled.

## Security notes (summary)

- **Default off**, **default loopback** bind; control is a **separate** opt-in (`--dashboard-control`).
- Non-loopback without a configured token → **startup hard-fail**.
- Control without a token → **startup hard-fail**, even on loopback.
- Token protects **metric APIs** when configured; control POSTs **always** require the token when control is on.
- Shell/static/health stay public and contain no load-test metrics.
- Prefer `Authorization: Bearer` for scripts; browsers use `?token=` for metrics (EventSource limits) and **Bearer only** for control POSTs (server rejects query tokens on `/api/v1/control/*`).
- Query tokens on metric URLs can appear in reverse-proxy access logs; prefer SSH tunnels or a local reverse proxy when that matters. Every response sends `Referrer-Policy: no-referrer`, so the page does not pass its `?token=` URL on as a `Referer`.
- `--dashboard-auth-token` is visible in process listings (`ps`) like any CLI argument; for shared hosts prefer a short-lived secret and restricted process visibility.
- Goose never logs the token value; the startup line is only `listening on http://{host:port} (read-only|control enabled)` with no query secret.
- The UI renders metric fields with `textContent` only (no `innerHTML`) and serves a strict Content-Security-Policy without `'unsafe-inline'` scripts.
- No session cookies and no permissive CORS; classic cross-site cookie CSRF does not apply.
- Every response sends `Cache-Control: no-store` (the first page load carries `?token=` and the APIs carry metrics) and `X-Content-Type-Options: nosniff`.
- On a loopback bind, requests whose `Host` is not loopback get 403 (DNS rebinding).
- A client gets 10 seconds to send a request's headers, on a new connection and between requests on a keep-alive one, and the server holds at most `2 × --dashboard-max-clients + 16` connections, so a client that sends nothing is closed after 10 seconds and the number of open connections is bounded. Long-lived SSE streams are not cut: the timeout covers headers only.

## Relationship to Controllers

| Need | Prefer |
|------|--------|
| Watch live charts and tables | Live Dashboard (`--dashboard`) |
| Start / Stop / set users, host, ramps and run time from a browser | Dashboard control (`--dashboard-control` + token) |
| Test plan from a script | Dashboard control (`/api/v1/control/settings`) or the Controllers |
| Full command set, `config` and `metrics` | [Telnet](telnet.md) / [WebSocket](websocket.md) Controllers |
| Process shutdown | Controllers only (`shutdown`) |

Dashboard control reuses the same phase-transition logic as Controllers (shared helpers), and Controllers remain available alongside the dashboard. One Controller command behaves differently since the settings form: `runtime` on a running users ramp changes this run, where it was refused unless idle; see [Run time on a running test](#run-time-on-a-running-test).

For full Controller documentation, see [Controlling A Running Goose Load Test](overview.md).
