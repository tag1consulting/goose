# Cookies

By default, Goose enables the Reqwest `cookies` feature. However, if you are running a load test that doesn't use cookies, you can disable this feature. This disables the feature in Reqwest, and allows an optimization during Goose startup (see https://github.com/tag1consulting/goose/pull/557 for more information).
 
To disable client cookies and optimize startup performance, disable default features and pick a tls client in `Cargo.toml`, for example:

```toml
[dependencies]
goose = { version = "^0.17", default-features = false, features = ["reqwest/default-tls"] }
```
