# Request Log

Goose can optionally log details about all the requests made during the load test to a file. This log file contains the running metrics Goose generates as the load test runs. To enable, add the `--request-log <request.log>` command line option, where `<request.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

If `--request-body` is also enabled, the request log will include the entire body of any client requests.

Logs include the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object which also includes the entire [`GooseRawRequest`](https://docs.rs/goose/*/goose/metrics/struct.GooseRawRequest.html) object, both created for all client requests.

## Log Format

By default, logs are written in JSON Lines format. For example (in this case with `--request-body` also enabled):

```json
{"coordinated_omission_elapsed":0,"elapsed":13219,"error":"","final_url":"http://apache/misc/jquery-extend-3.4.0.js?v=1.4.4","name":"static asset","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/misc/jquery-extend-3.4.0.js?v=1.4.4"},"redirected":false,"response_time":7,"status_code":200,"success":true,"update":false,"user":4,"user_cadence":0}
{"coordinated_omission_elapsed":0,"elapsed":13055,"error":"","final_url":"http://apache/node/1786#comment-114852","name":"(Auth) comment form","raw":{"body":"subject=this+is+a+test+comment+subject&comment_body%5Bund%5D%5B0%5D%5Bvalue%5D=this+is+a+test+comment+body&comment_body%5Bund%5D%5B0%5D%5Bformat%5D=filtered_html&form_build_id=form-U0L3wm2SsIKAhVhaHpxeL1TLUHW64DXKifmQeZsUsss&form_token=VKDel_jiYzjqPrekL1FrP2_4EqHTlsaqLjMUJ6pn-sE&form_id=comment_node_article_form&op=Save","headers":["(\"content-type\", \"application/x-www-form-urlencoded\")"],"method":"Post","url":"http://apache/comment/reply/1786"},"redirected":true,"response_time":172,"status_code":200,"success":true,"update":false,"user":1,"user_cadence":0}
{"coordinated_omission_elapsed":0,"elapsed":13219,"error":"","final_url":"http://apache/misc/drupal.js?q9apdy","name":"static asset","raw":{"body":"","headers":[],"method":"Get","url":"http://apache/misc/drupal.js?q9apdy"},"redirected":false,"response_time":7,"status_code":200,"success":true,"update":false,"user":0,"user_cadence":0}
```

The `--request-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`GooseRequestMetric`](https://docs.rs/goose/*/goose/metrics/struct.GooseRequestMetric.html) object.

## Gaggle Mode

When operating in Gaggle-mode, the `--request-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
