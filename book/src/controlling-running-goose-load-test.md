# Controlling Running Goose Load Test

By default, Goose will launch a telnet Controller thread that listens on `0.0.0.0:5116`, and a WebSocket Controller thread that listens on `0.0.0.0:5117`. The running Goose load test can be controlled through these Controllers. Goose can optionally be started with the `--no-autostart` run time option to prevent the load test from automatically starting, requiring instead that it be started with a Controller command. When Goose is started this way, a host is not required and can instead be configured via the Controller.

NOTE: The controller currently is not Gaggle-aware, and only functions correctly when running Goose as a single process in standalone mode.

### Telnet Controller

The host and port that the telnet Controller listens on can be configured at start time with `--telnet-host` and `--telnet-port`. The telnet Controller can be completely disabled with the `--no-telnet` command line option. The defaults can be changed with `GooseDefault::TelnetHost`,`GooseDefault::TelnetPort`, and `GooseDefault::NoTelnet`.

To learn about all available commands, telnet into the Controller thread and enter `help` (or `?`), for example:
```
% telnet localhost 5116
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'.
goose> ?
goose 0.12.0 controller commands:
 help (?)           this help
 exit (quit)        exit controller
 start              start an idle load test
 stop               stop a running load test and return to idle state
 shutdown           shutdown running load test (and exit controller)
 host HOST          set host to load test, ie http://localhost/
 users INT          set number of simulated users
 hatchrate FLOAT    set per-second rate users hatch
 runtime TIME       set how long to run test, ie 1h30m5s
 config             display load test configuration
 config-json        display load test configuration in json format
 metrics            display metrics for current load test
 metrics-json       display metrics for current load test in json format
goose>
```

### WebSocket Controller

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
