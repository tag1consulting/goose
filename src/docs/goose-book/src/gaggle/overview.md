# Gaggle: Distributed Load Test

**Preview: Gaggles return in Goose 0.20.** A Gaggle runs one load test from several processes, usually on several machines: one [manager](manager.md) and one or more [workers](worker.md). This page describes what works in this preview, and what comes next.

A Gaggle is built from your own load test: the same binary, compiled with the `gaggle` [cargo feature](technical.md#compile-time-feature), runs as the manager with `--manager` and as each worker with `--worker`. All the configuration is given to the manager. It splits the load test's users between its workers, hands each worker its share, starts them together, and merges what they measure into one set of results, as if one process had run every user.

## What a Gaggle produces

The manager produces everything a run of one process does, from every worker's merged metrics:

- the live [dashboard](../controller/dashboard.md), with `--dashboard` on the manager;
- the reports, as PDF, HTML, Markdown and JSON, with `--report-file` on the manager;
- the request, transaction, scenario, error and debug logs, with the log options on the manager: each worker writes its own and sends it to the manager at the end of the run, which keeps it in `workers/<worker name>/` in its run's folder;
- the console output, on the manager, with the merged metrics.

Thresholds are judged on the merged metrics, by the manager, as a run of one process judges its own: a threshold that fails stops every worker.

## What works in this preview

- A fixed set of workers: the manager waits for `--expect-workers` workers before it starts, and no worker joins later.
- Users, ramps, `--test-plan`, `--iterations`, `--throttle-requests`, `--scenarios`, coordinated omission mitigation and every metrics, report and log option, given to the manager.
- [Runtime control](manager.md#runtime-control): the manager's Controllers and dashboard control start, stop and change the whole Gaggle while it runs, users, rates, times, the test plan, the host and the throttle, as one process running every user would; `--no-autostart` on the manager waits for a `start`.
- `--throttle-requests` is [one limit across the workers](manager.md#the-throttle-across-workers), and holds when a worker is lost; with `--gaggle-rebalance` a lost worker's users move to the others.
- One metrics reset when every user has started, across the whole Gaggle.
- `test_start` and `test_stop` run once, on the manager.
- [Custom data for each worker](worker.md#custom-data-for-each-worker), such as its share of a pool of accounts.
- A lost worker does not stop the run: the run goes on with the others, and its reports say it is degraded. So does a worker whose metrics are not all in the results, as one that missed the metrics reset; its users are left out of the users graph too. A worker that loses its manager stops all of its load within seconds.
- Ctrl-C on the manager at any stage: while the workers join or prepare, during the run, or while their logs arrive. Every worker is told to stop, `test_stop` runs if `test_start` did, and a log not yet whole stays on its worker. In a run that was not canceled, one pressed while the workers hand over their final metrics ends that wait: every worker still owed its final metrics is lost, and the logs are left on the workers.
- A worker's rows reach back its request timeout and a minute, so a second is complete once it is that much older than every running worker's newest, about two minutes with the default `--timeout`. The live dashboard's graphs, and a report saved while the run goes on, show every second the workers have sent, the recent ones as far as they have arrived: the dashboard draws them lighter, as provisional, and a second with nothing yet as a gap. Each delta's rows for those seconds are added as it arrives, so they cost no build from nothing. The final reports cover the whole run, every second complete.
- Once every user has started, a Gaggle runs the same scenario mix one process would. During a ramp the order users start in differs, as each worker starts its own share at its own pace.

## What comes next

- Per worker tables in the reports, and a Gaggle view on the dashboard.
- TLS between the manager and its workers, workers joining a running Gaggle, and workers reconnecting: until TLS arrives, see [Trust](#trust).

## Trust

The token authenticates workers to the manager, not the manager to its workers. Until TLS arrives, whoever answers at the manager's address, or sits on the path between them, controls the workers' load, its target included, and sees all their traffic, the token among it, unencrypted: run Gaggle only on a network you trust, and give it a `--gaggle-token`.

## An example on one machine

Build the `simple` example with the `gaggle` feature, then start a manager waiting for two workers:

```bash
cargo build --release --features gaggle --example simple
target/release/examples/simple --manager --expect-workers 2 --host http://localhost:8080 \
    --users 20 --increase-rate 10 --run-time 30s --report-file --request-log
```

In two more terminals, start two workers:

```bash
target/release/examples/simple --worker --manager-host localhost
```

The run starts two seconds after both workers are ready. When it ends, the manager's run folder holds its reports and each worker's request log, in `workers/<worker name>/`. A worker never runs a Controller, so only the manager listens on the telnet Controller's port: `telnet localhost 5116` there, and `users 40` or `throttle 50` changes the whole Gaggle.

## The lifecycle

1. The manager listens on `--manager-bind-host` and `--manager-bind-port`, and waits for `--expect-workers` workers, at most `--gaggle-connect-timeout`.
2. Each worker connects, retrying with backoff for up to `--gaggle-connect-timeout`. The manager turns away a worker of another Goose version or protocol version, a worker with another token, and, unless `--no-hash-check`, a worker whose load test differs.
3. Once every worker has joined, the manager checks it can create its `--report-file` reports, runs `test_start`, and hands each worker its share: its users, numbered as one process would number them, its part of each test plan step, its part of the throttle, its custom data, and the metrics and log settings.
4. Each worker builds its users and says it is ready. The manager tells every worker to start two seconds later, and starts its own clock at the same instant.
5. Every second each worker sends what it measured since the last second. The manager merges it, judges the thresholds on the merge, and serves its dashboard from it. When every user has started, the manager resets the metrics on every worker at once.
6. At the end of the run, the manager tells every worker to stop. A user stops once the transaction it is in ends, so the manager waits up to the request timeout (`--timeout`, 60 seconds by default) and 30 seconds for each worker to say its users stopped, then up to 30 seconds more for its final metrics; a worker past either is lost. It runs `test_stop` and writes its reports, then collects the workers' logs, writing the reports again to name any log left on its worker, and tells the workers to exit.
7. Any failure after `test_start`, as a worker that cannot prepare, runs `test_stop` once before the manager exits with the error.

## Load testing at scale

In earlier releases, a Goose load test running from AWS was limited only by network speed: a smaller server simulated 2,000 users making over 6,500 requests per second and sustaining 2.5 Gbps, and two servers with faster uplinks simulated 12,000 users making over 41,000 requests per second and saturating 16 Gbps. With every response checked as it arrives, each machine generates as much load as its CPUs and uplink allow, and a Gaggle adds machines. For details and graphs, see [A Goose In The Clouds: Load Testing At Scale](https://www.tag1.com/blog/goose-clouds-load-testing-scale/), written for Goose 0.16.
