# Transaction Log

Goose can optionally log details about each time a transaction is run during a load test.  To enable, add the `--transaction-log <transaction.log>` command line option, where `<transaction.log>` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

Logs include the entire [`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object which is created each time any transaction is run.

## Log Format

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":22060,"name":"(Anon) front page","run_time":97,"success":true,"transaction_index":0,"scenario_index":0,"user":0}
{"elapsed":22118,"name":"(Anon) node page","run_time":41,"success":true,"transaction_index":1,"scenario_index":0,"user":5}
{"elapsed":22157,"name":"(Anon) node page","run_time":6,"success":true,"transaction_index":1,"scenario_index":0,"user":0}
{"elapsed":22078,"name":"(Auth) front page","run_time":109,"success":true,"transaction_index":1,"scenario_index":1,"user":6}
{"elapsed":22157,"name":"(Anon) user page","run_time":35,"success":true,"transaction_index":2,"scenario_index":0,"user":4}
```

In the first line of the above example, `GooseUser` thread 0 succesfully ran the `(Anon) front page` transaction in 97 milliseconds. In the second line `GooseUser` thread 5 succesfully ran the `(Anon) node page` transaction in 41 milliseconds.

The `--transaction-format` option can be used to log in `csv`, `json` (default), `raw` or `pretty` format. The `raw` format is Rust's debug output of the entire 
[`TransactionMetric`](https://docs.rs/goose/*/goose/metrics/struct.TransactionMetric.html) object.

For example, `csv` output of similar transactions as those logged above would like like:
```csv
elapsed,scenario_index,transaction_index,name,run_time,success,user
21936,0,0,"(Anon) front page",83,true,0
21990,1,3,"(Auth) user page",34,true,1
21954,0,0,"(Anon) front page",84,true,5
22009,0,1,"(Anon) node page",34,true,2
21952,0,0,"(Anon) front page",95,true,7
```

# Gaggle Mode

When operating in Gaggle-mode, the `--transaction-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.
