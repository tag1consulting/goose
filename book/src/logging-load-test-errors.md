# Logging Load Test Errors

Goose can optionally log details about all load test errors to a file. To enable, add the `--error-file=error.log` command line option, where `error.log` is either a relative or absolute path of the log file to create. Any existing file that may already exist will be overwritten.

When operating in Gaggle-mode, the `--error-file` option can only be enabled on the Worker processes, configuring Goose to spread out the overhead of writing logs.

By default, logs are written in JSON Lines format. For example:

```json
{"elapsed":2239,"error":"503 Service Unavailable: /comment/reply/8151","final_url":"http://apache/comment/reply/8151","method":"Post","name":"(Auth) comment form","redirected":false,"response_time":26,"status_code":503,"url":"http://apache/comment/reply/8151","user":1}
{"elapsed":2261,"error":"503 Service Unavailable: /node/9577","final_url":"http://apache/node/9577","method":"Get","name":"(Anon) node page","redirected":false,"response_time":143,"status_code":503,"url":"http://apache/node/9577","user":2}
{"elapsed":2267,"error":"503 Service Unavailable: /","final_url":"http://apache/","method":"Get","name":"(Auth) front page","redirected":false,"response_time":138,"status_code":503,"url":"http://apache/","user":1}
{"elapsed":2404,"error":"503 Service Unavailable: /user/4375","final_url":"http://apache/user/4375","method":"Get","name":"(Anon) user page","redirected":false,"response_time":5,"status_code":503,"url":"http://apache/user/4375","user":2}
```

Logs include the entire [`GooseErrorMetric`] object as defined in `src/goose.rs`, which are created when requests result in an error.

By default Goose logs errors in JSON Lines format. The `--errors-format` option can be used to log in `csv`, `json` or `raw` format. The `raw` format is Rust's debug output of the entire [`GooseErrorMetric`] object.

For example, `csv` output of similar errors as those logged above would like like:
```csv
elapsed,method,name,url,final_url,redirected,response_time,status_code,user,error
6250,GET,"(Auth) node page","http://apache/node/3781","http://apache/node/3781",false,5,503,1,"503 Service Unavailable: /node/3781"
6256,GET,"(Auth) front page","http://apache/","http://apache/",false,5,503,1,"503 Service Unavailable: /"
6262,GET,"(Auth) node page","http://apache/node/5452","http://apache/node/5452",false,8,503,1,"503 Service Unavailable: /node/5452"
6265,GET,"(Anon) node page","http://apache/node/1819","http://apache/node/1819",false,5,503,0,"503 Service Unavailable: /node/1819"
```
