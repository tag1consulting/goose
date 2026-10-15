# Telnet Controller

The host and port that the telnet Controller listens on can be configured at start time with `--telnet-host` and `--telnet-port`. The telnet Controller can be completely disabled with the `--no-telnet` command line option. The defaults can be changed with [`GooseDefault::TelnetHost`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.TelnetHost),[`GooseDefault::TelnetPort`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.TelnetPort), and [`GooseDefault::NoTelnet`](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.NoTelnet).

## Controller Commands

To learn about all available commands, telnet into the Controller thread and enter `help` (or `?`). For example:
```bash
% telnet localhost 5116
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'.
goose> ?
goose 0.17.2 controller commands:
help               this help
exit               exit controller

start              start an idle load test
stop               stop a running load test and return to idle state
shutdown           shutdown load test and exit controller

host HOST          set host to load test, (ie https://web.site/)
increaserate FLOAT set per-second rate users increase
increase-time TIME set total time to take increasing users
decreaserate FLOAT set per-second rate users decrease
decrease-time TIME set total time to take decreasing users
users INT          set number of simulated users (1 to 100000)
runtime TIME       set how long to run test, (ie 1h30m5s)
test-plan PLAN     define or replace test-plan, (ie 10,5m;10,1h;0,30s)

config             display load test configuration
config-json        display load test configuration in json format
metrics            display metrics for current load test
metrics-json       display metrics for current load test in json format
goose> q
goodbye!
goose> Connection closed by foreign host.
```

A command whose value can't be used is refused with what the value must be, and changes nothing:
```bash
goose> users 0
users must be an integer between 1 and 100000
goose> increaserate 0
increase_rate must be a number of at least 0.0000116 (one user a day)
goose> runtime 10d
run_time must be a time span such as 1h30m5s
```

`host`, `runtime`, `increase-time` and `decrease-time` are accepted only while the load test is idle, as they are read when it starts. `users` and `test-plan` also change a running load test. The rates are accepted at any time and stored: the next `users` change or Start uses them, so on a running load test set a rate, then `users`. A rate of at least one user a day (0.0000116 a second) is accepted. Setting a rate clears a configured test plan, and the reply then says so: `increase_rate configured, test plan cleared`. A test plan must start 1 to 100000 users in total, and `users` and `test-plan` are refused while a `stop`, `shutdown` or Ctrl-C is ramping the load test down. A command that gets no answer within 15 seconds is reported as timed out, and may still apply.

## Example

One possible use-case for the controller is to dynamically reconfigure the number of users being simulated by the load test. In the following example, the load test was launched with the following parameters:

```bash
% cargo run --release --example umami -- --no-autostart --host https://umami.ddev.site/ --increase-rate 50 --report-file report.html
```

Then the telnet controller is invoked as follows:
```bash
% telnet loadtest 5116
Trying loadtest...
Connected to loadtest.
Escape character is '^]'.
goose> start 
load test started
goose> users 20
users configured
goose> users 40
users configured
goose> users 80
users configured
goose> users 40 
users configured
goose> users 20
users configured
goose> users 160
users configured
goose> users 20
users configured
goose> increase_rate 5
increase_rate configured
goose> users 80
users configured
goose> users 20
users configured
goose> shutdown
load test shut down
goose> Connection closed by foreign host.
```

Initially the load test is configured with an increase rate of 50, so goose increases and decreases the user count by 50 user threads per second. Later we reconfigure the increase rate to 5, slowing down the rate that goose alters the number of user threads. The result is more clearly illustrated in the following graph generated at the end of the above example load test:

![Controller dynamic users and increase rate](controller-users.png)

The above commands are also summarized in the metrics overview:

```ignore
 === OVERVIEW ===
 ------------------------------------------------------------------------------
 Action       Started               Stopped             Elapsed    Users
 ------------------------------------------------------------------------------
 Increasing:  2022-05-05 07:09:34 - 2022-05-05 07:09:34 (00:00:00, 0 -> 10)
 Maintaining: 2022-05-05 07:09:34 - 2022-05-05 07:09:40 (00:00:06, 10)
 Increasing:  2022-05-05 07:09:40 - 2022-05-05 07:09:40 (00:00:00, 10 -> 20)
 Maintaining: 2022-05-05 07:09:40 - 2022-05-05 07:09:46 (00:00:06, 20)
 Increasing:  2022-05-05 07:09:46 - 2022-05-05 07:09:47 (00:00:01, 20 -> 40)
 Maintaining: 2022-05-05 07:09:47 - 2022-05-05 07:09:50 (00:00:03, 40)
 Increasing:  2022-05-05 07:09:50 - 2022-05-05 07:09:51 (00:00:01, 40 -> 80)
 Maintaining: 2022-05-05 07:09:51 - 2022-05-05 07:09:59 (00:00:08, 80)
 Decreasing:  2022-05-05 07:09:59 - 2022-05-05 07:10:00 (00:00:01, 40 <- 80)
 Maintaining: 2022-05-05 07:10:00 - 2022-05-05 07:10:05 (00:00:05, 40)
 Decreasing:  2022-05-05 07:10:05 - 2022-05-05 07:10:06 (00:00:01, 20 <- 40)
 Maintaining: 2022-05-05 07:10:06 - 2022-05-05 07:10:12 (00:00:06, 20)
 Increasing:  2022-05-05 07:10:12 - 2022-05-05 07:10:15 (00:00:03, 20 -> 160)
 Maintaining: 2022-05-05 07:10:15 - 2022-05-05 07:10:19 (00:00:04, 160)
 Decreasing:  2022-05-05 07:10:19 - 2022-05-05 07:10:22 (00:00:03, 20 <- 160)
 Maintaining: 2022-05-05 07:10:22 - 2022-05-05 07:10:35 (00:00:13, 20)
 Increasing:  2022-05-05 07:10:35 - 2022-05-05 07:10:50 (00:00:15, 20 -> 80)
 Maintaining: 2022-05-05 07:10:50 - 2022-05-05 07:10:54 (00:00:04, 80)
 Decreasing:  2022-05-05 07:10:54 - 2022-05-05 07:11:07 (00:00:13, 20 <- 80)
 Maintaining: 2022-05-05 07:11:07 - 2022-05-05 07:11:13 (00:00:06, 20)
 Canceling:   2022-05-05 07:11:13 - 2022-05-05 07:11:13 (00:00:00, 0 <- 20)

 Target host: https://umami.ddev.site/
 goose v0.18.1
 ------------------------------------------------------------------------------
```
