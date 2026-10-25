# Debug Log

Goose can optionally and efficiently log arbitrary details, and specifics about requests and responses for debug purposes.

To enable, add the `--debug-log <debug.log>` command line option, where `<debug.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

If `--debug-log <foo>` is not specified at run time, nothing will be logged and there is no measurable overhead in your load test.

Response bodies are included unless `--no-debug-body` is also given, which keeps the debug log small.

To write to the debug log, you must invoke [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug) from your load test transaction functions. The `tag` parameter allows you to record any arbitrary string: it can also identify where in the load test the log was generated, and/or why debug is being written, and/or other details such as the contents of a form the load test posts. Other parameters that can be included in the debug log are the complete Request that was made, as well as the Headers and Body of the Response.

(_The recorded request headers are only those set on the individual request: client default headers, the `User-Agent`, cookies, `Accept-Encoding`, `Host` and `Content-Length` are added later by Reqwest and are not recorded, see the [request log](requests.md) and <https://github.com/tag1consulting/goose/issues/336>._)

See [`examples/drupal_memcache`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs) for an example of how you might invoke [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug) from a load test.

## Request Failures

Calls to [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure) can be used to tell Goose that a request failed even though the server returned a successful status code, and will automatically invoke [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug) for you. See [`examples/drupal_memcache`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs) and [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami) for an example of how you might use [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure) to generate useful debug logs.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"tag":"login: no form_build_id on page: /user page","request":{"elapsed":333,"scenario_index":1,"scenario_name":"authbrowsinguser","transaction_index":0,"transaction_name":{"TransactionOnly":"(Auth) login"},"raw":{"method":"Get","url":"http://127.0.0.1:19150/user","headers":[],"body":"","custom_method":""},"name":"/user","final_url":"http://127.0.0.1:19150/user","redirected":false,"response_time":0,"status_code":200,"success":false,"update":true,"user":1,"error":"login: no form_build_id on page: /user page","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":625},"header":"{\"server\": \"BaseHTTP Python/3.15.0\", \"date\": \"Sun, 25 Oct 2026 20:57:26 GMT\", \"content-type\": \"text/html; charset=utf-8\", \"content-length\": \"86\"}","body":"<!doctype html><html><head><title>user</title></head><body><h1>user</h1></body></html>"}
```

This line is from the [`drupal_memcache`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs) example run against a server whose login page has no `form_build_id`, so the login transaction called [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure). The `header` field is the response headers as one string, Rust's debug output of the header map, and `header`, `request` and `body` are `null` when the call passed none.

The `--debug-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseDebug`](https://docs.rs/goose/*/goose/goose/struct.GooseDebug.html) object, one per line, and `pretty` is the same output spread over several indented lines per entry. The `csv` format starts with the header row `tag,request,header,body`; its `request`, `header` and `body` columns hold Rust's debug output of each optional value, such as `Some(...)` or `None`.
