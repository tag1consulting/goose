# Test Plan

The simplest way to configure a Goose load test is with the `--startup-time` or `--hatch-rate` options together with `--users` and `--run-time`. However, if you need a more complex test plan, you can instead use the `--test-plan` option.

A test plan is a series of integer pairs which define the number of users and number of seconds. For example, "10,60" means "10 users" for "60 seconds". Multiple pairs can be strung together if separated by a semicolon, for example, "10,60;10,300". In this example, Goose will take 60 seconds to launch 10 users, then it will leave those users running for 5 minutes, before shutting down.

## Simple Example

The following two examples are identical:
```bash
$ cargo run --release -- -H http://local.dev/ --test-plan "10,60;10,300"
```

```bash
$ cargo run --release -- -H http://local.dev/ --startup-time 60 --users 10 --run-time 300
```

## Complex Example

The next example shows how to craft a more complex test plan. It tells Goose to spend 60 seconds launching 10 users and then to let them run for 5 minutes, then to increase the user count to 1,000 in 30 seconds and to let them run for another minute, then to decrease back to 10 users again in 30 seconds leaving those 10 users running for another 5 minutes, and finally to spend 90 seconds shutting down the load test.

```bash
$ cargo run --release -- -H http://local.dev/ --test-plan "10,60;10,300;1000,30;1000,60;10,30;10,300;0,90"
```
