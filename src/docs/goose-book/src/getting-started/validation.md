# Validating Requests

## Error Handling

A transaction reports an error as a `TransactionError`. A transaction function returns a `TransactionResult`, which is `Result<(), Box<TransactionError>>`: the `?` operator and `.into()` box the error for you.

### Transaction Error Types

`TransactionError` has the following variants. Only some of them are counted as transaction errors, as the next sections explain:

- **`TransactionError::Custom`**: user defined errors, such as a failed business logic check
- **`TransactionError::InvalidMethod`**: unsupported HTTP methods
- **`TransactionError::LoggerFailed`**: errors sending log messages to the logger thread; `source` holds the `flume::SendError`, boxed
- **`TransactionError::MetricsFailed`**: errors sending metrics to the parent thread
- **`TransactionError::RequestCanceled`**: requests canceled when throttling is enabled and the test ends
- **`TransactionError::RequestFailed`**: a request marked failed with `GooseUser::set_failure`, or a failed request built with `GooseRequest::builder().error_on_fail()`; `raw_request` holds the request's `GooseRequestMetric`, boxed
- **`TransactionError::Reqwest`**: network and HTTP client errors, such as connection failures and timeouts
- **`TransactionError::Url`**: URL parsing errors when building requests

### Custom Error Handling

The `TransactionError::Custom` variant lets a transaction function return its own error, from a string literal or a `String`.

#### Using String Literals

Return a custom error from a string literal with `.into()`:

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

`response.response` is the server's response, or the Reqwest error Goose got instead, such as a connection failure; its `?` returns that error. `.text().await?` returns an error if the body cannot be read.

#### Using Formatted Strings

For an error message built at run time, use `format!`:

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

Each example starts with its own `use` line so it stands alone; in your `src/main.rs`, keep one `use goose::prelude::*;` at the top. Register each function in `main()` as in [Creating A Load Test](creating.md), for example with `.register_transaction(transaction!(validate_response))`, and run the load test with `--host`.

A transaction that returns any error is counted as failed in the transaction metrics. With `--transaction-log`, the [transaction log](../logging/transactions.md) records it with `"success":false` and, unless the request was canceled, the error in its `error` field.

A `TransactionError::Custom` or `Url` error, or a `Reqwest` error from building the request or from reading or decoding the response body, is counted as a transaction error. With `--error-log`, Goose writes each one to the [error log](../logging/errors.md). The dashboard, the PDF, HTML, Markdown and JSON reports, and the console summary each count them in a transaction errors table, grouped by scenario, transaction and message, most frequent first. For example, take a scenario named `ErrUser` whose third transaction is registered as `transaction!(flaky).set_name("flaky")`. If `flaky` returns `Err("every third call fails".into())` on one call in three, the console summary shows:

```text
 === TRANSACTION ERRORS ===
 ------------------------------------------------------------------------------
 Count       | Transaction: error
 ------------------------------------------------------------------------------
 21,320        1.3 ErrUser flaky: TransactionError: custom error (every third call fails)
 ------------------------------------------------------------------------------
```

When a request fails, Goose counts it once, when it is made: as a request error in the Errors table, and in the error log with `--error-log`. If the transaction then returns `RequestFailed`, or a `Reqwest` error from sending the request, such as a connection failure or too many redirects, Goose does not count it again; only the transaction log names it. A transaction that returns its own error after a failed request is in both tables: `business_logic_check` against a host that answers `401` puts the `POST /login` request in the Errors table and its `Custom` error in the transaction errors table.

A transaction that returns `RequestCanceled`, because the run ended while its request waited on the [throttle](throttle.md), is counted as failed and records no error.

`--no-error-summary` stops counting errors for the Errors and transaction errors tables: the dashboard shows both tables empty, and the reports and the console summary leave them out. The logs still get every error.

## Goose Eggs

[Goose Eggs](https://github.com/tag1consulting/goose-eggs) is a library of helpers for writing Goose load tests. Each goose-eggs release works with one Goose release, and goose-eggs 0.7 is the release for Goose 0.19. Add it to the dependencies in your `Cargo.toml`:

```toml
[dependencies]
goose = "^0.19"
goose-eggs = "0.7"
tokio = "^1"
```

goose-eggs turns on Goose's default features, which in Goose 0.19 is the `cookies` feature. To build without cookies, set `default-features = false` on both `goose` and `goose-eggs`.

To validate a page, bring the `Validate` structure and either the `validate_page` or the `validate_and_load_static_assets` function into scope:

```rust,ignore
use goose_eggs::{validate_and_load_static_assets, Validate};
```

The following transaction requests `/goose/`, checks that the response has a `200` status code and that the text `Gander` is on the page, and, if both checks pass, loads the page's static assets:

```rust,ignore
async fn goose_page(user: &mut GooseUser) -> TransactionResult {
    let goose = user.get("/goose/").await?;

    let validate = Validate::builder()
        .status(200)
        .text("Gander")
        .build();

    validate_and_load_static_assets(user, goose, &validate).await?;

    Ok(())
}
```

After its checks pass, `validate_and_load_static_assets` requests every `src="..."` link and every `href="..."` link to a `.css` file on the page, when the link is a relative path or points at the load test host. The request metrics list them as `GET static asset: js` for a URL containing `.js`, `GET static asset: img` for any other `src` link, and `GET static asset: css` for a stylesheet. In goose-eggs 0.7, the first stylesheet that comes after an `href` link without `.css` in it, such as a favicon link, is not loaded. `validate_page` runs the same checks without loading assets. When every check passes, both return the page's HTML as a `String`.

A page that fails a check is a failed request. It is counted in the request metrics and in the Errors table with the reason, and the transaction returns `RequestFailed`. If `/goose/` answered `200` without the text `Gander`, the console summary would show:

```text
 === ERRORS ===
 ------------------------------------------------------------------------------
 Count       | Error
 ------------------------------------------------------------------------------
 51,013        GET /goose/: http://127.0.0.1:8001/goose/: text not found on page: Gander
 ------------------------------------------------------------------------------
```

While the run goes on, Goose also prints an `INFO` line, `set_failure: ...`, for each failed check; `-q` hides them and keeps the tables.

With `--debug-log`, the [debug log](../logging/debug.md) records the check that failed with the response's headers and body.

### The `Validate` builder

[`Validate::builder()`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.Validate.html#method.builder) builds a `Validate` from any of the following checks, and `build()` finishes it. Text and header checks add up: every one you call must pass. There is one status check, one title check and one redirect check, so a second call to `status` or `not_status`, to `title` or `not_title`, or to `redirect` replaces the first:

```rust,ignore
let validate = Validate::builder()
    .status(200)
    .title("Goose")
    .text("Gander")
    .not_text("Error")
    .header("content-type")
    .header_value("content-type", "text/html")
    .redirect(false)
    .build();
```

Use it in place of the `validate` in `goose_page` above.

| Method | Passes when |
|---|---|
| [`status`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.status), [`not_status`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.not_status) | the response status code is, or is not, the one given |
| [`title`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.title), [`not_title`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.not_title) | the page title contains, or does not contain, the text given, ignoring case |
| [`text`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.text), [`not_text`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.not_text) | the page contains, or does not contain, the text given |
| [`texts`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.texts), [`not_texts`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.not_texts) | the page contains every text in the list, or none of them |
| [`header`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.header), [`not_header`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.not_header) | the response has, or does not have, the header given |
| [`header_value`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.header_value), [`not_header_value`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.not_header_value) | the header's value contains, or does not contain, the text given |
| [`redirect`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.ValidateBuilder.html#method.redirect) | the request was redirected (`true`) or was not (`false`) |

[`Validate::none()`](https://docs.rs/goose-eggs/latest/goose_eggs/struct.Validate.html#method.none) checks nothing.

### New in goose-eggs 0.7

For validation, goose-eggs 0.7 changes the following from 0.6:

- `not_header_value` checks the header's value. In 0.6 it failed whenever the header was set.
- `validate_and_load_static_assets` no longer loads protocol relative asset URLs on other hosts, such as `//cdn.example.com/a.js`.
- `header_is_set()` and `valid_header_value()` take an `http::HeaderMap` in place of `reqwest::header::HeaderMap`. They are the same type, so code that compiled with 0.6 still compiles.
- The `default` feature turns on Goose's default features, which in Goose 0.19 is the `cookies` feature, and no longer picks a TLS backend. The `rustls-tls` feature turns on `goose/rustls-tls` and nothing else.

goose-eggs 0.7 also adds [`wordpress`](https://docs.rs/goose-eggs/latest/goose_eggs/wordpress/index.html) and [`woocommerce`](https://docs.rs/goose-eggs/latest/goose_eggs/woocommerce/index.html) modules, with helpers for logging in, searching, commenting, posting through the REST API, and filling a WooCommerce cart and checking out. Its [CHANGELOG](https://github.com/tag1consulting/goose-eggs/blob/main/CHANGELOG.md) lists every change.

Read [the goose-eggs documentation](https://docs.rs/goose-eggs/latest/goose_eggs) for its other helpers, including the [Drupal](https://docs.rs/goose-eggs/latest/goose_eggs/drupal/index.html) ones.
