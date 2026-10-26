# Session Example 

The [`examples/session.rs`](https://github.com/tag1consulting/goose/blob/main/examples/session.rs) example demonstrates how you can add JWT authentication support to your load test, making use of the [`GooseUserData`](https://docs.rs/goose/*/goose/goose/trait.GooseUserData.html) trait. In this example, the session is recorded in the [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) object with [`set_session_data`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_session_data), and retrieved with [`get_session_data_unchecked`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.get_session_data_unchecked).

## Details

In this example, the [`GooseUserData`](https://docs.rs/goose/*/goose/goose/trait.GooseUserData.html) is a simple struct containing a string. Goose implements `GooseUserData` for every type that is `Clone`, `Send`, `Sync` and `'static`, so the struct only needs to derive `Clone`. Passing `set_session_data` a type that is not `Clone` does not compile:
```rust
{{#include ../../../../../examples/session.rs:24:27}}
```

The session data structure is created from json-formatted response data returned by an authentication request, uniquely stored in each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) instance:
```rust,ignore
{{#include ../../../../../examples/session.rs:67:69}}
```

The session data is retrieved from the [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) object with each subsequent request. To keep the example simple no validation is done:
```rust,ignore
{{#include ../../../../../examples/session.rs:75:95}}
```

_This example panics unless the server sets the JWT token it expects: if `/signup` does not return the JSON the example expects, the user has no session data, and `get_session_data_unchecked` panics with `Missing session data!`._

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/session.rs}}
```
