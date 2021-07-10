# Overview

Goose is a Rust load testing tool inspired by [Locust](https://locust.io/). User behavior is defined with standard Rust code. Load tests are applications that have a dependency on the Goose library. Web requests are made with the [Reqwest](https://docs.rs/reqwest) HTTP Client.

## Requirements

Minimum required `rustc` version is `1.49.0`: `goose` depends on [`flume`](https://docs.rs/flume) for communication between threads, which in turn depends on [`spinning_top`](https://docs.rs/spinning_top) which uses `hint::spin_loop` which stabilized in `rustc` version `1.49.0`. More detail in https://github.com/rust-lang/rust/issues/55002.