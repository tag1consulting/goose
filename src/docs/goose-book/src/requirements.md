# Requirements

* In order to write load tests, you must first install [Rust](https://www.rust-lang.org/tools/install).

* Goose load tests are managed with [Cargo](https://doc.rust-lang.org/cargo/), the Rust package manager.

* Goose requires a minimum [`rustc`](https://doc.rust-lang.org/rustc/what-is-rustc.html) version of [`1.49.0`](https://blog.rust-lang.org/2020/12/31/Rust-1.49.0.html) or later. This is because Goose depends on [`flume`](https://docs.rs/flume) for communication between threads, which in turn depends on [`spinning_top`](https://docs.rs/spinning_top) which uses [`hint::spin_loop`](https://doc.rust-lang.org/std/hint/fn.spin_loop.html) which stabilized in `rustc` version `1.49.0`. (See <https://github.com/rust-lang/rust/issues/55002> for more detail.)
