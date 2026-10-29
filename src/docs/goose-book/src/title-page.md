# The Goose Book

Have you ever been attacked by a goose?

## What Is Goose?

[Goose](https://docs.rs/goose) is a load testing tool written in [Rust](https://www.rust-lang.org/) and inspired by [Locust](https://locust.io/). A load test is a small Rust program that depends on the [`goose` crate](https://crates.io/crates/goose): you build it with Cargo and run it against a host, choosing the users and the run time on the command line.

Where Locust has a user class with tasks, Goose has a [scenario](./glossary.md#scenario) with [transactions](./glossary.md#transaction). Requests go through the [Reqwest](https://docs.rs/reqwest) HTTP client. A load test of another protocol, such as [raw TCP](./example/tcp.md), times each operation itself and records it in Goose's metrics.

## What It Looks Like

![The Goose live dashboard about 150 seconds into a load test of 50 users against the Drupal Umami demo site: the KPI row with 50 of 50 users, requests per second, no failures, and the p95 and average response times; the requests per second chart, which climbs during the ramp up and then levels off; the active users chart, which ramps to 50 in 50 seconds and holds; and the response time heatmap with its p50, p95 and p99 lines](title-dashboard.png)

*The live dashboard of a 50 user run whose users start one a second: the requests per second climb with them and level off, the tiles other than Users cover the whole run, and each column of the heatmap is one second of response times.*

That run came from the [Umami example](./example/umami.md) in a checkout of Goose, against a copy of the Drupal Umami demo site. `--no-reset-metrics` keeps the ramp up in its numbers; without it, Goose clears the metrics when the ramp up ends:

```bash
cargo run --release --features dashboard --example umami -- \
  --dashboard -H http://umami.example.com -u 50 -r 1 -t 3m --no-reset-metrics
```

## Why Use Goose

- **One process uses every CPU core.** A Goose load test runs its users as async tasks on Tokio's [multi-threaded runtime](https://docs.rs/tokio/latest/tokio/attr.main.html), which spreads them across a server's cores. Locust needs [a process per core](https://docs.locust.io/en/stable/running-distributed.html) to do the same.
- **Measured throughput.** In Tag1's [A Goose In The Clouds](https://www.tag1.com/blog/goose-clouds-load-testing-scale/) (2021, an earlier Goose release), one 8 core server simulated 2,000 users making over 6,500 requests per second and sustained 2.5 Gbps; one 32 core server made 22,900 requests per second, limited by its 9 Gbps uplink.
- **Load tests are Rust code.** A load test can use any crate and builds to one binary, to run in CI or a container. See [Creating A Load Test](./getting-started/creating.md).
- **Results a pipeline can act on.** [Thresholds](./getting-started/thresholds.md) fail a run on its metrics, and with a [baseline](./getting-started/common.md#comparing-against-a-baseline) the PDF, HTML and Markdown reports show each number's change since an earlier run.

## Watching A Run And Its Reports

The [live dashboard](./controller/dashboard.md) above needs the `dashboard` feature, which is off by default; [Watching a run](./getting-started/common.md#watching-a-run) shows how to turn it on. Goose then serves it at `http://127.0.0.1:5118/` while a load test runs with `--dashboard`, and saves the run's report when it ends. With `--dashboard-control` and a [dashboard token](./glossary.md#dashboard-token), the dashboard can also start and stop the test and change its users and settings; [Turning it on](./controller/dashboard.md#turning-it-on) explains the token, and `--dashboard-no-auth`, which turns authentication off.

When the run ends, each `--report-file <FILE>` writes one report, its type set by the file's extension: PDF (from a build with the `pdf-reports` feature), HTML, Markdown or JSON. [Writing reports](./getting-started/common.md#writing-an-html-formatted-report) shows each. Goose also prints its metrics to the console.

Goose 0.19 runs on one server: [Gaggle](./gaggle/overview.md), the distributed load test, was removed in Goose 0.17.0 and will be restored in Goose 0.20, before the 1.0 release. For a distributed load test today, use Goose 0.16.4.

## Brought To You By

Goose is made by [Tag1 Consulting](https://tag1.com/goose/) and led by [Jeremy Andrews](https://github.com/jeremyandrews), Tag1's CEO, with many [community contributions](https://github.com/tag1consulting/goose/graphs/contributors). Tag1 is a [member of the Rust Foundation](https://www.tag1.com/blog/tag1-joins-rust-foundation-first-silver-member/).

## More Documentation

- [Getting Started](./getting-started/overview.md), to build and run a first load test
- [Developer documentation](https://docs.rs/goose/)
- Blog posts at Tag1:
  - [Real life Goose load testing](https://www.tag1.com/blog/real-life-goose-load-testing/)
  - [The golden Goose egg, a compile-time adventure](https://www.tag1.com/blog/golden-goose-egg-compile-time-adventure/)
  - [A working Gaggle](https://www.tag1.com/blog/show-me-how-flock-flies-working-gaggle-goose/), from 2020, with an earlier Goose

Coming from Goose 0.18? See [Upgrading From 0.18](./getting-started/upgrading.md).
