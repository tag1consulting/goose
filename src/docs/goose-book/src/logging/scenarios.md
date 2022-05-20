# Scneario Log

Goose can optionally log details about each time a scenario is run during a load test.  To enable, add the `--scenario-log <scenario.log>` command line option, where `<scenario.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`ScenarioMetric`](https://docs.rs/goose/*/goose/metrics/struct.ScenarioMetric.html) object which is created each time any scenario is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":15751,"index":0,"name":"AnonBrowsingUser","run_time":1287,"user":7}
{"elapsed":15756,"index":0,"name":"AnonBrowsingUser","run_time":1308,"user":4}
{"elapsed":15760,"index":0,"name":"AnonBrowsingUser","run_time":1286,"user":9}
{"elapsed":15783,"index":0,"name":"AnonBrowsingUser","run_time":1301,"user":0}
{"elapsed":22802,"index":1,"name":"AuthBrowsingUser","run_time":13056,"user":8}
```

In the first line of the above example, `GooseUser` thread 7 ran the complete `AnonBrowsingUser` scenario in 1,287 milliseconds. In the fifth line `GooseUser` thread 8 succesfully ran the `AuthBrowsingUser` transaction in 13,056 milliseconds.

The `--scenario-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire 
[`ScenarioMetric`](https://docs.rs/goose/*/goose/metrics/struct.ScenarioMetric.html) object.

For example, `csv` output of similar transactions as those logged above would like like:
```csv
elapsed,scenario_index,transaction_index,name,run_time,success,user
15751,AnonBrowsingUser,0,1287,7
15756,AnonBrowsingUser,0,1308,4
15760,AnonBrowsingUser,0,1286,9
15783,AnonBrowsingUser,0,1301,0
22802,AuthBrowsingUser,1,13056,8
```

# Gaggle Mode

When operating in Gaggle-mode, the `--scenario-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
