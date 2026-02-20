# Cookies

By default, Goose enables cookies for HTTP clients, with each user getting their own cookie jar. However, if you are running a load test that doesn't use cookies, you can optimize performance by using a shared client without cookies. This provides significant memory savings for high user counts (see https://github.com/tag1consulting/goose/pull/557 for more information).

## Quick Decision Guide

| Configuration | Memory Usage | Cookie Support | Best For |
|---------------|-------------|----------------|----------|
| **Individual Clients with Cookies** (Default) | ~1MB per user | Full cookie jar per user | Session-based apps, authentication flows, shopping carts |
| **Shared Client without Cookies** | ~1MB total | None | Stateless APIs, high-scale load tests (1000+ users) |

**Memory Impact Example**: For 1000 users, individual clients use ~1GB while shared client uses ~1MB.

## Configuration Methods

### Default Behavior (Individual Clients with Cookies)

Your existing code continues to work unchanged. Goose automatically provides individual clients with cookies enabled:

```rust
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    // No configuration needed - cookies enabled by default
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("WebsiteUser")
                .register_transaction(transaction!(load_homepage))
                .register_transaction(transaction!(login_user))
        )
        .execute()
        .await?;
    Ok(())
}
```

### High-Scale Performance Optimization

For stateless APIs that don't require cookies, optimize memory usage with a shared client:

```rust
use goose::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        // Configure shared client without cookies for optimal performance
        .set_client_builder_without_cookies(
            GooseClientBuilder::new()
                .without_cookies()
                .timeout(Duration::from_secs(10))
        )?
        .register_scenario(
            scenario!("ApiUser")
                .register_transaction(transaction!(api_call))
        )
        .execute()
        .await?;
    Ok(())
}
```

### Session-Based Testing

For applications requiring session management, explicitly configure individual clients:

```rust
use goose::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        // Configure individual clients with cookies for session management
        .set_client_builder_with_cookies(
            GooseClientBuilder::new()
                .timeout(Duration::from_secs(30))
        )
        .register_scenario(
            scenario!("AuthenticatedUser")
                .register_transaction(transaction!(user_login).set_on_start())
                .register_transaction(transaction!(view_dashboard))
        )
        .execute()
        .await?;
    Ok(())
}
```

## Type-Safe Client Builder

Goose provides a type-safe client builder that prevents invalid cookie configurations at compile time. The builder uses phantom types to ensure proper usage:

```rust
use goose::prelude::*;
use std::time::Duration;

// ✅ Start with cookies enabled by default
let builder = GooseClientBuilder::new()
    .timeout(Duration::from_secs(30));

// ✅ Transition to cookies disabled
let builder = builder.without_cookies()
    .timeout(Duration::from_secs(15));

// ✅ Transition back to cookies enabled  
let builder = builder.with_cookies()
    .timeout(Duration::from_secs(25));

// The type system prevents invalid configurations at compile time
```

**Key Benefits:**
- **Compile-time Safety**: Invalid configurations are caught before runtime
- **Clear Intent**: The API makes your cookie choices explicit
- **Performance Optimization**: Shared clients are automatically used when cookies are disabled

## Advanced: Extreme Performance Optimization

For ultra-high-scale testing where even the shared client approach isn't sufficient, you can compile Goose without cookie support entirely:

```bash
# Compile without any cookies dependency
cargo build --no-default-features

# Run tests without cookies
cargo run --no-default-features -- --host https://example.com
```

**Important**: This removes cookie functionality completely - you cannot use `.set_client_builder_with_cookies()` or any cookie-related methods when compiled this way. This is only for specialized use cases where maximum performance and minimal binary size are critical.

**Note**: For most users, the type-safe client builder API (shown above) is the recommended way to configure cookie behavior. You don't need to interact with feature flags directly.
