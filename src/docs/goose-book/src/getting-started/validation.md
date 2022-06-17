# Validating Requests

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
