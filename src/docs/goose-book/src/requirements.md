# Requirements

* In order to write load tests, you must first install [Rust](https://www.rust-lang.org/tools/install).

* Goose load tests are managed with [Cargo](https://doc.rust-lang.org/cargo/), the Rust package manager.

* Goose sets no minimum Rust version in its `Cargo.toml`. With the newest releases of its dependencies, Goose 0.19 needs [`rustc`](https://doc.rust-lang.org/rustc/what-is-rustc.html) 1.88.0 or later: some of them, such as `cookie_store`, which Reqwest uses for cookies, require it. Use a current stable Rust.

* Building Goose compiles [`aws-lc-rs`](https://aws.github.io/aws-lc-rs/), the cryptography library behind its TLS support, which needs a C compiler. See its [build requirements](https://aws.github.io/aws-lc-rs/requirements/index.html).

## TLS

Goose makes requests with [Reqwest](https://docs.rs/reqwest) 0.13, whose default TLS backend is [rustls](https://docs.rs/rustls), so a load test needs no OpenSSL. The clients Goose builds trust the CAs the operating system trusts, and check certificates unless you accept invalid ones with `--accept-invalid-certs`. Since 0.19.2 the clients Goose builds that check certificates use HTTP/1.1 over rustls, even when Reqwest's `http2` or `native-tls` feature is on. [TLS And Certificates](config/tls.md) explains how Goose builds its clients, what they trust, when it reads the system's root certificates, and how to use HTTP/2 or native TLS.

Goose before 0.19 used Reqwest 0.12 and native TLS by default, with rustls through the `rustls-tls` feature. In 0.19 the `rustls-tls` feature still exists, so a `Cargo.toml` that enables it keeps building, but the feature does not change which TLS library HTTP requests use.

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
