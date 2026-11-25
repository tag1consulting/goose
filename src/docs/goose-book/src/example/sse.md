# Server Sent Events

The [`examples/sse.rs`](https://github.com/tag1consulting/goose/blob/main/examples/sse.rs) example load tests a [server sent events](https://html.spec.whatwg.org/multipage/server-sent-events.html) endpoint: a `GET` whose response is a `text/event-stream` that the server keeps open, writing an event to it whenever it has one.

## What the example does

It defines one scenario, `SseUser`, with one transaction, `events`, and no wait time. Each time the transaction runs, the user:

1. sends `GET /events` with `Accept: text/event-stream` and `Accept-Encoding: identity`, under the request name `events`;
2. marks the request failed if the status is not 200, if `Content-Type` does not start with `text/event-stream`, or if the response has a `Content-Encoding`;
3. reads the stream until it has seen 10 events, then drops the response, which closes the connection, and the transaction runs again on a new one.

An event ends at a blank line, which the spec allows to be written `\n\n`, `\r\n\r\n` or `\r\r`. An event counts only if it has a `data:` line: a comment, a line that starts with `:`, is what servers send as a keep alive, and a block of comments alone is not an event. The example never logs what an event holds, as events can carry tokens or user data: it reports counts and errors only.

The path, the number of events and the two caps below are constants at the top of the file: `EVENTS_PATH`, `EVENTS_PER_CONNECTION`, `EVENT_TIMEOUT` and `MAX_EVENT_BYTES`.

## Why `into_inner()`

An event stream never ends, so a body read the usual way never finishes:

- `text()`, `bytes()` and `json()` read the body to its end, so they wait until the client's timeout and then fail.
- `chunk()` reads one piece at a time, but the response stays Goose's: when the transaction drops it, Goose reads the rest of the body to the end so the connection can be used again, which on a stream never happens, and the user hangs there.
- [`GooseHttpResponse::into_inner()`](https://docs.rs/goose/*/goose/goose/struct.GooseHttpResponse.html#method.into_inner) returns the `reqwest::Response`, and Goose never reads a body taken that way. The example reads it with `reqwest::Response::chunk()`, and dropping it closes the connection.

A load test of your own that names `reqwest::Response` needs `reqwest` in its own `Cargo.toml`, as Goose does not re-export it.

## Uncompressed

Goose asks for gzip unless a request sets its own `Accept-Encoding`, and a body taken with `into_inner()` is as it came over the wire, so a gzip stream would arrive compressed. The example sends `Accept-Encoding: identity`, and fails the request if the server sends a `Content-Encoding` anyway.

## The two caps

- `EVENT_TIMEOUT`, 30 seconds: each read of the stream is wrapped in `tokio::time::timeout`, and the time runs from the last event, so a server that stops sending, or sends only keep alives, fails the request instead of holding the user. A read error, or a stream that ends before 10 events, fails it too.
- `MAX_EVENT_BYTES`, 64 KiB: each connection has one buffer, and each event is drained from it as it completes. If the buffer holds more than this with no complete event, the request fails, so a server that never sends a blank line cannot grow the user's memory without bound.

Each piece that arrives is added to the buffer, and only the new bytes are searched for the end of an event.

## What the metrics measure

Goose times the `events` request, `GET events` in the metrics, from sending it to the response headers, as it does every request: how long the server takes to open the stream. The example then records the time from sending the request to the tenth event with [`record_custom_request_duration`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.record_custom_request_duration), as `SSE events (stream)`, which fails when the stream does. The `events` transaction's time is the whole connection, from the request to the tenth event, including the close.

## Running it

Against a server with an event stream at `/events`:

```bash
cargo run --example sse -- --host http://localhost:8080 --users 10 --run-time 1m
```

Run with 20 users for 20 seconds against a small local server that writes an event, then a keep alive comment, each 50 ms apart, so 10 events take about a second:

```text
 === PER REQUEST METRICS ===
 ------------------------------------------------------------------------------
 Name                     |        # reqs |        # fails |    req/s |  fail/s
 ------------------------------------------------------------------------------
 GET events               |           419 |         0 (0%) |    20.95 |    0.00
 SSE events (stream)      |           419 |         0 (0%) |    20.95 |    0.00
 -------------------------+---------------+----------------+----------+--------
 Aggregated               |           838 |         0 (0%) |    41.90 |    0.00
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 GET events               |        1.53 |       0.89 |        8.92 |       1.43
 SSE events (stream)      |       1,006 |        954 |       1,046 |      1,004
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |         504 |       0.89 |       1,046 |        8.9
```

## Tests

The example's tests run with `cargo test --example sse`: unit tests of the function that finds the events in the buffer, and two load tests of 2 seconds with 1 user against a local server that never ends its response. Against one that sends an event every 20 ms, every request succeeds; against one that sends bytes with no blank line, every request fails on `MAX_EVENT_BYTES`. Both must end within 10 seconds, which a stream left open would prevent.

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/sse.rs}}
```
