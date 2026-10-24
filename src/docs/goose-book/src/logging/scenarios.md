# Scneario Log

Goose can optionally log details about each time a scenario is run during a load test.  To enable, add the `--scenario-log <scenario.log>` command line option, where `<scenario.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`ScenarioMetric`](https://docs.rs/goose/*/goose/metrics/struct.ScenarioMetric.html) object which is created each time any scenario is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":16257,"name":"WebsiteUser","index":0,"run_time":13743,"user":5,"run_time_fraction_us":763}
{"elapsed":21584,"name":"WebsiteUser","index":0,"run_time":21082,"user":1,"run_time_fraction_us":281}
{"elapsed":22090,"name":"WebsiteUser","index":0,"run_time":20582,"user":3,"run_time_fraction_us":454}
{"elapsed":24129,"name":"WebsiteUser","index":0,"run_time":22119,"user":4,"run_time_fraction_us":5}
{"elapsed":25525,"name":"WebsiteUser","index":0,"run_time":25523,"user":0,"run_time_fraction_us":239}
```

These lines are from the [`simple`](../example/simple.md) example. In the first line, `GooseUser` thread 5 ran the complete `WebsiteUser` scenario in 13,743.763 milliseconds, which includes the wait after each transaction. In the fifth line, `GooseUser` thread 0 ran it in 25,523.239 milliseconds.

The `--scenario-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire 
[`ScenarioMetric`](https://docs.rs/goose/*/goose/metrics/struct.ScenarioMetric.html) object.

For example, `csv` output of the same example looks like:
```csv
elapsed,name,index,run_time,user,run_time_us
18645,WebsiteUser,0,18308,2,18308967
20690,WebsiteUser,0,19851,5,19851836
20747,WebsiteUser,0,20579,1,20579243
22749,WebsiteUser,0,22749,0,22749115
```

`run_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `run_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `run_time_us` column holding the whole time in microseconds.

# Gaggle Mode

When operating in Gaggle-mode, the `--scenario-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
