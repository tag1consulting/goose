# Gaggle Technical Details

**NOTE: Goose 0.19 cannot run a Gaggle. Gaggle support was removed in Goose 0.17.0 and will be restored in Goose 0.20, before the 1.0 release. This page describes Goose 0.16.4, the last release with it; see the [Gaggle overview](overview.md).**

Goose uses [`nng`](https://docs.rs/nng/) to send network messages between the Manager and all Workers. [Serde](https://docs.serde.rs/serde/index.html) and [Serde CBOR](https://github.com/pyfisch/cbor) are used to serialize messages into [Concise Binary Object Representation](https://tools.ietf.org/html/rfc7049).

Workers initiate all network connections, and push metrics to the Manager process.

## Compile-time Feature

Gaggle support is a compile-time Cargo feature that must be enabled. Goose uses the [`nng`](https://docs.rs/nng/) library to manage network connections, and compiling `nng` requires that `cmake` be available.

The `gaggle` feature can be enabled from the command line by adding `--features gaggle` to your cargo command.

With Goose 0.16, a load test application could default to compiling in the Gaggle feature in the `dependencies` section of its `Cargo.toml`. This only works with Goose 0.16.4 or earlier: Goose 0.18 accepts the feature and ignores it, and Goose 0.17 and 0.19 have no `gaggle` feature, so Cargo refuses to build a dependency line that asks for it. For example, pinned to 0.16.4:

```toml
[dependencies]
goose = { version = "=0.16.4", features = ["gaggle"] }
```
