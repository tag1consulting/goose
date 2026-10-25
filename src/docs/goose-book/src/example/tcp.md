# TCP Load Test

The [`examples/tcp_loadtest.rs`](https://github.com/tag1consulting/goose/blob/main/examples/tcp_loadtest.rs) example demonstrates how to use Goose to load test a raw TCP server. While Goose is primarily an HTTP load testing framework, its metrics pipeline supports any protocol through [`GooseUser::record_custom_request`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.record_custom_request) and [`GooseUser::record_custom_request_duration`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.record_custom_request_duration), both new in 0.19; the example uses `record_custom_request_duration`.

## Why non-HTTP load testing?

Some services communicate over raw TCP, UDP, WebSocket, gRPC, or other protocols rather than HTTP. Previously, the only way to record metrics for these operations was to construct fake HTTP `GooseRequest` objects, which produced misleading results (spurious 404 errors). `record_custom_request` provides a direct path: time the operation yourself and hand the result to Goose.

## The API

```rust,ignore
user.record_custom_request(
    method,        // &str: protocol label shown in metrics (e.g. "TCP", "GRPC", "WS")
    name,          // &str: identifies this operation in metrics output
    response_time, // u64: how long the operation took, in milliseconds
    success,       // bool: whether the operation succeeded
    status_code,   // Option<u16>: protocol-specific status code, or None if not applicable
    error,         // Option<&str>: failure reason; ignored when success is true
).await?;
```

This is a fragment: it runs inside a transaction function, with the six values already set.

Recorded metrics appear in all Goose reports under the method label you provide. These are the final request tables of the command under [Running the example](#running-the-example), run against a local echo server on port 19161 in place of 9000:

```
 === PER REQUEST METRICS ===
 ------------------------------------------------------------------------------
 Name                     |        # reqs |        # fails |    req/s |  fail/s
 ------------------------------------------------------------------------------
 TCP tcp_echo             |             0 |         0 (0%) |     0.00 |    0.00
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 TCP tcp_echo             |        2.17 |       0.52 |          14 |       1.62
 ------------------------------------------------------------------------------
 Name                     |                                        Status codes 
 ------------------------------------------------------------------------------
 TCP tcp_echo             |                                           1,208 [0]
 -------------------------+----------------------------------------------------
 Aggregated               |                                           1,208 [0] 
```

In this release the `# reqs`, `# fails`, `req/s` and `fail/s` columns read 0 for a custom request, as above, while its response times and status codes are counted; the transaction tables count each `tcp_echo` transaction.

### Parameters

- **`method`**: A short label for the protocol (e.g. `"TCP"`, `"GRPC"`, `"WS"`, `"MQTT"`). Must not be empty or contain whitespace. Displayed in place of the HTTP method in all metrics output.
- **`name`**: Identifies the specific operation within that protocol. Must not be empty or only whitespace. Combined with `method` to form the metrics key (`"TCP tcp_echo"`).
- **`response_time`**: The measured duration in milliseconds. `as_millis()` truncates to whole milliseconds, so a sub-millisecond operation is recorded as `0 ms`; to keep its microseconds, as Goose does for HTTP requests, call `record_custom_request_duration` (below).
- **`success`**: Whether the operation succeeded. Unlike HTTP requests where Goose infers success from the status code, for custom protocols you control this directly.
- **`status_code`**: `None` when the protocol has no concept of status codes (e.g. raw TCP); `Some(code)` to record a protocol-specific code (e.g. a gRPC status code). Internally, `None` is stored as `0`, so status code tables show `[0]` for these requests unless `--no-status-codes` turns those tables off.
- **`error`**: An optional error description for failed operations. When `success` is `false` and `error` is `None`, Goose records `<method> <name>: request failed`.

`record_custom_request_duration` takes the same arguments, with the time as a `Duration` such as `started.elapsed()`, and keeps its microseconds.

Both return an error, and record nothing, when `method` or `name` breaks the rules above. They return `Ok(())` even when `success` is `false`: recording a failure does not fail the transaction. To fail it as well, return an error from the transaction function after recording, for example `Err(Box::new(TransactionError::Custom(message)))`. With `--no-metrics` nothing is recorded.

Coordinated Omission Mitigation applies to custom requests the same way it does to HTTP requests.

## Running the example

Start a local TCP echo server:

```bash
ncat -l 9000 -k -e /bin/cat
```

Then run the load test, pointing `--host` at the server. Goose requires a URL with a scheme for validation, and any scheme works (e.g. `tcp://`, `grpc://`, `ws://`). The example reads the host and port from it for the TCP connection, and fails each transaction when the URL has no port:

```bash
cargo run --example tcp_loadtest -- \
  --host tcp://localhost:9000 \
  --users 10 \
  --run-time 30s \
  --no-reset-metrics
```

## Adapting to other protocols

The same pattern works for any protocol: replace the `TcpStream` logic with UDP sockets, a WebSocket client, a gRPC stub, or any other I/O:

```rust,ignore
async fn my_custom_operation(user: &mut GooseUser) -> TransactionResult {
    let started = std::time::Instant::now();

    // Perform the actual operation here (TCP, UDP, gRPC, etc.)
    let result: Result<_, _> = do_my_protocol_operation().await;

    let response_time = started.elapsed();

    match result {
        Ok(_) => {
            user.record_custom_request_duration(
                "GRPC",
                "operation_name",
                response_time,
                true,
                None,
                None,
            )
            .await?;
        }
        Err(e) => {
            user.record_custom_request_duration(
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

This is a fragment: `do_my_protocol_operation()` stands for your own code.

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/tcp_loadtest.rs}}
```
