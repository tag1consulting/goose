# Request Log

Goose can optionally log details about all the requests made during the load test to a file. This log file contains the running metrics Goose generates as the load test runs. To enable, add the `--request-log <request.log>` command line option, where `<request.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

If `--request-body` is also enabled, the request log will include the entire body of any client requests.

Logs include the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object which also includes the entire [`GooseRawRequest`](https://docs.rs/goose/*/goose/metrics/struct.GooseRawRequest.html) object, both created for all client requests.

The `headers` field of the `GooseRawRequest` lists the headers set on the individual request (for example by `.header()`, `.bearer_auth()`, `.json()` or `.form()`); client default headers and cookies are added by Reqwest when the request is sent and are not included. Request headers are only captured when the request log, the debug log or the error log is enabled, otherwise `headers` is empty.

## Log Format

By default, logs are written in JSON Lines format. For example, from the [`simple`](../example/simple.md) example with `--request-body` also enabled:

```json
{"elapsed":0,"scenario_index":0,"scenario_name":"websiteuser","transaction_index":0,"transaction_name":{"TransactionOnly":""},"raw":{"method":"Post","url":"http://127.0.0.1:18765/login","headers":["(\"content-type\", \"application/x-www-form-urlencoded\")"],"body":"username=test_user&password=","custom_method":""},"name":"/login","final_url":"http://127.0.0.1:18765/login","redirected":false,"response_time":1,"status_code":200,"success":true,"update":false,"user":0,"error":"","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":583}
{"elapsed":1,"scenario_index":0,"scenario_name":"websiteuser","transaction_index":1,"transaction_name":{"TransactionOnly":""},"raw":{"method":"Get","url":"http://127.0.0.1:18765/","headers":[],"body":"","custom_method":""},"name":"/","final_url":"http://127.0.0.1:18765/","redirected":false,"response_time":0,"status_code":200,"success":true,"update":false,"user":0,"error":"","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":84}
{"elapsed":9694,"scenario_index":0,"scenario_name":"websiteuser","transaction_index":2,"transaction_name":{"TransactionOnly":""},"raw":{"method":"Get","url":"http://127.0.0.1:18765/about/","headers":[],"body":"","custom_method":""},"name":"/about/","final_url":"http://127.0.0.1:18765/about/","redirected":false,"response_time":0,"status_code":200,"success":true,"update":false,"user":1,"error":"","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":283}
```

The `--request-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object.

`response_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `response_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `response_time_us` column holding the whole time in microseconds. In the first line above the `/login` request took 1.583 milliseconds, and in the third the `/about/` request took 283 microseconds.

## Gaggle Mode

When operating in Gaggle-mode, the `--request-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
