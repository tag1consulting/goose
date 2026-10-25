# Error Log

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-log=<error.log>` command line option, where `<error.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object, created any time a request results in an error.

The `raw.headers` field follows the same rule as in the [request log](requests.md): it lists only the headers set on the individual request.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":3,"raw":{"method":"Get","url":"http://127.0.0.1:18765/node/3484","headers":[],"body":"","custom_method":""},"name":"/node/3484","final_url":"http://127.0.0.1:18765/node/3484","redirected":false,"response_time":0,"status_code":503,"user":0,"error":"503 Service Unavailable: /node/3484"}
{"elapsed":3,"raw":{"method":"Get","url":"http://127.0.0.1:18765/node/2652","headers":[],"body":"","custom_method":""},"name":"/node/2652","final_url":"http://127.0.0.1:18765/node/2652","redirected":false,"response_time":0,"status_code":503,"user":0,"error":"503 Service Unavailable: /node/2652"}
```

The `--errors-format` option can be used to change the log format to `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object.

## Gaggle Mode

When operating in Gaggle-mode, the `--error-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
