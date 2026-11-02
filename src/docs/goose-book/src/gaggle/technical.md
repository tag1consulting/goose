# Gaggle Technical Details

## Compile-time Feature

Gaggle support is the `gaggle` cargo feature, off by default. Enable it on the command line with `--features gaggle`, or in your load test's `Cargo.toml`:

```toml
[dependencies]
goose = { version = "0.20", features = ["gaggle"] }
```

It needs no system libraries and no `protoc`: the protocol's generated code is part of Goose.

## The protocol

The manager and its workers speak gRPC, with [tonic](https://docs.rs/tonic/) and [prost](https://docs.rs/prost/), over one HTTP/2 stream each way that stays open for the whole run. The protocol is `proto/gaggle.proto` in the Goose repository, package `goose.gaggle.v1`, protocol version 1. Workers open every connection; the manager only listens.

- A worker opens the stream with its token in the request's `authorization` metadata, as `Bearer <token>` and in no other form, which the manager checks before it reads any message, and says `Hello`, with its Goose version, the protocol version and the hash of its load test. The manager answers `Welcome`, with the worker's id, or `Reject`, naming what did not match; a wrong token is refused with `Unauthenticated` before that.
- The manager sends `Prepare`, the worker's share of the load test, and the worker answers when its users are ready. `StartAt` tells every worker to start 2 seconds later.
- Each second a worker sends `Metrics`: what it measured since the last second, as a delta [postcard](https://docs.rs/postcard/) encodes. A worker queues at most 8; past that, the newest is merged into the last queued, or dropped if its counts would overflow, which the manager records as a gap, so a slow link costs memory for 8 deltas at most, and a final one. The manager adds every worker's deltas into one set of metrics and graph data, which its reports, dashboard and thresholds read.
- `ResetMetrics` resets every worker's metrics when the run's users have all started. `Stop` ends the run; each worker answers `UsersStopped` once its users have stopped, then its final metrics, then `Stopped`. The manager waits for `UsersStopped` up to the request timeout and 30 seconds after `Stop`, as a user stops once the transaction it is in ends, and for the final metrics up to 30 seconds after `UsersStopped`; a worker gives up on its own metrics after 20, and says so, so a worker lost for it is named for the real cause. Ctrl-C during that wait ends it: every worker still owed its final metrics is lost, named as canceled, and `test_stop`, the reports and `Shutdown` run as after any run. A worker that says it stopped for a reason the manager does not know, or none, is lost. `SendLogs` asks for its logs, which come back in chunks of 1 MiB, and `Shutdown` tells it to exit.
- `Control` carries a runtime change from a Controller or the dashboard, the worker's share of it, [postcard](https://docs.rs/postcard/) encoded, with an id; the worker applies it with the checks one process makes and answers `ControlAck` with the same id, and why, if it refused. The manager answers the change's front end once every worker answered, and marks lost a worker that does not answer within 10 seconds. After a `stop` the stream stays open and the worker idle, and the next `start` sends `Prepare` and `StartAt` again.
- `Heartbeat` goes both ways every 2 seconds, on its own task, whatever else either side is doing, `--no-metrics` included. Until its `Prepare` has arrived a worker lets its manager go silent for up to 120 seconds, or the heartbeat timeout if longer: a large `Prepare` on a slow link holds back the heartbeats queued behind it, and no user runs before it.

Messages are limited to 64 MiB each way, which holds the largest custom data a worker may be handed, 16 MiB, and a worker's final metrics; a `Hello` is limited to 64 KiB. A worker never sends a delta past the limit, which would reset its stream: a running delta past it is left out, logged with its size, and the manager records the gap; a final one past it fails the worker, which the manager records as lost, naming the size. Before any worker is admitted, what a client can make the manager hold is bounded: at most `--expect-workers` plus 4 connections, each closed unless a worker is admitted on it within 10 seconds, and one stream at a time on each. The 4 spare connections absorb connections that never become workers, such as a client without the token or a worker turned away, until their deadline. Each connection refused before a worker is admitted on it is logged at debug level, and one warning a minute at most sums them up, with how many and the last one's address, so no client floods the manager's log.

The manager holds at most about 128 MiB that its workers sent and it has not read yet, across every connection: each connection's HTTP/2 stream and connection windows are an equal share of that budget less the 16 KiB of headers each may also hold, `128 MiB / (--expect-workers + 4) - 16 KiB`, at least 64 KiB and at most 1 MiB, set when the manager starts and fixed for the run. Windows and headers together come to about 6.1 MiB for 2 workers and the whole budget for 124; with more than 1,634 workers the 64 KiB floor takes the total past the budget, to 160.3 MiB at 2,048. A worker's transfers to the manager move at most one window each round trip, so at a round trip of 100 ms:

| Workers | Window | Each worker | All workers |
|---|---|---|---|
| 2 | 1 MiB | about 10 MB/s | about 20 MB/s |
| 100 | 1 MiB | about 10 MB/s | about 1 GB/s |
| 2,048 | 64 KiB | about 650 KB/s | about 1.3 GB/s |

A worker offers its manager windows of 1 MiB, and queues at most 16 of its manager's messages for its main loop, holding at most 64 MiB of run specs; past either it reads nothing more, and HTTP/2 flow control holds the manager back. `Shutdown`, and the loss of its manager, never wait behind that queue, and a worker whose connection is gone stops waiting at once, whatever its queue holds. A worker refuses a share of more than 1,000,000 users, a test plan of more than 100,000 steps, a log asked for twice, or custom data past 16 MiB; the manager refuses a run that would hand a worker more users or steps than that before it waits for any worker or runs `test_start`, and suggests a larger `--expect-workers` for the users. Neither side sends HTTP/2 keepalive pings: the heartbeats, every 2 seconds each way, find a peer that is gone. The manager queues at most 64 messages from its workers, holding at most 256 MiB: each message counts its encoded size, and a delta what it holds once decoded, which can be several times that, as a varint of 1 byte on the wire is 8 in memory; past either, a worker is held back by HTTP/2 flow control. Each worker's reader may hold one decoded delta more while it waits for room, so the manager can hold one such delta per worker at once. A manager keeps per worker counts of the gaps in each worker's sequence of deltas, never a list that grows with them, and ignores repeats, and it makes each worker's custom data, encodes it into that worker's `Prepare` and drops it before making the next worker's, so it never holds two workers' data unencoded. Each encoded `Prepare` waits in its worker's send queue until it has left, so on slow links the manager can hold every worker's `Prepare` at once.

Each worker sends each log's size before its first byte, and the manager refuses a log that grows past it. The manager trusts that size, as it trusts its workers: it does not cap a log's size itself, so a worker can fill the manager's disk with logs. The manager needs disk room for every worker's logs: it writes its reports and run record before any log arrives, so a full disk costs only logs, each not yet whole deleted and named in the reports as left on its worker. Ctrl-C while the logs arrive stops the wait the same way, and a worker that sends neither a full chunk nor a log's last for 60 seconds gives up every log of it not yet whole.

In `report.json`'s `gaggle` section each worker's `state` is `running`, `finished`, `users_exited` or `lost`. No environment variable turns a fault on in a worker: Goose's own faults are compiled into its unit tests only.

## The manager's memory

The manager keeps, for each worker, every second of its graph rows over the whole run, as well as their merge: what a worker sends grows with its request names and with the run's length, not with its users. A worker's rows take about 180 bytes for each request name each second (measured with the latency heatmap on; see the PR's figures), so a manager of 50 workers of 30 request names each holds about 3.6 GiB after 4 hours (50 x 30 x 14,400 seconds x 180 bytes), on top of what one process running the whole load test would hold. Give a long run's manager the memory for it, or fewer request names.

## Trust

The connection is not encrypted: the token, and all Gaggle traffic, cross the network in clear text until TLS arrives. The token authenticates workers to the manager, not the manager to its workers: whoever answers at the manager's address, or sits on the path between them, controls the workers' load, its target included, and sees all their traffic. Run Gaggle only on a network you trust, and give it a `--gaggle-token`.
