# Basic Authentication Example

The [`basic_auth.rs`](https://github.com/tag1consulting/goose/blob/main/examples/basic_auth.rs) example demonstrates different approaches to handling HTTP Basic Authentication in Goose load tests. This addresses the common question of how to test sites behind Basic Auth, particularly when static assets (CSS, JS, images) also require authentication.

## Problem Statement

When load testing a site protected by HTTP Basic Authentication, there are several challenges:

1. **Per-request authentication**: Adding Basic Auth to individual requests works for that specific request, but doesn't propagate to static asset requests
2. **Static asset failures**: If a page includes CSS, JavaScript, or images, these assets will fail to load if they also require Basic Auth
3. **Code duplication**: Manually adding authentication to every request leads to repetitive code

## Solutions Demonstrated

The example shows three different approaches to handle Basic Auth:

### 1. Custom Client with Default Headers (Recommended)

This approach sets up the HTTP client once per user with default Basic Auth headers. All subsequent requests automatically include the authentication headers, including requests for static assets.

```rust
async fn setup_basic_auth_client(user: &mut GooseUser) -> TransactionResult {
    let (username, password) = get_basic_auth_credentials()?;
    
    // Create a temporary client to generate the Basic Auth header
    let temp_client = reqwest::Client::new();
    let temp_request = temp_client
        .get("http://example.com")
        .basic_auth(&username, Some(&password))
        .build()
        .map_err(|e| TransactionError::Reqwest(e))?;
    
    // Extract the Authorization header
    let mut headers = header::HeaderMap::new();
    if let Some(auth_header) = temp_request.headers().get(header::AUTHORIZATION) {
        headers.insert(header::AUTHORIZATION, auth_header.clone());
    }
    
    // Set up the client builder with default headers
    let builder = Client::builder()
        .user_agent("Goose/1.0 BasicAuth Example")
        .default_headers(headers)
        .cookie_store(true)
        .timeout(Duration::from_secs(30));
    
    // Apply the custom client to this user
    user.set_client_builder(builder).await?;
    
    Ok(())
}
```

**Advantages:**
- Set up once per user
- Automatically applies to all requests, including static assets
- Clean and maintainable code
- Works with `validate_and_load_static_assets`

### 2. Helper Function Approach

This approach uses a helper function that adds Basic Auth to each request. It's more flexible than the custom client approach but requires calling the helper for every request.

```rust
async fn send_authenticated_request(
    user: &mut GooseUser,
    method: &GooseMethod,
    path: &str,
) -> Result<GooseResponse, Box<TransactionError>> {
    let mut reqwest_request_builder = user.get_request_builder(method, path)?;

    if let Ok((username, password)) = get_basic_auth_credentials() {
        reqwest_request_builder = reqwest_request_builder.basic_auth(username, Some(password));
    }

    let goose_request = GooseRequest::builder()
        .set_request_builder(reqwest_request_builder)
        .build();

    let goose_response = user.request(goose_request).await?;
    Ok(goose_response)
}
```

**Advantages:**
- More control over individual requests
- Can conditionally apply authentication
- Easier to debug individual requests

**Disadvantages:**
- Must be called for every request
- Doesn't automatically handle static assets
- More verbose code

### 3. Manual Per-Request Authentication

This approach manually adds Basic Auth to each individual request using the request builder.

```rust
async fn test_manual_basic_auth(user: &mut GooseUser) -> TransactionResult {
    let (username, password) = get_basic_auth_credentials()?;
    
    let reqwest_request_builder = user
        .get_request_builder(&GooseMethod::Get, "/basic-auth/user/passwd")?
        .basic_auth(username, Some(password));

    let goose_request = GooseRequest::builder()
        .set_request_builder(reqwest_request_builder)
        .build();

    let _goose_metrics = user.request(goose_request).await?;
    
    Ok(())
}
```

**Disadvantages:**
- Most verbose approach
- Does NOT propagate to static asset requests
- Lots of code duplication
- This is the original problem described in [issue #608](https://github.com/tag1consulting/goose/issues/608)

## Configuration

The example supports multiple ways to provide Basic Auth credentials:

### Environment Variables

**Separate variables:**
```bash
export BASIC_AUTH_USERNAME="your_username"
export BASIC_AUTH_PASSWORD="your_password"
```

**Combined format:**
```bash
export BASIC_AUTH="username:password"
```

### Default Credentials

If no environment variables are set, the example uses default credentials (`user:passwd`) suitable for testing with httpbin.org.

## Running the Example

```bash
# Using httpbin.org for testing (uses default credentials)
cargo run --example basic_auth -- --host http://httpbin.org --users 5 --run-time 30s

# With custom credentials
BASIC_AUTH_USERNAME="myuser" BASIC_AUTH_PASSWORD="mypass" \
cargo run --example basic_auth -- --host https://your-protected-site.com --users 10 --run-time 60s

# Using combined format
BASIC_AUTH="myuser:mypass" \
cargo run --example basic_auth -- --host https://your-protected-site.com --users 10 --run-time 60s
```

## Key Takeaways

1. **Use the custom client approach** for most Basic Auth scenarios - it's the most robust solution
2. **Set up authentication once per user** rather than per request for better performance and cleaner code
3. **Consider static assets** - only the custom client approach properly handles authentication for CSS, JS, and image requests
4. **Use environment variables** for credentials to keep them out of your code
5. **Test your approach** with a simple example like httpbin.org before applying to your actual load test

## Related Issues

This example addresses [GitHub issue #608](https://github.com/tag1consulting/goose/issues/608): "How can I test a site behind Basic Auth".
