# Transaction Log

Goose can optionally log details about each time a transaction is run during a load test.  To enable, add the `--transaction-log <transaction.log>` command line option, where `<transaction.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object which is created each time any transaction is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":504,"scenario_index":0,"transaction_index":0,"name":"","run_time":1,"success":true,"user":3,"run_time_fraction_us":253}
{"elapsed":506,"scenario_index":0,"transaction_index":1,"name":"","run_time":1,"success":true,"user":3,"run_time_fraction_us":221}
{"elapsed":6875,"scenario_index":0,"transaction_index":2,"name":"","run_time":0,"success":true,"user":2,"run_time_fraction_us":650}
{"elapsed":7982,"scenario_index":0,"transaction_index":2,"name":"","run_time":0,"success":true,"user":3,"run_time_fraction_us":737}
```

These lines are from the [`simple`](../example/simple.md) example, whose transactions have no names. In the first line, `GooseUser` thread 3 successfully ran transaction 0, the login, in 1.253 milliseconds. In the third line, `GooseUser` thread 2 successfully ran transaction 2, the `/about/` page, in 650 microseconds.

The `--transaction-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object, one per line, and `pretty` is the same output spread over several indented lines per transaction.

For example, `csv` output of the same example starts with a header row and looks like:

```csv
elapsed,scenario_index,transaction_index,name,run_time,success,user,run_time_us
0,0,0,,0,true,0,808
0,0,1,,0,true,0,194
169,0,0,,0,true,1,556
10177,0,2,,0,true,2,637
```

`run_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `run_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format ends with a `run_time_us` column holding the whole time in microseconds.
