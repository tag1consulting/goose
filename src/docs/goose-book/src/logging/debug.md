# Debug Log

Goose can optionally and efficiently log arbitrary details, and specifics about requests and responses for debug purposes.

To enable, add the `--debug-log <debug.log>` command line option, where `<debug.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

If `--debug-log <foo>` is not specified at run time, nothing will be logged and there is no measurable overhead in your load test.

To write to the debug log, you must invoke [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug) from your load test transaction functions. The `tag` parameter allows you to record any arbitrary string: it can also identify where in the load test the log was generated, and/or why debug is being written, and/or other details such as the contents of a form the load test posts. Other paramters that can be included in the debug log are the complete Request that was made, as well as the Headers and Body of the Response.

(_Known limitations in Reqwest prevent all headers from being recorded: <https://github.com/tag1consulting/goose/issues/336>_)

See [`examples/drupal_loadtest`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_loadtest.rs) for an example of how you might invoke [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug) from a load test.

## Request Failures

Calls to [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure) can be used to tell Goose that a request failed even though the server returned a successful status code, and will automatically invoke [`log_debug`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.log_debug) for you. See [`examples/drupal_loadtest`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_loadtest.rs) and [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami) for an example of how you might use [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure) to generate useful debug logs.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"tag":"post_comment: no form_build_id found on node/9298","request":{"elapsed":501,"scenario_index":1,"scenario_name":"authbrowsinguser","transaction_index":4,"transaction_name":{"TransactionOnly":"(Auth) comment form"},"raw":{"method":"Get","url":"http://127.0.0.1:18765/node/9298","headers":[],"body":"","custom_method":""},"name":"node/9298","final_url":"http://127.0.0.1:18765/node/9298","redirected":false,"response_time":0,"status_code":200,"success":false,"update":true,"user":2,"error":"post_comment: no form_build_id found on node/9298","coordinated_omission_elapsed":0,"user_cadence":0,"response_time_fraction_us":28},"header":"{\"content-type\": \"text/html; charset=utf-8\", \"content-length\": \"123\", \"date\": \"Fri, 23 Oct 2026 20:45:34 GMT\"}","body":"<!doctype html><html><head><title>Node 9298</title></head><body><h1>Node 9298</h1><p>Comments are closed.</p></body></html>"}
```

The `--debug-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseDebug`](https://docs.rs/goose/*/goose/goose/struct.GooseDebug.html) object.

## Gaggle Mode

When operating in Gaggle-mode, the `--debug-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
