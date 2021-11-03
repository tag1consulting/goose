# Session Example 

The [`examples/session.rs`](https://github.com/tag1consulting/goose/blob/main/examples/session.rs) example demonstrates how you can add JWT authentication support to your load test, making use of the [`GooseUserData`](https://docs.rs/goose/*/goose/goose/trait.GooseUserData.html) marker trait. In this example, the session is recorded in the [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) object with [`set_session_data`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_session_data), and retrieved with [`get_session_data_unchecked`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.get_session_data_unchecked).

## Details

In this example, the [`GooseUserData`](https://docs.rs/goose/*/goose/goose/trait.GooseUserData.html) is a simple struct containing a string:
```rust
{{#include ../../../../../examples/session.rs:24:26}}
```

The session data structure is created from json-formatted response data returned by an authentication request, uniquely stored in each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) instance:
```rust,ignore
{{#include ../../../../../examples/session.rs:66:68}}
```

The session data is retrieved from the [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) object with each subsequent request. To keep the example simple no validation is done:
```rust,ignore
{{#include ../../../../../examples/session.rs:75:83}}
```

_This example will panic if you run it without setting up a proper load test environment that actually sets the expected JWT token._

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/session.rs}}
```
