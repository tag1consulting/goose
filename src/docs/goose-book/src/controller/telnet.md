# Telnet Controller

The host and port that the telnet Controller listens on can be configured at start time with `--telnet-host` and `--telnet-port`. The telnet Controller can be completely disabled with the `--no-telnet` command line option. The defaults can be changed with `GooseDefault::TelnetHost`,`GooseDefault::TelnetPort`, and `GooseDefault::NoTelnet`.

To learn about all available commands, telnet into the Controller thread and enter `help` (or `?`), for example:
```bash
% telnet localhost 5116
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'.
goose> ?
goose 0.14.4 controller commands:
 help (?)           this help
 exit (quit)        exit controller
 start              start an idle load test
 stop               stop a running load test and return to idle state
 shutdown           shutdown running load test (and exit controller)
 host HOST          set host to load test, ie http://localhost/
 users INT          set number of simulated users
 hatchrate FLOAT    set per-second rate users hatch
 runtime TIME       set how long to run test, ie 1h30m5s
 config             display load test configuration
 config-json        display load test configuration in json format
 metrics            display metrics for current load test
 metrics-json       display metrics for current load test in json format
goose>
```

