# Validating Requests

## Error Handling

Goose transactions can return various types of errors through the `TransactionError` enum. Understanding these error types helps with debugging and writing robust load tests.

### Transaction Error Types

Goose supports several built-in error types that are automatically handled:

- **`TransactionError::Custom`** - User-defined errors for business logic validation
- **`TransactionError::InvalidMethod`** - Unsupported HTTP methods
- **`TransactionError::LoggerFailed`** - Errors sending log messages to the logger thread
- **`TransactionError::MetricsFailed`** - Errors sending metrics to the parent thread
- **`TransactionError::RequestCanceled`** - Requests canceled when throttling is enabled and the test ends
- **`TransactionError::RequestFailed`** - Requests that fail validation or return unexpected status codes
- **`TransactionError::Reqwest`** - Network and HTTP client errors (connection failures, timeouts, etc.)
- **`TransactionError::Url`** - URL parsing errors when building requests

### Custom Error Handling

The `TransactionError::Custom` variant provides flexible error handling, allowing you to easily return custom errors from your transaction functions using string literals or `String` objects.

#### Using String Literals

You can return custom errors directly using string literals:

```rust
use goose::prelude::*;

async fn validate_response(user: &mut GooseUser) -> TransactionResult {
    let response = user.get("/api/data").await?;
    let text = response.text().await?;

    if !text.contains("expected_content") {
        return Err("Missing expected content in response".into());
    }

    Ok(())
}
```

#### Using Formatted Strings

For more dynamic error messages, you can use formatted strings:

```rust
use goose::prelude::*;

async fn business_logic_check(user: &mut GooseUser) -> TransactionResult {
    let response = user.post("/login").await?;

    if !response.status().is_success() {
        return Err(format!("Login failed with status: {}", response.status()).into());
    }

    Ok(())
}
```

Custom errors are tracked in Goose metrics and visible in error logs, making it easy to identify validation failures and other business logic issues in your load tests.

## Goose Eggs
[Goose-eggs](https://github.com/tag1consulting/goose-eggs) are helpful in writing Goose load tests.

To leverage Goose Eggs when writing your load test, include the crate in the dependency section of your `Cargo.toml.

```toml
[dependencies]
goose-eggs = "0.4"
```

For example, to use the Goose Eggs validation functions, bring the `Validate` structure and either the `validate_page` or the `validate_and_load_static_assets` function into scope:
```rust,ignore
use goose_eggs::{validate_and_load_static_assets, Validate};
```

Now, it is simple to verify that we received a `200` HTTP response status code, and that the text `Gander` appeared somewhere on the page as expected:

```rust,ignore
let goose = user.get("/goose/").await?;

let validate = &Validate::builder()
    .status(200)
    .text("Gander")
    .build();

validate_and_load_static_assets(user, goose, &validate).await?;
```

Whether or not validation passed or failed will be visible in the Goose metrics when the load test finishes. You can enable the [debug log](https://book.goose.rs/logging/debug.html) to gain more insight into failures.

Read [the goose-eggs documentation](https://docs.rs/goose-eggs/latest/goose_eggs) to learn about other helpful functions useful in writing load tests, as well as other validation helpers, such as [headers](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.header), [header values](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.header_value), [the page title](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.title), and [whether the request was redirected](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.redirect).
