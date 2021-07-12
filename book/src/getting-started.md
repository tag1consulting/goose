# Getting Started

The [in-line documentation](https://docs.rs/goose/*/goose/#creating-a-simple-goose-load-test) offers much more detail about Goose specifics. For a general background to help you get started with Rust and Goose, read on.

[Cargo](https://doc.rust-lang.org/cargo/) is the Rust package manager. To create a new load test, use Cargo to create a new application (you can name your application anything, we've generically selected `loadtest`):

```bash
$ cargo new loadtest
     Created binary (application) `loadtest` package
$ cd loadtest/
```

This creates a new directory named `loadtest/` containing `loadtest/Cargo.toml` and `loadtest/src/main.rs`. Start by editing `Cargo.toml` adding Goose under the dependencies heading:

```toml
[dependencies]
goose = "^0.12"
```

At this point it's possible to compile all dependencies, though the resulting binary only displays "Hello, world!":

```
$ cargo run
    Updating crates.io index
  Downloaded goose v0.12.0
      ...
   Compiling goose v0.12.0
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [unoptimized + debuginfo] target(s) in 52.97s
     Running `target/debug/loadtest`
Hello, world!
```

To create an actual load test, you first have to add the following boilerplate to the top of `src/main.rs` to make Goose's functionality available to your code:

```rust
use goose::prelude::*;
```

Then create a new load testing function. For our example we're simply going to load the front page of the website we're load-testing. Goose passes all load testing functions a pointer to a GooseUser object, which is used to track metrics and make web requests. Thanks to the Reqwest library, the Goose client manages things like cookies, headers, and sessions for you. Load testing functions must be declared async, which helps ensure that your simulated users don't become CPU-locked.

In load test functions you typically do not set the host, and instead configure the host at run time, so you can easily run your load test against different environments without recompiling. The following `loadtest_index` function simply loads the front page of our web page:

```rust
async fn loadtest_index(user: &GooseUser) -> GooseTaskResult {
    let _goose_metrics = user.get("/").await?;

    Ok(())
}
```

The function is declared `async` so that we don't block a CPU-core while loading web pages. All Goose load test functions are passed in a reference to a `GooseUser` object, and return a `GooseTaskResult` which is either an empty `Ok(())` on success, or a `GooseTaskError` on failure. We use the `GooseUser` object to make requests, in this case we make a `GET` request for the front page, `/`. The `.await` frees up the CPU-core while we wait for the web page to respond, and the tailing `?` passes up any unexpected errors that may be returned from this request. When the request completes, Goose returns metrics which we store in the  `_goose_metrics` variable. The variable is prefixed with an underscore (`_`) to tell the compiler we are intentionally not using the results. Finally, after making a single successful request, we return `Ok(())` to let Goose know this task function completed successfully.

We have to tell Goose about our new task function. Edit the `main()` function, setting a return type and replacing the hello world text as follows:

```rust
fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_taskset(taskset!("LoadtestTasks")
            .register_task(task!(loadtest_index))
        )
        .execute()?
        .print();

    Ok(())
}
```

If you're new to Rust, `main()`'s return type of `Result<(), GooseError>` may look strange. It essentially says that `main` will return nothing (`()`) on success, and will return a `GooseError` on failure. This is helpful as several of `GooseAttack`'s methods can fail, returning an error. In our example, `initialize()` and `execute()` each may fail. The `?` that follows the method's name tells our program to exit and return an error on failure, otherwise continue on. The `print()` method consumes the `GooseMetrics` object returned by `GooseAttack.execute()` and prints a summary if metrics are enabled. The final line, `Ok(())` returns the empty result expected on success.

And that's it, you've created your first load test! Let's run it and see what happens.

```bash
$ cargo run
   Compiling loadtest v0.1.0 (/home/jandrews/devel/rust/loadtest)
    Finished dev [unoptimized + debuginfo] target(s) in 3.56s
     Running `target/debug/loadtest`
Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the GooseTaskSet.set_host() function (no host defined for LoadtestTasks)." }
```

Goose is unable to run, as it hasn't been told the host you want to load test. So, let's try again, this time passing in the `--host` flag. After running for a few seconds, we then press `ctrl-c` to stop the load test:

```bash
$ cargo run -- --host http://local.dev/
    Finished dev [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/loadtest --host 'http://local.dev/'`

=== PER TASK METRICS ===
------------------------------------------------------------------------------
 Name                    | # times run    | # fails        | task/s | fail/s
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 2,240          | 0 (0%)         | 280.0  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 15.54      | 6          | 136        | 14

=== PER REQUEST METRICS ===
------------------------------------------------------------------------------
 Name                    | # reqs         | # fails        | req/s  | fail/s
 -----------------------------------------------------------------------------
 GET /                   | 2,240          | 0 (0%)         | 280.0  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 GET /                   | 15.30      | 6          | 135        | 14

All 8 users hatched, resetting metrics (disable with --no-reset-metrics).

^C06:03:25 [ WARN] caught ctrl-c, stopping...

=== PER TASK METRICS ===
------------------------------------------------------------------------------
 Name                    | # times run    | # fails        | task/s | fail/s
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 2,054          | 0 (0%)         | 410.8  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 1: LoadtestTasks        |
   1:                    | 20.86      | 7          | 254        | 19

=== PER REQUEST METRICS ===
------------------------------------------------------------------------------
 Name                    | # reqs         | # fails        | req/s  | fail/s
 -----------------------------------------------------------------------------
 GET /                   | 2,054          | 0 (0%)         | 410.8  | 0.000
-------------------------------------------------------------------------------
 Name                    | Avg (ms)   | Min        | Max        | Median
 -----------------------------------------------------------------------------
 GET /                   | 20.68      | 7          | 254        | 19
-------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
 -----------------------------------------------------------------------------
 GET /                   | 19     | 21     | 53     | 69     | 250    | 250
```

By default, Goose will hatch 1 GooseUser per second, up to the number of CPU cores available on the server used for load testing. In the above example, the server has 8 CPU cores, so it took 8 seconds to hatch all users. After all users are hatched, Goose flushes all metrics collected during the hatching process so all subsequent metrics are taken with all users running. Before flushing the metrics, they are displayed to the console so the data is not lost.

The same metrics are displayed per-task and per-request. In our simple example, our single task only makes one request, so in this case both metrics show the same results.

The per-task metrics are displayed first, starting with the name of our Task Set, `LoadtestTasks`. Individual tasks in the Task Set are then listed in the order they are defined in our load test. We did not name our task, so it simply shows up as `1: `. All defined tasks will be listed here, even if they did not run, so this can be useful to confirm everything in your load test is running as expected.

Next comes the per-request metrics. Our single task makes a `GET` request for the `/` path, so it shows up in the metrics as `GET /`. Comparing the per-task metrics collected for `1: ` to the per-request metrics collected for `GET /`, you can see that they are the same.

There are two common tables found in each type of metrics. The first shows the total number of requests made (2,054), how many of those failed (0), the average number of requests per second (410.8), and the average number of failed requests per second (0).

The second table shows the average time required to load a page (20.68 milliseconds), the minimum time to load a page (7 ms), the maximum time to load a page (254 ms) and the median time to load a page (19 ms).

The per-request metrics include a third table, showing the slowest page load time for a range of percentiles. In our example, in the 50% fastest page loads, the slowest page loaded in 19 ms. In the 75% fastest page loads, the slowest page loaded in 21 ms, etc.

In real load tests, you'll most likely have multiple task sets each with multiple tasks, and Goose will show you metrics for each along with an aggregate of them all together.

Refer to the [examples directory](https://github.com/tag1consulting/goose/tree/master/examples) for more complicated and useful load test examples.
