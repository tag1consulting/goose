# Controlling A Running Goose Load Test

There are four ways to watch or change a load test while it runs: the Live Dashboard and three Controllers.

- The [Live Dashboard](dashboard.md) is a page in your browser that shows the test as it runs and saves its report, and, with control turned on, starts and stops it and changes its users and settings. Pick it to watch a test, or to steer one by hand.
- The [telnet Controller](telnet.md) takes typed commands. Pick it for `config` and `metrics` at a prompt, a test plan typed by hand, or shutting Goose down mid run.
- The [WebSocket Controller](websocket.md) takes the same commands as JSON. Pick it to drive Goose from a program.
- The [MCP Controller](mcp.md) serves the same commands as Model Context Protocol tools. Pick it to let an AI agent, such as Claude Code, watch or drive the test. It needs Goose built with the `mcp` feature.

## What each can do

| | Live Dashboard | Telnet | WebSocket | MCP |
|---|---|---|---|---|
| Observe | Live charts, KPIs and tables | `metrics`, `metrics-json` | `metrics`, `metrics-json` | `get_metrics` |
| Start, stop | With control on | `start`, `stop` | `start`, `stop` | `start`, `stop` |
| Users | With control on | `users` | `users` | `set_users` |
| Host and ramps | With control on, in the Edit settings form | `host`, `increaserate`, `increase-time`, `decreaserate`, `decrease-time` | the same commands | `set_host`, `increase_rate`, `increase_time`, `decrease_rate`, `decrease_time` |
| Test plan | With control on, through [`POST /api/v1/control/settings` or `POST /api/v1/control/start`](dashboard-reference.md#run-settings); the form has no plan field | `test-plan` | `test-plan` | `set_test_plan` |
| Run time | With control on, in the Edit settings form | `runtime` | `runtime` | `set_run_time` |
| Throttle | With control on, through [`POST /api/v1/control/settings`](dashboard-reference.md#the-throttle-on-a-running-test); the form has no throttle field | `throttle` | `throttle` | `set_throttle` |
| Save report | On by default with `--dashboard`; turning it off and Save report now need control | No | No | No |
| `config` | No | `config`, `config-json` | `config`, `config-json` | `get_config` |
| Saved runs | The runs panel: list, open, compare and delete | `runs`, `runs-json`, `report`, `report-json`, `compare` | the same commands | `list_runs`, `get_run_report`, `compare_runs` |
| Shut Goose down | With control on: Quit, from Idle only | `shutdown`, from any phase | `shutdown`, from any phase | `shutdown`, from any phase |
| Default | Off; `--dashboard` turns it on, on `127.0.0.1:5118` | On, on `127.0.0.1:5116` | Off; `--websocket` turns it on, on `127.0.0.1:5117` | Off; needs the `mcp` feature, then `--mcp` turns it on, on `127.0.0.1:5119` |
| Authentication | Read-only on loopback: none, unless a token is set. Control, or any bind off loopback: a token, or `--dashboard-no-auth` | None | None | None on loopback, unless `--mcp-token` is set; off loopback, `--mcp-token` is required |

The dashboard is read-only unless control is turned on with `--dashboard-control` and a token or `--dashboard-no-auth`; see [Turning it on](dashboard.md#turning-it-on). It needs Goose built with the `dashboard` feature.

## The telnet and WebSocket Controllers

By default, Goose launches a telnet Controller thread that listens on `127.0.0.1:5116`. With `--websocket` it also launches a WebSocket Controller thread that listens on `127.0.0.1:5117`; that one is off by default, as any web page a browser opens could reach it. Neither takes any authentication, so anyone who can reach their ports can start, stop and reconfigure the load test, and read every saved run in the run folder, which is why both listen only on loopback unless told otherwise. To control the load test from another machine, forward the port over ssh, or name a host with `--telnet-host` or `--websocket-host`, such as `0.0.0.0`; a Controller listening on any address that is not loopback prints a warning at startup. Turn the telnet Controller off with `--no-telnet`.

Goose can be started with the `--no-autostart` option to keep the load test from starting on its own. A Controller's `start` command then starts it, as does the dashboard's Start under `--dashboard-control`. Goose refuses `--no-autostart` when neither is available. When Goose is started this way, a host is not required and can instead be configured through a Controller or the dashboard. A load test that a Controller or the dashboard starts again after it stopped counts its requests from zero.

With `--telnet-host 0.0.0.0`, Goose starts and writes this warning straight to standard error, where neither `-q` nor the log level hides it, and to the Goose log at warn level; the WebSocket Controller's warning names `--websocket-host` and port 5117:

```text
==============================================================================
WARNING: the Telnet Controller on 0.0.0.0:5116 has no authentication.
Anyone who can reach this port can start, stop and reconfigure the load test,
and read every saved run in the run folder.
Safer: leave --telnet-host unset to listen on 127.0.0.1 only, and use an
ssh forward (ssh -L 5116:127.0.0.1:5116 host).
==============================================================================
```

A Controller that can't listen on its port, because another process already holds it, logs an error and the load test runs without it:

```text
22:10:32 [ERROR] [controller]: the Telnet Controller couldn't listen on 127.0.0.1:5116: Address already in use (os error 48); the load test runs without it
```

With `--no-autostart`, if no Controller could listen and dashboard control is off, nothing could start the load test, so Goose refuses to run:

```text
Error: invalid option or value specified (`configuration.no_autostart` "true": `configuration.no_autostart` requires a Controller or dashboard control, and no Controller could listen on its port.)
```

## Saved runs

Goose saves each run in a folder of its own, in `goose-runs`, or in the folder `--output-dir` sits in, as the [Live Dashboard's runs panel](dashboard.md) shows them. Every Controller reads those saved runs too, whether the load test is idle or running:

| Telnet and WebSocket | MCP | What it gives |
|---|---|---|
| `runs [BEFORE]`, `runs-json [BEFORE]` | `list_runs`, with `before` | The newest 20 saved runs, newest first, or the 20 after the run `BEFORE`: each run's folder name, when it started, how long it ran, its most users, requests and failed requests, and how it ended, `running` for the run in progress; then whether older runs follow. The JSON form is `{"dir", "runs", "more"}`, each run with the fields `GET /api/v1/reports` gives it |
| `report [RUN]`, `report-json [RUN]` | `get_run_report`, with `run` and `format` (`markdown` or `json`) | The run's Markdown report, as the runs panel serves it: the folder's `report.md`, or one rendered from its `report.json`. The JSON form is its `report.json` without the per second graph data and heatmap, refused when what is left is over 4 MiB; `report` gives the Markdown of such a run |
| `compare [RUN] BASE` | `compare_runs`, with `run` and `baseline` | The run compared with the run `BASE`, as Markdown, as the runs panel's compare serves it, starting with which run is compared with which |

A run is named by its folder's name, such as `20261106-101500`, as `runs` lists it; with no run named, `report` and `compare` take the newest saved run. A name is only ever a folder in the run folder: anything else, a path, `..`, a hidden folder, a symlink or a folder with no regular `report.json`, is refused with one message, `no saved run named "..."`, which names no path. No Controller writes, deletes or renames a run; Delete is the dashboard's. The Controller reads the run folder itself, off the load test's main loop, so a large report never slows the load test, and it waits at most 15 seconds for the read.

To browse old runs without running a load test, start Goose with `--no-autostart` from the folder that holds `goose-runs`; no host is needed until a run starts:

```bash
cargo run --release --example simple -- --no-autostart
```

For runs saved somewhere else, give a run folder in that folder, which only a run started later would use, with an output flag that needs it: `--no-autostart --output-dir /var/loadtests/next --report-file` reads the runs in `/var/loadtests`.

Anyone who can reach a Controller can read every saved run in the run folder, reports and comparisons included, as they can start and stop the load test: telnet and WebSocket take no authentication, so keep them on loopback, and keep the MCP Controller on loopback or behind its token.

## Controlling a Gaggle

On a [Gaggle](../gaggle/overview.md) manager, the telnet and WebSocket Controllers and dashboard control take the same commands, and each change reaches every worker as its share, so the workers together run what one process running the whole load test would. The manager checks a change against the rules one process applies before it sends anything, applies it to its own run, the run's clock, and answers once every worker has received its share: within 10 seconds, inside the 15 a client waits, however long the workers take to apply it. Later commands wait their turn meanwhile; the load test runs on.

| Command | What each worker gets |
|---------|-----------------------|
| `users`, the dashboard's users, `users` in dashboard settings | Its users from now: more users take the next ordinals, dealt to the workers running fewest, each with the scenario one process would give it; fewer users stop those with the highest ordinals, as one process stops its last started first. Each worker's ramp takes as long as the whole load test's |
| `increaserate`, `decreaserate` | The rate times its share of the users |
| `increase-time`, `decrease-time` | The same time |
| `test-plan` | The plan split step by step, so each step's users add up to the plan's |
| `throttle` | Its share by its share of the users, the remainder spread, never 0; refused below one request a second for each worker |
| `host` | The same host |
| `start`, `stop`, `shutdown`, the dashboard's Quit and Start with settings | Each, as the manager takes it |
| `runtime` | The same run time: each worker's run keeps the same clock as the manager's |
| `config`, `metrics` and their JSON forms, Save report, the report switches | Nothing: the manager answers from its configuration and the merged metrics |
| `runs`, `report`, `compare` and their JSON forms | Nothing: the manager's Controllers read the manager's run folder |

A change the manager makes in Idle is kept for the next Start, whose shares carry it. A worker that does not say it received a change within 10 seconds is marked lost, sent `Shutdown` and disconnected, and the reply names it; the change stands on the other workers. The manager's Controllers listen on loopback like any others, so reaching them from another machine takes `--telnet-host`, or `--websocket` with `--websocket-host`, and the warning that comes with them. See [Runtime control](../gaggle/manager.md#runtime-control).
