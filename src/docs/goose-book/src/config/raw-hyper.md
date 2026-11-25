# Raw Hyper Client

Goose sends every request through [reqwest](https://docs.rs/reqwest), which is built on [hyper](https://docs.rs/hyper) and adds cookies, redirects, TLS setup, decompression, proxies and a friendly API. That layer costs CPU on every request. Goose's off by default `raw-hyper` feature adds a second way to send a request: over one plain HTTP/1.1 hyper connection per user, without reqwest's layer. It is for a load test that has to get the most requests out of one machine against a simple target, and can live without what reqwest adds. reqwest stays the default, and a load test that does not use the raw client compiles and runs exactly as before.

This page gives the measurements behind the feature, what tuning reqwest recovered, and how to use the raw client.

## What was measured

`benches/client_overhead.rs` runs a hyper HTTP/1.1 server in a process of its own, answering every request with the same 13 byte body, and each client in a fresh process. Each of 16 users is a task with one keep-alive connection, sending its next request as soon as it has read the last response. The figure is the client process's CPU time, user plus system, from `getrusage`, over 10 seconds after a 2 second warm up, divided by the requests it completed; each client ran 5 times, interleaved with the others.

| Client | What it is |
|---|---|
| `goose` | A Goose load test, each transaction `user.get("/")` then `discard_body()`: Goose as it is |
| `goose-tuned` | The same, each user's client replaced with the tuned reqwest client below (`set_client_builder`) |
| `goose-raw` | The same, each request `user.raw_get("/")` over the raw client |
| `reqwest` | A bare loop over a reqwest client per user, built as Goose builds each user's: user agent, 60 s timeout, cookie store, Goose's own `Accept-Encoding: gzip` |
| `reqwest-tuned` | The same, with every reqwest setting Goose does not use that could help: no cookie store, no redirects, no retries, no proxies, no `Referer` |
| `reqwest-tuned+no-timeout` | The same, without the timeout as well |
| `hyper`, `hyper+timeout` | A bare loop over a `hyper_util` legacy client per user (a pool holding the user's one connection), without and with a 60 s timeout |
| `hyper-conn`, `hyper-conn+timeout` | A bare loop over one `hyper::client::conn::http1` connection per user, with no pool, without and with a 60 s timeout: the raw client's design |

Measured on an Apple M3 Ultra (24 performance and 8 efficiency cores), macOS, 4 client worker threads, 8 server threads, plain HTTP:

RESULTS_TABLE

macOS cannot pin a thread to a core, so these runs were not pinned; the spread is the lowest and highest of the 5 runs. On Linux, `--pin` runs the server and each client under `taskset` with fixed cores. No HTTPS row was measured: the raw client has no TLS, and over HTTPS the handshake and encryption would add the same cost to both.

To reproduce:

```bash
cargo bench --bench client_overhead --features raw-hyper -- suite
# Options, with their defaults:
cargo bench --bench client_overhead --features raw-hyper -- suite --users 16 --workers 4 \
    --server-threads 8 --warmup 2 --seconds 10 --runs 5 [--modes goose,hyper,...] [--pin]
```

Run it on a quiet machine: other load on the cores widens the spread, and changes the medians.

## Where reqwest's time goes

Sampling `reqwest-tuned` and `hyper` with macOS's `sample` and charging each on CPU sample to the crate of its innermost Rust frame gives, per request (about 2,800 on CPU samples each, so read these as approximate):

| Cost reqwest adds over a bare hyper client | µs per request |
|---|---:|
| reqwest's own code: `Client::execute_request` (default headers merged, the headers cloned for a redirect, the `Url` turned into a `Uri` through a string), `Pending` and `PendingRequest` (boxed), `Response` and its boxed body | about 2.0 |
| The `url` and `idna` crates: every response's URL is parsed again from the request's `Uri`, turned to a string, because tower-http's redirect layer sets that `Uri` on every response, even with redirects off | about 1.1 |
| Generic code for reqwest's types (drop glue, futures) | about 1.0 |
| `http`: the extra header maps and requests built along the way | about 0.7 |
| `hyper_util`'s pool and client, with reqwest's connector | about 0.7 |
| tokio, the kernel and `bytes`: more wakeups and copies | about 1.4 |
| The tower layers: retry, cookie, redirect and decompression | about 0.3 |

The bare hyper client in that comparison, `hyper`, still uses the `hyper_util` legacy client's pool. Dropping the pool for one connection per user (`hyper-conn`) saves about 3 µs more per request.

## What tuning reqwest recovers

Very little. Each setting alone, and all of them together, measured within the spread of plain `reqwest` (medians of 5 runs; the first suite, run on its own):

| reqwest setting | CPU µs per request, median |
|---|---:|
| Goose's settings (`reqwest`) | 30.02 |
| `cookie_store(false)` | 29.56 |
| `redirect(Policy::none())` | 30.43 |
| `retry(retry::never())` | 30.34 |
| `no_proxy()` | 29.93 |
| no `timeout` | 29.74 |
| all but the timeout (`reqwest-tuned`) | 31.24 |
| all of them | 29.20 |

None of these settings removes a layer: the redirect, cookie, retry and decompression layers stay in reqwest's service stack and take their cheap path, and the costs above that matter most (the response URL parsed again, the boxing, the header copies) happen whatever the settings. Tuning reqwest is not the answer; the gap is in reqwest's structure.

## The decision

The bar for adding a raw client was a cut of at least 25% in CPU per request against the best tuned reqwest, with no type parameter on `GooseUser`, `GooseAttack` or `Scenario`. One hyper connection per user with a timeout, the raw client's design, cuts DECISION_CUT against tuned reqwest without a timeout, the best tuned figure, in the bare loops. The `hyper_util` legacy client, with its pool, does not meet the bar. Inside Goose, where Goose's own work per request is the same for both, `goose-raw` costs GOOSE_RAW_CUT less CPU per request than `goose`. Goose adds the raw client behind the `raw-hyper` feature.

## Using the raw client

Turn on the feature in the load test's `Cargo.toml`. The feature is on `main`, to be released with Goose 0.20; until then, take Goose from the repository:

```toml
goose = { git = "https://github.com/tag1consulting/goose", branch = "main", features = ["raw-hyper"] }
```

Then send a request with `GooseUser::raw_get` or `raw_get_named` in place of `get` or `get_named`:

```rust
use goose::prelude::*;

async fn website_index(user: &mut GooseUser) -> TransactionResult {
    let _ = user.raw_get("/").await?.discard_body().await;

    Ok(())
}
```

Or build any request without a body as raw, with a name, another method or an expected status code:

```rust
use goose::prelude::*;

async fn website_about(user: &mut GooseUser) -> TransactionResult {
    let request = GooseRequest::builder()
        .path("/about")
        .name("about")
        .expect_status_code(200)
        .raw(true)
        .build();
    let _ = user.request(request).await?.discard_body().await;

    Ok(())
}
```

A load test can mix the two: a login through `post_form`, which keeps its cookie in the user's reqwest client, and raw requests after it, which send no cookie. The raw client is the per request switch: nothing changes for a request that does not ask for it, and there is no command line option.

### What the raw client does

- Each user opens one HTTP/1.1 connection with its first raw request and keeps it alive, with `TCP_NODELAY` set as reqwest sets it. It opens another when the connection closes or fails, or when a request goes to another host or port.
- Each request sends `Host` and `User-Agent: goose/<version>`, and no body.
- Like the reqwest path, it sends a `GET`, `HEAD`, `PUT`, `DELETE` or `OPTIONS` request once more, on a new connection, when a reused connection closes before the response head arrives, and counts it in the resends Goose logs at the end of the run.
- Goose's request timeout (`--timeout`, 60 seconds by default) covers the time to the response head.
- It records the same request metrics as the reqwest path: name, method, URL, status code, response time (to the response head), success, and the error.
- It returns the same `GooseResponse`. The response is a `reqwest::Response` made from hyper's, its body streamed from the connection as it is read, so `status()`, `headers()`, `text()`, `bytes()`, `json()`, `discard_body()` and the rest work as on any response, and Goose reads a body the test drops unread, before the user's next request, so the connection is kept.

### What it does not do

- **Cookies.** None are stored or sent.
- **Redirects.** A redirect comes back as it came, a 3xx, which fails the request unless it was expected.
- **Decompression.** It does not ask for gzip, and does not decompress a body.
- **Proxies.** It connects to the host directly, whatever the proxy environment says.
- **TLS.** `http://` only: a request for an `https://` URL fails.
- **HTTP/2.** HTTP/1.1 only.
- **Request bodies and headers of the test's own.** A request built with a reqwest `RequestBuilder` cannot be raw.
- **Settings from a client builder.** The user agent, timeout and other settings of `set_client_builder` or `GooseClientBuilder` apply to the reqwest client only.
- **A timeout on the body.** The timeout ends at the response head; reqwest's covers reading the body too.

### When a request gets no response

On the reqwest path, a request that gets no response at all (no connection, a closed connection, a timeout) returns a `GooseResponse` whose `response` holds the `reqwest::Error`. A raw request has no `reqwest::Error` to hold, so Goose records the failed request, with its error, and returns `TransactionError::RequestFailed`, which fails the transaction when the test passes it on with `?`.

## Why it is shaped this way

- **A Cargo feature.** Without `raw-hyper`, Goose builds no raw client code, and has the same API, defaults and dependency tree as before. hyper and `hyper-util` are already in the tree through reqwest; the feature turns on hyper's client in the versions reqwest uses, and adds no crate.
- **No client trait.** `GooseUser` holds the reqwest client as before, and, with the feature, one more private field for its raw connection. Nothing is generic over the client: no type parameter on `GooseUser`, `GooseAttack` or `Scenario`, and no trait object on any request path.
- **The same `GooseResponse`.** reqwest builds a `reqwest::Response` from any `http::Response`, so a raw response reaches the test as any other does, and the test reads it with the same code. Only an error can't take that road, which is why a request with no response returns `RequestFailed`.
- **A separate entry point, not a switch on `get`.** A switch that sent every `get` raw would drop cookies and redirects from load tests written to rely on them, with no change to their code. `raw_get` and `raw(true)` say at each request that it does without them.
