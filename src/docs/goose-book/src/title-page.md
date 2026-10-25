# The Goose Book

Have you ever been attacked by a goose?

## What Is Goose?

[Goose](https://docs.rs/goose) is a [Rust](https://www.rust-lang.org/) load testing tool inspired by [Locust](https://locust.io/). User behavior is defined with standard Rust code. Load tests are applications that have a dependency on the [Goose library](https://crates.io/crates/goose). Web requests are made with the [Reqwest](https://docs.rs/reqwest) HTTP Client. Load tests of other protocols, such as [raw TCP](./example/tcp.html), time each operation themselves and record it in Goose's metrics.

![Request statistics report](what_is_goose_graph.png)

## Advantages

Goose generates at least 11x as much traffic as Locust per-CPU-core, with even larger gains for more complex load tests (such as those using third-party libraries to scrape form content). While Locust requires you to manage a distributed load test simply to use multiple CPU cores on a single server, Goose leverages all available CPU cores with a single process, drastically simplifying the process for running larger load tests. Ongoing improvements to the codebase continue to bring new features and faster performance. Goose scales far better than Locust, efficiently using available resources to accomplish its goal. It also supports asynchronous processes enabling many more simultaneous processes to ramp up thousands of users from a single server, easily and consistently.

Goose 0.19 cannot run a distributed load test: [Gaggle](./gaggle/overview.html), the Manager and Workers that ran one load test across many servers, was removed in Goose 0.17.0 and is not part of this release. A single Goose process uses every CPU core of the server it runs on.

Goose has a number of unique [debugging and logging mechanisms](./logging/overview.html) not found in other load testing tools, simplifying the writing of load tests and the analysis of results. Goose also provides more [comprehensive metrics](./getting-started/metrics.html) with multiple simple views into the data, and makes it easy to confirm that the load test is doing what you expect it to as you scale it up or down. It exposes the algorithms used to allocate scenarios and contained transactions, giving [more granular control](./config/scheduler.html) over the order and consistency of operations, important for easily repeatable testing.

## Watching A Load Test

Goose writes an [HTML report](./getting-started/common.html#writing-an-html-formatted-report) at the end of a load test, and, built with the `dashboard` feature, serves a [live web dashboard](./controller/dashboard.html) that shows a load test's metrics while it runs and, with `--dashboard-control`, can start and stop it and change its number of users.

## Brought To You By

Goose development is sponsored by [Tag1 Consulting](https://tag1.com/), led by Tag1's CEO, [Jeremy Andrews](https://github.com/jeremyandrews), along with many [community contributions](https://github.com/tag1consulting/goose/graphs/contributors). Tag1 is a [member of the Rust Foundation](https://www.tag1consulting.com/blog/tag1-joins-rust-foundation-first-silver-member).

## Additional Documentation

- [Developer documentation](https://docs.rs/goose/)
- [Blogs and more](https://tag1.com/goose/)
  - [Goose vs Locust and jMeter](https://www.tag1consulting.com/blog/jmeter-vs-locust-vs-goose)
  - [Real-life load testing with Goose](https://www.tag1consulting.com/blog/real-life-goose-load-testing)
  - [Gaggle: a distributed load test](https://www.tag1consulting.com/blog/show-me-how-flock-flies-working-gaggle-goose), written for an earlier Goose; Goose 0.19 cannot run a Gaggle
  - [Optimizing Goose performance](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure)
