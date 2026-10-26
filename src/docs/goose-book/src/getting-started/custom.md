# Custom Run Time Options

It can sometimes be necessary to add custom run-time options to your load test. `GooseAttack::initialize()` parses the command line itself, with [clap](https://docs.rs/clap), and exits with an error when it receives an option it does not know. There are two alternatives here: environment variables, or parsing the command line yourself and handing Goose its part.

## Environment Variables

One option is to use environment variables. An example of this can be found in the [Umami example](../example/umami.html) which [uses environment variables to allow the configuration of a custom username and password](https://github.com/tag1consulting/goose/blob/main/examples/umami/admin.rs#L11).

Alternatively, you can use this method to set configurable custom defaults. The [earlier example](./creating.md#creating-the-load-test) can be enhanced to use an environment variable to set a custom default hostname:

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
    Finished `release` profile [optimized] target(s) in 0.07s
     Running `target/release/loadtest`
21:04:16 [INFO] Output verbosity level: INFO
21:04:16 [INFO] Logfile verbosity level: WARN
21:04:16 [INFO] users defaulted to number of CPUs = 18
21:04:16 [INFO] host for LoadtestTransactions configured: https://local.dev/
```

It's still possible to override this custom default from the command line with standard Goose options, for example here the load test will run against the hostname configured by the `--host` option:

```bash,ignore
% HOST="http://local.dev/" cargo run --release -- --host https://example.com/
    Finished `release` profile [optimized] target(s) in 0.07s
     Running `target/release/loadtest --host 'https://example.com/'`
21:04:19 [INFO] Output verbosity level: INFO
21:04:19 [INFO] Logfile verbosity level: WARN
21:04:19 [INFO] users defaulted to number of CPUs = 18
21:04:19 [INFO] global host configured: https://example.com/
```

If the `HOST` variable and the `--host` option are not set, Goose will display the expected error:

```bash,ignore
% cargo run --release
     Running `target/release/loadtest`
21:04:11 [INFO] Output verbosity level: INFO
21:04:11 [INFO] Logfile verbosity level: WARN
21:04:11 [INFO] users defaulted to number of CPUs = 18
Error: InvalidOption { option: "--host", value: "", detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the Scenario.set_host() function (no host defined for LoadtestTransactions)." }
```

## Command Line Arguments

To add command line arguments of your own, parse the command line yourself and give Goose its options with `GooseAttack::initialize_with_config` in place of `GooseAttack::initialize()`. `initialize_with_config` does not read the command line; it takes a `GooseConfiguration`.

### Adding options to Goose's

`GooseConfiguration` derives `clap::Args`, so a load test can flatten it into a `clap::Parser` of its own and add its options beside Goose's. Every Goose option keeps working, `-h` and `--help` list both, and an option neither knows is still an error. Add `clap` to your `Cargo.toml` with its `derive` feature, such as `clap = { version = "4.5", features = ["derive"] }`:

```rust
use clap::Parser;
use goose::config::GooseConfiguration;
use goose::prelude::*;

/// Every Goose option, plus the load test's own.
#[derive(Parser)]
struct Options {
    #[command(flatten)]
    goose: GooseConfiguration,

    /// The ID of the product the load test buys.
    #[arg(long, default_value_t = 1)]
    product_id: u32,
}

async fn loadtest_index(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("").await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let options = Options::parse();
    println!("product_id = {}", options.product_id);

    GooseAttack::initialize_with_config(options.goose)?
        .register_scenario(
            scenario!("User").register_transaction(transaction!(loadtest_index)),
        )
        .execute()
        .await?;

    Ok(())
}
```

The program above can be invoked with a command such as:

```bash,ignore
cargo run -- --host http://localhost:8080 --product-id 42
```

Your options must not reuse the names or short letters Goose's take; `--help` lists them.

### Building the configuration yourself

You can also build a `GooseConfiguration` without reading the command line: `GooseConfiguration::default()` leaves every option unset, and `GooseConfiguration::parse_args_default(&["--host", "http://localhost:8080"])` parses a list of Goose options, leaving every option you do not pass unset, and returns a `Result`. Given `-h` or `--help`, it prints nothing and returns `GooseError::InvalidOption`, with the flag as `option` and the detail `help was requested`. `GooseConfiguration` also implements `clap::Parser`, so `GooseConfiguration::try_parse_from(["loadtest", "--host", "http://localhost:8080"])` parses the same options, with the program name first. Each option that takes a value is an `Option`, `None` when it is not set, so a value is set with `Some`. Goose then applies any `GooseDefault` and its own defaults to every option left unset, as it does for the command line.

Options you do not set this way are not read from the command line. In particular, a host is required, from the configuration, `GooseDefault::Host` or `Scenario::set_host`:

```rust
use goose::config::GooseConfiguration;
use goose::prelude::*;

async fn loadtest_index(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("").await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let mut config = GooseConfiguration::default();
    config.host = Some("http://localhost:8080".to_string());
    config.users = Some(10);

    GooseAttack::initialize_with_config(config)?
        .register_scenario(
            scenario!("User").register_transaction(transaction!(loadtest_index)),
        )
        .execute()
        .await?;

    Ok(())
}
```

Before 0.19, Goose parsed its command line with `gumdrop`, and a load test that called `parse_args_default` imported `use gumdrop::Options;`. Remove that import: `parse_args_default` and `parse_args_default_or_exit` are methods of `GooseConfiguration` itself now.
