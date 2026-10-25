# Cookies

Goose's `cookies` feature is one of its default features. With it, each `GooseUser` gets its own Reqwest client with its own cookie store, so a cookie the server sets for one user, such as a session cookie after a login, is sent back with that user's later requests and never with another user's.

## Default Behavior

No configuration is needed:

```rust
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    // No configuration needed: each user has its own cookie store.
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("WebsiteUser")
                .register_transaction(transaction!(login).set_on_start())
                .register_transaction(transaction!(load_homepage)),
        )
        .execute()
        .await?;

    Ok(())
}

async fn login(user: &mut GooseUser) -> TransactionResult {
    let params = [("username", "test_user"), ("password", "")];
    // A session cookie the server sets here is sent with this user's later requests.
    let _goose = user.post_form("/login", &params).await?;
    Ok(())
}

async fn load_homepage(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}
```

Goose builds each user's client with the user agent `goose/0.19.0`, the `--timeout`, gzip unless `--no-gzip`, invalid certificates accepted only with `--accept-invalid-certs`, and a cookie store when the `cookies` feature is enabled. Every user has a client of its own whether or not it keeps cookies. In a run of `examples/simple`, 1,000 users used about 57 MB more memory than 10 users.

## A Client Without Cookies

To change the client of a user, build a [`reqwest::ClientBuilder`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html) and pass it to [`GooseUser::set_client_builder`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_client_builder), usually in a transaction run once with `set_on_start()`. A client built without `.cookie_store(true)` keeps no cookies:

```rust
use goose::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("ApiUser")
                .register_transaction(transaction!(setup_client).set_on_start())
                .register_transaction(transaction!(api_call)),
        )
        .execute()
        .await?;

    Ok(())
}

async fn setup_client(user: &mut GooseUser) -> TransactionResult {
    // Without .cookie_store(true) this user's client keeps no cookies.
    let builder = reqwest::Client::builder()
        .user_agent("my-loadtest/1.0")
        .timeout(Duration::from_secs(10));
    user.set_client_builder(builder).await?;
    Ok(())
}

async fn api_call(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/api/status").await?;
    Ok(())
}
```

The new client replaces all of Goose's defaults for that user, so set again any of them you need, such as the timeout. See the [Basic Authentication example](../example/basic-auth.md) for a custom client that keeps cookies and adds default headers.

## Building Without Cookie Support

To leave cookie support out of the build, turn off Goose's default features:

```toml
[dependencies]
goose = { version = "^0.19", default-features = false }
```

Reqwest is then built without its `cookies` feature, unless another dependency turns it on, and no user's client keeps cookies. `ClientBuilder::cookie_store` and `cookie_provider` only exist with Reqwest's `cookies` feature, so a load test that calls them needs it, for example through its own `reqwest = { version = "0.13", features = ["cookies"] }` dependency.

## The Type-Safe Client Builder

The `goose::client` module, which is not part of the prelude, has a [`GooseClientBuilder`](https://docs.rs/goose/*/goose/client/struct.GooseClientBuilder.html) that records its cookie choice in its type: it starts in the `CookiesEnabled` state, `without_cookies()` moves it to `CookiesDisabled` and `with_cookies()` moves it back. `timeout`, `user_agent`, `gzip` and `accept_invalid_certs` are available in both states. [`GooseAttack::set_client_builder_with_cookies`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.set_client_builder_with_cookies) takes a `CookiesEnabled` builder and [`set_client_builder_without_cookies`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.set_client_builder_without_cookies) a `CookiesDisabled` one, and both return a `Result`:

```rust
use goose::client::GooseClientBuilder;
use goose::prelude::*;
use std::time::Duration;

fn main() -> Result<(), GooseError> {
    // Starts in the CookiesEnabled state.
    let with_cookies = GooseClientBuilder::new().timeout(Duration::from_secs(30));

    // without_cookies() moves to the CookiesDisabled state, with_cookies() back.
    let without_cookies = GooseClientBuilder::new()
        .without_cookies()
        .timeout(Duration::from_secs(15))
        .user_agent("my-loadtest/1.0");

    // Each setter only accepts a builder in its own state. In this release
    // neither changes the clients Goose builds for its users.
    let _attack = GooseAttack::initialize()?
        .set_client_builder_with_cookies(with_cookies)?
        .set_client_builder_without_cookies(without_cookies)?;

    Ok(())
}
```

In this release both setters accept the builder and then discard it: every user still gets the default client described above, with the default user agent, timeout and cookie store, and no client is shared between users. To change a user's client, use `GooseUser::set_client_builder` as shown above.

## Creating a GooseUser Directly

Load tests don't normally create a `GooseUser`, but code that does, such as a test of a transaction function, passes the scenario's name to [`GooseUser::new`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.new) as an `Arc<str>`; it was a `String` in 0.18. The last argument is an optional `reqwest::Client`; with `None` the user gets the default client.

```rust
use goose::prelude::*;
use goose::config::GooseConfiguration;
use std::sync::Arc;

fn main() -> Result<(), GooseError> {
    let configuration = GooseConfiguration::default();
    let base_url = reqwest::Url::parse("http://127.0.0.1:8080/").unwrap();
    let _user = GooseUser::new(0, Arc::from("websiteuser"), base_url, &configuration, 0, None)?;
    Ok(())
}
```
