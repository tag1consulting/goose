# TLS And Certificates

This page describes Goose 0.19.2 and later 0.19 releases.

## How Goose builds its clients

Every user has its own `reqwest::Client`, unless the load test calls `set_client_builder_without_cookies`, which gives every user one shared client; see [Cookies](cookie.md).

A client that checks certificates, which is every client unless you pass `--accept-invalid-certs`, uses rustls. All of them share one certificate verifier, which Goose builds with the first of them, even when the target is plain HTTP. Each client still gets its own rustls `ClientConfig`, with its own TLS session resumption store, so no user resumes another user's TLS session.

The verifier is [`rustls-platform-verifier`](https://docs.rs/rustls-platform-verifier), so the clients trust the CAs the operating system trusts. On Linux it reads the system's root certificates once per process, when it is built, so a CA added to the system while a test runs is not seen: restart the load test to trust it. When Goose builds the verifier it also takes the process default rustls crypto provider, or aws-lc-rs if there is none, so install a provider of your own at the start of `main`, before any Goose call.

These clients use rustls and HTTP/1.1 even if the load test, or any crate it depends on, turns on Reqwest's `http2` or `native-tls` feature. Any `reqwest` 0.13 dependency with default features turns on `http2`; `cargo tree -e features -i reqwest@0.13` shows which features are on. Before 0.19.2 these clients used HTTP/2 with an HTTPS server that offered it when `http2` was on, and native TLS when `native-tls` was on.

With `--accept-invalid-certs` (`GooseDefault::AcceptInvalidCerts`, or `accept_invalid_certs(true)` on a `GooseClientBuilder`), the client checks no certificate and Goose leaves its TLS to Reqwest, which builds it the way it would for any client: with native TLS when Reqwest's `native-tls` feature is on, and HTTP/2 over HTTPS when its `http2` feature is on. Use it only for a test server with a self signed or expired certificate.

## HTTP/2 or native TLS

A load test that needs HTTP/2, or the platform's TLS library (OpenSSL on Linux, Security.framework on macOS, SChannel on Windows), with certificates checked, builds its own `reqwest::ClientBuilder` and passes it to [`GooseUser::set_client_builder`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_client_builder), usually in a transaction run once with `set_on_start()`. Goose builds that user's client from your builder as it is, so it does not use the shared verifier, and Reqwest picks its TLS library and HTTP versions from the features on. Goose does not re-export Reqwest, so add it to the load test's `Cargo.toml` with the features you need:

```toml
[dependencies]
goose = "^0.19.2"
reqwest = { version = "0.13", default-features = false, features = ["http2", "native-tls"] }
tokio = "^1"
```

`http2` lets the client offer HTTP/2 and HTTP/1.1 over HTTPS, and the server chooses. `native-tls` makes Reqwest use the platform's TLS library for every client it builds itself.

```rust
use goose::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("WebsiteUser")
                .register_transaction(transaction!(setup_client).set_on_start())
                .register_transaction(transaction!(load_homepage)),
        )
        .execute()
        .await?;

    Ok(())
}

async fn setup_client(user: &mut GooseUser) -> TransactionResult {
    // Goose's defaults don't apply to this client: set the ones you need.
    let builder = reqwest::Client::builder()
        .user_agent("my-loadtest/1.0")
        .timeout(Duration::from_secs(60))
        .gzip(true)
        .cookie_store(true);
    user.set_client_builder(builder).await?;
    Ok(())
}

async fn load_homepage(user: &mut GooseUser) -> TransactionResult {
    let _ = user.get("/").await?.discard_body().await;
    Ok(())
}
```

The client replaces all of Goose's defaults for that user: `--timeout`, `--no-gzip` and `--accept-invalid-certs` don't apply to it, and it keeps cookies only with `.cookie_store(true)`, which needs Reqwest's `cookies` feature; Goose's default `cookies` feature turns it on.

Native TLS builds a certificate store for every client, so memory and start up time grow with the number of users: Goose 0.18.1, which used native TLS, measured about 700 KiB per user. Over rustls on Linux, Reqwest reads the system's root certificates for each client you build this way, so it is slower to build than the clients Goose builds.
