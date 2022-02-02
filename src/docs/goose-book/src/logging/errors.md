# Error Log

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-log=<error.log>` command line option, where `<error.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object, created any time a request results in an error.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":9318,"error":"503 Service Unavailable: /","final_url":"http://apache/","name":"(Auth) front page","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/"},"redirected":false,"response_time":6,"status_code":503,"user":1}
{"elapsed":9318,"error":"503 Service Unavailable: /node/8211","final_url":"http://apache/node/8211","name":"(Anon) node page","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/node/8211"},"redirected":false,"response_time":6,"status_code":503,"user":3}
```

The `--errors-format` option can be used to change the log format to `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object.

## Gaggle Mode

When operating in Gaggle-mode, the `--error-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
