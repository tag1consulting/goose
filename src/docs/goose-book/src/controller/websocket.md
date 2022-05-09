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
{"response":"{\"help\":false,\"version\":false,\"list\":false,\"host\":\"\",\"users\":10,\"hatch_rate\":null,\"startup_time\":\"0\",\"run_time\":\"0\",\"goose_log\":\"\",\"log_level\":0,\"quiet\":0,\"verbose\":0,\"running_metrics\":null,\"no_reset_metrics\":false,\"no_metrics\":false,\"no_transaction_metrics\":false,\"no_print_metrics\":false,\"no_error_summary\":false,\"report_file\":\"\",\"no_granular_report\":false,\"request_log\":\"\",\"request_format\":\"Json\",\"request_body\":false,\"transaction_log\":\"\",\"transaction_format\":\"Json\",\"error_log\":\"\",\"error_format\":\"Json\",\"debug_log\":\"\",\"debug_format\":\"Json\",\"no_debug_body\":false,\"no_status_codes\":false,\"test_plan\":null,\"no_telnet\":false,\"telnet_host\":\"0.0.0.0\",\"telnet_port\":5116,\"no_websocket\":false,\"websocket_host\":\"0.0.0.0\",\"websocket_port\":5117,\"no_autostart\":true,\"no_gzip\":false,\"timeout\":null,\"co_mitigation\":\"Disabled\",\"throttle_requests\":0,\"sticky_follow\":false,\"manager\":false,\"expect_workers\":null,\"no_hash_check\":false,\"manager_bind_host\":\"\",\"manager_bind_port\":0,\"worker\":false,\"manager_host\":\"\",\"manager_port\":0}","success":true}
{"request":"stop"}
{"response":"load test not running, failed to stop","success":false}
{"request":"shutdown"}
{"response":"load test shut down","success":true}
```

