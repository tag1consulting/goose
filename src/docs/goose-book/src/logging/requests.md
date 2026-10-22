# Request Log

Goose can optionally log details about all the requests made during the load test to a file. This log file contains the running metrics Goose generates as the load test runs. To enable, add the `--request-log <request.log>` command line option, where `<request.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

If `--request-body` is also enabled, the request log will include the entire body of any client requests.

Logs include the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object which also includes the entire [`GooseRawRequest`](https://docs.rs/goose/*/goose/metrics/struct.GooseRawRequest.html) object, both created for all client requests.

The `headers` field of the `GooseRawRequest` lists the headers set on the individual request (for example by `.header()`, `.bearer_auth()`, `.json()` or `.form()`); client default headers and cookies are added by Reqwest when the request is sent and are not included. Request headers are only captured when the request log, the debug log or the error log is enabled, otherwise `headers` is empty.

## Log Format

By default, logs are written in JSON Lines format. For example, from the [`simple`](../example/simple.md) example with `--request-body` also enabled:

```json
{"coordinated_omission_elapsed":0,"elapsed":0,"error":"","final_url":"http://127.0.0.1:8765/login","name":"/login","raw":{"body":"username=test_user&password=","custom_method":"","headers":["(\"content-type\", \"application/x-www-form-urlencoded\")"],"method":"Post","url":"http://127.0.0.1:8765/login"},"redirected":false,"response_time":0,"response_time_fraction_us":535,"scenario_index":0,"scenario_name":"websiteuser","status_code":200,"success":true,"transaction_index":0,"transaction_name":{"TransactionOnly":""},"update":false,"user":0,"user_cadence":0}
{"coordinated_omission_elapsed":0,"elapsed":0,"error":"","final_url":"http://127.0.0.1:8765/","name":"/","raw":{"body":"","custom_method":"","headers":[],"method":"Get","url":"http://127.0.0.1:8765/"},"redirected":false,"response_time":0,"response_time_fraction_us":106,"scenario_index":0,"scenario_name":"websiteuser","status_code":200,"success":true,"transaction_index":1,"transaction_name":{"TransactionOnly":""},"update":false,"user":0,"user_cadence":0}
{"coordinated_omission_elapsed":0,"elapsed":8648,"error":"","final_url":"http://127.0.0.1:8765/about/","name":"/about/","raw":{"body":"","custom_method":"","headers":[],"method":"Get","url":"http://127.0.0.1:8765/about/"},"redirected":false,"response_time":1405,"response_time_fraction_us":306,"scenario_index":0,"scenario_name":"websiteuser","status_code":200,"success":true,"transaction_index":2,"transaction_name":{"TransactionOnly":""},"update":false,"user":0,"user_cadence":0}
```

The `--request-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object.

`response_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `response_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `response_time_us` column holding the whole time in microseconds. In the first line above the `/login` request took 535 microseconds, and in the third the `/about/` request took 1,405.306 milliseconds.

## Gaggle Mode

When operating in Gaggle-mode, the `--request-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
