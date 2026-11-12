# Requirements

* In order to write load tests, you must first install [Rust](https://www.rust-lang.org/tools/install).

* Goose load tests are managed with [Cargo](https://doc.rust-lang.org/cargo/), the Rust package manager.

* Goose sets no minimum Rust version in its `Cargo.toml`. With the newest releases of its dependencies, Goose 0.19 needs [`rustc`](https://doc.rust-lang.org/rustc/what-is-rustc.html) 1.88.0 or later: some of them, such as `cookie_store`, which Reqwest uses for cookies, require it. Use a current stable Rust.

* Building Goose compiles [`aws-lc-rs`](https://aws.github.io/aws-lc-rs/), the cryptography library behind its TLS support, which needs a C compiler. See its [build requirements](https://aws.github.io/aws-lc-rs/requirements/index.html).

## TLS

Goose makes requests with [Reqwest](https://docs.rs/reqwest) 0.13, whose default TLS backend is [rustls](https://docs.rs/rustls), so a load test needs no OpenSSL. The clients Goose builds check certificates with [`rustls-platform-verifier`](https://docs.rs/rustls-platform-verifier), which uses the operating system's certificate store, so they trust a CA you have added to the system. They check certificates unless you accept invalid ones with `--accept-invalid-certs`, `GooseDefault::AcceptInvalidCerts` or `accept_invalid_certs(true)` on a `GooseClientBuilder`. Accept invalid certificates only for a test server with a self signed or expired certificate.

Goose builds the certificate verifier once per process, with the first of its clients set to check certificates, even when the target is plain HTTP and no certificate is ever checked. At that moment, Goose takes the process default rustls crypto provider, or aws-lc-rs if there is none. If you install a provider, do it at the start of `main`, before any Goose call. On Linux, the verifier also reads the system's root certificates at that moment, so restart the load test process to trust a CA added later.

Goose before 0.19 used Reqwest 0.12 and native TLS by default, with rustls through the `rustls-tls` feature. In 0.19 the `rustls-tls` feature still exists, so a `Cargo.toml` that enables it keeps building, but the feature does not change which TLS library HTTP requests use.

## Which HTTP version Goose speaks

Over HTTPS, Goose offers HTTP/2 and HTTP/1.1 and the server chooses, as a browser does. Plain HTTP is HTTP/1.1. `--http1-only` (`GooseDefault::Http1Only`, or `http1_only(true)` on a `GooseClientBuilder`) keeps Goose to HTTP/1.1, for example to compare with runs from Goose 0.19 or earlier.

Aimed at a CDN, a load balancer or a TLS front end, Goose tests the HTTP/2 path browsers use. Aimed at an origin that offers only HTTP/1.1, as most CDNs reach their origins, it speaks HTTP/1.1. Goose does not support HTTP/3, which carries a large share of browser traffic, because Reqwest still marks its HTTP/3 client unstable.

A client you pass to `GooseUser::set_client_builder` follows your `reqwest::ClientBuilder`, not `--http1-only`: it offers HTTP/2 and HTTP/1.1 unless your builder calls `.http1_only()`, and it uses native TLS when your `reqwest` dependency has `features = ["native-tls"]`. [A Client Without Cookies](./config/cookie.html#a-client-without-cookies) shows how to pass one. It replaces Goose's defaults for that user, so set the ones you need on your builder, such as the timeout, or `.cookie_store(true)` for cookies.

## Features

Goose has these Cargo features:

| Feature | Default | What it adds |
|-|-|-|
| `cookies` | on | Each `GooseUser` keeps its own cookies; see [Cookies](./config/cookie.html). Build with `default-features = false` to leave it out. |
| `dashboard` | off | The [live web dashboard](./controller/dashboard.html), served with `--dashboard`. |
| `pdf-reports` | off | PDF reports, written with a `.pdf` `--report-file`. |
| `rustls-tls` | off | Kept so that older `Cargo.toml` files build; see [TLS](#tls). |

For example, to build a load test with the dashboard:

```toml
[dependencies]
goose = { version = "^0.19", features = ["dashboard"] }
```

## Chrome for PDF reports

A `.pdf` `--report-file` needs the `pdf-reports` feature and a Chrome or Chromium browser to render the report. Goose uses the browser the `CHROME` environment variable names, then an installed Chrome, Chromium or Edge, and otherwise downloads a Chrome for Testing headless shell once into the user cache directory, which needs `unzip` (`tar` on Windows) and, on Linux, glibc. See [PDF report system requirements](./getting-started/metrics.html#system-requirements).

`--pdf-print-html` writes a print-ready HTML report to turn into a PDF in any browser, and needs neither Chrome nor the `pdf-reports` feature.
