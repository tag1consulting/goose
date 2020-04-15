# Project Goose

Goose is a Rust load testing tool, based on Locust.

### Todo MVP

- [ ] TaskSequence  
  - [ ] add special-case 'on-start' that is always first in TaskSet  
  - [ ] allow weighting of tasks to always run in a given order  
  - [ ] add special-case 'on-exit' that is always last in TaskSet  
- [ ] --stop-timeout to gracefully exit client threads  
- [ ] turn Goose into a library, create a loadtest by creating an app with Cargo  
  - [ ] compare the pros/cons of this w/ going the dynamic library approach  
- [ ] automated testing of Goose logic

### In progress

- [x] POST request method helper  

### Future (post-MVP)

- [ ] async clients
- [ ] metaprogramming, impleent goose_codegen macros to simplify goosefile creation  
- [ ] detect terminal width and adjust statistics output (when wide enough collapse into a single table, etc)  
- [ ] more complicated wait_time implementations
  - [ ] constant pacing (https://github.com/locustio/locust/blob/795b5a14dd5b0991fec5a7f96f0d6491ce19e3d0/locust/wait_time.py#L30)
  - [ ] custom wait_time implementations
- [ ] documentation  
- [ ] website  
- [ ] gaggle support (distributed Geese)  
- [ ] web UI  

### Completed Column ✓

- [x] --list TaskSets and Tasks  
- [x] --log-level to increase debugging verbosity to log file  
- [x] --log-file to specify path and name of log file  
- [x] --verbose to increase debugging verbosity to stdout  
- [x] --print-stats to show statistics at end of load test  
- [x] --clients to specify number of concurrent clients to simulate  
- [x] --run-time to control how long load test runs  
- [x] weighting of TaskSets and Tasks  
- [x] spawn clients in threads  
- [x] move counters into a per-request HashMap instead of a per-Task Vector (currently limited to including only one request per task for accurate statistics)  
  - [x] remove per-Task atomic counters (rely instead on per-request statistics)  
- [x] --reset-stats to optionally reset stats after all threads have hatched  
- [x] GET request method helper  
  - [x] properly identify method in stats  
- [x] optionally track fine-grained per-request response codes (ie, GET /index.html: 5 200, 2 500)  
- [x] provide useful statistics at end of load-test  
  - [x] merge statistics from client threads into parent  
  - [x] response time calculations: min, max, average, mean  
  - [x] show total and per-second success and fail counts  
  - [x] include aggregated totals for all tasks/requests  
  - [x] break down percentage of requests within listed times for all tasks/requests  
  - [x] optionally provide running statistics  
  - [x] only sync client threads to parent when needing to display statistics  
  - [x] don't collect response time and other statistics if not displaying them  
  - [x] catch ctrl-c and exit gracefully, displaying statistics if enabled  
- [x] host configuration  
  - [x] -H --host cli option  
  - [x] host attribute  
- [x] wait_time attribute, configurable pause after each Task runs  
- [x] HEAD request method helper  
- [x] PUT request method helper  
- [x] PATCH request method helper  
- [x] DELETE request method helper  
