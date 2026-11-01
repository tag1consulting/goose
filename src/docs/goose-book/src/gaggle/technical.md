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

- A worker opens the stream with its token in the request's `authorization` metadata, which the manager checks before it reads any message, and says `Hello`, with its Goose version, the protocol version and the hash of its load test. The manager answers `Welcome`, with the worker's id, or `Reject`, naming what did not match; a wrong token is refused with `Unauthenticated` before that.
- The manager sends `Prepare`, the worker's share of the load test, and the worker answers when its users are ready. `StartAt` tells every worker to start 2 seconds later.
- Each second a worker sends `Metrics`: what it measured since the last second, as a delta [postcard](https://docs.rs/postcard/) encodes. A worker queues at most 8; past that, the newest is merged into the last queued, or dropped if its counts would overflow, which the manager records as a gap, so a slow link costs memory for 8 deltas at most. The manager adds every worker's deltas into one set of metrics and graph data, which its reports, dashboard and thresholds read.
- `ResetMetrics` resets every worker's metrics when the run's users have all started. `Stop` ends the run; each worker answers with its final metrics, then `Stopped`. `SendLogs` asks for its logs, which come back in chunks of 1 MiB, and `Shutdown` tells it to exit.
- `Heartbeat` goes both ways every 2 seconds, on its own task, whatever else either side is doing, `--no-metrics` included.

Messages are limited to 64 MiB each way, which holds the largest custom data a worker may be handed, 16 MiB, and a worker's final metrics; a `Hello` is limited to 64 KiB. Before any worker is admitted, what a client can make the manager hold is bounded: at most `--expect-workers` plus 4 connections, each closed unless a worker is admitted on it within 10 seconds, and one stream at a time on each. The 4 spare connections absorb connections that never become workers, such as a client without the token or a worker turned away, until their deadline.

The manager never holds more than 128 MiB that its workers sent and it has not read yet, across every connection: each connection's HTTP/2 stream and connection windows are an equal share of that budget, `128 MiB / (--expect-workers + 4)`, at least 64 KiB and at most 1 MiB, set when the manager starts and fixed for the run. (With more than 2,044 workers the 64 KiB floor takes the total just past the budget, to 128.25 MiB at 2,048.) A worker's transfers to the manager move at most one window each round trip, so at a round trip of 100 ms:

| Workers | Window | Each worker | All workers |
|---|---|---|---|
| 2 | 1 MiB | about 10 MB/s | about 20 MB/s |
| 100 | 1 MiB | about 10 MB/s | about 1 GB/s |
| 2,048 | 64 KiB | about 640 KB/s | about 1.3 GB/s |

A worker offers its manager windows of 1 MiB. Neither side sends HTTP/2 keepalive pings: the heartbeats, every 2 seconds each way, find a peer that is gone. The manager queues at most 64 messages from its workers; past that, a worker is held back by HTTP/2 flow control.

The connection is not encrypted: the token, and all Gaggle traffic, cross the network in clear text until TLS arrives. Run a Gaggle on a network you trust, and give it a `--gaggle-token`.
