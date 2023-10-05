# Run-Time Options

The `-h` flag will show all run-time configuration options available to Goose load tests. For example, you can pass the `-h` flag to our example loadtest as follows, `cargo run --release -- -h`:

```ignore
Usage: target/release/loadtest [OPTIONS]

Goose is a modern, high-performance, distributed HTTP(S) load testing tool,
written in Rust. Visit https://book.goose.rs/ for more information.

The following runtime options are available when launching a Goose load test:

Optional arguments:
  -h, --help                  Displays this help
  -V, --version               Prints version information
  -l, --list                  Lists all transactions and exits

  -H, --host HOST             Defines host to load test (ie http://10.21.32.33)
  -u, --users USERS           Sets concurrent users (default: number of CPUs)
  -r, --hatch-rate RATE       Sets per-second user hatch rate (default: 1)
  -s, --startup-time TIME     Starts users for up to (30s, 20m, 3h, 1h30m, etc)
  -t, --run-time TIME         Stops load test after (30s, 20m, 3h, 1h30m, etc)
  -G, --goose-log NAME        Enables Goose log file and sets name
  -g, --log-level             Increases Goose log level (-g, -gg, etc)
  -q, --quiet                 Decreases Goose verbosity (-q, -qq, etc)
  -v, --verbose               Increases Goose verbosity (-v, -vv, etc)

Metrics:
  --running-metrics TIME      How often to optionally print running metrics
  --no-reset-metrics          Doesn't reset metrics after all users have started
  --no-metrics                Doesn't track metrics
  --no-transaction-metrics    Doesn't track transaction metrics
  --no-scenario-metrics       Doesn't track scenario metrics
  --no-print-metrics          Doesn't display metrics at end of load test
  --no-error-summary          Doesn't display an error summary
  --report-file NAME          Create an html-formatted report
  --no-granular-report        Disable granular graphs in report file
  -R, --request-log NAME      Sets request log file name
  --request-format FORMAT     Sets request log format (csv, json, raw, pretty)
  --request-body              Include the request body in the request log
  -T, --transaction-log NAME  Sets transaction log file name
  --transaction-format FORMAT Sets log format (csv, json, raw, pretty)
  -S, --scenario-log NAME     Sets scenario log file name
  --scenario-format FORMAT    Sets log format (csv, json, raw, pretty)
  -E, --error-log NAME        Sets error log file name
  --error-format FORMAT       Sets error log format (csv, json, raw, pretty)
  -D, --debug-log NAME        Sets debug log file name
  --debug-format FORMAT       Sets debug log format (csv, json, raw, pretty)
  --no-debug-body             Do not include the response body in the debug log
  --no-status-codes           Do not track status code metrics

Advanced:
  --test-plan "TESTPLAN"      Defines a more complex test plan ("10,60s;0,30s")
  --iterations ITERATIONS     Sets how many times to run scenarios then exit
  --scenarios "SCENARIO"      Limits load test to only specified scenarios
  --scenarios-list            Lists all scenarios and exits
  --no-telnet                 Doesn't enable telnet Controller
  --telnet-host HOST          Sets telnet Controller host (default: 0.0.0.0)
  --telnet-port PORT          Sets telnet Controller TCP port (default: 5116)
  --no-websocket              Doesn't enable WebSocket Controller
  --websocket-host HOST       Sets WebSocket Controller host (default: 0.0.0.0)
  --websocket-port PORT       Sets WebSocket Controller TCP port (default: 5117)
  --no-autostart              Doesn't automatically start load test
  --no-gzip                   Doesn't set the gzip Accept-Encoding header
  --timeout VALUE             Sets per-request timeout, in seconds (default: 60)
  --co-mitigation STRATEGY    Sets coordinated omission mitigation strategy
  --throttle-requests VALUE   Sets maximum requests per second
  --sticky-follow             Follows base_url redirect with subsequent requests
  --accept-invalid-certs      Disables validation of https certificates
```

All of the above configuration options are [defined in the developer documentation](https://docs.rs/goose/*/goose/config/struct.GooseConfiguration.html).