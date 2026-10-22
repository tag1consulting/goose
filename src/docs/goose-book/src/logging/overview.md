# Logging

With logging, it's possible to record all Goose activity. This can be useful for debugging errors, for validating the load test, and for creating graphs.

When logging is enabled, a central logging thread maintains a buffer to minimize the IO overhead, and controls the writing to ensure that multiple threads don't corrupt each other's messages. All log messages are sent through a channel to the logging thread and written asynchronously, minimizing the impact on the load test.

Each log option takes a file name, such as `--request-log requests.log`, and writes that file. Given without one, as `--request-log`, it writes into the run's folder, named for the log and its format: `requests.json`, `requests.csv`, or `requests.log` for raw and pretty. A name ending in `.gz` writes the log gzipped, and `--compress` gzips every log in the run's folder. See [Output files](../getting-started/output-files.md).
