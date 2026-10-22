# Transaction Log

Goose can optionally log details about each time a transaction is run during a load test.  To enable, add the `--transaction-log <transaction.log>` command line option, where `<transaction.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object which is created each time any transaction is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":837,"name":"","run_time":0,"run_time_fraction_us":921,"scenario_index":0,"success":true,"transaction_index":0,"user":5}
{"elapsed":838,"name":"","run_time":0,"run_time_fraction_us":356,"scenario_index":0,"success":true,"transaction_index":1,"user":5}
{"elapsed":5763,"name":"","run_time":1441,"run_time_fraction_us":862,"scenario_index":0,"success":true,"transaction_index":2,"user":2}
{"elapsed":5880,"name":"","run_time":1454,"run_time_fraction_us":681,"scenario_index":0,"success":true,"transaction_index":2,"user":0}
{"elapsed":7178,"name":"","run_time":1400,"run_time_fraction_us":259,"scenario_index":0,"success":true,"transaction_index":2,"user":1}
```

These lines are from the [`simple`](../example/simple.md) example, whose transactions have no names. In the first line, `GooseUser` thread 5 successfully ran transaction 0, which logs in, in 921 microseconds. In the third line, `GooseUser` thread 2 successfully ran transaction 2, which loads `/about/`, in 1,441.862 milliseconds.

The `--transaction-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire 
[`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object.

For example, `csv` output of the same example looks like:
```csv
elapsed,scenario_index,transaction_index,name,run_time,success,user,run_time_us
838,0,0,,0,true,5,596
838,0,1,,0,true,5,267
8355,0,2,,1322,true,1,1322171
8318,0,2,,1405,true,0,1405552
9700,0,2,,1368,true,2,1368577
```

`run_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `run_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `run_time_us` column holding the whole time in microseconds.

# Gaggle Mode

When operating in Gaggle-mode, the `--transaction-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
