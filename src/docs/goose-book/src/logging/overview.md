# Logging

With logging, it's possible to record all Goose activity. This can be useful for debugging errors, for validating the load test, and for creating graphs.

When logging is enabled, a central logging thread maintains a buffer to minimize the IO overhead, and controls the writing to ensure that multiple threads don't corrupt each other's messages. All log messages are sent through a channel to the logging thread and written asynchronously, minimizing the impact on the load test.

Each log option takes a file name, such as `--request-log requests.log`, and writes that file. Given without one, as `--request-log`, it writes a file named for the log and its format into the run's folder: `requests.json`, `requests.csv`, or `requests.log` for raw and pretty. A name ending in `.gz` writes the log gzipped, and `--compress` gzips every log in the run's folder. See [Output files](../getting-started/output-files.md).

Every log takes a format option: `--request-format`, `--transaction-format`, `--scenario-format`, `--error-format` and `--debug-format`. Each accepts `json` (the default), `csv`, `raw` or `pretty`. A JSON log writes one object per line, with its keys in the order of the logged structure's fields. A CSV log starts with a header row. The `elapsed` field of the request, transaction, scenario and error logs counts milliseconds since the load test started, a clock every user shares. The `raw` format writes Rust's debug output of each entry on one line, and `pretty` spreads the same output over several indented lines.

The request, transaction, scenario and error logs are built from Goose metrics, so they can't be combined with `--no-metrics`. The debug log can.

## What logging costs

With Goose on two cores running `examples/simple` with 1,000 users and no waits, the JSON request log added about 3.4 µs of CPU to each request, a tenth of what Goose spent per request. Gzip at the default level 6 roughly doubles that. Each request writes about 500 bytes, so at 25,000 requests a second the log grows by about 45 GB an hour. Write it uncompressed unless the disk can't keep up or space is short; then gzip it at the default level.
