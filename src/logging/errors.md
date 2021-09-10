# Error Log

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-file=error.log` command line option, where `error.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--error-file` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":9318,"error":"503 Service Unavailable: /","final_url":"http://apache/","name":"(Auth) front page","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/"},"redirected":false,"response_time":6,"status_code":503,"user":1}
{"elapsed":9318,"error":"503 Service Unavailable: /node/8211","final_url":"http://apache/node/8211","name":"(Anon) node page","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/node/8211"},"redirected":false,"response_time":6,"status_code":503,"user":3}
```

Logs include the entire [`GooseErrorMetric`] object as defined in `src/goose.rs`, which are created when requests result in an error.

By default Goose logs errors in JSON Lines format. The `--errors-format` option can be used to log in `csv`, `json`, `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseErrorMetric`] object.
