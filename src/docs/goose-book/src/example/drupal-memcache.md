# Drupal Memcache Example

The [`examples/drupal_memcache.rs`](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs) example is used to validate the performance of each release of the [Drupal Memcache Module](https://www.drupal.org/project/memcache).

This page has no picture of a run, because the example needs its own test site to run cleanly. It requests random nodes up to `/node/9999` and profiles up to `/user/5000`, and logs in as `user3` to `user5001` with the password `12345`. It also posts Drupal 7 forms: the login form `user_login`, and comments with the field `comment_body[und][0][value]`. It needs a Drupal 7 site prepared as the [drupal-loadtest](https://github.com/tag1consulting/drupal-loadtest) repository describes. A 30 second run with 10 users against the Drupal 11 Umami site that the [Umami page](umami.md) uses, a site with 19 nodes, made 17,083 requests. Of those, 1,435 failed with 404s, nearly all of its node and profile requests; the front page and all 14,076 static asset requests succeeded.

## Background

Prior to every release of the [Drupal Memcache Module](https://www.drupal.org/project/memcache), [Tag1 Consulting](https://www.tag1.com/) has run a load test to check that the release performs as well as the last; the module runs on [tens of thousands of Drupal websites](https://www.drupal.org/project/usage/memcache).

The load test began as a [JMeter testplan](https://github.com/tag1consulting/drupal-loadtest/tree/206716d2bd3fdd199febba34a964117e1fd0fbde), was converted to a [Locust testplan](https://github.com/tag1consulting/drupal-loadtest), and is now a [Goose testplan](https://github.com/tag1consulting/goose/blob/main/examples/drupal_memcache.rs).

The Goose port is kept here as an example of a real load test.

## Details

The load test defines two scenarios. `AnonBrowsingUser` has a weight of 4 and `AuthBrowsingUser` a weight of 1, so four of every five users browse anonymously. Each scenario loads the front page (weight 15), a random node from `/node/1` to `/node/9999` (weight 10) and a random user profile from `/user/2` to `/user/5000` (weight 3); the authenticated scenario also posts comments (weight 3). The front page transaction then loads each image and script the page references whose path contains `/misc` or `/themes`, as requests named `static asset`. Transactions are named `(Anon) front page`, `(Auth) node page` and so on.

Users of the `AuthBrowsingUser` scenario log in once, when each starts:
```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs:53:59}}
```

Each `AuthBrowsingUser` logs in as a random user from `user3` to `user5001`, accounts that exist only on a site prepared for this test:
```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs:178:188}}
```

The test also includes an example of how to post a comment during a load test, after reading the form's `form_build_id`, `form_token` and `form_id` from the node page:
```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs:291:303}}
```

The [Goose Eggs library](https://docs.rs/goose-eggs) has [Drupal helpers](https://docs.rs/goose-eggs/*/goose_eggs/drupal/index.html) that shorten much of this.

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/drupal_memcache.rs}}
```
