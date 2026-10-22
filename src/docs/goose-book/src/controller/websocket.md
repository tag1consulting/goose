# WebSocket Controller

The host and port that the WebSocket Controller listens on can be configured at start time with `--websocket-host` and `--websocket-port`. The WebSocket Controller can be completely disabled with the `--no-websocket` command line option. The defaults can be changed with [`GooseDefault::WebSocketHost`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.WebSocketHost),[`GooseDefault::WebSocketPort`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.WebSocketPort), and [`GooseDefault::NoWebSocket`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.NoWebSocket).

## Details

The WebSocket Controller supports the same commands listed in the [telnet controller](telnet.md). Requests and Responses are in JSON format.

Requests must be made in the following format:
```json
{"request":String}
```

For example, a client should send the follow json to request the current load test metrics:
```json
{"request":"metrics"}
```

A request may be at most 131,072 bytes (128 KiB), the longest single command line argument Linux takes with 4 KiB pages. A test plan sent to a Controller is at most 4096 bytes and 100 steps, as from the dashboard; see the [bounds every front end shares](telnet.md). A longer message is answered with a close frame, code 1009 (message too big), with the reason `message too big, at most 131072 bytes`, and the connection ends.

Responses will always be in the following format:
```json
{"response":String,"success":Boolean}
```

For example:
```bash
% websocat ws://127.0.0.1:5117
foo
{"response":"invalid json, see Goose book https://book.goose.rs/controller/websocket.html","success":false}
{"request":"foo"}
{"response":"unrecognized command, see Goose book https://book.goose.rs/controller/websocket.html","success":false}
{"request":"config"}
{"response":"{\"host\":null,\"users\":10,\"increase_rate\":null,\"increase_time\":null,\"run_time\":null,\"decrease_rate\":null,\"decrease_time\":null,\"iterations\":null,\"test_plan\":null,\"scenarios\":null,\"throttle_requests\":null,\"version\":false,\"list\":false,\"scenarios_list\":false,\"verbose\":0,\"quiet\":0,\"goose_log\":null,\"log_level\":0,\"running_metrics\":null,\"no_print_metrics\":false,\"no_error_summary\":false,\"report_file\":[],\"output_dir\":null,\"compress\":null,\"notes\":null,\"report_graph_data\":false,\"render_report\":null,\"baseline_file\":null,\"no_granular_report\":false,\"pdf_print_html\":null,\"no_reset_metrics\":false,\"no_metrics\":false,\"no_transaction_metrics\":false,\"no_scenario_metrics\":false,\"no_status_codes\":false,\"co_mitigation\":\"Disabled\",\"request_log\":null,\"request_format\":\"Json\",\"request_body\":false,\"transaction_log\":null,\"transaction_format\":\"Json\",\"scenario_log\":null,\"scenario_format\":\"Json\",\"error_log\":null,\"error_format\":\"Json\",\"debug_log\":null,\"debug_format\":\"Json\",\"no_debug_body\":false,\"dashboard\":false,\"dashboard_host\":null,\"dashboard_port\":null,\"dashboard_auth_token\":null,\"dashboard_control\":false,\"dashboard_max_clients\":null,\"no_percentiles_over_time\":false,\"no_autostart\":true,\"no_telnet\":false,\"telnet_host\":\"0.0.0.0\",\"telnet_port\":5116,\"no_websocket\":false,\"websocket_host\":\"0.0.0.0\",\"websocket_port\":5117,\"timeout\":null,\"no_gzip\":false,\"sticky_follow\":false,\"accept_invalid_certs\":false}","success":true}
{"request":"stop"}
{"response":"load test not running, failed to stop","success":false}
{"request":"shutdown"}
{"response":"load test shut down","success":true}
```

