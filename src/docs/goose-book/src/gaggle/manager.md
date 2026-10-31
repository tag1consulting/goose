# Gaggle Manager

The manager is your load test started with `--manager`, compiled with the `gaggle` [feature](technical.md#compile-time-feature). It runs no users itself: it waits for its workers, splits the load test between them, and merges what they measure. Every option that shapes the load, the metrics, the reports or the logs is given to the manager, never to a worker.

`--expect-workers` is required: the manager waits for exactly that many workers before the run starts, at most `--gaggle-connect-timeout` (60 seconds by default). If fewer join in time, the manager tells those that did to exit, and exits with an error.

## Example

_A manager listening on every interface on the default port, 5115, waiting for 3 workers, with a token so no other process can join, and the live dashboard on port 5118:_

```bash
GOOSE_GAGGLE_TOKEN=s3cret target/release/examples/simple --manager --expect-workers 3 \
    --manager-bind-host 0.0.0.0 --host http://localhost:8080 \
    --users 900 --increase-rate 30 --run-time 5m --dashboard --report-file
```

A manager that listens on any address but loopback needs `--gaggle-token`, or the `GOOSE_GAGGLE_TOKEN` environment variable. The token is never shown: a Controller's `config` and `config-json` show it as `(set)`.

## How it splits the load test

- Users are split as evenly as they go, the remainder one each to the first workers. Each worker's users are numbered as one process running every user would number them, and run the scenarios that process would give them, so the merged per scenario counts are the same.
- Each step of the test plan, or the ramp `--users`, `--increase-rate` and `--run-time` give, is split the same way: every step's users add up to the step's. A worker may hold 0 users in a step.
- `--throttle-requests` is split in proportion to each worker's users, and never 0 for a worker: a throttle smaller than the number of workers is refused.
- Custom data: see [Custom data for each worker](worker.md#custom-data-for-each-worker).

## While it runs

The manager's phase, run time and test plan are the run's: its console, Controllers and dashboard show the merged metrics, and the users count is the one its plan calls for. Its telnet Controller, WebSocket Controller and dashboard answer `config`, `metrics` and their JSON forms, and the dashboard's report switches and Save report now work. Every command that changes the run is refused with the error code `gaggle_unsupported` and the message `not supported in Gaggle mode yet`.

Ctrl-C on the manager stops every worker, waits for their final metrics, and writes the reports, as at the end of a run.

## Lost workers

The manager marks a worker lost when nothing has come from it for `--gaggle-heartbeat-timeout` (10 seconds by default), when its connection closes before its final metrics, or when it is stopped on its own machine. What the worker sent stays in the results, the run goes on with the others, and the manager tells the lost worker to exit if it can still reach it. The run's record, in `report.json`, lists every worker with its state, and for a lost worker when it was lost and the last metrics it sent; the HTML, Markdown and PDF reports open with a line saying the run is degraded. If every worker is lost, the run ends, recorded as ended by losing its workers.

## Logs

With `--request-log`, `--transaction-log`, `--scenario-log`, `--error-log` or `--debug-log` on the manager, each worker writes that log in its own run folder, in the format and compression the manager's options give. When the run ends, after every worker has stopped, the manager asks each for its logs, and keeps them in its run's folder as `workers/<worker name>/<file>`, under the file names its options give. One deadline covers every transfer: 10 minutes. A log not complete by then stays on its worker; the manager's log, and its reports, name the worker and the path. Each process keeps its own `--goose-log`.
