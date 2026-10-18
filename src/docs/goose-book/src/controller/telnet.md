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
increaserate FLOAT set per-second rate users increase (0.01 to 100000)
increase-time TIME set total time to take increasing users (0 clears)
decreaserate FLOAT set per-second rate users decrease (0.01 to 100000)
decrease-time TIME set total time to take decreasing users (0 clears)
users INT          set number of simulated users (1 to 100000)
runtime TIME       set how long to hold users after the ramp up, (ie 1h30m5s, 0 clears)
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
increase_rate must be a number from 0.01 to 100000
goose> runtime 31d
run_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear
```

Telnet, WebSocket and the dashboard share these bounds:

| Setting | Accepted |
|---------|----------|
| `host` | An `http://` or `https://` url with no user name or password, which would show to observers and in reports |
| `users` | 1 to 100000 |
| `increaserate`, `decreaserate` | 0.01 to 100000 users per second |
| `increase-time`, `decrease-time`, `runtime` | 1 second to 30 days; `0` clears the setting, so Goose's default applies |
| `test-plan` | At most 100 steps of 0 seconds to 30 days each, in at most 4096 bytes, starting 1 to 100000 users in total |

`host`, `increase-time` and `decrease-time` are accepted only while the load test is idle, as they are read when it starts. `runtime`, `users` and `test-plan` also change a running load test. A `runtime` on a running users ramp moves the end of the hold in progress to its start plus the new run time, and starts the ramp down at once if that has passed; before the hold starts it replaces the hold ahead, and on a run holding until stopped it holds from now for the new run time, then ramps down. `runtime 0` on a running test holds until stopped, and is refused while a ramp down is configured. `runtime` is refused during the ramp down, while stopping and while a test plan runs, as the plan sets how long it runs. See [Run time on a running test](dashboard.md#run-time-on-a-running-test). A `users` change on a running users ramp keeps its run time and ramp down: it ramps to the new count, then holds until the hold's end and ramps down for the new count, and during the ramp down it abandons the ramp down and holds the new count until stopped. `users` is refused while a test plan runs, as the plan sets users. See [Users on a running test](dashboard.md#users-on-a-running-test).

The rates are accepted in Idle and on a running users ramp, and are refused during its ramp down, while stopping and while a test plan runs. On a running test, a rate is refused while its own time is set, since a time changes only in Idle; in Idle a rate clears its own time and a time clears its own rate. A decrease rate on a running users ramp regenerates the ramp down still ahead, or adds one where all users would stop at once when the hold ends; an increase rate is for the next `users` increase.

A test plan sets users, so while one is configured a rate or a time is refused with `set users first to switch to a users ramp`: `users` switches to a users ramp and clears the plan, and `test-plan` switches back and clears users, the rates and the times. The command line's rules also apply: `runtime` and `test-plan` are refused under `--iterations`, `test-plan` under `--no-reset-metrics`, and in Idle `increase-time` or `decrease-time` with fewer than 2 users, or `users 1` while either is set. `users` and `test-plan` are refused while a `stop`, `shutdown` or Ctrl-C is ramping the load test down. A command that gets no answer within 15 seconds is reported as timed out, and may still apply.

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
