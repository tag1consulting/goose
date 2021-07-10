# Overview

Goose is a Rust load testing tool inspired by [Locust](https://locust.io/). User behavior is defined with standard Rust code. Load tests are applications that have a dependency on the Goose library. Web requests are made with the [Reqwest](https://docs.rs/reqwest) HTTP Client.