# Custom Run Time Options

It can sometimes be necessary to add custom run time options to your load test. As Goose "owns" the command line, you can't simply add another option with [gumpdrop](https://docs.rs/gumdrop) (used by Goose) or another command line parser, as Goose will throw an error if it receives an unexpected command line option.

Instead, you can use environment variables. One example of this can be found in the [Umami example](../example/umami.html) which [uses environment variables to allow the configuration of a custom username and password](https://github.com/tag1consulting/goose/blob/main/examples/umami/admin.rs#L9).

Alternatively, you can use this method to set configurable custom defaults. The [earlier example](./custom.md) can be enhanced to use an environment variable to set a custom default hostname:

```rust
use goose::prelude::*;

async fn loadtest_index(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("").await?;

    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), GooseError> {
    // Get optional custom default hostname from `HOST` environment variable.
    let custom_host = match std::env::var("HOST") {
        Ok(host) => host,
        Err(_) => "".to_string(),
    };

    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadtestTransactions")
            .register_transaction(transaction!(loadtest_index))
        )
        // Set optional custom default hostname.
        .set_default(GooseDefault::Host, custom_host.as_str())?
        .execute()
        .await?;

    Ok(())
}
```

This can now be used to set a custom default for the scenario, in this example with no `--host` set Goose will execute a load test against the hostname defined in `HOST`:

```bash,ignore
% HOST="https://local.dev/" cargo run --release                  
    Finished release [optimized] target(s) in 0.07s
     Running `target/release/loadtest`
07:28:20 [INFO] Output verbosity level: INFO
07:28:20 [INFO] Logfile verbosity level: WARN
07:28:20 [INFO] users defaulted to number of CPUs = 10
07:28:20 [INFO] iterations = 0
07:28:20 [INFO] host for LoadtestTransactions configured: https://local.dev/
```

It's still possible to override this custom default from the command line with standard Goose options, for example here the load test will run against the hostname configured by the `--host` option:

```bash,ignore
% HOST="http://local.dev/" cargo run --release -- --host https://example.com/
    Finished release [optimized] target(s) in 0.07s
     Running `target/release/loadtest --host 'https://example.com/'`
07:32:36 [INFO] Output verbosity level: INFO
07:32:36 [INFO] Logfile verbosity level: WARN
07:32:36 [INFO] users defaulted to number of CPUs = 10
07:32:36 [INFO] iterations = 0
07:32:36 [INFO] global host configured: https://example.com/
```

If the `HOST` variable and the `--host` option are not set, Goose will display the expected error:

```bash,ignore
% cargo run --release
     Running `target/release/loadtest`
07:07:45 [INFO] Output verbosity level: INFO
07:07:45 [INFO] Logfile verbosity level: WARN
07:07:45 [INFO] users defaulted to number of CPUs = 10
07:07:45 [INFO] iterations = 0
Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the Scenario.set_host() function (no host defined for LoadtestTransactions)." }
```