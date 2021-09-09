# Logging

With logging, it's possible to record all Goose activity. This can be useful for debugging errors, for validating the load test, and for creating graphs.

When logging is enabled, a dedicated thread is started. All log messages are sent through a channel to the logging thread and written asynchronously, minimizing the impact on the load test.
