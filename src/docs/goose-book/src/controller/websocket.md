# WebSocket Controller

The WebSocket Controller is off unless Goose is started with `--websocket`, or a load test sets [`GooseDefault::WebSocket`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.WebSocket). It takes no authentication and accepts a connection from any web page, so a page open in any browser that can reach its port can start, stop and reconfigure the load test.

When on, it listens on `127.0.0.1:5117` by default, so only this machine can reach it. To reach it from another machine, either forward the port over ssh to the default bind (`ssh -L 5117:127.0.0.1:5117 loadtest`, then connect to `ws://127.0.0.1:5117`), or start Goose with an explicit host such as `--websocket-host 0.0.0.0`, which prints a warning at startup that the Controller has no authentication.

The host and port that the WebSocket Controller listens on can be configured at start time with `--websocket-host` and `--websocket-port`, and their defaults changed with [`GooseDefault::WebSocketHost`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.WebSocketHost) and [`GooseDefault::WebSocketPort`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.WebSocketPort). `--no-websocket` and [`GooseDefault::NoWebSocket`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.NoWebSocket), which turned it off in Goose 0.18, are still accepted and only log a warning; either one with `--websocket` or `GooseDefault::WebSocket` is refused.

## Details

The WebSocket Controller supports the same commands listed in the [telnet controller](telnet.md). Requests and Responses are in JSON format.

Requests must be made in the following format:
```json
{"request":String}
```

For example, a client should send the following JSON to request the current load test metrics:
```json
{"request":"metrics"}
```

A request may be at most 131,072 bytes (128 KiB), the longest single command line argument Linux takes with 4 KiB pages. A longer message is answered with a close frame, code 1009 (message too big), with the reason `message too big, at most 131072 bytes`, and the connection ends. A test plan sent to a Controller is at most 4096 bytes and 100 steps, as from the dashboard; see the [bounds every front end shares](telnet.md#controller-commands).

Responses will always be in the following format:
```json
{"response":String,"success":Boolean}
```

For example, from a load test built with the `pdf-reports` feature, so its configuration includes `pdf_scale` and `pdf_timeout`, which a build without that feature leaves out:
```bash
% websocat ws://127.0.0.1:5117
foo
{"response":"invalid json, see Goose book https://book.goose.rs/controller/websocket.html","success":false}
{"request":"foo"}
{"response":"unrecognized command, see Goose book https://book.goose.rs/controller/websocket.html","success":false}
{"request":"config"}
{"response":"{\"host\":null,\"users\":10,\"increase_rate\":null,\"increase_time\":null,\"run_time\":null,\"decrease_rate\":null,\"decrease_time\":null,\"iterations\":null,\"test_plan\":null,\"scenarios\":null,\"throttle_requests\":null,\"version\":false,\"list\":false,\"scenarios_list\":false,\"verbose\":0,\"quiet\":0,\"goose_log\":null,\"log_level\":0,\"running_metrics\":null,\"no_print_metrics\":false,\"no_error_summary\":false,\"report_file\":[],\"output_dir\":null,\"compress\":null,\"notes\":null,\"report_graph_data\":false,\"render_report\":null,\"baseline_file\":null,\"no_granular_report\":false,\"pdf_print_html\":null,\"pdf_scale\":0.8,\"pdf_timeout\":60,\"no_reset_metrics\":false,\"no_metrics\":false,\"no_transaction_metrics\":false,\"no_scenario_metrics\":false,\"no_status_codes\":false,\"co_mitigation\":\"Disabled\",\"threshold\":[],\"no_threshold_abort\":false,\"threshold_abort_delay\":null,\"request_log\":null,\"request_format\":\"Json\",\"request_body\":false,\"transaction_log\":null,\"transaction_format\":\"Json\",\"scenario_log\":null,\"scenario_format\":\"Json\",\"error_log\":null,\"error_format\":\"Json\",\"debug_log\":null,\"debug_format\":\"Json\",\"no_debug_body\":false,\"dashboard\":false,\"dashboard_host\":null,\"dashboard_port\":null,\"dashboard_auth_token\":null,\"dashboard_control\":false,\"dashboard_no_auth\":false,\"dashboard_max_clients\":null,\"no_percentiles_over_time\":false,\"no_autostart\":true,\"no_telnet\":false,\"telnet_host\":\"127.0.0.1\",\"telnet_port\":5116,\"websocket\":true,\"no_websocket\":false,\"websocket_host\":\"127.0.0.1\",\"websocket_port\":5117,\"timeout\":null,\"no_gzip\":false,\"sticky_follow\":false,\"accept_invalid_certs\":false}","success":true}
{"request":"stop"}
{"response":"load test not running, failed to stop","success":false}
{"request":"shutdown"}
{"response":"load test shut down","success":true}
```

This session is Goose started with `--websocket --no-autostart -u 10`. `config` sends the configuration as JSON, with an option that is not set shown as `null`, as the telnet Controller's `config-json` does. A dashboard auth token, when one is set, is shown as `"(set)"`, never the token itself. `metrics` sends the metrics as the telnet Controller's `metrics-json` does, with each request's response times in microsecond buckets, `times_us`; see [Controller Commands](telnet.md#controller-commands).

After `exit` (or `quit` or `q`) the Controller answers `goodbye!`, and after `shutdown` it answers `load test shut down`; either way it then sends a close frame, code 1000, and ends the connection. A request that gets no answer from Goose within 15 seconds is answered `{"response":"control timed out, it may still apply: check before retrying","success":false}`.

