# Gaggle Options

Each of these options needs Goose built with the `gaggle` feature.

## Manager

* `--manager`: run as the manager. Can't be used with `--worker`, `--no-autostart` or `--gaggle-shadow-merge`.
* `--expect-workers <N>`: how many workers to wait for, at least 1. Required with `--manager`.
* `--manager-bind-host <HOST>`: the address the manager listens on, `127.0.0.1` by default. Any address but loopback needs `--gaggle-token`.
* `--manager-bind-port <PORT>`: the port the manager listens on, `5115` by default.

## Worker

* `--worker`: run as a worker. Can't be used with `--manager`.
* `--manager-host <HOST>`: the manager's DNS name or address. Required with `--worker`.
* `--manager-port <PORT>`: the manager's port, `5115` by default.
* `--gaggle-connect-timeout <TIME>`: how long a worker keeps trying to reach its manager, and how long a manager waits for all of its workers, `60s` by default.

## Both

* `--gaggle-token <TOKEN>`: a secret the manager and every worker share. Also read from the `GOOSE_GAGGLE_TOKEN` environment variable. Never shown in output.
* `--no-hash-check`: let a worker whose load test differs from the manager's join. Not recommended: the merged results then mix two load tests.
* `--gaggle-heartbeat-timeout <TIME>`: how long the manager and a worker wait to hear from each other, `10s` by default, and at least `3s`, as heartbeats go every 2 seconds. A manager marks a silent worker lost and carries on; a worker that loses its manager stops all of its users at once and exits with an error.

## Everything else goes to the manager

Every other option that shapes the load, the metrics, the reports or the logs is given to the manager, and a worker refuses it: `--users`, `--increase-rate`, `--increase-time`, `--run-time`, `--decrease-rate`, `--decrease-time`, `--test-plan`, `--iterations`, `--host`, `--throttle-requests`, `--scenarios`, `--co-mitigation`, `--no-reset-metrics`, `--no-metrics` and the other metrics options, `--threshold` and its options, `--timeout` and the other HTTP client options, every report option but `--output-dir`, every log option but `--goose-log`, and the dashboard options. A worker keeps `--output-dir`, `--goose-log`, `--verbose`, `--quiet` and `--log-level`.
