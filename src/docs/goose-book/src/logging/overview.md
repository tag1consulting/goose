# Logging

With logging, it's possible to record all Goose activity. This can be useful for debugging errors, for validating the load test, and for creating graphs.

When logging is enabled, a central logging thread maintains a buffer to minimize the IO overhead, and controls the writing to ensure that multiple threads don't corrupt each other's messages. All log messages are sent through a channel to the logging thread and written asynchronously, minimizing the impact on the load test.

Each log option takes a file name, such as `--request-log requests.log`, and writes that file. Given without one, as `--request-log`, it writes a file named for the log and its format into the run's folder: `requests.json`, `requests.csv`, or `requests.log` for raw and pretty. A name ending in `.gz` writes the log gzipped, and `--compress` gzips every log in the run's folder. See [Output files](../getting-started/output-files.md).

## What logging costs

These numbers come from `examples/simple` with its wait removed: 1,000 users, unthrottled, against a small Rust server on a second machine over a 2.5 Gb/s network. Goose ran on Apple silicon under Linux. One setup used two efficiency cores. The other used eight cores, four of each kind. Each number is the mean of the middle three of five runs of three minutes. The two core numbers come from this release. The eight core numbers, the gzip level comparison and the logger's queue come from a build made for testing, with the same logger. CPU is user plus system time. Instructions are user space instructions only, so they leave out the kernel's work to send and receive.

| Per request | Two cores: instructions | Two cores: CPU | Eight cores: instructions | Eight cores: CPU |
|---|---|---|---|---|
| No request log | 77,400 | 33.9 µs | 81,900 | 65.6 µs |
| JSON request log adds | 12,400 (16%) | 3.4 µs (10%) | 12,900 (16%) | 5.5 µs (8%) |
| JSON request log, gzipped at level 6, adds | 28,500 (37%) | 6.5 µs (19%) | 28,500 (35%) | 8.0 µs (12%) |

On eight cores Goose used about twice the CPU for each request, with or without a request log. The request log added about the same number of instructions as on two cores.

The JSON request log writes about 504 bytes per request. At the two core rate of 25,000 requests a second, that is 12.6 MB/s, or 45 GB an hour. Gzip shrinks it to 23.8 bytes per request at level 1, 10.9 at level 6 and 10.8 at level 9. Level 1 costs about 2,800 more instructions per request than an uncompressed log. That count includes the lines still queued when the load ended. Level 6 costs 16,100 more, and level 9 costs 19,300 more. Level 9 saves almost nothing over level 6.

On two cores the logger fell behind at level 1. Its queue grew in four of five runs and reached 1.6 million lines. Goose took up to 8.6 seconds to finish after the load ended. These tests did not find why level 1's queue kept growing when level 6, which costs more, caught up. At level 6 its queue once reached 110,000 lines and stayed above 50,000 for about 36 seconds. In two of five level 6 runs of this release, Goose used about 100 MB more memory than in the others. At levels 6 and 9 the logger caught up within a second of the load ending. When Goose's cores are full, level 1 is a poor choice, because its queue grows.

On two cores, both were full. Goose used about half of their time. The kernel's interrupt handling for Goose's connections took the other half. It is not charged to Goose, so it is not in Goose's CPU above. On two cores logging lowered the rate: by 5% for the JSON request log and by 9% with gzip at level 6. On eight cores Goose's cores had time to spare. The rate was about 27,800 requests a second with no request log, 27,400 with the JSON request log and 27,100 with gzip at level 6. The three differ by less than the noise. So the request log lowered the rate only when Goose's cores were full.

A network file system that wrote at least 108 MB/s took the uncompressed log with no change in rate or CPU. A disk slower than the log was not tested.

| Your load test | Request log |
|---|---|
| Goose's cores are full, the disk keeps up with the uncompressed log, and space is not short | Write it uncompressed. Gzip costs CPU that would otherwise send requests. |
| The disk is slower than the uncompressed log, or space is short | Gzip at the default level 6. |
| Goose's cores are not full, the disk keeps up, and space is not short | Write it uncompressed. |
