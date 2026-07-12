# Controlling A Running Goose Load Test

By default, Goose will launch a telnet Controller thread that listens on `0.0.0.0:5116`, and a WebSocket Controller thread that listens on `0.0.0.0:5117`. The running Goose load test can be controlled through these Controllers. Goose can optionally be started with the `--no-autostart` run time option to prevent the load test from automatically starting, requiring instead that it be started with a Controller command. When Goose is started this way, a host is not required and can instead be configured via the Controller.

NOTE: The controller currently is not Gaggle-aware, and only functions correctly when running Goose as a single process in standalone mode.

To **observe** a running load test in a browser without controlling it, enable the optional [Live Dashboard](dashboard.md) (`--dashboard`, default `127.0.0.1:5118`). The dashboard is read-only; use the Controllers below to change users, host, or start/stop the test.
