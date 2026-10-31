# Gaggle Worker

A worker is your load test started with `--worker`, compiled with the `gaggle` [feature](technical.md#compile-time-feature), and given the manager's address with `--manager-host`: a DNS name or an address. It connects, runs the share of the load test the manager hands it, and sends what it measures to the manager once a second.

A worker takes every option that shapes the load, the metrics, the reports and the logs from its manager, so it refuses them: users, rates, the test plan, the run time, iterations, the host, the throttle, `--scenarios`, coordinated omission mitigation, `--no-reset-metrics`, thresholds, every report and log option, and the dashboard. It keeps `--output-dir`, where its own run folder goes and its logs are written, `--goose-log` and the verbosity options. A worker runs no telnet or WebSocket Controller and no dashboard. A worker never runs `test_start` or `test_stop`: the manager runs them once.

## Examples

A worker connecting to a manager on the same machine:

```bash
target/release/examples/simple --worker --manager-host localhost --no-telnet
```

A worker on another machine, with the Gaggle's token:

```bash
GOOSE_GAGGLE_TOKEN=s3cret target/release/examples/simple --worker --manager-host manager.example.internal
```

A worker that can't reach its manager tries again after 1, 2, 4 and 8 seconds, then every 10 seconds, and gives up with an error after `--gaggle-connect-timeout` (60 seconds by default). A worker the manager turns away, for another token, another load test or another Goose version, exits at once with an error naming why.

## When the manager is lost

Load never outlives its manager. If nothing comes from the manager for `--gaggle-heartbeat-timeout` (10 seconds by default), or the connection closes without the manager telling the worker to exit, the worker stops every user at once, logs why, and exits with an error within the heartbeat timeout plus 5 seconds. The manager and every worker send each other a heartbeat every 2 seconds, whatever else they are doing.

Ctrl-C on a worker stops its users, sends the manager its final metrics, and exits; the manager marks it lost and carries on with the others.

## Custom data for each worker

A load test can give each worker data of its own, such as its share of a pool of accounts, with [`GooseAttack::set_worker_data`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.set_worker_data). The manager calls the hook once for each worker, after `test_start`, with the worker's index, the number of workers, the worker's users and the ordinal of its first user, and sends what it returns, at most 16 MiB, to that worker. Every user on the worker reads it with [`GooseUser::worker_data`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.worker_data). A run of one process calls the hook once, as worker 0 of 1, so a load test behaves the same with or without a Gaggle.

```rust,ignore
GooseAttack::initialize()?
    .set_worker_data(|share: &WorkerShare| {
        // This worker's accounts, one per line.
        let pool = accounts();
        let end = (share.user_offset + share.users).min(pool.len());
        pool[share.user_offset.min(end)..end].join("\n").into_bytes()
    })
```

The data is not part of the load test's hash, so it may change from run to run.
