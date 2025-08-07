# Cookies

By default, Goose enables cookies for HTTP clients, with each user getting their own cookie jar. However, if you are running a load test that doesn't use cookies, you can optimize performance by using a shared client without cookies. This provides significant memory savings for high user counts (see https://github.com/tag1consulting/goose/pull/557 for more information).

## When to Use Each Approach

| Configuration | Memory Usage | Cookie Support | Best For |
|---------------|-------------|----------------|----------|
| **Individual Clients with Cookies** | Higher (one client per user) | Full cookie jar per user | Session-based apps, authentication flows, login flows, shopping carts |
| **Shared Client without Cookies** | Lower (single shared client) | None | Stateless APIs, high-scale load tests (1000+ users) |

## Complete Examples

### Default Behavior (Individual Clients with Cookies)

Your existing code continues to work unchanged. Goose automatically provides individual clients with cookies enabled:

```rust
use goose::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("WebsiteUser")
                .set_wait_time(Duration::from_secs(1), Duration::from_secs(3))?
                .register_transaction(transaction!(load_homepage))
                .register_transaction(transaction!(load_about_page))
        )
        .execute()
        .await?;

    Ok(())
}

async fn load_homepage(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

async fn load_about_page(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/about").await?;
    Ok(())
}
```

### High-Scale Performance (1000+ Users, Stateless APIs)

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
                .user_agent("high-scale-loadtest/1.0")
        )?
        .register_scenario(
            scenario!("ApiUser")
                .set_wait_time(Duration::from_millis(100), Duration::from_millis(500))?
                .register_transaction(transaction!(api_get_users))
                .register_transaction(transaction!(api_get_products))
                .register_transaction(transaction!(api_health_check))
        )
        .execute()
        .await?;

    Ok(())
}

async fn api_get_users(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/api/users").await?;
    Ok(())
}

async fn api_get_products(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/api/products").await?;
    Ok(())
}

async fn api_health_check(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/api/health").await?;
    Ok(())
}
```

### Session-Based Testing (Login/Authentication Flows)

For applications requiring session management, use individual clients with cookies and longer timeouts:

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
                .user_agent("session-loadtest/1.0")
        )
        .register_scenario(
            scenario!("AuthenticatedUser")
                .set_wait_time(Duration::from_secs(2), Duration::from_secs(5))?
                // Login runs once per user at the start
                .register_transaction(transaction!(user_login).set_on_start())
                // These run repeatedly during the test
                .register_transaction(transaction!(view_dashboard))
                .register_transaction(transaction!(update_profile))
                .register_transaction(transaction!(view_settings))
        )
        .execute()
        .await?;

    Ok(())
}

async fn user_login(user: &mut GooseUser) -> TransactionResult {
    // Login form submission - cookies will be automatically stored
    let params = [
        ("username", "testuser"),
        ("password", "testpass"),
        ("csrf_token", "abc123")
    ];
    let _goose = user.post_form("/login", &params).await?;
    Ok(())
}

async fn view_dashboard(user: &mut GooseUser) -> TransactionResult {
    // Session cookies automatically sent with this request
    let _goose = user.get("/dashboard").await?;
    Ok(())
}

async fn update_profile(user: &mut GooseUser) -> TransactionResult {
    // Session cookies ensure this request is authenticated
    let params = [("name", "Updated Name"), ("email", "new@example.com")];
    let _goose = user.post_form("/profile/update", &params).await?;
    Ok(())
}

async fn view_settings(user: &mut GooseUser) -> TransactionResult {
    // Session cookies automatically included
    let _goose = user.get("/settings").await?;
    Ok(())
}
```

## Type-Safe Client Configuration

Goose provides a type-safe client builder that prevents invalid cookie configurations at compile time. The builder uses phantom types to ensure you can only call cookie-related methods when cookies are enabled:

```rust
// ✅ This compiles - cookie methods available on default state
let builder = GooseClientBuilder::new()
    .cookie_store(true)
    .timeout(Duration::from_secs(30));

// ✅ This compiles - transition to cookies disabled
let builder = builder.without_cookies()
    .timeout(Duration::from_secs(15));

// ❌ This would NOT compile - cookie methods not available after without_cookies()
// let builder = builder.cookie_store(true); // Compile error!

// ✅ This compiles - transition back to cookies enabled
let builder = builder.with_cookies()
    .cookie_store(true); // Now available again
```

## Performance Impact

### Memory Usage Comparison

- **Individual Clients**: ~1MB per user (each user has their own HTTP client and cookie jar)
- **Shared Client**: ~1MB total (single HTTP client shared across all users)

For 1000 users:
- Individual clients: ~1GB memory usage
- Shared client: ~1MB memory usage

### When to Choose Each Approach

**Use Individual Clients (Default) When:**
- Your application requires session management
- Users need to maintain login state
- You're testing shopping carts, user preferences, or personalized content
- User count is moderate (< 1000 users)

**Use Shared Client (Optimized) When:**
- Testing stateless APIs or microservices
- Running high-scale tests (1000+ users)
- Your application doesn't use cookies
- Memory usage is a constraint
- Testing public endpoints that don't require authentication

The type-safe client builder approach provides compile-time safety and prevents invalid configurations, making it the preferred method for configuring cookie behavior.
