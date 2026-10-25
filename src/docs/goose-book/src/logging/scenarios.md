# Scenario Log

Goose can optionally log details about each time a scenario is run during a load test.  To enable, add the `--scenario-log <scenario.log>` command line option, where `<scenario.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`ScenarioMetric`](https://docs.rs/goose/*/goose/metrics/struct.ScenarioMetric.html) object which is created each time any scenario is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":20926,"name":"WebsiteUser","index":0,"run_time":20756,"user":1,"run_time_fraction_us":471}
{"elapsed":21402,"name":"WebsiteUser","index":0,"run_time":21064,"user":2,"run_time_fraction_us":471}
{"elapsed":22791,"name":"WebsiteUser","index":0,"run_time":22285,"user":3,"run_time_fraction_us":334}
{"elapsed":24811,"name":"WebsiteUser","index":0,"run_time":24810,"user":0,"run_time_fraction_us":432}
```

These lines are from the [`simple`](../example/simple.md) example. In the first line, `GooseUser` thread 1 ran the complete `WebsiteUser` scenario in 20,756.471 milliseconds, which includes the wait after each transaction. In the fourth line, `GooseUser` thread 0 ran it in 24,810.432 milliseconds.

The `--scenario-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`ScenarioMetric`](https://docs.rs/goose/*/goose/metrics/struct.ScenarioMetric.html) object, one per line, and `pretty` is the same output spread over several indented lines per scenario run.

For example, `csv` output of the same example starts with a header row and looks like:

```csv
elapsed,name,index,run_time,user,run_time_us
19283,WebsiteUser,0,18945,2,18945361
19724,WebsiteUser,0,18882,5,18882239
20468,WebsiteUser,0,20467,0,20467323
24027,WebsiteUser,0,23857,1,23857874
```

`run_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `run_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `run_time_us` column holding the whole time in microseconds.
