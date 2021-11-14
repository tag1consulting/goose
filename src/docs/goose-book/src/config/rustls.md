# RustLS

By default Reqwest (and therefore Goose) uses the system-native transport layer security to make HTTPS requests. This means `schannel` on Windows, `Security-Framework` on macOS, and `OpenSSL` on Linux. If you'd prefer to use a [pure Rust TLS implementation](https://github.com/ctz/rustls), disable default features and enable `rustls-tls` in `Cargo.toml` as follows:

```toml
[dependencies]
goose = { version = "^0.16", default-features = false, features = ["rustls-tls"] }
```
