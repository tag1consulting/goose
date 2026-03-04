# TCP Load Test

The [`examples/tcp_loadtest.rs`](https://github.com/tag1consulting/goose/blob/main/examples/tcp_loadtest.rs) example demonstrates how to use Goose to load test a raw TCP server. While Goose is primarily an HTTP load testing framework, its metrics pipeline supports any protocol through [`GooseUser::record_custom_request`].

## Why non-HTTP load testing?

Some services communicate over raw TCP, UDP, WebSocket, gRPC, or other protocols rather than HTTP. Previously, the only way to record metrics for these operations was to construct fake HTTP `GooseRequest` objects, which produced misleading results (spurious 404 errors). `record_custom_request` provides a clean, first-class path: manually time the operation yourself and hand the result to Goose.

## The API

```rust
user.record_custom_request(
    method,        // &str         — protocol label shown in metrics (e.g. "TCP", "GRPC", "WS")
    name,          // &str         — identifies this operation in metrics output
    response_time, // u64          — how long the operation took, in milliseconds
    success,       // bool         — whether the operation succeeded
    status_code,   // Option<u16>  — protocol-specific status code, or None if not applicable
    error,         // Option<&str> — failure reason; ignored when success is true
).await?;
```

Recorded metrics appear in all Goose reports under the method label you provide:

```
 === PER REQUEST METRICS ===
 Name                     |  # reqs |  # fails |  req/s |  fail/s
 TCP tcp_echo             |    1000 |    0 (0%)|  100.0 |    0.00
```

### Parameters

- **`method`** — A short label for the protocol (e.g. `"TCP"`, `"GRPC"`, `"WS"`, `"MQTT"`). Must not contain whitespace. Displayed in the method column of all metrics output.
- **`name`** — Identifies the specific operation within that protocol. Combined with `method` to form the metrics key (`"TCP tcp_echo"`).
- **`response_time`** — The measured duration in milliseconds. `as_millis()` truncates to whole milliseconds; sub-millisecond operations are recorded as `0 ms`, matching the resolution of HTTP timing in Goose.
- **`success`** — Whether the operation succeeded. Unlike HTTP requests where Goose infers success from the status code, for custom protocols you control this directly.
- **`status_code`** — `None` when the protocol has no concept of status codes (e.g. raw TCP); `Some(code)` to record a protocol-specific code (e.g. a gRPC status code). Internally, `None` is stored as `0`; if status code reporting is enabled, `0` will appear in status code tables for these requests.
- **`error`** — An optional error description for failed operations.

Coordinated Omission Mitigation applies to custom requests the same way it does to HTTP requests.

## Running the example

Start a local TCP echo server:

```bash
ncat -l 9000 -k -e /bin/cat
```

Then run the load test, pointing `--host` at the server. Goose requires a URL with a scheme for validation — any scheme works (e.g. `tcp://`, `grpc://`, `ws://`). The host and port are extracted and used for the TCP connection:

```bash
cargo run --example tcp_loadtest -- \
  --host tcp://localhost:9000 \
  --users 10 \
  --run-time 30s \
  --no-reset-metrics
```

## Adapting to other protocols

The same pattern works for any protocol — replace the `TcpStream` logic with UDP sockets, a WebSocket client, a gRPC stub, or any other I/O:

```rust
async fn my_custom_operation(user: &mut GooseUser) -> TransactionResult {
    let started = std::time::Instant::now();

    // Perform the actual operation here (TCP, UDP, gRPC, etc.)
    let result: Result<_, _> = do_my_protocol_operation().await;

    let response_time = started.elapsed().as_millis() as u64;

    match result {
        Ok(_) => {
            user.record_custom_request("GRPC", "operation_name", response_time, true, None, None)
                .await?;
        }
        Err(e) => {
            user.record_custom_request(
                "GRPC",
                "operation_name",
                response_time,
                false,
                None,
                Some(&e.to_string()),
            )
            .await?;
        }
    }

    Ok(())
}
```

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/tcp_loadtest.rs}}
```
