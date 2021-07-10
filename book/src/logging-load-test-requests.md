# Logging Load Test Requests

Goose can optionally log details about all load test requests to a file. To enable, add the `--request-log=request.log` command line option, where `request.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--request-log` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"coordinated_omission_elapsed":0,"elapsed":23189,"error":"","final_url":"http://apache/misc/drupal.js?q9apdy","method":"Get","name":"static asset","redirected":false,"response_time":8,"status_code":200,"success":true,"update":false,"url":"http://apache/misc/drupal.js?q9apdy","user":5,"user_cadence":0}
{"coordinated_omission_elapsed":0,"elapsed":23192,"error":"","final_url":"http://apache/misc/jquery.once.js?v=1.2","method":"Get","name":"static asset","redirected":false,"response_time":6,"status_code":200,"success":true,"update":false,"url":"http://apache/misc/jquery.once.js?v=1.2","user":6,"user_cadence":0}
{"coordinated_omission_elapsed":0,"elapsed":23181,"error":"","final_url":"http://apache/misc/jquery-extend-3.4.0.js?v=1.4.4","method":"Get","name":"static asset","redirected":false,"response_time":16,"status_code":200,"success":true,"update":false,"url":"http://apache/misc/jquery-extend-3.4.0.js?v=1.4.4","user":1,"user_cadence":0}
```

Logs include the entire [`GooseRequestMetric`] object as defined in `src/goose.rs`, which are created on all requests.

In the first line of the above example, `GooseUser` thread 7 made a successful `GET` request for `/misc/feed.png`, which takes 4 milliseconds. The second line is `GooseUser` thread 2 making a successful `GET` request for `/user/4816`, which takes 28 milliseconds.

By default Goose logs requests in JSON Lines format. The `--request-format` option can be used to log in `csv`, `json` or `raw` format. The `raw` format is Rust's debug output of the entire [`GooseRequestMetric`] object.

For example, `csv` output of similar requests as those logged above would like like:
```csv
elapsed,method,name,url,final_url,redirected,response_time,status_code,success,update,user,error,coordinated_omission_elapsed,user_cadence
22143,GET,"(Anon) user page","http://apache/user/4","http://apache/user/4",false,25,200,true,false,3,,0,0
22153,GET,"static asset","http://apache/misc/jquery-extend-3.4.0.js?v=1.4.4","http://apache/misc/jquery-extend-3.4.0.js?v=1.4.4",false,16,200,true,false,6,,0,0
22165,GET,"static asset","http://apache/misc/jquery.js?v=1.4.4","http://apache/misc/jquery.js?v=1.4.4",false,3,200,true,false,0,,0,0
22165,GET,"static asset","http://apache/misc/feed.png","http://apache/misc/feed.png",false,4,200,true,false,1,,0,0
```