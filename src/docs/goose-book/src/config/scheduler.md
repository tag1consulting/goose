# Scheduling Users And Tasks

When starting a load test, Goose assigns one [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) to each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) thread. By default, it assigns [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) (and then [`GooseTask`](https://docs.rs/goose/*/goose/goose/struct.GooseTask.html) within the task set) in a round robin order. As new [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) threads are launched, the first will be assigned the first defined [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html), the next will be assigned the next defined [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html), and so on, looping through all available [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html). Weighting is respected during this process, so if one [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) is weighted heavier than others, that [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) will get assigned to [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) more at the end of the launching process.

The [`GooseScheduler`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html) can be configured to instead launch [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) and [`GooseTask`](https://docs.rs/goose/*/goose/goose/struct.GooseTask.html) in a [`Serial`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Serial) or a [`Random order`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Random). When configured to allocate in a [`Serial`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Serial) order, [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) and [`GooseTask`](https://docs.rs/goose/*/goose/goose/struct.GooseTask.html) are launched in the extact order they are defined in the load test (see below for more detail on how this works). When configured to allocate in a [`Random`](https://docs.rs/goose/*/goose/enum.GooseScheduler.html#variant.Random) order, running the same load test multiple times can lead to different amounts of load being generated.

Prior to Goose `0.10.6` [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) were allocated in a serial order. Prior to Goose `0.11.1` [`GooseTask`](https://docs.rs/goose/*/goose/goose/struct.GooseTask.html) were allocated in a serial order. To restore the old behavior, you can use the [`GooseAttack::set_scheduler()`](https://docs.rs/goose/*/goose/struct.GooseAttack.html#method.set_scheduler) method as follows:

```rust,ignore
    GooseAttack::initialize()?
        .set_scheduler(GooseScheduler::Serial);
```

To instead randomize the order that [`GooseTaskSet`](https://docs.rs/goose/*/goose/goose/struct.GooseTaskSet.html) and [`GooseTask`](https://docs.rs/goose/*/goose/goose/struct.GooseTask.html) are allocated, you can instead configure as follows:

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
        .register_taskset(taskset!("TaskSet1")
            .register_task(task!(task1).set_weight(2)?)
            .register_task(task!(task2))
            .set_weight(2)?
        )
        .register_taskset(taskset!("TaskSet2")
            .register_task(task!(task1))
            .register_task(task!(task2).set_weight(2)?)
        )
        .execute()
        .await?;

    Ok(())
}
```

## Round Robin Scheduler

This first example assumes the default of `.set_scheduler(GooseScheduler::RoundRobin)`.

If Goose is told to launch only two users, the first [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) will run `TaskSet1` and the second user will run `TaskSet2`. Even though `TaskSet1` has a weight of 2 [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) are allocated round-robin so with only two users the second instance of `TaskSet1` is never launched.

The [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) running `TaskSet1` will then launch tasks repeatedly in the following order: `task1`, `task2`, `task1`. If it runs through twice, then it runs all of the following tasks in the following order: `task1`, `task2`, `task1`, `task1`, `task2`, `task1`.

## Serial Scheduler

This second example assumes the manual configuration of `.set_scheduler(GooseScheduler::Serial)`.

If Goose is told to launch only two users, then both [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) will launch `TaskSet1` as it has a weight of 2. `TaskSet2` will not get assigned to either of the users.

Both [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) running `TaskSet1` will then launch tasks repeatedly in the following order: `task1`, `task1`, `task2`. If it runs through twice, then it runs all of the following tasks in the following order: `task1`, `task1`, `task2`, `task1`, `task1`, `task2`.

## Random Scheduler

This third example assumes the manual configuration of `.set_scheduler(GooseScheduler::Random)`.

If Goose is told to launch only two users, the first will be randomly assigned either `TaskSet1` or `TaskSet2`. Regardless of which is assigned to the first user, the second will again be randomly assigned either `TaskSet1` or `TaskSet2`. If the load test is stopped and run again, there users are randomly re-assigned, there is no consistency between load test runs.

Each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) will run tasks in a random order. The random order will be determined at start time and then will run repeatedly in this random order as long as the user runs.

