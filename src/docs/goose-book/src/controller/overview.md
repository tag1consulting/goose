# Controlling A Running Goose Load Test

There are three ways to watch or change a load test while it runs: the Live Dashboard and two Controllers.

- The [Live Dashboard](dashboard.md) is a page in your browser that shows the test as it runs and saves its report, and, with control turned on, starts and stops it and changes its users and settings. Pick it to watch a test, or to steer one by hand.
- The [telnet Controller](telnet.md) takes typed commands. Pick it for `config` and `metrics` at a prompt, a test plan typed by hand, or shutting Goose down mid run.
- The [WebSocket Controller](websocket.md) takes the same commands as JSON. Pick it to drive Goose from a program.

## What each can do

| | Live Dashboard | Telnet | WebSocket |
|---|---|---|---|
| Observe | Live charts, KPIs and tables | `metrics`, `metrics-json` | `metrics`, `metrics-json` |
| Start, stop | With control on | `start`, `stop` | `start`, `stop` |
| Users | With control on | `users` | `users` |
| Host and ramps | With control on, in the Edit settings form | `host`, `increaserate`, `increase-time`, `decreaserate`, `decrease-time` | the same commands |
| Test plan | With control on, through [`POST /api/v1/control/settings` or `POST /api/v1/control/start`](dashboard-reference.md#run-settings); the form has no plan field | `test-plan` | `test-plan` |
| Run time | With control on, in the Edit settings form | `runtime` | `runtime` |
| Save report | On by default with `--dashboard`; turning it off and Save report now need control | No | No |
| `config` | No | `config`, `config-json` | `config`, `config-json` |
| Shut Goose down | With control on: Quit, from Idle only | `shutdown`, from any phase | `shutdown`, from any phase |
| Default | Off; `--dashboard` turns it on, on `127.0.0.1:5118` | On, on `127.0.0.1:5116` | Off; `--websocket` turns it on, on `127.0.0.1:5117` |
| Authentication | Read-only on loopback: none, unless a token is set. Control, or any bind off loopback: a token, or `--dashboard-no-auth` | None | None |

The dashboard is read-only unless control is turned on with `--dashboard-control` and a token or `--dashboard-no-auth`; see [Turning it on](dashboard.md#turning-it-on). It needs Goose built with the `dashboard` feature.

## The telnet and WebSocket Controllers

By default, Goose launches a telnet Controller thread that listens on `127.0.0.1:5116`. With `--websocket` it also launches a WebSocket Controller thread that listens on `127.0.0.1:5117`; that one is off by default, as any web page a browser opens could reach it. Neither takes any authentication, so anyone who can reach their ports can start, stop and reconfigure the load test, which is why both listen only on loopback unless told otherwise. To control the load test from another machine, forward the port over ssh, or name a host with `--telnet-host` or `--websocket-host`, such as `0.0.0.0`; a Controller listening on any address that is not loopback prints a warning at startup. Turn the telnet Controller off with `--no-telnet`.

Goose can be started with the `--no-autostart` option to keep the load test from starting on its own. A Controller's `start` command then starts it, as does the dashboard's Start under `--dashboard-control`. Goose refuses `--no-autostart` when neither is available. When Goose is started this way, a host is not required and can instead be configured through a Controller or the dashboard. A load test that a Controller or the dashboard starts again after it stopped counts its requests from zero.

With `--telnet-host 0.0.0.0`, Goose starts and writes this warning straight to standard error, where neither `-q` nor the log level hides it, and to the Goose log at warn level; the WebSocket Controller's warning names `--websocket-host` and port 5117:

```text
==============================================================================
WARNING: the Telnet Controller on 0.0.0.0:5116 has no authentication.
Anyone who can reach this port can start, stop and reconfigure the load test.
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
Error: InvalidOption { option: "`configuration.no_autostart`", value: "true", detail: "`configuration.no_autostart` requires a Controller or dashboard control, and no Controller could listen on its port." }
```
