# Gaggle Manager

**NOTE: Gaggle support was temporarily removed as of Goose 0.17.0 (see https://github.com/tag1consulting/goose/pull/529). Use Goose 0.16.4 if you need the functionality described in this section.**

To launch a Gaggle, you first must start a Goose application in Manager mode. All configuration happens in the Manager. To start, add the `--manager` flag and `--expect-workers` option, the latter necessary to tell the Manager process how many Worker processes it will be coordinating.

## Example

_Configure a Goose Manager to listen on all interfaces on the default port (0.0.0.0:5115), waiting for 2 Goose Worker processes._

```bash
cargo run --features gaggle --example simple -- --manager --expect-workers 2 --host http://local.dev/
```