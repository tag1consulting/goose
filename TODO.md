# Project Goose

Goose is a Rust load testing tool, based on Locust.

### Todo MVP

- [ ] POST request method helper  
- [ ] HEAD request method helper  
- [ ] move counters into a per-request HashMap instead of a per-Task Vector (currently limited to including only one request per task for accurate statistics)  
  - [ ] remove per-Task atomic counters (rely instead on per-request statistics)  
- [ ] optionally track fine-grained per-request response codes (ie, GET /index.html: 5 200, 2 500)  
- [ ] wait_time attribute, configurable pause after each Task runs  
- [ ] host configuration  
  - [ ] -H --host cli option  
  - [ ] host attribute  
- [ ] TaskSequence  
  - [ ] add special-case 'on-start' that is always first in TaskSet  
  - [ ] allow weighting of tasks to always run in a given order  
  - [ ] add special-case 'on-exit' that is always last in TaskSet  
- [ ] --stop-timeout to gracefully exit client threads  
- [ ] --reset-stats to optionally reset stats after all threads have hatched  
- [ ] automated testing  

### In progress

- [ ] provide useful statistics at end of load-test  
  - [x] merge statistics from client threads into parent  
  - [x] response time calculations: min, max, average, mean  
  - [x] show total and per-second success and fail counts  
  - [ ] include aggregated totals for all tasks/requests  
  - [ ] break down percentage of requests within listed times for all tasks/requests  
  - [ ] optionally provide running statistics  
  - [ ] only sync client threads to parent when needing to display statistics  
  - [ ] don't collect response time and other statistics if not displaying them  
  - [ ] detect terminal width and adjust what is displayed (when wide enough collapse into a single table)  
  - [ ] catch ctrl-c and exit gracefully, displaying statistics if enabled  
- [ ] GET request method helper  
  - [ ] properly identify method in stats  

### Future (post-MVP)

- [ ] PUT request method helper  
- [ ] PATCH request method helper  
- [ ] DELETE request method helper  
- [ ] turn Goose into a library, create a loadtest by creating an app with Cargo  
  - [ ] compare the pros/cons of this w/ going the dynamic library approach  
- [ ] metaprogramming, impleent goose_codegen macros to simplify goosefile creation  
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
