# Proxy Configuration

Goose supports routing traffic through HTTP, HTTPS, and SOCKS proxies by leveraging reqwest's proxy capabilities. This is useful for testing applications through corporate proxies, debugging with tools like Burp Suite or OWASP ZAP, or routing traffic through specific network paths.

## HTTP/HTTPS Proxies (No Setup Required)

HTTP and HTTPS proxies work out of the box with Goose - **no compile-time changes needed**. Simply set environment variables:

**Important**: `HTTP_PROXY` controls proxying for HTTP requests (port 80), `HTTPS_PROXY` controls proxying for HTTPS requests (port 443). The proxy server itself typically runs on HTTP regardless of which requests it proxies.

```bash
# Set both to handle HTTP and HTTPS traffic
export HTTPS_PROXY=http://proxy.example.com:8080
export HTTP_PROXY=http://proxy.example.com:8080

# With authentication
export HTTPS_PROXY=http://username:password@proxy.example.com:8080
export HTTP_PROXY=http://username:password@proxy.example.com:8080

# Run your load test (no Cargo.toml changes needed)
cargo run --example simple -- --host https://example.com
```

## SOCKS Proxy Support (Requires Setup)

To use SOCKS proxies with Goose, you need to enable SOCKS support in your project's reqwest dependency.

### Dependencies Setup

Add reqwest with the `socks` feature to your `Cargo.toml`:

```toml
[dependencies]
goose = "^0.19"
tokio = "^1"
reqwest = { version = "0.13", features = ["socks"] }
```

**Note**: Cargo automatically unifies features across all dependencies. Since Goose already requires `gzip` and `json` features from reqwest, adding just `socks` will result in reqwest being compiled with `["gzip", "json", "socks"]`.

### Environment Variables

Configure SOCKS proxy using environment variables:

```bash
# Set both to handle HTTP and HTTPS traffic
export HTTPS_PROXY=socks5h://127.0.0.1:1080
export HTTP_PROXY=socks5h://127.0.0.1:1080

# Run your load test
cargo run --example simple -- --host https://example.com
```

### SOCKS5 vs SOCKS5h

- **`socks5://`** - DNS resolution happens locally, only network traffic goes through proxy
- **`socks5h://`** - Both DNS resolution and network traffic go through the proxy (recommended)

Use `socks5h://` when you need DNS queries to be resolved through the SOCKS proxy, which is typically the case for accessing internal networks or when you want all traffic routed through the proxy.

## Common Use Cases

### Security Testing with Burp Suite
```bash
export HTTPS_PROXY=http://127.0.0.1:8080
export HTTP_PROXY=http://127.0.0.1:8080
cargo run --example simple -- --host https://target.example.com
```

### SSH SOCKS Tunnel
```bash
# Set up SSH tunnel with SOCKS proxy
ssh -D 1080 -N user@jump-server.example.com

# In another terminal, run load test through tunnel
export HTTPS_PROXY=socks5h://127.0.0.1:1080
export HTTP_PROXY=socks5h://127.0.0.1:1080
cargo run --example simple -- --host https://internal-app.example.com
```

### Corporate Proxy
```bash
# Set both to proxy all traffic through corporate proxy
export HTTPS_PROXY=http://corporate-proxy.example.com:8080
export HTTP_PROXY=http://corporate-proxy.example.com:8080
cargo run --example simple -- --host https://external-site.com
```

## Example Load Test

### HTTP/HTTPS Proxy Example (Simple)

**Cargo.toml (no special dependencies needed):**
```toml
[package]
name = "my-loadtest"
version = "0.1.0"
edition = "2021"

[dependencies]
goose = "^0.19"
tokio = { version = "^1", features = ["macros", "rt-multi-thread"] }
# No special reqwest configuration needed for HTTP/HTTPS proxies
```

**src/main.rs:**
```rust,ignore
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadTest")
            .register_transaction(transaction!(homepage))
        )
        .set_default(GooseDefault::Host, "https://example.com")?
        .execute()
        .await?;

    Ok(())
}

async fn homepage(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("/").await?;
    Ok(())
}
```

**Running with HTTP proxy:**
```bash
export HTTPS_PROXY=http://proxy.example.com:8080
export HTTP_PROXY=http://proxy.example.com:8080
cargo run
```

### SOCKS Proxy Example (Requires Setup)

**Cargo.toml (with socks feature):**
```toml
[package]
name = "my-loadtest"
version = "0.1.0"
edition = "2021"

[dependencies]
goose = "^0.19"
tokio = { version = "^1", features = ["macros", "rt-multi-thread"] }
# Add socks feature - Cargo will unify with Goose's gzip+json features
reqwest = { version = "0.13", features = ["socks"] }
```

**src/main.rs (same as above):**
```rust,ignore
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadTest")
            .register_transaction(transaction!(homepage))
        )
        .set_default(GooseDefault::Host, "https://example.com")?
        .execute()
        .await?;

    Ok(())
}

async fn homepage(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("/").await?;
    Ok(())
}
```

**Running with SOCKS proxy:**
```bash
export HTTPS_PROXY=socks5h://127.0.0.1:1080
export HTTP_PROXY=socks5h://127.0.0.1:1080
cargo run
```

## Troubleshooting

### DNS Resolution Issues
If you see errors like "failed to lookup address information: No address associated with hostname", try:
- Use `socks5h://` instead of `socks5://` to route DNS through the proxy
- Verify your SOCKS proxy is running and accessible
- Test with curl: `curl --proxy socks5h://127.0.0.1:1080 https://example.com`

### Authentication Issues
For proxies requiring authentication:
- Include credentials in the proxy URL: `socks5h://username:password@proxy:1080`
- Ensure special characters in credentials are URL-encoded

### Feature Not Available
If you get compilation errors about SOCKS features:
- Verify `reqwest = { version = "0.13", features = ["socks"] }` is in your Cargo.toml
- Run `cargo clean` and rebuild your project

## Additional Resources

For more detailed proxy configuration options, see the official reqwest documentation:
- **[Reqwest Proxy Documentation](https://docs.rs/reqwest/latest/reqwest/#proxies)** - Complete guide to proxy configuration in reqwest
- **Environment Variables**: `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` (and lowercase variants)
- **Programmatic Configuration**: Using `reqwest::Proxy` for advanced proxy setups

## Notes

- All GooseUsers in your load test will route through the configured proxy
- The proxy configuration applies to the entire load test
- Both HTTP and HTTPS requests will use the proxy when configured
- Environment variables (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`) are **standard system proxy variables automatically recognized by reqwest**, the HTTP client library that Goose uses internally
- **Goose does not define or manage these environment variables** - they are set by you in your shell environment and consumed by reqwest
- **HTTP/HTTPS proxy support is enabled by default** in reqwest via the `system-proxy` feature _(enabled by default)_, so Goose automatically inherits this functionality
- **SOCKS proxy support requires** the `socks` feature to be explicitly enabled in your project's reqwest dependency
