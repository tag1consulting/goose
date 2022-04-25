# Scheduling Scenarios And Transactions

When starting a load test, Goose assigns one [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) to each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) thread. By default, it assigns [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) (and then [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) within the scenario) in a round robin order. As new [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) threads are launched, the first will be assigned the first defined [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html), the next will be assigned the next defined [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html), and so on, looping through all available [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html). Weighting is respected during this process, so if one [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) is weighted heavier than others, that [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) will get assigned to [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) more at the end of the launching process.

The [`GooseScheduler`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html) can be configured to instead launch [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) and [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) in a [`Serial`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Serial) or a [`Random order`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Random). When configured to allocate in a [`Serial`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Serial) order, [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) and [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) are launched in the extact order they are defined in the load test (see below for more detail on how this works). When configured to allocate in a [`Random`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Random) order, running the same load test multiple times can lead to different amounts of load being generated.

Prior to Goose `0.10.6` [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) were allocated in a serial order. Prior to Goose `0.11.1` [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) were allocated in a serial order. To restore the old behavior, you can use the [`GooseAttack::set_scheduler()`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.set_scheduler) method as follows:

```rust,ignore
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::Serial);
```

To instead randomize the order that [`Scenario`](https://docs.rs/goose/*/goose/goose/struct.Scenario.html) and [`Transaction`](https://docs.rs/goose/*/goose/goose/struct.Transaction.html) are allocated, you can instead configure as follows:

```rust,ignore
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::Random);
```

The following configuration is possible but superfluous because it is the scheduling default, and is therefor how Goose behaves even if the [`.set_scheduler()`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.set_scheduler) method is not called at all:

```rust,ignore
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::RoundRobin);
```

## Scheduling Example

The following simple example helps illustrate how the different schedulers work.

```rust,ignore
use goose::prelude::*;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(scenario!("Scenario1")
            .register_transaction(transaction!(transaction1).set_weight(2)?)
            .register_transaction(transaction!(transaction2))
            .set_weight(2)?
        )
        .register_scenario(scenario!("Scenario2")
            .register_transaction(transaction!(transaction1))
            .register_transaction(transaction!(transaction2).set_weight(2)?)
        )
        .execute()
        .await?;

    Ok(())
}
```

## Round Robin Scheduler

This first example assumes the default of `.set_scheduler(GooseScheduler::RoundRobin)`.

If Goose is told to launch only two users, the first [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) will run `Scenario1` and the second user will run `Scenario2`. Even though `Scenario1` has a weight of 2 [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) are allocated round-robin so with only two users the second instance of `Scenario1` is never launched.

The [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) running `Scenario1` will then launch transactions repeatedly in the following order: `transactions1`, `transactions2`, `transaction1`. If it runs through twice, then it runs all of the following transactions in the following order: `transaction1`, `transaction2`, `transaction1`, `transaction1`, `transaction2`, `transaction1`.

## Serial Scheduler

This second example assumes the manual configuration of `.set_scheduler(GooseScheduler::Serial)`.

If Goose is told to launch only two users, then both [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) will launch `Scenario1` as it has a weight of 2. `Scenario2` will not get assigned to either of the users.

Both [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) running `Scenario1` will then launch transactions repeatedly in the following order: `transaction1`, `transaction1`, `transaction2`. If it runs through twice, then it runs all of the following transactions in the following order: `transaction1`, `transaction1`, `transaction2`, `transaction1`, `transaction1`, `transaction2`.

## Random Scheduler

This third example assumes the manual configuration of `.set_scheduler(GooseScheduler::Random)`.

If Goose is told to launch only two users, the first will be randomly assigned either `Scenario1` or `Scenario2`. Regardless of which is assigned to the first user, the second will again be randomly assigned either `Scenario1` or `Scenario2`. If the load test is stopped and run again, there users are randomly re-assigned, there is no consistency between load test runs.

Each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) will run transactions in a random order. The random order will be determined at start time and then will run repeatedly in this random order as long as the user runs.

