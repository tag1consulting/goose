# Logging Load Test Tasks

Goose can optionally log details about all load test tasks to a file. To enable, add the `--task-log=task.log` command line option, where `task.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--task-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":22060,"name":"(Anon) front page","run_time":97,"success":true,"task_index":0,"taskset_index":0,"user":0}
{"elapsed":22118,"name":"(Anon) node page","run_time":41,"success":true,"task_index":1,"taskset_index":0,"user":5}
{"elapsed":22157,"name":"(Anon) node page","run_time":6,"success":true,"task_index":1,"taskset_index":0,"user":0}
{"elapsed":22078,"name":"(Auth) front page","run_time":109,"success":true,"task_index":1,"taskset_index":1,"user":6}
{"elapsed":22157,"name":"(Anon) user page","run_time":35,"success":true,"task_index":2,"taskset_index":0,"user":4}
```

Logs include the entire [`GooseTaskMetric`] object as defined in `src/goose.rs`, which are created each time any task is run.

In the first line of the above example, `GooseUser` thread 0 succesfully ran the `(Anon) front page` task in 97 milliseconds. In the second line `GooseUser` thread 5 succesfully ran the `(Anon) node page` task in 41 milliseconds.

By default Goose logs tass in JSON Lines format. The `--task-format` option can be used to log in `csv`, `json` or `raw` format. The `raw` format is Rust's debug output of the entire [`GooseTaskMetric`] object.

For example, `csv` output of similar tasks as those logged above would like like:
```csv
elapsed,taskset_index,task_index,name,run_time,success,user
21936,0,0,"(Anon) front page",83,true,0
21990,1,3,"(Auth) user page",34,true,1
21954,0,0,"(Anon) front page",84,true,5
22009,0,1,"(Anon) node page",34,true,2
21952,0,0,"(Anon) front page",95,true,7
```