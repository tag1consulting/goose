# Controlling A Running Goose Load Test

By default, Goose will launch a telnet Controller thread that listens on `0.0.0.0:5116`, and a WebSocket Controller thread that listens on `0.0.0.0:5117`. The running Goose load test can be controlled through these Controllers. Neither takes any authentication, so anyone who can reach their ports can control the load test: bind them to loopback with `--telnet-host 127.0.0.1` and `--websocket-host 127.0.0.1`, or turn them off with `--no-telnet` and `--no-websocket`. Goose can optionally be started with the `--no-autostart` run time option to prevent the load test from automatically starting, requiring instead that it be started with a Controller command, or from the dashboard with `--dashboard-control`; at least one of them must be enabled. When Goose is started this way, a host is not required and can instead be configured via the Controller or the dashboard.

A Controller that can't listen on its port, because another process already holds it, logs an error and the load test runs without it:

```text
[ERROR] [controller]: the Telnet Controller couldn't listen on 0.0.0.0:5116: Address already in use (os error 48); the load test runs without it
```

With `--no-autostart`, if no Controller could listen and dashboard control is off, nothing could start the load test, so Goose refuses to run:

```text
Error: InvalidOption { option: "`configuration.no_autostart`", value: "true", detail: "`configuration.no_autostart` requires a Controller or dashboard control, and no Controller could listen on its port." }
```

To **observe** a running load test in a browser, enable the optional [Live Dashboard](dashboard.md) (`--dashboard`, default `127.0.0.1:5118`). With `--dashboard-control` and a required auth token, the dashboard can also start and stop the load test, set users, the host, the ramps and the run time, save reports, and shut Goose down from Idle. The Controllers remain the path for `config` and `metrics`, a test plan typed at a prompt, and shutting Goose down from any phase.
