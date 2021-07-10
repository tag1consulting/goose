# Goose

Have you ever been attacked by a goose?

[![crates.io](https://img.shields.io/crates/v/goose.svg)](https://crates.io/crates/goose)
[![Documentation](https://docs.rs/goose/badge.svg)](https://docs.rs/goose)
[![Apache-2.0 licensed](https://img.shields.io/crates/l/goose.svg)](./LICENSE)
[![CI](https://github.com/tag1consulting/goose/workflows/CI/badge.svg)](https://github.com/tag1consulting/goose/actions?query=workflow%3ACI)
[![Docker Repository on Quay](https://quay.io/repository/tag1consulting/goose/status "Docker Repository on Quay")](https://quay.io/repository/tag1consulting/goose)

### Documentation

- [README](https://github.com/tag1consulting/goose/blob/main/README.md)
- [Developer documentation](https://docs.rs/goose/)
- [Blogs and more](https://tag1.com/goose/)
  - [Goose vs Locust and jMeter](https://www.tag1consulting.com/blog/jmeter-vs-locust-vs-goose)
  - [Real-life load testing with Goose](https://www.tag1consulting.com/blog/real-life-goose-load-testing)
  - [Gaggle: a distributed load test](https://www.tag1consulting.com/blog/show-me-how-flock-flies-working-gaggle-goose)
  - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)


## RustLS

By default Reqwest (and therefore Goose) uses the system-native transport layer security to make HTTPS requests. This means `schannel` on Windows, `Security-Framework` on macOS, and `OpenSSL` on Linux. If you'd prefer to use a [pure Rust TLS implementation](https://github.com/ctz/rustls), disable default features and enable `rustls` in `Cargo.toml` as follows:

```toml
[dependencies]
goose = { version = "^0.12", default-features = false, features = ["rustls"] }
```
