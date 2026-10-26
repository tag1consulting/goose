# Transaction Log

Goose can optionally log details about each time a transaction is run during a load test.  To enable, add the `--transaction-log <transaction.log>` command line option, where `<transaction.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object which is created each time any transaction is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":0,"scenario_index":0,"transaction_index":0,"name":"","run_time":0,"success":true,"user":0,"run_time_fraction_us":334,"error":""}
{"elapsed":0,"scenario_index":0,"transaction_index":1,"name":"","run_time":0,"success":true,"user":0,"run_time_fraction_us":92,"error":""}
{"elapsed":6172,"scenario_index":0,"transaction_index":2,"name":"","run_time":1,"success":true,"user":0,"run_time_fraction_us":41,"error":""}
{"elapsed":8600,"scenario_index":0,"transaction_index":2,"name":"","run_time":0,"success":true,"user":2,"run_time_fraction_us":257,"error":""}
```

These lines are from the [`simple`](../example/simple.md) example, whose transactions have no names, run with `-u 4 -r 4 -t 12s` against a local server. In the first line, `GooseUser` thread 0 successfully ran transaction 0, the login, in 334 microseconds. In the third line, the same thread successfully ran transaction 2, the `/about/` page, in 1.041 milliseconds.

Each line ends with an `error` field. It is empty when the transaction succeeded. When the transaction returned an error, `success` is `false` and `error` holds the error as its `Display` writes it. This line is from a load test whose `flaky` transaction returns `TransactionError::Custom("every third call fails")` one time in three:

```json
{"elapsed":0,"scenario_index":0,"transaction_index":2,"name":"flaky","run_time":0,"success":false,"user":0,"run_time_fraction_us":68,"error":"TransactionError: custom error (every third call fails)"}
```

A transaction that failed because one of its requests failed names the request error in `error`, as `request failed: <method> <name>: <error>`; the request error itself is counted in the [errors table](../getting-started/metrics.md) and the [error log](errors.md). A transaction canceled as the run ended has no error. A transaction that returns any other error, such as `TransactionError::Custom`, is counted, by scenario, transaction and error, in the Transaction Errors tables of the [metrics](../getting-started/metrics.md) and reports, and is written to the [error log](errors.md).

The `--transaction-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire [`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object, one per line, and `pretty` is the same output spread over several indented lines per transaction. In each of them the `error` field comes last:

```text
TransactionMetric { elapsed: 2, scenario_index: 0, transaction_index: 2, name: "flaky", run_time: 0, success: false, user: 0, run_time_fraction_us: 101, error: "TransactionError: custom error (every third call fails)" }
```

For example, `csv` output of the `simple` example starts with a header row and looks like:

```csv
elapsed,scenario_index,transaction_index,name,run_time,success,user,run_time_us,error
0,0,0,,0,true,0,455,
0,0,1,,0,true,0,143,
251,0,0,,0,true,1,471,
251,0,1,,0,true,1,163,
7872,0,2,,0,true,1,220,
```

The `error` column is last, and is empty for a transaction that succeeded. A failed transaction fills it:

```csv
0,0,2,flaky,0,false,0,77,TransactionError: custom error (every third call fails)
```

`run_time` is in whole milliseconds, truncated. The `json`, `raw` and `pretty` formats also carry `run_time_fraction_us`, the microseconds past those milliseconds (0 to 999), and the `csv` format has a `run_time_us` column, before `error`, holding the whole time in microseconds.
