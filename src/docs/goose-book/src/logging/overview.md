# Logging

With logging, it's possible to record all Goose activity. This can be useful for debugging errors, for validating the load test, and for creating graphs.

When logging is enabled, a central logging thread maintains a buffer to minimize the IO overhead, and controls the writing to ensure that multiple threads don't corrupt each other's messages. All log messages are sent through a channel to the logging thread and written asynchronously, minimizing the impact on the load test.
