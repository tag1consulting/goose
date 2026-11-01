# Gaggle Options

Each of these options needs Goose built with the `gaggle` feature.

## Manager

* `--manager`: run as the manager. Can't be used with `--worker`, `--no-autostart` or `--gaggle-shadow-merge`. A `--throttle-requests` below the number of workers that run users is refused.
* `--expect-workers <N>`: how many workers to wait for, at least 1 and at most 2048. Required with `--manager`.
* `--manager-bind-host <HOST>`: the address the manager listens on, `127.0.0.1` by default. Any address but loopback needs `--gaggle-token`.
* `--manager-bind-port <PORT>`: the port the manager listens on, `5115` by default.

## Worker

* `--worker`: run as a worker. Can't be used with `--manager`.
* `--manager-host <HOST>`: the manager's DNS name or address. Required with `--worker`.
* `--manager-port <PORT>`: the manager's port, `5115` by default.

## Both

* `--gaggle-connect-timeout <TIME>`: how long a worker keeps trying to reach its manager, each attempt included whole, and how long a manager waits for all of its workers, `60s` by default.
* `--gaggle-token <TOKEN>`: a secret the manager and every worker share, printable ASCII. Also read from the `GOOSE_GAGGLE_TOKEN` environment variable. Never shown in output. The token, and all Gaggle traffic, cross the network unencrypted until TLS arrives: run a Gaggle on a network you trust.
* `--no-hash-check`: let a worker whose load test differs from the manager's join. Not recommended: the merged results then mix two load tests.
* `--gaggle-heartbeat-timeout <TIME>`: how long the manager and a worker wait to hear from each other, `10s` by default, and at least `3s`, as heartbeats go every 2 seconds. A manager marks a silent worker lost and carries on; a worker that loses its manager stops all of its users at once and exits with an error.

## Everything else goes to the manager

Every other option that shapes the load, the metrics, the reports or the logs is given to the manager, and a worker refuses it: `--users`, `--increase-rate`, `--increase-time`, `--run-time`, `--decrease-rate`, `--decrease-time`, `--test-plan`, `--iterations`, `--host`, `--throttle-requests`, `--scenarios`, `--co-mitigation`, `--no-reset-metrics`, `--no-metrics` and the other metrics options, `--threshold` and its options, `--timeout` and the other HTTP client options, every report option but `--output-dir`, every log option but `--goose-log`, every dashboard option, `--dashboard-max-clients` included, and every Controller option: `--no-autostart`, `--no-telnet`, `--telnet-host`, `--telnet-port`, `--websocket`, `--no-websocket`, `--websocket-host` and `--websocket-port`. A worker keeps `--output-dir`, `--goose-log`, `--verbose`, `--quiet`, `--log-level`, `--no-print-metrics` and `--no-error-summary`, and its own Gaggle options. A log the load test sets in code is not written on a worker unless the manager asks for that log too.
