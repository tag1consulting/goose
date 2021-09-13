# Simple Closure Example 

The [`examples/simple_closure.rs`](https://github.com/tag1consulting/goose/blob/main/examples/simple_closure.rs) example loads three different pages on a web site. Instead of defining a hard coded [`GooseTask`](https://docs.rs/goose/*/goose/goose/struct.GooseTask.html) function for each, the paths are passed in via a [vector](https://doc.rust-lang.org/std/vec/index.html) and the [GooseTaskFunction](https://docs.rs/goose/*/goose/goose/type.GooseTaskFunction.html) is dynamically created in a [closure](https://doc.rust-lang.org/rust-by-example/fn/closures.html).

## Source Code

```rust,ignore
{{#include ../../../../../examples/simple_closure.rs}}
```
