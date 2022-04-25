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
{"body":"<!DOCTYPE html>\n<html>\n  <head>\n    <title>503 Backend fetch failed</title>\n  </head>\n  <body>\n    <h1>Error 503 Backend fetch failed</h1>\n    <p>Backend fetch failed</p>\n    <h3>Guru Meditation:</h3>\n    <p>XID: 1506620</p>\n    <hr>\n    <p>Varnish cache server</p>\n  </body>\n</html>\n","header":"{\"date\": \"Mon, 19 Jul 2021 09:21:58 GMT\", \"server\": \"Varnish\", \"content-type\": \"text/html; charset=utf-8\", \"retry-after\": \"5\", \"x-varnish\": \"1506619\", \"age\": \"0\", \"via\": \"1.1 varnish (Varnish/6.1)\", \"x-varnish-cache\": \"MISS\", \"x-varnish-cookie\": \"SESSd7e04cba6a8ba148c966860632ef3636=Z50aRHuIzSE5a54pOi-dK_wbxYMhsMwrG0s2WM2TS20\", \"content-length\": \"284\", \"connection\": \"keep-alive\"}","request":{"coordinated_omission_elapsed":0,"elapsed":9162,"error":"503 Service Unavailable: /node/1439","final_url":"http://apache/node/1439","name":"(Auth) comment form","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/node/1439"},"redirected":false,"response_time":5,"status_code":503,"success":false,"update":false,"user":1,"user_cadence":0},"tag":"post_comment: no form_build_id found on node/1439"}
```

The `--debug-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseDebug`](https://docs.rs/goose/*/goose/goose/struct.GooseDebug.html) object.

## Gaggle Mode

When operating in Gaggle-mode, the `--debug-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
