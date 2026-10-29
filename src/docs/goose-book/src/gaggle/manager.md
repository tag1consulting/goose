# Gaggle Manager

**NOTE: Goose 0.19 cannot run a Gaggle. Gaggle support was removed in Goose 0.17.0 and will be restored in Goose 0.20, before the 1.0 release. This page describes Goose 0.16.4, the last release with it; see the [Gaggle overview](overview.md).**

To launch a Gaggle, you first must start a Goose application in Manager mode. All configuration happens in the Manager. To start, add the `--manager` flag and `--expect-workers` option, the latter necessary to tell the Manager process how many Worker processes it will be coordinating.

## Example

_Configure a Goose Manager to listen on all interfaces on the default port (0.0.0.0:5115), waiting for 2 Goose Worker processes._

```bash
cargo run --features gaggle --example simple -- --manager --expect-workers 2 --host http://local.dev/
```