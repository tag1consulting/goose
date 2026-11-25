# Raw Hyper Client

Goose sends every request through [reqwest](https://docs.rs/reqwest), which is built on [hyper](https://docs.rs/hyper) and adds redirects, decompression, proxies, HTTP/2 and a friendly API. That layer costs CPU on every request. With the `raw-hyper` feature, a load test can send any request it chooses over hyper directly instead, on one HTTP/1.1 connection per user, for about a fifth to a quarter less CPU per request in Goose (measured below).

reqwest stays the default. Nothing changes until a load test turns the feature on, and then only the requests it marks raw take the other road.

## When to use it

When a load test runs out of client CPU before the target runs out of capacity: one machine has to send as many requests as it can, and the requests are simple ones (pages, API calls, form posts) that need nothing on the list under [What a raw request does not do](#what-a-raw-request-does-not-do). Mark those requests raw, and keep any request that needs a redirect followed, a proxy, a compressed response or HTTP/2 on reqwest. A user can mix both in one transaction: they share the user's cookies.

## Turning it on

In the load test's `Cargo.toml`:

```toml
[dependencies]
goose = { version = "^0.20", features = ["raw-hyper"] }
```

Then, per request:

- `user.raw_get(path)` and `user.raw_get_named(path, name)`, in place of `get` and `get_named`;
- `user.raw_post(path, body)`, in place of `post`;
- any request built with a `GooseRequest`, with `.raw(true)` on its builder: any method, a name, an expected status code, and headers, a body, query parameters and a timeout of its own set on a reqwest `RequestBuilder` given with `set_request_builder`, as on the reqwest path.

## Example

A user logs in over reqwest, then loads pages and sends an API call over the raw client, with the session cookie the login set:

```rust
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("RawPages")
                .register_transaction(transaction!(login).set_on_start())
                .register_transaction(transaction!(pages)),
        )
        .execute()
        .await?;

    Ok(())
}

/// Over reqwest: the session cookie the site sets lands in the user's cookie jar.
async fn login(user: &mut GooseUser) -> TransactionResult {
    let params = [("username", "test_user"), ("password", "")];
    let _ = user.post_form("/login", &params).await?.discard_body().await;

    Ok(())
}

/// Over the raw client, with the session cookie.
async fn pages(user: &mut GooseUser) -> TransactionResult {
    let _ = user.raw_get("/").await?.discard_body().await;
    let _ = user.raw_get_named("/node/1", "node").await?.discard_body().await;

    // A header and a body of the request's own, set as for any request.
    let request_builder = user
        .get_request_builder(&GooseMethod::Put, "/api/item/1")?
        .header("content-type", "application/json")
        .body(r#"{"title":"Goose"}"#);
    let goose_request = GooseRequest::builder()
        .set_request_builder(request_builder)
        .name("api item")
        .raw(true)
        .build();
    let _ = user.request(goose_request).await?.discard_body().await;

    Ok(())
}
```

## What a raw request does

- **One HTTP/1.1 connection per user**, over `hyper::client::conn::http1`, kept alive, with `TCP_NODELAY` set as reqwest sets it, and opened again when it closes, fails or times out, or a request goes to another scheme, host or port. An idempotent request (GET, HEAD, PUT, DELETE, OPTIONS) whose kept alive connection closes before the response is sent once more on a new connection, as on the reqwest path, and counted with the reqwest path's resends.
- **`http://` and `https://`.** HTTPS uses rustls, the TLS library reqwest uses in Goose, with the same certificate verifier Goose's clients share (the platform's), offering HTTP/1.1 by ALPN. `--accept-invalid-certs`, or `GooseClientBuilder::accept_invalid_certs`, accepts any certificate, as it does for reqwest. Each user keeps its own TLS session cache, so a new connection resumes the user's session. Raw HTTPS uses rustls even when Goose is built with the `native-tls` feature.
- **The user's cookies.** With the `raw-hyper` feature, Goose gives each user's reqwest client a cookie jar Goose holds, the same jar `cookie_store(true)` would build, so reqwest sends the same cookies it always did. A raw request sends the jar's `Cookie` header for its URL, unless the request sets its own `Cookie` header, and stores any `Set-Cookie` of its response back in the jar, so a cookie set over either road is sent over both. A client the load test builds itself, with `GooseUser::set_client_builder` or by handing one to `GooseUser::new`, holds a jar Goose can't see: that user's raw requests then send no cookies. Without Goose's `cookies` feature there are no cookies on either road.
- **The headers reqwest sends:** `Host`, `User-Agent` (Goose's, or the one a `GooseClientBuilder` set) and `Accept: */*`, each unless the request sets its own, then every header the request set.
- **The timeout covers the whole exchange:** connecting, the TLS handshake, sending, the response head, and the body, whether the test reads it or Goose reads a body the test dropped unread. It is Goose's `--timeout` (or a `GooseClientBuilder`'s), or the request's own `RequestBuilder::timeout`. A server that sends a response head and stalls fails the body read when the time is up, and the user goes on with a new connection.
- **The same metrics**: name, method, URL, status code, response time to the response head, success and error, recorded through the same code as the reqwest path, so reports, thresholds and the Controllers see raw requests as any other.
- **The same `GooseResponse`.** A raw response reaches the test as a `reqwest::Response`: `status()`, `headers()`, `text()`, `json()`, `discard_body()` and the rest work, and Goose reads a body the test drops unread.

## What a raw request does not do

These are the reasons to keep a request on reqwest:

- **Redirects.** A 3xx comes back as it came, and counts as a failure unless the request expects that status; `--sticky-follow` does not apply.
- **Proxies.** A raw request connects directly, ignoring `HTTP_PROXY`, `HTTPS_PROXY` and any proxy on the client.
- **Decompression.** A raw request does not send `Accept-Encoding: gzip`, so the server answers uncompressed, unless the request sets that header itself, and then the test reads the compressed bytes.
- **HTTP/2.** A raw request is always HTTP/1.1; one whose `RequestBuilder` sets another HTTP version is refused with `TransactionError::Custom` before anything is sent.
- **The rest of a client builder.** Settings of a `reqwest::ClientBuilder` other than those above (default headers, a referer, local address, DNS overrides, client certificates) do not reach raw requests.

A builder call a raw request can't honour is refused with an error, never ignored. `GooseMethod::Custom` is refused as it is on the reqwest path.

## How errors differ

When a raw request gets a response, of any status, it returns a `GooseResponse` as `get` does. When it gets none at all (the connection can't be opened, the TLS handshake fails, the connection fails, or the timeout passes before the head arrives), Goose records the failed request with its error, as it does for reqwest, and then returns `TransactionError::RequestFailed` holding the request's metric, where the reqwest path returns a `GooseResponse` whose `response` holds the error. Nothing outside reqwest can build a `reqwest::Error`. So `user.raw_get(path).await?` fails the transaction on such a request; to go on instead, match the error:

```rust
match user.raw_get("/").await {
    Ok(goose) => {
        let _ = goose.discard_body().await;
    }
    Err(e) => match *e {
        // Already recorded as a failed request.
        TransactionError::RequestFailed { .. } => {}
        _ => return Err(e),
    },
}
```

A body read that fails, a timeout part way through the body for example, fails as a reqwest body read does, with the cause in the error's source.

## Measurements

`benches/client_overhead.rs` runs a hyper HTTP/1.1 server in a process of its own, answering every request with the same 13 byte body, and each client in a fresh process. Each of 16 users is a task with one keep-alive connection, sending its next request as soon as it has read the last response. The figure is the client process's CPU time, user plus system, from `getrusage`, over 10 seconds after a 2 second warm up, divided by the requests it completed. The runs of the clients in a suite are interleaved. Every mode counts a request only when it succeeded and its whole body was read; a run in which any request failed, or returned another body, is thrown out. `self-check` checks this for every mode, against a server that answers another body.

| Client | What it is |
|---|---|
| `goose` | A Goose load test, each transaction `user.get("/")` then `discard_body()` |
| `goose-tuned` | The same, each user's client replaced with a tuned reqwest client (no cookie store, redirects, retries, proxies or `Referer`) through `set_client_builder` |
| `goose-raw` | The same, each request `user.raw_get("/")` |
| `+headers` | Each GET built on a reqwest `RequestBuilder` with two headers of its own, raw with `.raw(true)` |
| `+cookie` | Each user first requests `/login` over reqwest, which sets a cookie, so every request after it carries the cookie |
| `goose-post`, `goose-raw-post` | `user.post("/", body)` and `user.raw_post("/", body)` of a 32 byte body |
| `reqwest-tuned+no-timeout` | A bare loop over the tuned reqwest client, with no timeout either: the cheapest reqwest |
| `hyper-conn+timeout` | A bare loop over one `hyper::client::conn::http1` connection per user, each request under a 60 s timeout: the floor a raw client in Goose works toward |

Measured on an Apple M3 Ultra (24 performance and 8 efficiency cores), macOS, 4 client worker threads, 8 server threads, 9 runs of each, the load average from other work on the machine between 10 and 19:

```bash
cargo bench --bench client_overhead --features raw-hyper -- suite --runs 9 \
    --modes goose,goose-tuned,goose-raw,goose+headers,goose-raw+headers,goose+cookie,\
goose-raw+cookie,goose-post,goose-raw-post,reqwest-tuned+no-timeout,hyper-conn+timeout
```

Plain HTTP:

| Client | CPU µs per request, median | lowest | highest | requests/s, median |
|---|---:|---:|---:|---:|
| `goose` | 33.39 | 30.68 | 35.51 | 86,246 |
| `goose-tuned` | 32.69 | 31.96 | 34.73 | 87,325 |
| `goose-raw` | 25.47 | 21.08 | 28.97 | 98,693 |
| `goose+headers` | 35.13 | 33.48 | 38.16 | 84,227 |
| `goose-raw+headers` | 27.23 | 26.70 | 28.23 | 94,153 |
| `goose+cookie` | 33.98 | 33.47 | 35.10 | 85,434 |
| `goose-raw+cookie` | 26.90 | 25.37 | 28.05 | 95,206 |
| `goose-post` | 34.05 | 33.12 | 35.97 | 84,787 |
| `goose-raw-post` | 28.04 | 26.53 | 28.43 | 91,758 |
| `reqwest-tuned+no-timeout` | 28.22 | 24.33 | 30.71 | 93,654 |
| `hyper-conn+timeout` | 21.43 | 18.45 | 21.90 | 105,982 |

HTTPS, the same server over rustls with the test certificate in `tests/fixtures/tls`, offering HTTP/1.1 by ALPN, both clients run with `--accept-invalid-certs` as the certificate's CA is not trusted (the handshake happens once per user, so what this measures is each request over an open TLS connection):

```bash
cargo bench --bench client_overhead --features raw-hyper -- suite --runs 9 --tls \
    --modes goose,goose-raw,goose-post,goose-raw-post
```

| Client | CPU µs per request, median | lowest | highest | requests/s, median |
|---|---:|---:|---:|---:|
| `goose` | 34.49 | 31.02 | 38.80 | 85,905 |
| `goose-raw` | 27.21 | 26.13 | 30.11 | 94,014 |
| `goose-post` | 35.66 | 34.07 | 38.63 | 84,731 |
| `goose-raw-post` | 27.79 | 27.28 | 29.54 | 91,823 |

The cut in CPU per request, raw against reqwest in Goose, from the medians:

| Request | reqwest | raw | Cut |
|---|---:|---:|---:|
| GET, plain HTTP | 33.39 | 25.47 | 23.7% |
| GET, against `goose-tuned` | 32.69 | 25.47 | 22.1% |
| GET with two headers | 35.13 | 27.23 | 22.5% |
| GET with a cookie | 33.98 | 26.90 | 20.8% |
| POST, 32 byte body | 34.05 | 28.04 | 17.7% |
| GET, HTTPS | 34.49 | 27.21 | 21.1% |
| POST, HTTPS | 35.66 | 27.79 | 22.1% |
| Bare loops: `hyper-conn+timeout` against `reqwest-tuned+no-timeout` | 28.22 | 21.43 | 24.1% |

The cookie path's own cost, timed apart from the network with `cargo bench --bench client_overhead --features raw-hyper -- cookie-cost` (a million lookups, median of 9 runs): reading `Cookie` for the URL from an empty jar, 0.019 µs per request; from a jar holding one cookie, 0.288 µs; checking a response for `Set-Cookie` when it sets none, 0.003 µs. That is well under 1 µs, so raw requests always use the user's cookies, with no switch to turn them off.

macOS cannot pin a thread to a core, so none of these runs was pinned, on a machine with two kinds of core and other work on it; the spread is the lowest and highest of the runs, and in the plain HTTP `goose-raw` row it is wide. On Linux, `--pin` runs the server and each client under `taskset` with fixed cores. Run it on a quiet machine: other load on the cores widens the spread, and changes the medians.

To reproduce, and the options with their defaults:

```bash
cargo bench --bench client_overhead --features raw-hyper -- self-check
cargo bench --bench client_overhead --features raw-hyper -- suite
cargo bench --bench client_overhead --features raw-hyper -- suite --users 16 --workers 4 \
    --server-threads 8 --warmup 2 --seconds 10 --runs 5 [--modes goose,goose-raw,...] [--pin] [--tls]
```

### Earlier measurements

Before the raw client was added, a prototype (plain HTTP GET only, no headers, body, cookies or TLS, a timeout that ended at the response head) measured in the quietest of four suites, 9 runs each: `goose` 33.94 µs, `goose-tuned` 33.32 µs, `goose-raw` 24.85 µs (a 26.8% cut against `goose`), `reqwest-tuned+no-timeout` 27.99 µs and `hyper-conn+timeout` 21.45 µs (a 23.4% cut in bare loops). The bar then was 25%, so the prototype was not added; a cut of over 20% was later judged worth offering as an opt in. The raw client above does more per request than the prototype (headers, cookies, a deadline over the body), and keeps a 23.7% cut on a plain GET.

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

The bare hyper client in that comparison, `hyper`, still uses the `hyper_util` legacy client's pool. Dropping the pool for one connection per user (`hyper-conn`), as the raw client does, saved 1.4 to 3.8 µs more per request, depending on the suite.

Tuning reqwest recovers very little. Each setting Goose does not use that could help (no cookie store, no redirects, no retries, no proxies, no `Referer`, no timeout), alone and all together, measured within the spread of reqwest as Goose builds it, 29.20 to 31.24 µs against 30.02 µs (medians of 5 runs):

```bash
cargo bench --bench client_overhead -- suite --modes reqwest,reqwest+no-cookies,\
reqwest+no-redirect,reqwest+no-retry,reqwest+no-proxy,reqwest+no-timeout,\
reqwest-tuned,reqwest-tuned+no-timeout
```

None of these settings removes a layer: the redirect, cookie, retry and decompression layers stay in reqwest's service stack and take their cheap path, and the costs above that matter most (the response URL parsed again, the boxing, the header copies) happen whatever the settings. Two of them are worth raising with reqwest, as fixing them helps every reqwest user: the response URL parsed again on every response, and the header map cloned on every request for a redirect that most requests never follow.
