# Basic Authentication Example

The [`basic_auth.rs`](https://github.com/tag1consulting/goose/blob/main/examples/basic_auth.rs) example demonstrates different approaches to handling HTTP Basic Authentication in Goose load tests. This addresses the common question of how to test sites behind Basic Auth, particularly when static assets (CSS, JS, images) also require authentication.

## Problem Statement

When load testing a site protected by HTTP Basic Authentication, there are several challenges:

1. **Per-request authentication**: Adding Basic Auth to individual requests works for that specific request, but doesn't propagate to static asset requests
2. **Static asset failures**: If a page includes CSS, JavaScript, or images, these assets will fail to load if they also require Basic Auth
3. **Code duplication**: Manually adding authentication to every request leads to repetitive code

## Solutions Demonstrated

The example shows three different approaches to handle Basic Auth, one per scenario:

| Scenario | Weight | Transactions | Requests |
|---|---|---|---|
| `CustomClientAuth` | 3 | `setup_basic_auth_client` (on start), `test_basic_auth_endpoint`, `test_user_agent_endpoint`, `test_json_endpoint` | `GET /basic-auth/user/passwd`, `GET /user-agent`, `GET /json` |
| `HelperFunctionAuth` | 2 | `test_with_helper_function`, `test_post_with_helper` | `GET /basic-auth/user/passwd`, `POST /post` |
| `ManualPerRequestAuth` | 1 | `test_manual_basic_auth`, `test_manual_with_validation` | `GET /basic-auth/user/passwd`, twice |

Every scenario waits 1 to 3 seconds after each transaction. The paths are those of [httpbin.org](https://httpbin.org/), where `/basic-auth/user/passwd` answers 200 only to the user `user` with the password `passwd`.

The excerpts below are taken from the example, where they sit inside a module compiled only with the `cookies` feature. That feature is on by default; built without it, the example prints an error and exits.

### 1. Custom Client with Default Headers (Recommended)

This approach sets up the HTTP client once per user with default Basic Auth headers. All subsequent requests automatically include the authentication headers, including requests for static assets.

```rust,ignore
async fn setup_basic_auth_client(user: &mut GooseUser) -> TransactionResult {
    // Get credentials from environment variables
    let (username, password) = get_basic_auth_credentials()?;

    // Use reqwest's built-in basic_auth() to generate a properly encoded
    // Authorization header, then extract it for use as a default header.
    let temp_request = reqwest::Client::new()
        .get("http://localhost")
        .basic_auth(&username, Some(&password))
        .build()
        .map_err(TransactionError::Reqwest)?;

    let mut headers = header::HeaderMap::new();
    if let Some(auth_header) = temp_request.headers().get(header::AUTHORIZATION) {
        headers.insert(header::AUTHORIZATION, auth_header.clone());
    }

    // Set up the client builder with default headers and other common settings
    // Enable cookie store for maintaining session state with Basic Auth
    let builder = Client::builder()
        .user_agent("Goose/1.0 BasicAuth Example")
        .default_headers(headers)
        .timeout(Duration::from_secs(30))
        .cookie_store(true);

    // Apply the custom client to this user
    user.set_client_builder(builder).await?;

    Ok(())
}
```

**Advantages:**
- Set up once per user
- Automatically applies to all requests, including static assets
- Clean and maintainable code
- Works with helpers that load a page's static assets, such as `validate_and_load_static_assets` in the [Umami example](umami.md)

### 2. Helper Function Approach

This approach uses a helper function that adds Basic Auth to each request. It's more flexible than the custom client approach but requires calling the helper for every request.

```rust,ignore
async fn send_authenticated_request(
    user: &mut GooseUser,
    method: &GooseMethod,
    path: &str,
) -> Result<GooseResponse, Box<TransactionError>> {
    let mut reqwest_request_builder = user.get_request_builder(method, path)?;

    // Check for Basic Auth credentials in environment
    if let Ok((username, password)) = get_basic_auth_credentials() {
        reqwest_request_builder = reqwest_request_builder.basic_auth(username, Some(password));
    }

    // Build the GooseRequest and make the request
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

The request is built from a Reqwest `RequestBuilder` without calling `.path()` or `.name()` on the `GooseRequest` builder, so Goose names it by its method alone: the metrics show these requests as `GET` and `POST` rather than `GET /basic-auth/user/passwd` and `POST /post`. Add `.name("...")` to the `GooseRequest` builder to give them a name.

### 3. Manual Per-Request Authentication

This approach manually adds Basic Auth to each individual request using the request builder.

```rust,ignore
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
- Named by its method alone in the metrics, as in the helper function approach

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

`BASIC_AUTH_USERNAME` and `BASIC_AUTH_PASSWORD` are used when both are set; otherwise `BASIC_AUTH` is used, split at its first colon, so the password may contain colons. A `BASIC_AUTH` without a colon is an error: the transactions of the custom client and manual approaches fail, and the helper function sends its requests without credentials.

### Default Credentials

If none of these environment variables are set, the example logs a warning and uses default credentials (`user:passwd`) suitable for testing with httpbin.org.

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

Against another site the example still requests the httpbin.org paths in the table above, so change them to paths on your site first.

## Key Takeaways

1. **Use the custom client approach** for most Basic Auth scenarios: it sends credentials with every request the user makes
2. **Set up authentication once per user** rather than per request for better performance and cleaner code
3. **Consider static assets**: only the custom client approach sends credentials with CSS, JS, and image requests
4. **Use environment variables** for credentials to keep them out of your code
5. **Test your approach** with a simple example like httpbin.org before applying to your actual load test

## Related Issues

This example addresses [GitHub issue #608](https://github.com/tag1consulting/goose/issues/608): "How can I test a site behind Basic Auth".
