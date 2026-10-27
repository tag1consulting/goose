# Controlling A Running Goose Load Test

By default, Goose will launch a telnet Controller thread that listens on `127.0.0.1:5116`. With `--websocket` it also launches a WebSocket Controller thread that listens on `127.0.0.1:5117`; that one is off by default, as any web page a browser opens could reach it. The running Goose load test can be controlled through these Controllers. Neither takes any authentication, so anyone who can reach their ports can start, stop and reconfigure the load test, which is why both listen only on loopback unless told otherwise. To control the load test from another machine, forward the port over ssh, or name a host with `--telnet-host` or `--websocket-host`, such as `0.0.0.0`; a Controller listening on any address that is not loopback prints a warning at startup. Turn the telnet Controller off with `--no-telnet`. Goose can optionally be started with the `--no-autostart` run time option to prevent the load test from automatically starting, requiring instead that it be started with a Controller command, or from the dashboard with `--dashboard-control`; at least one of them must be enabled. When Goose is started this way, a host is not required and can instead be configured via the Controller or the dashboard. A load test that a Controller starts again after it stopped counts its requests from zero.

With `--telnet-host 0.0.0.0`, Goose starts and writes this warning straight to standard error, where neither `-q` nor the log level hides it, and to the Goose log at warn level; the WebSocket Controller's names `--websocket-host` and port 5117:

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
[ERROR] [controller]: the Telnet Controller couldn't listen on 127.0.0.1:5116: Address already in use (os error 48); the load test runs without it
```

With `--no-autostart`, if no Controller could listen and dashboard control is off, nothing could start the load test, so Goose refuses to run:

```text
Error: InvalidOption { option: "`configuration.no_autostart`", value: "true", detail: "`configuration.no_autostart` requires a Controller or dashboard control, and no Controller could listen on its port." }
```

To **observe** a running load test in a browser, enable the optional [Live Dashboard](dashboard.md) (`--dashboard`, default `127.0.0.1:5118`). With `--dashboard-control` and an auth token, or `--dashboard-no-auth` for [control without a token](dashboard.md#control-without-a-token), the dashboard can also start and stop the load test, set users, the host, the ramps and the run time, save reports, and shut Goose down from Idle. The Controllers remain the path for `config` and `metrics`, a test plan typed at a prompt, and shutting Goose down from any phase.
