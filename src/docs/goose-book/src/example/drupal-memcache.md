# Drupal Memcache Example

The [`examples/drupal_memcache.rs`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs) example is used to validate the performance of each release of the [Drupal Memcache Module](https://www.drupal.org/project/memcache).

## Background

Prior to every release of the [Drupal Memcache Module](https://www.drupal.org/project/memcache), [Tag1 Consulting](https://www.tag1.com/) has run a load test to ensure consistent performance of the module which is dependend on by [tens of thousands of Drupal websites](https://www.drupal.org/project/usage/memcache).

The load test was initially implemented as a [JMeter testplan](https://github.com/tag1consulting/drupal-loadtest/tree/206716d2bd3fdd199febba34a964117e1fd0fbde). It was later converted to a [Locust testplan](https://github.com/tag1consulting/drupal-loadtest). Most recently it was converted to a [Goose testplan](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs).

Thie testplan is maintained as a simple real-world Goose load test example.

## Details

The authenticated [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) is labeled as `AuthBrowsingUser` and demonstrates logging in one time at the start of the load test:
```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs:53:59}}
```

Each [`GooseUser`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html) thread logs in as a random user (depending on a properly configured test environment):
```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs:178:189}}
```

The test also includes an example of how to post a comment during a load test:
```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs:75:79}}
```

Note that much of this functionality can be simplified by using the [Goose Eggs library](https://docs.rs/goose-eggs) which includes some [Drupal-specific functionality](https://docs.rs/goose-eggs/*/goose_eggs/drupal/index.html).

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs}}
```
