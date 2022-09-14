# Custom Run Time Options

It can sometimes be necessary to add custom run-time options to your load test. As Goose "owns" the command line, adding another option with [gumdrop](https://docs.rs/gumdrop) (used by Goose) or another command line parser can be tricky, as Goose will throw an error if it receives an unexpected command line option. There are two alternatives here.

## Environment Variables

One option is to use environment variables. An example of this can be found in the [Umami example](../example/umami.html) which [uses environment variables to allow the configuration of a custom username and password](https://github.com/tag1consulting/goose/blob/main/examples/umami/admin.rs#L9).

Alternatively, you can use this method to set configurable custom defaults. The [earlier example](./custom.md) can be enhanced to use an environment variable to set a custom default hostname:

```rust,ignore
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

## Command Line Arguments

If you really need to have custom command line arguments, there is a way to make Goose not throw an error due to unexpected arguments. You can do that by, instead of calling `GooseAttack::initialize()`, using `GooseAttack::initialize_with_config`. This method differs from the first one in that it does not parse arguments from the command line, but instead takes a `GooseConfiguration` value as parameter. Since this type has quite a lot of configuration options, with some private fields, currently the only way you can obtain an instance of it is via the `Default` trait: `GooseConfiguration::default()`.

Note that by initializing the `GooseAttack` in this way you are preventing Goose from reading command line arguments, so if you want to have the ability of passing the arguments that Goose allows, you will need to parse them and set them in the `GooseConfiguration` instance. In particular, the `--host` parameter is mandatory, so don't forget to set it in the configuration somehow.

The example below should illustrate these points:

```rust,ignore
use goose::config::GooseConfiguration;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    // here we could be using a crate such as `clap` to parse CLI arguments:
    let opt = MyCustomConfig::parse();

    let mut config = GooseConfiguration::default();

    // we added a `host` field to our custom argument parser that matches
    // the `host` field used by Goose
    config.host = opt.host;

    // ... here you should do the same for all the other command line parameters
    // offered by Goose that you care about, otherwise they will not be taken
    // into account.

    // Initialize the `GooseAttack` using the `GooseConfiguration`:
    GooseAttack::initialize_with_config(config)?
        .register_scenario(
            scenario!("User")
                .register_transaction(transaction!(loadtest_index))
        )
        .execute()
        .await?;

    Ok(())
}
```

Assuming that `MyCustomConfig` has a `my_custom_arg` field, the program above can be invoked with a command such as:

```bash,ignore
cargo run -- --host https://localhost:8080 --my-custom-arg 42
```
