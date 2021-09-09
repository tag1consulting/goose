# Gaggle Manager

To launch a Gaggle, you first must start a Goose application in Manager mode. All configuration happens in the Manager. To start, add the `--manager` flag and the `--expect-workers` flag, the latter necessary to tell the Manager process how many Worker processes it will be coordinating. For example:

## Example

_Configure a Goose Manager to listen on all interfaces on the default port (0.0.0.0:5115), waiting for 2 Goose Worker processes._

```bash
cargo run --features gaggle --example simple -- --manager --expect-workers 2 --host http://local.dev/ -v
```