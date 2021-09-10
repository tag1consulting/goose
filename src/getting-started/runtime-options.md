# Run-Time Options

The `-h` flag will show all run-time configuration options available to Goose load tests. For example, you can pass the `-h` flag to our example loadtest as follows, `cargo run --release -- -h`:

```bash
Usage: target/release/loadtest [OPTIONS]

Options available when launching a Goose load test.


Optional arguments:
  -h, --help                 Displays this help
  -V, --version              Prints version information
  -l, --list                 Lists all tasks and exits

  -H, --host HOST            Defines host to load test (ie http://10.21.32.33)
  -u, --users USERS          Sets concurrent users (default: number of CPUs)
  -r, --hatch-rate RATE      Sets per-second user hatch rate (default: 1)
  -s, --startup-time TIME    Starts users for up to (30s, 20m, 3h, 1h30m, etc)
  -t, --run-time TIME        Stops load test after (30s, 20m, 3h, 1h30m, etc)
  -G, --goose-log NAME       Enables Goose log file and sets name
  -g, --log-level            Sets Goose log level (-g, -gg, etc)
  -v, --verbose              Sets Goose verbosity (-v, -vv, etc)

Metrics:
  --running-metrics TIME     How often to optionally print running metrics
  --no-reset-metrics         Doesn't reset metrics after all users have started
  --no-metrics               Doesn't track metrics
  --no-task-metrics          Doesn't track task metrics
  --no-error-summary         Doesn't display an error summary
  --report-file NAME         Create an html-formatted report
  -R, --request-log NAME     Sets request log file name
  --request-format FORMAT    Sets request log format (csv, json, raw, pretty)
  --request-body             Include the request body in the request log
  -T, --task-log NAME        Sets task log file name
  --task-format FORMAT       Sets task log format (csv, json, raw, pretty)
  -E, --error-log NAME       Sets error log file name
  --error-format FORMAT      Sets error log format (csv, json, raw, pretty)
  -D, --debug-log NAME       Sets debug log file name
  --debug-format FORMAT      Sets debug log format (csv, json, raw, pretty)
  --no-debug-body            Do not include the response body in the debug log
  --status-codes             Tracks additional status code metrics

Advanced:
  --no-telnet                Doesn't enable telnet Controller
  --telnet-host HOST         Sets telnet Controller host (default: 0.0.0.0)
  --telnet-port PORT         Sets telnet Controller TCP port (default: 5116)
  --no-websocket             Doesn't enable WebSocket Controller
  --websocket-host HOST      Sets WebSocket Controller host (default: 0.0.0.0)
  --websocket-port PORT      Sets WebSocket Controller TCP port (default: 5117)
  --no-autostart             Doesn't automatically start load test
  --co-mitigation STRATEGY   Sets coordinated omission mitigation strategy
  --throttle-requests VALUE  Sets maximum requests per second
  --sticky-follow            Follows base_url redirect with subsequent requests

Gaggle:
  --manager                  Enables distributed load test Manager mode
  --expect-workers VALUE     Sets number of Workers to expect
  --no-hash-check            Tells Manager to ignore load test checksum
  --manager-bind-host HOST   Sets host Manager listens on (default: 0.0.0.0)
  --manager-bind-port PORT   Sets port Manager listens on (default: 5115)
  --worker                   Enables distributed load test Worker mode
  --manager-host HOST        Sets host Worker connects to (default: 127.0.0.1)
  --manager-port PORT        Sets port Worker connects to (default: 5115)
```

All of the above configuration options are [defined in the developer documentation](https://docs.rs/goose/*/goose/config/struct.GooseConfiguration.html).