# Validation

## Goose Eggs
[Goose-eggs](https://github.com/tag1consulting/goose-eggs) is a validation library that can be used with Goose. 

Include the goose-eggs crate in the dependency section of your `Cargo.toml`. 

```toml
[dependencies]
goose-eggs = "0.4.0"
```

Bring the Validate structure and the validation function that the object is passed to, into the scope of your load test.
```rust 
use goose_eggs::{validate_and_load_static_assets, Validate};
```

We'd probably want to verify the HTTP response status of a request and whether some text appears as we'd expect.

```rust
let goose = user.get("/goose/").await?;

let validate = &Validate::builder()
    .status(200)
    .text("Gander")
    .build();

validate_and_load_static_assets(user, goose, &validate).await?;
```

Both passes and fails of each validation will be presented by the given metrics. 

The full `goose-eggs` documentation is located [here](https://docs.rs/goose-eggs/latest/goose_eggs/).
