//! Microbenchmark of the request-hooks dispatch mechanism alone, no
//! network: the `Vec::is_empty` check a zero-hook request pays, versus
//! calling one, three and five sync `before_send` hooks, versus one async
//! hook. Part of the design investigation at
//! `src/docs/goose-book/src/design/request-hooks.md`.
//!
//! Run with: `cargo bench --bench hook_dispatch --features request-hooks`

use std::hint::black_box;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use goose::goose::hooks::{BeforeSendHook, BeforeSendOutcome, HookRequestParts};
use reqwest::{Method, Request};
use url::Url;

fn add_header_hook() -> BeforeSendHook {
    Arc::new(
        |request: &mut HookRequestParts, _user: &mut goose::goose::GooseUser| {
            request
                .headers_mut()
                .insert("x-bench-trace", "1".parse().unwrap());
            BeforeSendOutcome::Continue
        },
    )
}

/// A request carrying a handful of headers, representative of a real
/// request about to go out (not an empty, freshly allocated one): isolates
/// hook dispatch cost from request/header-map construction cost, which
/// `black_box`-returning the whole fresh request on every iteration would
/// otherwise fold in.
fn request_with_some_headers() -> Request {
    let mut request = Request::new(Method::GET, Url::parse("http://127.0.0.1/").unwrap());
    request
        .headers_mut()
        .insert("accept", "text/html".parse().unwrap());
    request
        .headers_mut()
        .insert("user-agent", "goose-bench".parse().unwrap());
    request
}

fn bench_is_empty_check(c: &mut Criterion) {
    let hooks: Vec<BeforeSendHook> = Vec::new();
    c.bench_function("zero_hook_is_empty_check", |b| {
        b.iter(|| black_box(hooks.is_empty()))
    });
}

/// Calling N sync hooks against a request needs a `&mut GooseUser` for the
/// hook signature; building one without a running attack needs
/// `GooseUser::single`, so this measures the hook loop itself, with a
/// user built once outside the timed loop (building a user is a one-time,
/// per-user-lifetime cost in real use, not a per-request one). The request
/// itself is also built once outside the loop: only `HookRequestParts::new`
/// and the hook calls are inside the timed closure.
fn bench_n_sync_hooks(c: &mut Criterion) {
    let config = goose::config::GooseConfiguration::parse_args_default(&[] as &[&str]).unwrap();
    let base_url = Url::parse("http://127.0.0.1/").unwrap();

    for &n in &[1usize, 3, 5] {
        let hooks: Vec<BeforeSendHook> = (0..n).map(|_| add_header_hook()).collect();
        let mut user = goose::goose::GooseUser::single(base_url.clone(), &config).unwrap();
        let mut request = request_with_some_headers();
        c.bench_function(&format!("sync_hooks_{n}"), |b| {
            b.iter(|| {
                let mut parts = HookRequestParts::new(&mut request);
                for hook in &hooks {
                    black_box(hook(&mut parts, &mut user));
                }
            })
        });
    }
}

fn bench_one_async_hook(c: &mut Criterion) {
    use futures::executor::block_on;
    let hook: goose::goose::hooks::AsyncBeforeSendHook =
        Arc::new(|mut headers: http::HeaderMap| {
            headers.insert("x-bench-trace", "1".parse().unwrap());
            Box::pin(async move { (headers, BeforeSendOutcome::Continue) })
        });
    let base_headers = request_with_some_headers().headers().clone();
    c.bench_function("one_async_hook", |b| {
        b.iter(|| black_box(block_on(hook(base_headers.clone()))))
    });
}

criterion_group!(
    benches,
    bench_is_empty_check,
    bench_n_sync_hooks,
    bench_one_async_hook
);
criterion_main!(benches);
