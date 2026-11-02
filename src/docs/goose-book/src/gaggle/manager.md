# Gaggle Manager

The manager is your load test started with `--manager`, compiled with the `gaggle` [feature](technical.md#compile-time-feature). It runs no users itself: it waits for its workers, splits the load test between them, and merges what they measure. Every option that shapes the load, the metrics, the reports or the logs is given to the manager, never to a worker.

`--expect-workers` is required: the manager waits for exactly that many workers before the run starts, at most `--gaggle-connect-timeout` (60 seconds by default). If fewer join in time, the manager tells those that did to exit, and exits with an error. With `--no-autostart`, the manager waits for its workers and then for a `start` from a Controller or the dashboard.

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
- Each step of the test plan is split the same way: every step's users add up to the step's. A worker may hold 0 users in a step.
- A users ramp, `--users` with `--increase-rate` or `--increase-time`, `--run-time` and a ramp down, runs as a users ramp on every worker when each has at least one user: its share of the users, each rate times its share of the users, and the same times, so every worker ramps up, holds and ramps down with the manager, and a Controller can change its users, rates and run time as one process's. With more workers than users, each worker runs its share of the ramp as a test plan instead, and a running users ramp's users, rates and run time can not change.
- `--throttle-requests` is split in proportion to each worker's users, the remainder spread, never 0: a worker with no users keeps 1, as it may get users later. The shares add up to the throttle, and a throttle smaller than the number of workers is refused.
- Custom data: see [Custom data for each worker](worker.md#custom-data-for-each-worker).

## While it runs

The manager's phase, run time and test plan are the run's: its console, Controllers and dashboard show the merged metrics, and the users count is the one its plan calls for. Ctrl-C on the manager stops every worker, waits for their final metrics, and writes the reports, as at the end of a run.

## Runtime control

The manager's telnet Controller, WebSocket Controller and dashboard control change the whole Gaggle while it runs: users, the rates and times, the test plan, the host and the throttle, and `start`, `stop`, `shutdown` and the dashboard's Quit. Each change is applied as one process running the whole load test would apply it:

1. The manager checks the change against the rules one process applies, the phase, the bounds and the settings it conflicts with, and against what a Gaggle needs, before it sends anything. A refused change reaches no worker.
2. It applies the change to its own run, which keeps the run's clock.
3. It sends each worker its share, and answers once every worker has applied it: within 10 seconds, inside the 15 a Controller or the dashboard waits. Its main loop keeps running meanwhile, and a command sent after this one waits until it is answered, in the order they came.

What each worker gets is in [Controlling a Gaggle](../controller/overview.md#controlling-a-gaggle). In short: `users` keeps the users one process would run, each user's ordinal and scenario included, an increase taking the next ordinals and a decrease stopping the highest; the rates follow each worker's share of the users; the times, host and run time are the same everywhere; a test plan is split step by step; and the throttle is split by each worker's share of the users. A change made in Idle is kept for the next Start, which hands every worker its share with it.

A worker that does not answer within 10 seconds is marked lost, sent `Shutdown` and disconnected; the reply names it, and the change stands on the others. A worker refusing a change the manager accepted is a bug: the manager logs it, marks the worker lost the same way, and the reply names it.

`--no-autostart` works on a manager: once every worker has joined it waits, and an accepted `start` answers first, then runs `test_start`, the worker data hook, each worker's share and the common start, as an autostarted run does. `stop` ends the run and keeps every worker connected and idle, waiting with the manager; the next `start` hands them new shares, a new host or throttle among them. `shutdown` and the dashboard's Quit tell every worker to exit with the manager.

The manager's Controllers listen on loopback, as on one process: from another machine, use an ssh forward, `--telnet-host`, or `--websocket` with `--websocket-host`. Neither Controller takes authentication, so anyone who reaches one controls the whole Gaggle's load; the dashboard's control takes its token.

## The throttle across workers

`--throttle-requests` is one limit for the whole Gaggle. Each worker throttles its share, in proportion to its share of the users and never 0, and the shares add up to the throttle. A `throttle` command splits the new rate the same way, refused below one request a second for each worker, or when it would raise a worker's share past 100 times the share it started with. A change of users moves the shares with them. When a worker is lost, the manager splits the throttle again over the workers left and sends their new shares, so the run's limit holds; if the throttle is below the number of workers left, each keeps 1 request a second, and the manager logs it.

## Rebalancing a lost worker's users

By default a lost worker's users are gone for the rest of the run: the run goes on with the users the other workers run. With `--gaggle-rebalance` on the manager, its users move to the workers left, dealt to those running fewest, each keeping its ordinal and scenario, so the run keeps its users and its scenario mix; they start over a second, with the throttle split again over the workers left. Rebalancing moves the users of a users ramp; a test plan's are not moved, and the manager logs it.

## Lost workers

The manager marks a worker lost when nothing has come from it for `--gaggle-heartbeat-timeout` (10 seconds by default), when its connection closes before its final metrics, or when it is stopped on its own machine. What the worker sent stays in the results, the run goes on with the others, and the manager tells the lost worker to exit if it can still reach it. The run's record, in `report.json`, lists every worker with its state, and for a lost worker when it was lost and the last metrics it sent; the HTML, Markdown and PDF reports open with a line saying the run is degraded. A worker whose metrics are not all in the results degrades the run too, named with why: one that missed the metrics reset, whose metrics the merge leaves out, one whose metrics the manager refused, or one some of whose metrics never arrived. A worker that cannot hand over its final metrics within 30 seconds, or reset its metrics within 10, tells the manager it failed, which marks it lost, and exits with an error. If every worker is lost, the run ends, recorded as ended by losing its workers.

## Logs

With `--request-log`, `--transaction-log`, `--scenario-log`, `--error-log` or `--debug-log` on the manager, each worker writes that log in its own run folder, in the format and compression the manager's options give. When the run ends, after every worker has stopped, the manager asks each for its logs, and keeps them in its run's folder as `workers/<worker name>/<file>`, under the file names its options give. A worker's transfer fails when neither a full 1 MiB chunk of its logs nor a log's last arrives for 60 seconds; a transfer that keeps moving has no deadline, however large the log. A log that fails part way, or whose worker is lost, stays on its worker, and nothing of it is kept under its name on the manager; the manager's log, and its reports, name the worker and the path. Two logs may not share a file name, ignoring case: the manager refuses, for example, `--request-log a/x.log` with `--error-log b/x.log`. Each process keeps its own `--goose-log`.
