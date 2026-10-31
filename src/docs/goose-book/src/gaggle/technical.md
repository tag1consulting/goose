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

- A worker says `Hello`, with its Goose version, the protocol version, the hash of its load test and the token. The manager answers `Welcome`, with the worker's id, or `Reject`, naming what did not match.
- The manager sends `Prepare`, the worker's share of the load test, and the worker answers when its users are ready. `StartAt` tells every worker to start 2 seconds later.
- Each second a worker sends `Metrics`: what it measured since the last second, as a delta [postcard](https://docs.rs/postcard/) encodes. A worker queues at most 8; past that, the newest is merged into the last queued, so a slow link costs memory for 8 deltas at most. The manager adds every worker's deltas into one set of metrics and graph data, which its reports, dashboard and thresholds read.
- `ResetMetrics` resets every worker's metrics when the run's users have all started. `Stop` ends the run; each worker answers with its final metrics, then `Stopped`. `SendLogs` asks for its logs, which come back in chunks of 1 MiB, and `Shutdown` tells it to exit.
- `Heartbeat` goes both ways every 2 seconds, on its own task, whatever else either side is doing, `--no-metrics` included.

Messages are limited to 64 MiB each way, which holds the largest custom data a worker may be handed, 16 MiB, and a worker's final metrics.

The connection is not encrypted. Run a Gaggle on a network you trust, and give it a `--gaggle-token`.
