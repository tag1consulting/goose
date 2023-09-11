# Goose

Have you ever been attacked by a goose?

[![crates.io](https://img.shields.io/crates/v/goose.svg)](https://crates.io/crates/goose)
[![Documentation](https://docs.rs/goose/badge.svg)](https://docs.rs/goose)
[![Apache-2.0 licensed](https://img.shields.io/crates/l/goose.svg)](./LICENSE)
[![CI](https://github.com/tag1consulting/goose/workflows/CI/badge.svg)](https://github.com/tag1consulting/goose/actions?query=workflow%3ACI)

## A Powerful Load Testing Framework

Goose is a highly efficient load testing tool crafted in [Rust](https://www.rust-lang.org/), designed to simulate users interacting with web applications. Whether you're testing a simple website or an intricate application, Goose provides a flexible and scalable solution to ensure your system can handle real-world traffic patterns.

## Why Choose Goose?

* Performance: Built with Rust, Goose is designed for speed and scalability, allowing you to simulate a large number of users with minimal resource overhead.

* Flexibility: Goose supports both simple and complex load tests, making it suitable for a wide range of applications. With its extensive set of options, you can tailor your tests to closely mimic real-world user behavior.

* Real-world Testing: Goose goes beyond just sending requests; it can simulate user behaviors like logging in, filling out forms, and navigating through your application, providing a more realistic load test scenario.

* Community and Support: Developed by [Tag1 Consulting](https://tag1.com/), Goose has a growing community and a series of [blog posts and podcasts](https://www.tag1.com/goose/) detailing its features, comparisons with other tools, and real-life testing scenarios.

## Getting Started

It is essential to understand that Goose is not a pre-compiled application but a library. This means you can't simply run Goose to load test a website. Instead, you'll need to write your own Rust application using the Goose library, then compile it to create a tailored load testing tool specific to your needs. Dive into [The Goose Book](https://book.goose.rs/) for a comprehensive guide or check the [developer documentation](https://docs.rs/goose/) for detailed API information.
