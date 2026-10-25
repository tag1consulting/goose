# Request Log

Goose can optionally log details about all the requests made during the load test to a file. This log file contains the running metrics Goose generates as the load test runs. To enable, add the `--request-log <request.log>` command line option, where `<request.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

If `--request-body` is also enabled, the request log will include the entire body of any client requests, in the `body` field of the raw request.

Logs include the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object which also includes the entire [`GooseRawRequest`](https://docs.rs/goose/*/goose/metrics/struct.GooseRawRequest.html) object, both created for all client requests.

The `headers` field of the `GooseRawRequest` lists the headers set on the individual request (for example by `.header()`, `.bearer_auth()`, `.json()` or `.form()`); headers that Reqwest adds when it sends the request are not included: the client's default headers (those set with `.default_headers()`, the `User-Agent` and `Accept: */*`), the `Cookie` header from the cookie store, `Accept-Encoding` when compression is enabled, and the `Host` and `Content-Length` headers. Reqwest offers no way to read them: <https://github.com/tag1consulting/goose/issues/336>. Request headers are only captured when the request log, the debug log or the error log is enabled, otherwise `headers` is empty.

## Log Format

By default, logs are written in JSON Lines format. For example, from the [`simple`](../example/simple.md) example with `--request-body` also enabled:

```json
{"elapsed":505,"scenario_index":0,"scenario_name":"websiteuser","transaction_index":0,"transaction_name":{"TransactionOnly":""},"raw":{"method":"Post","url":"http://127.0.0.1:19150/login","headers":["(\"content-type\", \"application/x-www-form-urlencoded\")"],"body":"username=test_user&password=","custom_method":""},"name":"/login","final_url":"http://127.0.0.1:19150/login","redirected":false,"response_time":1,"status_code":200,"success":true,"update":false,"user":3,"error":"","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":221}
{"elapsed":506,"scenario_index":0,"scenario_name":"websiteuser","transaction_index":1,"transaction_name":{"TransactionOnly":""},"raw":{"method":"Get","url":"http://127.0.0.1:19150/","headers":[],"body":"","custom_method":""},"name":"/","final_url":"http://127.0.0.1:19150/","redirected":false,"response_time":1,"status_code":200,"success":true,"update":false,"user":3,"error":"","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":202}
{"elapsed":6875,"scenario_index":0,"scenario_name":"websiteuser","transaction_index":2,"transaction_name":{"TransactionOnly":""},"raw":{"method":"Get","url":"http://127.0.0.1:19150/about/","headers":[],"body":"","custom_method":""},"name":"/about/","final_url":"http://127.0.0.1:19150/about/","redirected":false,"response_time":0,"status_code":200,"success":true,"update":false,"user":2,"error":"","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":632}
```

The `--request-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object, one per line, and `pretty` is the same output spread over several indented lines per request.

The `csv` format starts with a header row. Its `transaction_name` column holds the transaction's name, empty in this example, and its `raw` column holds the debug output of the `GooseRawRequest`. The first two requests of the same example look like:

```csv
elapsed,scenario_index,scenario_name,transaction_index,transaction_name,raw,name,final_url,redirected,response_time,status_code,success,update,user,error,coordinated_omission_elapsed,user_cadence,response_time_us
0,0,websiteuser,0,,"GooseRawRequest { method: Post, url: ""http://127.0.0.1:19150/login"", headers: [""(\""content-type\"", \""application/x-www-form-urlencoded\"")""], body: ""username=test_user&password="", custom_method: """" }",/login,http://127.0.0.1:19150/login,false,0,200,true,false,0,,0,0,756
0,0,websiteuser,1,,"GooseRawRequest { method: Get, url: ""http://127.0.0.1:19150/"", headers: [], body: """", custom_method: """" }",/,http://127.0.0.1:19150/,false,0,200,true,false,0,,0,0,188
```

The `transaction_name` field is an object naming how the transaction was named: `{"TransactionOnly":"..."}` for a name set with `set_name`, which only the transaction carries, or `{"InheritNameByRequests":"..."}` for a name set with `set_name_for_transaction_and_requests`, which its requests take too. Goose 0.18 wrote it as a plain string.

`response_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `response_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `response_time_us` column holding the whole time in microseconds. In the first JSON line above the `/login` request took 1.221 milliseconds, and in the third the `/about/` request took 632 microseconds. In the first CSV row the `/login` request took 756 microseconds.
