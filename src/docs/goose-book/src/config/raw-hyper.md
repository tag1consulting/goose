# Raw Hyper Client

Goose sends every request through [reqwest](https://docs.rs/reqwest), which is built on [hyper](https://docs.rs/hyper) and adds cookies, redirects, TLS setup, decompression, proxies and a friendly API. That layer costs CPU on every request. A load test that has to get the most requests out of one machine against a simple target might give up what reqwest adds for a cheaper request, by sending over hyper directly.

Goose does not offer such a raw hyper client. This page gives the measurements that decided it, what tuning reqwest recovers, and the design a raw client would take, should the numbers change.

## What was measured

`benches/client_overhead.rs` runs a hyper HTTP/1.1 server in a process of its own, answering every request with the same 13 byte body, and each client in a fresh process. Each of 16 users is a task with one keep-alive connection, sending its next request as soon as it has read the last response. The figure is the client process's CPU time, user plus system, from `getrusage`, over 10 seconds after a 2 second warm up, divided by the requests it completed. The runs of the clients in a suite are interleaved.

| Client | What it is |
|---|---|
| `goose` | A Goose load test, each transaction `user.get("/")` then `discard_body()`: Goose as it is |
| `goose-tuned` | The same, each user's client replaced with the tuned reqwest client below (`set_client_builder`) |
| `goose-raw` | The same, each request `user.raw_get("/")` over a prototype raw client (see The decision) |
| `reqwest` | A bare loop over a reqwest client per user, built as Goose builds each user's: user agent, 60 s timeout, cookie store, Goose's own `Accept-Encoding: gzip` |
| `reqwest-tuned` | The same, with every reqwest setting Goose does not use that could help: no cookie store, no redirects, no retries, no proxies, no `Referer` |
| `reqwest-tuned+no-timeout` | The same, without the timeout as well |
| `hyper`, `hyper+timeout` | A bare loop over a `hyper_util` legacy client per user (a pool holding the user's one connection), without and with a 60 s timeout |
| `hyper-conn`, `hyper-conn+timeout` | A bare loop over one `hyper::client::conn::http1` connection per user, with no pool, without and with a 60 s timeout: the design below |

Every mode counts a request only when it succeeded and its whole body was read; a run in which any request failed, or returned another body, is thrown out. `cargo bench --bench client_overhead -- self-check` checks this, against a server that answers another body.

Measured on an Apple M3 Ultra (24 performance and 8 efficiency cores), macOS, 4 client worker threads, 8 server threads, plain HTTP:

All ten clients, 5 runs each (`suite` with its defaults). The machine was shared, and its load average rose from 13 to over 100 during this run, which shows in the spread:

| Client | CPU µs per request, median | lowest | highest | requests/s, median |
|---|---:|---:|---:|---:|
| `goose` | 35.66 | 26.25 | 37.88 | 80,769 |
| `goose-tuned` | 34.62 | 31.41 | 39.62 | 81,622 |
| `goose-raw` | 24.32 | 17.98 | 27.99 | 89,186 |
| `reqwest` | 31.94 | 29.16 | 34.19 | 79,875 |
| `reqwest-tuned` | 30.53 | 28.77 | 33.08 | 72,187 |
| `reqwest-tuned+no-timeout` | 29.20 | 28.01 | 31.69 | 91,428 |
| `hyper` | 22.81 | 22.36 | 24.52 | 96,971 |
| `hyper+timeout` | 25.00 | 21.88 | 26.30 | 92,153 |
| `hyper-conn` | 21.43 | 19.25 | 22.82 | 98,235 |
| `hyper-conn+timeout` | 21.91 | 18.84 | 23.00 | 92,562 |

The clients the decision rests on, 9 runs each, with the load average between 8 and 19 throughout, the quietest run:

```bash
cargo bench --bench client_overhead -- suite --runs 9 \
    --modes goose,goose-tuned,reqwest-tuned+no-timeout,hyper-conn+timeout
```

| Client | CPU µs per request, median | lowest | highest | requests/s, median |
|---|---:|---:|---:|---:|
| `goose` | 33.94 | 32.62 | 36.99 | 86,657 |
| `goose-tuned` | 33.32 | 32.08 | 33.87 | 87,604 |
| `goose-raw` | 24.85 | 24.11 | 25.45 | 99,289 |
| `reqwest-tuned+no-timeout` | 27.99 | 27.38 | 29.66 | 94,495 |
| `hyper-conn+timeout` | 21.45 | 20.97 | 22.20 | 104,922 |

The `goose-raw` rows come from a prototype of the design below, built behind a `raw-hyper` feature, with a `goose-raw` mode in this benchmark; it is in the history of [#854](https://github.com/tag1consulting/goose/pull/854), commit `0a0f1610`, and is not in Goose.

reqwest as Goose builds it (`reqwest`) costs 8 to 10 µs more CPU per request than the design below (`hyper-conn+timeout`), in bare loops of each.

macOS cannot pin a thread to a core, so none of these runs was pinned, on a machine with two kinds of core; the spread is the lowest and highest of the runs. On a result this close to the bar, a pinned run could move the call either way. On Linux, `--pin` runs the server and each client under `taskset` with fixed cores. No HTTPS row was measured: the raw client has no TLS, and over HTTPS the handshake and encryption would add the same cost to both.

To reproduce:

```bash
cargo bench --bench client_overhead -- suite
# Options, with their defaults:
cargo bench --bench client_overhead -- suite --users 16 --workers 4 \
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

The bare hyper client in that comparison, `hyper`, still uses the `hyper_util` legacy client's pool. Dropping the pool for one connection per user (`hyper-conn`) saved 1.4 to 3.8 µs more per request, depending on the suite.

## What tuning reqwest recovers

Very little. Each setting alone, and all of them together, measured within the spread of plain `reqwest` (medians of 5 runs, from a suite of their own, its spreads 1 to 7 µs):

```bash
cargo bench --bench client_overhead -- suite --modes reqwest,reqwest+no-cookies,\
reqwest+no-redirect,reqwest+no-retry,reqwest+no-proxy,reqwest+no-timeout,\
reqwest-tuned,reqwest-tuned+no-timeout
```


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

The bar for adding a raw client was a cut of at least 25% in CPU per request against the best tuned reqwest, `reqwest-tuned+no-timeout`, with no type parameter on `GooseUser`, `GooseAttack` or `Scenario`. The design below meets the second part, and misses the first, narrowly:

| Comparison (medians) | Cut |
|---|---:|
| **The bar:** `hyper-conn+timeout` against `reqwest-tuned+no-timeout`, bare loops, quietest run | **23.4%** |
| The same, in the three other suites: 27.1%, 23.7% (the noisiest) and 25.0% | 23.4% to 27.1% |
| Bare loops, both with a timeout: `hyper-conn+timeout` against `reqwest-tuned` | 27% to 32% |
| Prototype in Goose, both with a timeout: `goose-raw` against `goose-tuned` | 25.4% |
| Prototype in Goose: `goose-raw` against `goose`, Goose as it is | 26.8% |

The raw client keeps a timeout, which a load test needs, while the best tuned reqwest has none; against that figure, the cut is under 25% in the quietest run and in one other, and at most 27% in any. The `hyper_util` legacy client, with its pool (`hyper+timeout`), cuts only 13% to 14%. So Goose adds no raw client: about a quarter less CPU per request, at the edge of noise, does not pay for a second request path, with its own behaviour on every point where it differs from `get`. A pinned run on Linux, or a change in reqwest or hyper, could change this; the benchmark is here to run again.

What does not need a raw client: the costs above come from reqwest's structure. Two of them are worth raising upstream, as fixing them helps every reqwest user: the response URL parsed again on every response, and the header map cloned on every request for a redirect that most requests never follow.

## The design, had it been added

The shape the prototype took, and what it showed a raw client would have to get right.

- **An off by default Cargo feature, `raw-hyper`.** hyper and `hyper-util` are already in the tree through reqwest; the feature turns on hyper's client in the versions reqwest uses, and adds no crate. Without the feature, nothing changes: the same API, defaults and dependency tree.
- **A per request switch, not a per test one.** `GooseUser::raw_get(path)` and `raw_get_named(path, name)`, and `GooseRequestBuilder::raw(true)` for any request without a body, with a name, another method or an expected status code. A switch that sent every `get` raw would drop cookies and redirects from load tests written to rely on them, with no change to their code.
- **No client trait.** `GooseUser` keeps its reqwest client, and gets one more private field, its raw connection. Nothing is generic over the client: no type parameter on `GooseUser`, `GooseAttack` or `Scenario`, and no trait object on any request path.
- **One HTTP/1.1 connection per user**, over `hyper::client::conn::http1`, kept alive, with `TCP_NODELAY` set as reqwest sets it, and opened again when it closes or fails, or a request goes to another host or port. Each request sends `Host` and `User-Agent: goose/<version>`.
- **The same metrics**: name, method, URL, status code, response time to the response head, success and error, recorded through the same code as the reqwest path.
- **The same `GooseResponse`.** reqwest builds a `reqwest::Response` from any `http::Response`, and `reqwest::Body::wrap` takes hyper's body, so a raw response reaches the test as any other does: `status()`, `text()`, `discard_body()` and the rest work, and Goose reads a body the test drops unread. Only an error can't take that road, as nothing outside reqwest can build a `reqwest::Error`: a request that gets no response at all records the failed request and returns `TransactionError::RequestFailed`, where the reqwest path returns a `GooseResponse` holding the error.

What it would not do: cookies, redirects (a 3xx comes back as it came), decompression (it does not ask for gzip), proxies, TLS (`http://` only), HTTP/2, request bodies, headers of the test's own, or the settings of a client builder.

What the prototype showed a raw client must get right:

- **The timeout has to cover the body, and Goose's read of a body the test dropped unread.** A timeout that ends at the response head, as the prototype's did, leaves a user waiting for good on a server that sends its head and stalls, even in a transaction that never reads the body, since Goose reads it before the user's next request. reqwest's timeout covers the body.
- **A request sent again has to be counted even when it then times out.** The prototype counted the resend only when the second attempt returned, so a resend that timed out went uncounted.
