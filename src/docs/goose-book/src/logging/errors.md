# Error Log

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-log=<error.log>` command line option, where `<error.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

The log holds two kinds of entries:

- a request error: the entire [`GooseErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseErrorMetric.html) object, created any time a request results in an error, including a request the load test marks as failed with [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure);
- a transaction error: a [`GooseTransactionErrorMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseTransactionErrorMetric.html), created each time a transaction returns an error that is not already a request error: a `TransactionError::Custom` or `Url`, or a `Reqwest` error from reading the response body or building the request. A transaction that fails because one of its requests failed, as with `set_failure`, gets no entry of its own: the request error has one. It has `elapsed`, `scenario_index`, `scenario_name` (the machine name, as in the request log), `transaction_index`, `transaction_name`, `run_time`, `user` and `error`, the error as its `Display` writes it. Errors from `test_start` and `test_stop` are not logged.

The `raw.headers` field of a request error follows the same rule as in the [request log](requests.md): it lists only the headers set on the individual request.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":0,"scenario_index":0,"scenario_name":"erruser","transaction_index":2,"transaction_name":"flaky","run_time":0,"user":0,"error":"TransactionError: custom error (every third call fails)"}
{"elapsed":0,"raw":{"method":"Get","url":"http://127.0.0.1:8080/about/","headers":[],"body":"","custom_method":""},"name":"/about/","final_url":"http://127.0.0.1:8080/about/","redirected":false,"response_time":0,"status_code":404,"user":0,"error":"404 Not Found: /about/"}
```

These lines are from a load test run against a local server that answers `/about/` with a `404`. Its `ErrUser` scenario requests `/` and `/about/`, and its `flaky` transaction returns `TransactionError::Custom("every third call fails")` one time in three. The first line is a transaction error, the second a request error.

The `--error-format` option can be used to change the log format to `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire `GooseErrorMetric` or `GooseTransactionErrorMetric` object, one per line, and `pretty` is the same output spread over several indented lines per error.

The `csv` format starts with a header row. Its first nine columns are a request error's fields, and its `raw` column holds the debug output of the `GooseRawRequest`. The last five columns, `scenario_index`, `scenario_name`, `transaction_index`, `transaction_name` and `run_time`, are a transaction error's. A transaction error fills `elapsed`, `user`, `error` and those five, leaving the other request columns empty, and a request error leaves the last five empty, so every row has every column:

```csv
elapsed,raw,name,final_url,redirected,response_time,status_code,user,error,scenario_index,scenario_name,transaction_index,transaction_name,run_time
0,,,,,,,0,TransactionError: custom error (every third call fails),0,erruser,2,flaky,0
0,"GooseRawRequest { method: Get, url: ""http://127.0.0.1:8080/about/"", headers: [], body: """", custom_method: """" }",/about/,http://127.0.0.1:8080/about/,false,0,404,0,404 Not Found: /about/,,,,,
```

`--no-error-summary` leaves errors out of the errors tables of the [metrics](../getting-started/metrics.md) and reports, not out of this log.
