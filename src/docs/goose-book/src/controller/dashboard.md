# Live Dashboard

Goose can optionally serve a **read-only live web dashboard** while a load test runs. The dashboard streams compact metric snapshots (including trailing RPS, users, and latency series) over Server-Sent Events (SSE), with a short poll fallback if SSE is unavailable.

The dashboard is for **observation only**. Start, stop, user counts, and other control remain on the [Telnet](telnet.md) and [WebSocket](websocket.md) Controllers.

## Enabling the dashboard

The dashboard is off by default. Enable it with `--dashboard`:

```bash
cargo run --release --example simple -- \
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
| `--dashboard` | off | Enable the live dashboard HTTP server |
| `--dashboard-host HOST` | `127.0.0.1` | Bind address |
| `--dashboard-port PORT` | `5118` | Bind port |
| `--dashboard-auth-token TOKEN` | empty | Shared secret for metric APIs (required when host is not loopback) |
| `--dashboard-max-clients COUNT` | `32` | Max concurrent SSE clients (`GET /api/v1/events`); further clients receive HTTP 503 |

Defaults can also be set programmatically with `GooseDefault::Dashboard`, `GooseDefault::DashboardHost`, `GooseDefault::DashboardPort`, `GooseDefault::DashboardAuthToken`, and `GooseDefault::DashboardMaxClients`.

> **Feature flag:** the HTTP server is compiled behind the `dashboard` crate feature (enabled by default). Builds with `--no-default-features` (and without `--features dashboard`) reject `--dashboard` at startup.

## Loopback default and auth policy

By default the dashboard binds to **loopback only** (`127.0.0.1`). That is intentional: metrics snapshots include request names, error strings, hostnames, and rates that should not be world-readable.

Goose treats a bind host as loopback when it is:

- `localhost` (case-insensitive, no DNS lookup)
- any IPv4 address in `127.0.0.0/8` (for example `127.0.0.1`, `127.0.0.2`)
- IPv6 loopback `::1` (with or without brackets: `[::1]`)

**Non-loopback** binds — including unspecified addresses that listen on all interfaces (`0.0.0.0`, `::`) and LAN/hostnames — **require** `--dashboard-auth-token` at startup. Goose hard-fails with a clear error if the token is missing.

If a token **is** configured, it is enforced on metric APIs even on loopback.

## What requires a token

| Path | Auth when token configured |
|------|----------------------------|
| `GET /` (SPA shell) | **Public** |
| `GET /static/*` | **Public** |
| `GET /api/v1/health` | **Public** (liveness + ops counters; no load-test metrics) |
| `GET /api/v1/snapshot` | **Required** |
| `GET /api/v1/events` (SSE) | **Required** |

The shell and static assets stay public because classic `<script src>` loads cannot forward a document `?token=` query (or an in-memory secret). Sensitive data lives only in the snapshot APIs.

### Browser bootstrap with `?token=`

Browsers' `EventSource` cannot set `Authorization` headers. For remote or token-protected dashboards, open:

```text
http://{host}:{port}/?token=SECRET
```

The embedded UI reads `token` from the URL on first load, strips it from the address bar with `history.replaceState`, and attaches `?token=` to `/api/v1/snapshot` and `/api/v1/events` requests.

If the shell loads without a token when one is required, the page still renders, but metric calls return **401** and the UI shows:

```text
Open this dashboard as http://host:port/?token=… (token required for metrics).
```

Non-browser clients may use either:

```bash
curl -H "Authorization: Bearer SECRET" http://host:5118/api/v1/snapshot
# or
curl "http://host:5118/api/v1/snapshot?token=SECRET"
```

### Example: bind on all interfaces

```bash
export DASHBOARD_TOKEN="replace-me"

cargo run --release --example simple -- \
  --dashboard \
  --dashboard-host 0.0.0.0 \
  --dashboard-auth-token "$DASHBOARD_TOKEN" \
  -H https://staging.example.com \
  -u 200 -t 1h
```

Open `http://ci-host:5118/?token=$DASHBOARD_TOKEN`.

## SSH tunnels

Prefer keeping Goose on loopback and tunneling from your laptop:

```bash
ssh -L 5118:127.0.0.1:5118 loadgen.example.com
# open http://127.0.0.1:5118/ locally
```

With a loopback bind and no auth token, no `?token=` is required over the tunnel. If you still set a token on loopback, pass it in the browser URL as above.

TLS termination is not provided by Goose; use an SSH tunnel or reverse proxy when you need encryption in transit.

## Health endpoint exception

`GET /api/v1/health` is intentionally **unauthenticated** so external monitors can check that the dashboard process is up without holding the metrics secret. Example response:

```json
{
  "ok": true,
  "version": "<goose package version>",
  "last_build_ms": 2,
  "build_count": 42,
  "active_sse_clients": 1
}
```

| Field | Meaning |
|-------|---------|
| `ok` | Always `true` when the handler runs |
| `version` | Goose crate package version (`CARGO_PKG_VERSION`) |
| `last_build_ms` | Wall-clock duration of the last successful snapshot build in milliseconds (`0` if none yet) |
| `build_count` | Successful hub snapshot builds since process start |
| `active_sse_clients` | Concurrent SSE clients currently holding a slot |

These counters are ops-safe (timing and client counts only). The response does **not** include rates, hosts under test, request names, or error strings. Snapshot build duration is also logged at `debug` with a `[dashboard]` prefix when a build completes.

## What the UI shows

- **Phase badge** — `idle` (gray), `increase` (blue), `maintain` (green), `decrease` (orange), `shutdown` (red)
- **Connection indicator** — green for SSE, yellow for poll fallback, red when disconnected
- **KPI strip** — users, RPS, fail %, p95, average latency
- **Charts** — trailing series window (default 300 seconds) for RPS + failures/s, active users, and average latency
- **Sortable tables** — top request and error rows (truncated server-side for large runs)

There are **no control buttons**. Use the Controllers to change the running test.

## GraphData memory cost

Enabling `--dashboard` turns on the same per-second **GraphData** series collection used for HTML report graphs (also enabled by `--report-file`). That cost applies for the whole run even when no browser is connected:

- Per-second counters for requests, errors, users, and average latency are retained for the duration of the test.
- Memory scales with unique request names × run length (same class of cost as generating HTML graphs).
- Snapshot tables truncate to the top rows (`flags.requests_truncated` / `flags.errors_truncated` when capped); series charts use a fixed trailing window (default 5 minutes).

With zero connected clients the dashboard issues **no** snapshot builds (no extra metrics-processor work beyond GraphData recording). When clients are connected, snapshots are coalesced to about **1 Hz** for all viewers. Concurrent SSE clients are capped (default **32**, tunable with `--dashboard-max-clients`); further clients receive HTTP 503.

For extreme runs with tens of thousands of unique request names, expect higher GraphData memory (same as `--report-file`). Disable the dashboard if you need absolute minimal overhead, just as you can disable Controllers with `--no-telnet --no-websocket`.

## Metrics disabled

If Goose is started with `--no-metrics`, the dashboard still serves the shell and health endpoint, but snapshots report `flags.metrics_disabled: true` with empty request/error tables and empty series. The UI surfaces an honest empty state rather than fabricating data.

## Security notes (summary)

- **Default off**, **default loopback** bind.
- Non-loopback without a configured token → **startup hard-fail**.
- Token protects **metric APIs only**; shell/static/health stay public and contain no metrics.
- Prefer `Authorization: Bearer` for scripts; browsers use `?token=` because of EventSource limits.
- Query tokens can appear in reverse-proxy access logs and `Referer` headers — prefer SSH tunnels or a local reverse proxy when that matters.
- Goose never logs the token value; the startup line is only `listening on http://{host:port} (read-only)` with no query secret.
- The UI renders metric fields with `textContent` only (no `innerHTML`) and serves a strict Content-Security-Policy without `'unsafe-inline'` scripts.

## Observe vs control

| Capability | Live Dashboard | Controllers (telnet / WebSocket) |
|------------|----------------|----------------------------------|
| Live rates, percentiles, errors | Yes | `metrics` / `metrics-json` |
| Trailing RPS / users / latency charts | Yes | No |
| Start / stop / users / host / test plan | **No** | Yes |
| Default bind | `127.0.0.1:5118` | `0.0.0.0:5116` / `:5117` |
| Auth (v1) | Token on metric APIs when configured | None (see Controller docs) |

For control of a running load test, see [Controlling A Running Goose Load Test](overview.md).
