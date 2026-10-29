# Creating A Load Test

## Cargo

[Cargo](https://doc.rust-lang.org/cargo/) is the Rust package manager. To create a new load test, use Cargo to create a new application. You can give it any name; this page uses `loadtest`:

```bash
$ cargo new loadtest
    Creating binary (application) `loadtest` package
note: see more `Cargo.toml` keys and their definitions at https://doc.rust-lang.org/cargo/reference/manifest.html
$ cd loadtest/
```

This creates a new directory named `loadtest/` containing `loadtest/Cargo.toml` and `loadtest/src/main.rs`. Edit `Cargo.toml` and add Goose and [Tokio](https://tokio.rs/) under the `[dependencies]` heading, so the file reads:

```toml
[package]
name = "loadtest"
version = "0.1.0"
edition = "2024"

[dependencies]
goose = "^0.19"
tokio = "^1"
```

The `edition` line is the one your version of Cargo wrote; leave it as it is.

You can now compile all dependencies, though the program still only prints "Hello, world!". The first build downloads and compiles every crate Goose depends on, so it takes a while:

```bash
$ cargo run
    Updating crates.io index
     Locking 253 packages to highest Rust 1.99.0 compatible versions
      Adding generic-array v0.14.7 (available: v0.14.9)
 Downloading crates ...
  Downloaded goose v0.19.0
      ...
   Compiling goose v0.19.0
   Compiling loadtest v0.1.0 (/home/you/loadtest)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 27.70s
     Running `target/debug/loadtest`
Hello, world!
```

The package count, the Rust version and the build time depend on your toolchain and your machine. This debug build only checks that everything compiles; [Running A Load Test](running.md) builds with `--release`, which compiles the dependencies once more.

## Creating the load test

To create an actual load test, first add the following line to the top of `src/main.rs` to make Goose's functionality available to your code:

```rust,ignore
use goose::prelude::*;
```

> **Note:** The prelude adds the following `use` statements for you, so you don't need to write them out:
>
> ```rust,ignore
> use goose::config::{GooseDefault, GooseDefaultType};
> use goose::goose::{
>     GooseMethod, GooseRequest, GooseUser, Scenario, Transaction, TransactionError,
>     TransactionFunction, TransactionResult,
> };
> use goose::metrics::{GooseCoordinatedOmissionMitigation, GooseMetrics};
> use goose::{scenario, transaction, GooseAttack, GooseError, GooseScheduler};
> ```

Then create a transaction function: an async function that makes requests, which Goose runs as a [transaction](../glossary.md#transaction). Goose passes every transaction function a mutable reference to a `GooseUser`, which makes the web requests and records their metrics. Each `GooseUser` has its own [Reqwest](https://docs.rs/reqwest/) client, and with Goose's default features that client keeps the user's cookies from one request to the next (see [Cookies](../config/cookie.md)). A transaction function usually does not set the host: you give the host when you run the load test, so the same load test runs against different environments without recompiling.

Below the `use` line, add the following `loadtest_index` function, which loads the front page:

```rust,ignore
async fn loadtest_index(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("").await?;

    Ok(())
}
```

The function is declared `async`, so a user waiting for a page does not block a CPU core. It returns a `TransactionResult`, which is either an empty `Ok(())` on success or a boxed `TransactionError` on failure. Here the `GooseUser` makes a `GET` request for the front page, given as the empty path `""`. The `.await` lets other users run while the server responds. The trailing `?` returns early in two cases: Goose could not build or record the request, for example because the URL is invalid, or the run ended while the request waited on a [throttle](throttle.md). If the request fails because the server answers with a `404` or because Goose cannot connect to it, the `?` does not return. Goose records a failed request, and the function carries on.

The request returns a `GooseResponse`, which holds Goose's record of the request and the server's response. This function stores it in `_goose_metrics`, and the leading underscore (`_`) tells the compiler the variable is unused on purpose. The function then returns `Ok(())`, so Goose counts the transaction as a success even if its request failed. [Validating Requests](validation.md) shows how to fail a transaction.

Replace the `main()` function with the following, which gives it a return type and registers `loadtest_index` with Goose:

```rust,ignore
#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadtestTransactions")
            .register_transaction(transaction!(loadtest_index))
        )
        .execute()
        .await?;

    Ok(())
}
```

`scenario!` creates a [scenario](../glossary.md#scenario), the list of transactions each user runs over and over, and `register_transaction` adds `loadtest_index` to it.

`#[tokio::main]` is a Tokio macro. Goose is an asynchronous library, and the macro lets you declare `main()` as `async`.

If you're new to Rust, the return type `Result<(), GooseError>` says that `main` returns nothing (`()`) on success and a `GooseError` on failure. `initialize()` and `execute()` can each fail, and the `?` after each one ends the program with its error. `.execute()` is asynchronous, so `.await` comes before its `?`. The final line, `Ok(())`, returns the empty result expected on success.

The whole `src/main.rs` now reads:

```rust,ignore
use goose::prelude::*;

async fn loadtest_index(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("").await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(scenario!("LoadtestTransactions")
            .register_transaction(transaction!(loadtest_index))
        )
        .execute()
        .await?;

    Ok(())
}
```

That is your first load test.

### Paths and the host

Goose joins each path to the host the way a browser resolves a link. With a host that has no path, such as `http://local.dev/`, `about/` and `/about/` both request `http://local.dev/about/`. With a host that has a path, use paths that do not start with a `/` so they stay under it: with `--host http://local.dev/site/`, `about/` requests `http://local.dev/site/about/`, while `/about/` requests `http://local.dev/about/`. End such a host with a `/`. Without it, `--host http://local.dev/site` drops `site`, and `about/` requests `http://local.dev/about/`.

## Naming Transactions

A transaction without a name is listed by its number alone in the transaction metrics. Goose has two methods for naming a transaction, which you call on the `transaction!(...)` you register. They differ in what they do to the names of the transaction's requests.

### `set_name()`

Names the transaction only. Each request the transaction makes keeps its own name, which is its path unless the request was given a name.

```rust,ignore
.register_transaction(transaction!(loadtest_index).set_name("Homepage Load"))
```

**Use when:** you want a readable transaction name and still want each request listed by its own path.

### `set_name_for_transaction_and_requests()`

Names the transaction, and every request it makes that has no name of its own. A request named with `GooseRequest::builder().name()` keeps its own name; every other request in the transaction is listed under the transaction's name in the request metrics, in place of its path.

Here `checkout_flow` stands for a transaction function of your own that makes several requests:

```rust,ignore
.register_transaction(transaction!(checkout_flow).set_name_for_transaction_and_requests("Checkout"))
```

**Use when:** a transaction is a multistep flow, such as a login or a checkout, and you want its requests counted together under one name.

[Running A Load Test](running.md) runs the load test; to watch a run live in your browser, turn on the `dashboard` feature and add `--dashboard` (see [Live Dashboard](../controller/dashboard.md)).
