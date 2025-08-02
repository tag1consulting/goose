# Cookies

By default, Goose enables cookies for HTTP clients, with each user getting their own cookie jar. However, if you are running a load test that doesn't use cookies, you can optimize performance by using a shared client without cookies. This provides significant memory savings for high user counts (see https://github.com/tag1consulting/goose/pull/557 for more information).

## Type-Safe Client Configuration

Goose provides a type-safe client builder that prevents invalid cookie configurations at compile time:

### Individual Clients with Cookies (Default)
```rust
use goose::prelude::*;
use std::time::Duration;

GooseAttack::initialize()?
    .set_client_builder_with_cookies(
        GooseClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .user_agent("my-loadtest/1.0")
    )
    .register_scenario(scenario!("ExampleScenario")
        .register_transaction(transaction!(example_transaction))
    )
    .execute().await?;
```

### Shared Client without Cookies (Optimized)
```rust
use goose::prelude::*;
use std::time::Duration;

GooseAttack::initialize()?
    .set_client_builder_without_cookies(
        GooseClientBuilder::new()
            .without_cookies()
            .timeout(Duration::from_secs(15))
    )?
    .register_scenario(scenario!("ExampleScenario")
        .register_transaction(transaction!(example_transaction))
    )
    .execute().await?;
```

## Legacy Feature Flag Approach

If you prefer to disable cookies at compile time, you can still use the feature flag approach in `Cargo.toml`:

```toml
[dependencies]
goose = { version = "^0.18", default-features = false, features = ["reqwest/default-tls"] }
```

However, the type-safe client builder approach is recommended as it provides better compile-time safety and more flexible configuration options.
