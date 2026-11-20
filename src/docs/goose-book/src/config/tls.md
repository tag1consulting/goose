# TLS And Certificates

This page describes Goose 0.20. Goose 0.19.2 differs: see [Goose 0.19](#goose-019) below.

## How Goose builds its clients

Every user has its own `reqwest::Client`, unless the load test calls `set_client_builder_without_cookies`, which gives every user one shared client; see [Cookies](cookie.md).

A client that checks certificates, which is every client unless you pass `--accept-invalid-certs`, uses rustls. All of them share one certificate verifier, which Goose builds with the first of them. Each client still gets its own rustls `ClientConfig`, with its own TLS session resumption store, so no user resumes another user's TLS session.

The verifier is [`rustls-platform-verifier`](https://docs.rs/rustls-platform-verifier), so the clients trust the CAs the operating system trusts. On Linux it reads the system's root certificates once per process, when it is built, so a CA added to the system while a test runs is not seen: restart the load test to trust it. When Goose builds the verifier it also takes the process default rustls crypto provider, or aws-lc-rs if there is none, so install a provider of your own at the start of `main`, before any Goose call.

With `--accept-invalid-certs` (`GooseDefault::AcceptInvalidCerts`, or `accept_invalid_certs(true)` on a `GooseClientBuilder`), the client checks no certificate and Goose leaves its TLS to Reqwest, which builds it the way it would for any client. Use it only for a test server with a self signed or expired certificate.

## HTTP/2

Over HTTPS, every client Goose builds offers HTTP/2 and HTTP/1.1, and the server chooses. Plain HTTP is HTTP/1.1. `--http1-only` (`GooseDefault::Http1Only`, or `http1_only(true)` on a `GooseClientBuilder`) keeps the clients to HTTP/1.1. See [Which HTTP version Goose speaks](../requirements.md#which-http-version-goose-speaks).

## Native TLS

Goose's `native-tls` feature makes every client Goose builds use Reqwest's `native-tls` backend, the platform's TLS library: OpenSSL on Linux, Security.framework on macOS, SChannel on Windows. These clients do not use the shared verifier, and still offer HTTP/2 over HTTPS unless `--http1-only` is given:

```toml
[dependencies]
goose = { version = "^0.20", features = ["native-tls"] }
```

It costs memory and start up time: native TLS builds a certificate store for every client, so both grow with the number of users. Goose 0.18.1, which used native TLS, measured about 700 KiB per user.

Turning on Reqwest's `native-tls` feature in your own `Cargo.toml`, with `reqwest = { version = "0.13", features = ["native-tls"] }`, does not do this. Goose cannot see the features a dependency turns on, so the clients it builds that check certificates stay on rustls. Turn on Goose's feature instead.

## A client of your own

For anything Goose's client settings do not cover, such as a client certificate or a CA you trust only in this test, build a `reqwest::ClientBuilder` and pass it to [`GooseUser::set_client_builder`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_client_builder), usually in a transaction run once with `set_on_start()`. Goose builds that user's client from your builder as it is: no shared verifier, no `--http1-only`, `--timeout` or `--no-gzip`, and no cookies unless you ask for them. Over rustls on Linux, Reqwest reads the system's root certificates for each such client, so it is slower to build than the clients Goose builds.

```rust
{{#include ../../../../../examples/custom_client.rs}}
```

This is [`examples/custom_client.rs`](https://github.com/tag1consulting/goose/blob/main/examples/custom_client.rs); run it with `cargo run --example custom_client -- --host https://example.com/ --users 1 --run-time 10s`.

The load test needs its own `reqwest` dependency, as Goose does not re-export Reqwest. `.cookie_store(true)` needs Reqwest's `cookies` feature, which Goose's default `cookies` feature turns on.

## Goose 0.19

In Goose 0.19.2 the clients Goose builds that check certificates also share one verifier, but use HTTP/1.1 and rustls even when a load test turns on Reqwest's `http2` or `native-tls` feature, and Goose has no `native-tls` feature. A 0.19.2 load test that needs HTTP/2 or native TLS passes its own `reqwest::ClientBuilder` to `GooseUser::set_client_builder`, as above, with `features = ["http2"]` or `features = ["native-tls"]` on its `reqwest` dependency.
