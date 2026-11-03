# Gaggle Options

Each of these options needs Goose built with the `gaggle` feature. In this preview they are set on the command line, or the token in the environment, only: none has a `GooseDefault`, so a load test cannot set them in code.

## Manager

* `--manager`: run as the manager. Can't be used with `--worker` or `--gaggle-shadow-merge`. A `--throttle-requests` below the number of workers is refused. With `--no-autostart`, it waits for a `start` once every worker has joined.
* `--expect-workers <N>`: how many workers to wait for, at least 1 and at most 2048. Required with `--manager`.
* `--manager-bind-host <HOST>`: the address the manager listens on, `127.0.0.1` by default. Any address but loopback needs `--gaggle-token`.
* `--manager-bind-port <PORT>`: the port the manager listens on, `5115` by default.
* `--no-hash-check`: let a worker whose load test differs from the manager's join. Not recommended: the merged results then mix two load tests. Only the manager checks, so a worker refuses it.
* `--gaggle-rebalance`: when a worker is lost during a run, move its users to the workers left, each keeping its ordinal and scenario, so the run keeps its users. Off by default: the run goes on with the users the other workers run. See [Rebalancing a lost worker's users](manager.md#rebalancing-a-lost-workers-users).

## Worker

* `--worker`: run as a worker. Can't be used with `--manager`.
* `--manager-host <HOST>`: the manager's DNS name or address. Required with `--worker`.
* `--manager-port <PORT>`: the manager's port, `5115` by default.

## Both

* `--gaggle-connect-timeout <TIME>`: how long a worker keeps trying to reach its manager, each attempt included whole, and how long a manager waits for all of its workers, `60s` by default.
* `--gaggle-token <TOKEN>`: a secret the manager and every worker share, printable ASCII without spaces. Also read from the `GOOSE_GAGGLE_TOKEN` environment variable, which is the better way to give it: Goose never shows it in its output, but a token on the command line shows in `ps`, in `/proc/<pid>/cmdline` and in your shell's history. The token, and all Gaggle traffic, cross the network unencrypted until TLS arrives: run a Gaggle on a network you trust.
* `--gaggle-heartbeat-timeout <TIME>`: how long the manager and a worker wait to hear from each other, `10s` by default, and at least `3s`, as heartbeats go every 2 seconds. A manager marks a silent worker lost and carries on; a worker that loses its manager stops all of its users at once and exits with an error.

## Everything else goes to the manager

Every other option is given to the manager, which hands each worker what shapes the load, the metrics, the reports and the logs. A worker keeps only `--output-dir`, `--goose-log`, `--verbose`, `--quiet`, `--log-level`, `--no-print-metrics`, `--no-error-summary`, `--help`, `--version`, `--list`, `--scenarios-list`, and its own Gaggle options: `--worker`, `--manager-host`, `--manager-port`, `--gaggle-connect-timeout`, `--gaggle-token` and `--gaggle-heartbeat-timeout`, and refuses every other option, naming it. A log the load test sets in code is not written on a worker unless the manager asks for that log too.
