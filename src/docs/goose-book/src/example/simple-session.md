# Simple With Session Example 

The [`examples/simple_with_session.rs`](https://github.com/tag1consulting/goose/blob/main/examples/simple_with_session.rs) example demonstrates how you can add JWT authentication support to your load test, making use of the [`GooseUserData`](https://docs.rs/goose/*/goose/goose/trait.GooseUserData.html) marker trait. In this example, the session is recorded in the [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) object with [`set_session_data`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_session_data), and retrieved with [`get_session_data_unchecked`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.get_session_data_unchecked).

_This example will panic if you run it without setting up a proper load test environment that actually sets the expected JWT token._

## Source Code

```rust,ignore
{{#include ../../../../../examples/simple_with_session.rs}}
```
