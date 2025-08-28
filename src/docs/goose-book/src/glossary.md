# Glossary

## Controller
An interface that allows real-time control of a running Goose load test. Goose provides both [Telnet](./controller/telnet.html) and [WebSocket](./controller/websocket.html) controllers for dynamically adjusting test parameters like user count, hatch rate, and runtime during execution.

## Coordinated Omission
A phenomenon that occurs in load testing when the measurement system inadvertently excludes the results of requests that were affected by server slowdowns, leading to misleadingly optimistic performance metrics. Goose includes [Coordinated Omission Mitigation](./coordinated-omission/mitigation.html) functionality to detect and correct for this.

## Gaggle
Goose's distributed load testing functionality that allows running coordinated load tests across multiple machines. A Gaggle consists of one Manager and multiple Workers. **Note:** Gaggle support was temporarily removed in Goose 0.17.0.

## GooseAttack
A load test defined by one or more [Scenarios](#scenario) with one or more [Transactions](#transaction).

## GooseConfiguration
A structure that defines all configuration options for a Goose load test, including user count, hatch rate, runtime, host, and various other parameters. Can be set via command line arguments, configuration files, or programmatically.

## GooseError
A helper that defines all possible errors returned by Goose. A [Transaction](#transaction) returns a [TransactionResult](#transactionresult), which is either [`Ok(())`](https://doc.rust-lang.org/std/result/enum.Result.html#variant.Ok) or [`Err(TransactionError)`](https://doc.rust-lang.org/std/result/enum.Result.html#variant.Err).

## GooseUser
A thread that repeatedly runs a single [**scenario**](./getting-started/metrics.html#scenarios) for the duration of the load test. For example, when Goose starts, you may use the [`--users`](./getting-started/common.html#how-many-users-to-simulate) command line option to configure how many GooseUser threads are started. This is not intended to be a 1:1 correlation between GooseUsers and real website users.

## Hatch Rate
The rate at which new [GooseUsers](#gooseuser) are launched during the ramp-up phase of a load test, typically specified as users per second.

## Request
A single [**request**](./getting-started/metrics.html#requests) based around HTTP verbs.

## Scenario
A [**scenario**](./getting-started/metrics.html#scenarios) is a collection of [**transactions**](./getting-started/metrics.html#transactions) (aka steps) a user would undertake to achieve a specific user journey.

## Test Plan
A flexible approach to scheduling load test phases, allowing you to define complex load patterns like gradual ramp-up, sustained load periods, spike testing, and graceful ramp-down. Test plans use the format `users,duration;users,duration` to specify multiple phases.

## Throttle
A mechanism to limit the request rate of individual [GooseUsers](#gooseuser), helping simulate more realistic user behavior by introducing delays between requests rather than sending requests as fast as possible.

## Transaction
A [**transaction**](./getting-started/metrics.html#transactions) is a collection of one or more [**requests**](./getting-started/metrics.html#request) and any desired validation. For example, this may include loading the front page and all contained static assets, logging into the website, or adding one or more items to a shopping chart. Transactions typically include assertions or expectation validation.

## TransactionResult
A [`Result`](https://doc.rust-lang.org/std/result/enum.Result.html) returned by [Transaction](#transaction) functions. A transaction can return `Ok(())` on success, or `Err(TransactionError)` on failure. The `TransactionError::Custom` variant allows returning custom error messages using string literals or formatted strings with `.into()`.

## Weight
A value that controls the frequency with which a [Transaction](#transaction) or [Scenario](#scenario) runs, relative to the other transactions in the same scenario, or scenarios in the same load test. For example, if one transaction has a weight of 3 and another transaction in the same scenario has a weight of 1, the first transaction will run 3 times as often as the second.
