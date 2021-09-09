# WebSocket Controller

The host and port that the WebSocket Controller listens on can be configured at start time with `--websocket-host` and `--websocket-port`. The WebSocket Controller can be completely disabled with the `--no-websocket` command line option. The defaults can be changed with `GooseDefault::WebSocketHost`,`GooseDefault::WebSocketPort`, and `GooseDefault::NoWebSocket`.

The WebSocket Controller supports the same commands listed above. Requests and Response are in JSON format.

Requests must be made in the following format:
```json
{
  "request": String,
}
```

For example, a client should send the follow json to request the current load test metrics:
```json
{
  "request": "metrics",
}
```

Responses will always be in the following format:
```json
{
  "response": String,
  "success": Boolean,
}
```

For example:
```
% websocat ws://127.0.0.1:5117
foo
{"response":"unable to parse json, see Goose README.md","success":false}
{"request": "foo"}
{"response":"unrecognized command, see Goose README.md","success":false}
{"request": "config"}
{"response":"{\"help\":false,\"version\":false,\"list\":false,\"host\":\"http://apache/\",\"users\":5,\"hatch_rate\":\".5\",\"run_time\":\"\",\"log_level\":0,\"goose_log\":\"\",\"verbose\":1,\"running_metrics\":null,\"no_reset_metrics\":false,\"no_metrics\":false,\"no_task_metrics\":false,\"no_error_summary\":false,\"report_file\":\"\",\"request_log\":\"\",\"request_format\":\"json\",\"debug_log\":\"\",\"debug_format\":\"json\",\"no_debug_body\":false,\"status_codes\":false,\"no_telnet\":false,\"telnet_host\":\"0.0.0.0\",\"telnet_port\":5116,\"no_websocket\":false,\"websocket_host\":\"0.0.0.0\",\"websocket_port\":5117,\"no_autostart\":true,\"throttle_requests\":0,\"sticky_follow\":false,\"manager\":false,\"expect_workers\":null,\"no_hash_check\":false,\"manager_bind_host\":\"\",\"manager_bind_port\":0,\"worker\":false,\"manager_host\":\"\",\"manager_port\":0}","success":true}
{"request": "stop"}
{"response":"load test not running, failed to stop","success":false}
{"request": "exit"}
{"response":"goodbye!","success":true}
```

