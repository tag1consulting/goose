# Live Dashboard

Goose can optionally serve a **live web dashboard** while a load test runs. The dashboard streams compact metric snapshots (including trailing RPS, users, and latency series) over Server-Sent Events (SSE), with a short poll fallback if SSE is unavailable.

By default the dashboard is **observe-only**: start, stop, user counts, and other control remain on the [Telnet](telnet.md) and [WebSocket](websocket.md) Controllers. With an explicit opt-in (`--dashboard-control` plus a required auth token), the same browser can also **Start**, **Stop**, and **set the target user count**. Controllers remain the power-user path for host/test-plan changes, rate tuning, and process **shutdown** (not available from the dashboard).

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

### Related flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--dashboard` | off | Enable the live dashboard HTTP server (observe) |
| `--dashboard-control` | off | Enable Start/Stop/Users control endpoints and UI (requires `--dashboard`; does **not** auto-enable the dashboard) |
| `--dashboard-host HOST` | `127.0.0.1` | Bind address |
| `--dashboard-port PORT` | `5118` | Bind port. CLI value `0` means “unset” and is rewritten to `5118` when the dashboard is enabled (not an ephemeral OS port). |
| `--dashboard-auth-token TOKEN` | empty | Shared secret for metric APIs when configured; **always required** when `--dashboard-control` is set (even on loopback). Visible in process listings (`ps`) like any CLI flag. |
| `--dashboard-max-clients COUNT` | `32` | Max concurrent SSE clients (`GET /api/v1/events`); further clients receive HTTP 503. CLI value `0` means “unset” and becomes `32`. |

Defaults can also be set programmatically with `GooseDefault::Dashboard`, `GooseDefault::DashboardControl`, `GooseDefault::DashboardHost`, `GooseDefault::DashboardPort`, `GooseDefault::DashboardAuthToken`, and `GooseDefault::DashboardMaxClients`.

> **Feature flag:** the HTTP server is compiled behind the opt-in `dashboard` crate feature (not in default features, same pattern as `pdf-reports`). Enable it with `--features dashboard`. Builds without that feature reject `--dashboard` at startup with a clear rebuild hint.

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
| `GET /api/v1/health` | **Public** | Liveness + ops counters; includes `control_enabled` (no load-test metrics) |
| `GET /api/v1/snapshot` | **Required** | |
| `GET /api/v1/events` (SSE) | **Required** | |
| `POST /api/v1/control/*` | **Always required** when control is enabled | Even on loopback; **404** when control is off |

The shell and static assets stay public because classic `<script src>` loads cannot forward a document `?token=` query (or an in-memory secret). Sensitive data lives only in the snapshot APIs; mutation lives only on control POSTs.

### Browser bootstrap with `?token=`

Browsers' `EventSource` cannot set `Authorization` headers. For remote or token-protected dashboards, open:

```text
http://{host}:{port}/?token=SECRET
```

The embedded UI reads `token` from the URL on first load, strips it from the address bar with `history.replaceState`, and attaches `?token=` to `/api/v1/snapshot` and `/api/v1/events` requests.

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

Open `http://127.0.0.1:5118/?token=$DASHBOARD_TOKEN`. The header shows `Live dashboard · control`, and a control panel appears with **Start**, **Stop**, and a target-users input (`Apply`, `−` / `+` step buttons).

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
| `POST` | `/api/v1/control/start` | empty or `{}` | Start an idle load test → enters **Increase** |
| `POST` | `/api/v1/control/stop` | empty or `{}` | Begin cancel → enters **Decrease** (eventual Idle) |
| `POST` | `/api/v1/control/users` | `{"users": N}` | Set absolute target user count (`N` integer, 1–100_000) |

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

# Set absolute user count
curl -sS -X POST \
  -H "Authorization: Bearer $DASHBOARD_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"users":100}' \
  "$BASE/api/v1/control/users"
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

Logical rejections (wrong phase, prepare failure) return **HTTP 200** with `"ok": false` and a stable `error` code (for example `invalid_phase`). Malformed bodies return **400**.

### Start and Stop semantics

**Start success means the test entered Increase**, not that `test_start` transactions finished. Goose replies as soon as the phase transitions (same timing as Controllers), then runs `reset_run_state` / user `test_start` on the main loop. A long `test_start` does not block the HTTP 200.

**Stop is not instantaneous idle.** A successful Stop begins a **cancel ramp** through `AttackPhase::Decrease` (Canceling). Active users wind down according to decrease rate/time settings. The process returns to **Idle** only after decrease finishes (and Goose is not configured to shut down after stop). While phase is `decrease`:

- **Stop** is disabled (cannot stop again mid-ramp)
- **Start** stays disabled until phase is `idle` again
- **Users** remains allowed (same as Controllers)

### UI walkthrough (control panel)

When `health.control_enabled` is true and the SPA has a token:

1. **Phase badge** still shows `idle` / `increase` / `maintain` / `decrease` / `shutdown`.
2. **Start** is enabled only in `idle`.
3. **Stop** is enabled in `increase` and `maintain` only.
4. **Target users** — enter an absolute count and **Apply**, or use **−** / **+** with a configurable step (default 10). Active user count is a read-only label from the latest snapshot.
5. Status line shows server `message` on success, or an error banner on soft failure / 401 / 503.

There is no process-shutdown button in the dashboard. Use a Controller `shutdown` command when you need to exit the Goose process.

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
  "build_count": 42,
  "active_sse_clients": 1,
  "control_enabled": false
}
```

| Field | Meaning |
|-------|---------|
| `ok` | Always `true` when the handler runs |
| `version` | Goose crate package version (`CARGO_PKG_VERSION`) |
| `last_build_ms` | Wall-clock duration of the last successful snapshot build in milliseconds (`0` if none yet) |
| `build_count` | Successful hub snapshot builds since process start |
| `active_sse_clients` | Concurrent SSE clients currently holding a slot |
| `control_enabled` | Whether Start/Stop/Users routes are registered (`--dashboard-control`) |

These counters are ops-safe (timing and client counts only). The response does **not** include rates, hosts under test, request names, or error strings. Snapshot build duration is also logged at `debug` with a `[dashboard]` prefix when a build completes.

## What the UI shows

- **Phase badge** — `idle` (gray), `increase` (blue), `maintain` (green), `decrease` (orange), `shutdown` (red)
- **Connection indicator** — green for SSE, yellow for poll fallback, red when disconnected
- **KPI strip** — users, RPS, fail %, p95, average latency
- **Charts** — trailing series window (default 300 seconds) for RPS + failures/s, active users, and average latency
- **Sortable tables** — top request and error rows (truncated server-side for large runs)
- **Control panel** (only when `--dashboard-control`) — Start / Stop / target users; see [Controlling a load test from the dashboard](#controlling-a-load-test-from-the-dashboard)

When control is off there are **no control buttons**. Use the Controllers (or enable dashboard control) to change the running test.

## Building the dashboard UI (TypeScript)

The browser SPA lives under `src/dashboard/static/`. **Edit TypeScript, not the compiled JavaScript:**

| File | Role |
|------|------|
| `app.ts` | Source of truth for dashboard client logic |
| `chart-global.d.ts` | Ambient types for the vendored Chart.js UMD build |
| `tsconfig.json` / `package.json` | TypeScript toolchain config |
| `app.js` | **Compiled output** embedded by the Rust server via `include_str!` |
| `index.html`, `app.css`, `chart.min.js` | Shell, styles, and vendored Chart.js (not generated by `tsc`) |

Cargo builds do **not** run npm. The committed `app.js` is what gets embedded, so a normal `cargo build` works without Node.js. After changing `app.ts` (or the related type declarations), regenerate `app.js` and commit both files together.

### Prerequisites

- [Node.js](https://nodejs.org/) (with npm)
- Network access once to install the TypeScript dev dependency

### Rebuild

From the repository root:

```bash
cd src/dashboard/static
npm install
npm run build
```

That runs `tsc -p tsconfig.json` and overwrites `app.js` in the same directory. Then rebuild Goose so the new script is embedded:

```bash
cargo build
# or exercise the dashboard:
cargo run --release --features dashboard --example simple -- --dashboard ...
```

### Type-check without emitting

```bash
cd src/dashboard/static
npm run check
```

### Notes

- Do not hand-edit `app.js`; changes there will be lost on the next `npm run build`.
- `node_modules/` is gitignored; only `package.json` / `package-lock.json` are committed so installs stay reproducible.
- Chart.js is loaded as a separate UMD script (`chart.min.js`); the TypeScript build does not bundle it.
- CI runs `npm run check` and rebuilds `app.js`, failing if the committed file differs from the TypeScript compiler output.

## GraphData memory cost

Enabling `--dashboard` turns on the same per-second **GraphData** series collection used for HTML report graphs (also enabled by `--report-file`). That cost applies for the whole run even when no browser is connected:

- Per-second counters for requests, errors, users, and average latency are retained for the duration of the test.
- Memory scales with unique request names × run length (same class of cost as generating HTML graphs).
- Snapshot tables truncate to the top rows (`flags.requests_truncated` / `flags.errors_truncated` when capped); series charts use a fixed trailing window (default 5 minutes).

With zero connected clients the dashboard issues **no** snapshot builds (no extra metrics-processor work beyond GraphData recording). When clients are connected, snapshots are coalesced to about **1 Hz** for all viewers. Concurrent SSE clients are capped (default **32**, tunable with `--dashboard-max-clients`); further clients receive HTTP 503.

Each snapshot build (while clients are connected) runs on the **metrics processor** task: it drains pending metrics, exports the trailing series window, and builds percentile maps for the aggregate plus up to 100 request rows. That cost scales with unique request names and the timing histograms Goose already maintains — the attack main loop does **not** await the build. Under extreme cardinality, prefer fewer unique request names, a shorter test, or disable the dashboard if you need absolute minimal overhead (same trade-off as `--report-file`).

For extreme runs with tens of thousands of unique request names, expect higher GraphData memory (same as `--report-file`). Disable the dashboard if you need absolute minimal overhead, just as you can disable Controllers with `--no-telnet --no-websocket`.

## Metrics disabled

If Goose is started with `--no-metrics`, the dashboard still serves the shell and health endpoint, but snapshots report `flags.metrics_disabled: true` with empty request/error tables and empty series. The UI surfaces an honest empty state rather than fabricating data.

## Security notes (summary)

- **Default off**, **default loopback** bind; control is a **separate** opt-in (`--dashboard-control`).
- Non-loopback without a configured token → **startup hard-fail**.
- Control without a token → **startup hard-fail**, even on loopback.
- Token protects **metric APIs** when configured; control POSTs **always** require the token when control is on.
- Shell/static/health stay public and contain no load-test metrics.
- Prefer `Authorization: Bearer` for scripts; browsers use `?token=` for metrics (EventSource limits) and **Bearer only** for control POSTs (server rejects query tokens on `/api/v1/control/*`).
- Query tokens on metric URLs can appear in reverse-proxy access logs and `Referer` headers — prefer SSH tunnels or a local reverse proxy when that matters.
- `--dashboard-auth-token` is visible in process listings (`ps`) like any CLI argument; for shared hosts prefer a short-lived secret and restricted process visibility.
- Goose never logs the token value; the startup line is only `listening on http://{host:port} (read-only|control enabled)` with no query secret.
- The UI renders metric fields with `textContent` only (no `innerHTML`) and serves a strict Content-Security-Policy without `'unsafe-inline'` scripts.
- No session cookies and no permissive CORS; classic cross-site cookie CSRF does not apply.

## Relationship to Controllers

| Need | Prefer |
|------|--------|
| Watch live charts and tables | Live Dashboard (`--dashboard`) |
| Start / Stop / set users from a browser | Dashboard control (`--dashboard-control` + token) |
| Host, test plan, increase/decrease rates, full command set | [Telnet](telnet.md) / [WebSocket](websocket.md) Controllers |
| Process shutdown | Controllers only (`shutdown`) |

Dashboard control reuses the same phase-transition logic as Controllers (shared helpers). Controllers are unchanged and remain available alongside the dashboard.

For full Controller documentation, see [Controlling A Running Goose Load Test](overview.md).
