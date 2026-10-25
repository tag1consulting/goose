# Error Log

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-log=<error.log>` command line option, where `<error.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object, created any time a request results in an error, including a request the load test marks as failed with [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure).

The `raw.headers` field follows the same rule as in the [request log](requests.md): it lists only the headers set on the individual request.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":1,"raw":{"method":"Get","url":"http://127.0.0.1:19150/node/3197","headers":[],"body":"","custom_method":""},"name":"/node/3197","final_url":"http://127.0.0.1:19150/node/3197","redirected":false,"response_time":0,"status_code":503,"user":0,"error":"503 Service Unavailable: /node/3197"}
{"elapsed":2,"raw":{"method":"Get","url":"http://127.0.0.1:19150/node/8624","headers":[],"body":"","custom_method":""},"name":"/node/8624","final_url":"http://127.0.0.1:19150/node/8624","redirected":false,"response_time":0,"status_code":503,"user":0,"error":"503 Service Unavailable: /node/8624"}
```

These lines are from the [`drupal_memcache`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs) example run against a server that returned `503` for every node page.

The `--error-format` option can be used to change the log format to `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object, one per line, and `pretty` is the same output spread over several indented lines per error.

The `csv` format starts with a header row, and its `raw` column holds the debug output of the `GooseRawRequest`:

```csv
elapsed,raw,name,final_url,redirected,response_time,status_code,user,error
1,"GooseRawRequest { method: Get, url: ""http://127.0.0.1:19150/node/2956"", headers: [], body: """", custom_method: """" }",/node/2956,http://127.0.0.1:19150/node/2956,false,0,503,0,503 Service Unavailable: /node/2956
2,"GooseRawRequest { method: Get, url: ""http://127.0.0.1:19150/node/8873"", headers: [], body: """", custom_method: """" }",/node/8873,http://127.0.0.1:19150/node/8873,false,0,503,0,503 Service Unavailable: /node/8873
```
