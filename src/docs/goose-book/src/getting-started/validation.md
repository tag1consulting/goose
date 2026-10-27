# Validating Requests

## Error Handling

Goose transactions can return various types of errors through the `TransactionError` enum. A transaction function returns a `TransactionResult`, which is `Result<(), Box<TransactionError>>`: the `?` operator and `.into()` box the error for you.

### Transaction Error Types

Goose supports several built-in error types that are automatically handled:

- **`TransactionError::Custom`** - User-defined errors for business logic validation
- **`TransactionError::InvalidMethod`** - Unsupported HTTP methods
- **`TransactionError::LoggerFailed`**: Errors sending log messages to the logger thread; `source` holds the `flume::SendError`, boxed
- **`TransactionError::MetricsFailed`** - Errors sending metrics to the parent thread
- **`TransactionError::RequestCanceled`** - Requests canceled when throttling is enabled and the test ends
- **`TransactionError::RequestFailed`**: A request marked failed with `GooseUser::set_failure`, or a failed request built with `GooseRequest::builder().error_on_fail()`; `raw_request` holds the request's `GooseRequestMetric`, boxed
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
    let response_data = response.response?;
    let text = response_data.text().await?;

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
    let response = user.post("/login", "").await?;
    let response_data = response.response?;

    if !response_data.status().is_success() {
        return Err(format!("Login failed with status: {}", response_data.status()).into());
    }

    Ok(())
}
```

A transaction that returns any error is counted as failed in the transaction metrics, and the transaction log records it with `"success":false` and the error's message in its `error` field.

A `TransactionError::Custom` or `Url` error, or a `Reqwest` error from reading or decoding the response body or from building the request, is also a transaction error. Goose writes each one to the [error log](../logging/errors.md), and the console summary, the HTML, Markdown, PDF and JSON reports and the dashboard count them in a Transaction Errors table, by scenario, transaction and message, most frequent first. For example, a transaction that returns `Err("every third call fails".into())` one time in three shows in the console summary as:

```text
 === TRANSACTION ERRORS ===
 ------------------------------------------------------------------------------
 Count       | Transaction: error
 ------------------------------------------------------------------------------
 27,621        1.3 ErrUser flaky: TransactionError: custom error (every third call fails)
 ------------------------------------------------------------------------------
```

A request that failed is already counted as a request error, in the error log and the reports' errors, when it is made. So a transaction that returns `RequestFailed`, or a `Reqwest` error from sending the request, a redirect or an error status, is not counted again: only its transaction log line names the error. A `RequestCanceled` transaction, canceled as the run ended, has no error. `--no-error-summary` leaves the Transaction Errors table out of the console summary and the reports, not out of the logs.

## Goose Eggs
[Goose-eggs](https://github.com/tag1consulting/goose-eggs) are helpful in writing Goose load tests.

To use Goose Eggs in your load test, add the crate to the dependency section of your `Cargo.toml`. Each goose-eggs release works with one Goose release: goose-eggs 0.6, the latest, is for Goose 0.18. Use goose-eggs with Goose 0.19 once a release built for it is on [crates.io](https://crates.io/crates/goose-eggs).

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

Whether or not validation passed or failed will be visible in the Goose metrics when the load test finishes. You can enable the [debug log](../logging/debug.md) to gain more insight into failures.

Read [the goose-eggs documentation](https://docs.rs/goose-eggs/latest/goose_eggs) to learn about other helpful functions useful in writing load tests, as well as other validation helpers, such as [headers](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.header), [header values](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.header_value), [the page title](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.title), and [whether the request was redirected](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.redirect).
