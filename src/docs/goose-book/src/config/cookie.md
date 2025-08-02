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

## Performance Considerations

### Individual Clients (Default)
- **Memory usage**: Higher (one client per user)
- **Cookie support**: Full cookie jar per user
- **Use cases**: Applications requiring session management, user authentication

### Shared Client (Optimized)
- **Memory usage**: Lower (single shared client)
- **Cookie support**: None
- **Performance**: Optimized for high user counts (1000+ users)
- **Use cases**: Stateless API testing, high-scale load tests

The type-safe client builder approach provides compile-time safety and prevents invalid configurations, making it the preferred method for configuring cookie behavior.
