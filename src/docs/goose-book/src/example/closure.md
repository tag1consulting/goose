# Closure Example 

The [`examples/closure.rs`](https://github.com/tag1consulting/goose/blob/main/examples/closure.rs) example loads three different pages on a web site. Instead of defining a hard coded [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) function for each, the paths are passed in via a [vector](https://doc.rust-lang.org/std/vec/index.html) and the [TransactionFunction](https://docs.rs/goose/*/goose/goose/type.TransactionFunction.html) is dynamically created in a [closure](https://doc.rust-lang.org/rust-by-example/fn/closures.html).

## Details

The paths to be loaded are first defiend in a vector:
```rust
{{#include ../../../../../examples/closure.rs:30}}
```

A transaction function for each path is then dynamically created as a closure:
```rust,ignore
{{#include ../../../../../examples/closure.rs:31:40}}
```

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/closure.rs}}
```
