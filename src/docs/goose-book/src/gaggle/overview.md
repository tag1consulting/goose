# Gaggle: Distributed Load Test

**NOTE: Gaggle support was temporarily removed as of Goose 0.17.0 (see https://github.com/tag1consulting/goose/pull/529). Use Goose 0.16.4 if you need the functionality described in this section.**

Goose also supports distributed load testing. A Gaggle is one Goose process running in [Manager mode](manager.md), and 1 or more Goose processes running in [Worker mode](worker.md). The Manager coordinates starting and stopping the Workers, and collects aggregated metrics. Gaggle support is a [cargo feature that must be enabled at compile-time](technical.md#compile-time-feature). To launch a Gaggle, you must copy your load test application to all servers from which you wish to generate load.

It is strongly recommended that the same load test application be copied to all servers involved in a Gaggle. By default, Goose will verify that the load test is identical by comparing a hash of all load test rules. Telling it to skip this check can cause the load test to panic (for example, if a Worker defines a different number of transactions or scenarios than the Manager).

## Load Testing At Scale

Experimenting with running Goose load tests from AWS, Goose has proven to make fantastic use of all available system resources, so that it is only generally limited by network speeds. A smaller server instance was able to simulate 2,000 users generating over 6,500 requests per second and saturating a 2.6 Gbps uplink. As more uplink speed was added, Goose was able to scale linearly -- by distributing the test across two servers with faster uplinks, it comfortably simulated 12,000 active users generating over 41,000 requests per second and saturating 16 Gbps.

Generating this much traffic in and of itself is not fundamentally difficult, but with Goose each request is fully analyzed and validated. It not only confirms the response code for each response the server returns, but also inspects the returned HTML to confirm it contains all expected elements. Links to static elements such as images and CSS are extracted from each response and also loaded, with each simulated user behaving similar to how a real user would. Goose excels at providing consistent and repeatable load testing.

For full details and graphs, refer to the blog [A Goose In The Clouds: Load Testing At Scale](https://www.tag1consulting.com/blog/goose-clouds-load-testing-scale).
