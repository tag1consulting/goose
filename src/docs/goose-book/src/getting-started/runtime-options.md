# Run-Time Options

Every Goose load test takes the same run-time options. `-h` prints a summary of each one, and `--help` prints the full reference: what each option does, its default, the values it takes, the options it works with or excludes, an example, and the `GooseDefault` that sets it in code. For example, to see the summary for our example load test, run `cargo run --release -- -h`. The output below comes from a build with the `pdf-reports` feature:

```text
{{#include runtime-options-short.txt}}
```

`--pdf-scale` and `--pdf-timeout` exist only when Goose is built with the `pdf-reports` feature.

Help wraps to the width of the terminal, or to `COLUMNS` when it is set, up to 120 columns, and to 80 columns when neither is available. Both outputs on this page are generated from the help at 80 columns, and Goose's tests check that they still match.

## Full reference

`cargo run --release -- --help` prints:

```text
{{#include runtime-options-long.txt}}
```

All of the above configuration options are [defined in the developer documentation](https://docs.rs/goose/*/goose/config/struct.GooseConfiguration.html).
