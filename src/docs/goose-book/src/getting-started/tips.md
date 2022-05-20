# Tips

## Best Practices

* When writing load tests, avoid [`unwrap()`](https://doc.rust-lang.org/std/option/enum.Option.html#method.unwrap) (and variations) in your transaction functions -- Goose generates a lot of load, and this tends to trigger errors. Embrace Rust's warnings and properly handle all possible errors, this will save you time debugging later.
* When running your load test, use the cargo `--release` flag to generate optimized code. This can generate considerably more load test traffic. Learn more about this and other optimizations in ["The golden Goose egg, a compile-time adventure"](https://www.tag1consulting.com/blog/golden-goose-egg-compile-time-adventure).

## Errors

### Timeouts

By default, Goose will time out requests that take longer than 60 seconds to return, and display a `WARN` level message saying, "operation timed out". For example:

```ignore
11:52:17 [WARN] "/node/3672": error sending request for url (http://apache/node/3672): operation timed out
```

These will also show up in the error summary displayed with the final metrics. For example:

```ignore
 === ERRORS ===
 ------------------------------------------------------------------------------
 Count       | Error
 ------------------------------------------------------------------------------
 51            GET (Auth) comment form: error sending request (Auth) comment form: operation timed out
```

To change how long before requests time out, use `--timeout VALUE` when starting a load test, for example `--timeout 30` will time out requests that take longer than 30 seconds to return. To configure the timeout programatically, use [`.set_default()`](https://docs.rs/goose/*/goose/config/trait.GooseDefaultType.html#tymethod.set_default) to set [GooseDefault::Timeout](https://docs.rs/goose/*/goose/config/enum.GooseDefault.html#variant.Timeout).

To completely disable timeouts, you must build a custom Reqwest Client with [`GooseUser::set_client_builder`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_client_builder). Alternatively, you can just set a very high timeout, for example `--timeout 86400` will let a request take up to 24 hours.

## Debugging HTML Responses

Sometimes, while developing and debugging a load test we'd like to view HTML responses in a browser to actually see where each request is actually taking us. We may want to run this test with one user to avoid debug noise.

We can create a debug log by passing the ```--debug-log NAME``` command line option.

Each row in the debug log defaults to a JSON object and we can use [jq](https://stedolan.github.io/jq/) for processing JSON or the faster Rust port that supports the same commands [jaq](https://crates.io/crates/jaq)

To extract the HTML response from the first log entry, for example, you could use the following commands:

```bash,ignore
cat debug.log | head -n 1 | jaq -r .body > page.html
```

This HTML page can then be viewed in a web browser. You may need to disable JavaScript.
