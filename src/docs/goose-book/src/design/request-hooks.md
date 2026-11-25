# Request Hooks: Design Investigation

**Decision: build smaller in 0.20** — a `before_send` hook and an `on_response` hook
(which also drives a bounded, fully-measured retry), behind an off-by-default
Cargo feature (`request-hooks`), with no change to any built-in. These two
hook points *design for* five of the six use cases below: signing/auth,
correlation headers, response checks, token-pull into the next request, and
fault injection — of which this branch's prototype actually implements the
first four; fault injection's mechanism (`before_send`'s `FailWith`) is
specified but not honored by the prototype, see
[Follow-ups](#follow-ups). The sixth, recording traffic to an arbitrary
sink, is **not**
served by 0.20's two hook points — a third hook, `on_metric`, would serve it
but is cut from the 0.20 scope (see the [hook points table](#hook-points) and
[Follow-ups](#follow-ups)): it is sound, but this investigation found no
concrete load test asking for it yet, against the generally-applicable
principle that the smaller the shipped surface, the smaller the thing a
maintainer has to keep compatible past 1.0. See [The bar](#the-bar) for the
numbers this rests on.

This document is the record of that investigation: the problem, the survey of
Goose's own code and of five other load testing tools plus two Rust
middleware crates, the design and its review history, and the benchmark that
backs the decision. The prototype behind `request-hooks` exists so these
numbers are reproducible, not because the feature is recommended for general
use yet.

## The problem

Goose's first requirement is a low cost per request: it has to spend less CPU
making a request than the thing it's testing spends answering one, or it
becomes the bottleneck instead of the measuring stick. Users repeatedly ask
for the same handful of things that today require copy-pasted code in every
transaction function: signing or adding auth to every request, correlation
and trace headers, a response check applied uniformly, pulling a token from
one response into the next request, recording traffic to a side channel, and
injecting faults. A hook layer is the generic answer to all of these — but
only if it costs close to nothing when unused and very little when used.

Four rulings bound the design, repeated here because every choice below cites
one of them:

1. By default, everything a hook does counts in the request's measured time.
2. A hook may change what counts (e.g. by retrying, or by excluding its own
   time), but any such change must be visible — Goose measures the hidden
   time and the extra attempts and reports both. Nothing is silently excluded.
3. Performance comes first. A design that costs measurable CPU when no hooks
   are registered fails.
4. This is a pre-1.0 window: breaking changes are allowed in 0.20 if worth
   it; deferring past 1.0 is an acceptable outcome.

## Survey of Goose's own code

`GooseUser::request()` (`src/goose.rs:2333-2583`, as of `main` at the time of
this investigation, before the prototype below split it into
`prepare_request`/`send_request`/`finish_request` and moved these line
ranges — the function names are stable reference points past this point in
the document, exact line numbers in this section and in the critic-round
log are not) is one `async fn` that already does everything a hook
layer would generalize, hardcoded in sequence:

| Step | What runs | State available there |
|---|---|---|
| 1 | `read_unread_bodies()` — drain bodies a prior response dropped unread | `&mut GooseUser` only; no request yet |
| 2 | Build or take the `RequestBuilder`, determine the request's name | `GooseRequest` (path/method/name), `&GooseUser` |
| 3 | Throttle wait (`is_throttled`, `throttle` channel) | nothing about this request; a channel send |
| 4 | **`started = Instant::now()`** | — the request is "officially" timed from here |
| 5 | `request_builder.build()` → `built_request: reqwest::Request` | the built request: method, URL, headers, body |
| 6 | Copy headers/body for request/debug/error logs, only if one of those logs is on | `&built_request`, config flags |
| 7 | Add `Accept-Encoding: gzip` if `self.gzip` and the test didn't set its own | `&mut built_request.headers_mut()` |
| 8 | Clone the request if idempotent, for a possible resend | `&built_request` |
| 9 | `self.client.execute(built_request).await` → `response: Result<Response, Error>` | the live network call |
| 10 | Resend once if the server closed the connection before any response arrived | the cloned request, the error |
| 11 | `request_metric.set_response_duration(started.elapsed())` | TTFB is recorded here, body not yet read |
| 12 | Wrap the response for unread-body tracking (`GooseHttpResponse::tracked`/`untracked`) | `response`, `&GooseUser.unread_bodies` |
| 13 | Decide success/failure from status code, `expect_status_code`; handle `sticky_follow` | `&response` (status, URL), `GooseRequest.expect_status_code` |
| 14 | Coordinated omission mitigation bookkeeping | `&request_metric`, `&mut GooseUser.request_cadence` |
| 15 | Send the finished `GooseRequestMetric` to the parent for aggregation | `request_metric` (owned, about to be cloned) |
| 16 | `error_on_fail` — turn a failed request into a `TransactionError` | `&request_metric` |

Every plausible hook point is a seam between two of these steps. The table
in [Hook points](#hook-points) below maps each candidate seam back to this
list.

A planned refactor (not yet on `main`) splits this into three private parts
without changing behavior: **prepare** (steps 1-2, everything before the
throttle), **send** (steps 3-11, the throttle token through the response
duration), and **finish** (steps 12-16, on `&mut self`). A planned
`GooseUser::fetch_all` would run many **send**s in parallel. This design
targets hook points on exactly those three seams, because they are where the
refactor already draws its boundaries, and because `fetch_all`'s parallel
sends only work if a `send`'s hooks are self-contained per call (see
[Fit with `fetch_all`](#fit-with-fetch_all-and-a-non-reqwest-client)).

A second planned spike may add a raw hyper client beside reqwest. Hook state
is therefore built from `http::Request`/`http::Response` parts (method, URI,
`http::HeaderMap`, and body bytes where needed), never `reqwest::Request` or
`reqwest::Response` directly, so a hook written against Goose's hook API does
not have to change if the client underneath does.

## Survey of other tools

Full detail (with doc citations) is in the review history; summarized here
grouped by how each tool treats a hook that retries or adds its own time —
the question this design cares most about, since rulings 1 and 2 are exactly
about that.

- **JMeter** (PreProcessor/PostProcessor, run before/after a Sampler,
  attachable per-sampler or scoped to a Thread Group) has an explicit
  toggle, unchecked by default: "include duration of timer and pre/post
  processors in generated sample." Hidden by default, but the user can turn
  visibility *on* for the whole run, not per-hook.
  (jmeter.apache.org/usermanual/glossary.html,
  jmeter.apache.org/usermanual/component_reference.html)
- **Artillery** (`beforeRequest`/`afterResponse`, per-request or
  per-scenario) does not document whether hook time counts toward its
  reported TTFB, and has no built-in retry — this is simply unspecified.
  (artillery.io/docs/reference/engines/http)
- **Locust** (global `request` event, and a `measure()` context manager for
  custom clients) puts the choice entirely on the test author: Locust's own
  instrumentation times the network call, but a hand-rolled retry loop is
  timed only if the author wraps it in `measure()`. Nothing is hidden or
  shown automatically either way.
  (docs.locust.io/en/stable/writing-a-locustfile.html)
- **Gatling** (`sign`/`transformResponse` around the request/response,
  `tryMax(n)` for retries) logs **every** attempt of a `tryMax` loop as its
  own separate sample — retries are never merged into one number.
  (docs.gatling.io/concepts/timings/, docs.gatling.io/reference/script/core/scenario/)
- **k6** has no interceptor chain at all; a retry is just another explicit
  `http.get()` call in the script, so it's automatically its own full,
  independent sample. (grafana.com/docs/k6/latest/using-k6/http-requests/)
- **tower** (`Service`/`Layer`) is the general Rust shape: layers compose
  through generics by default (static dispatch, no allocation), and only
  `BoxService`/`BoxCloneService` erase to a trait object with boxed futures
  when heterogeneous storage is needed. (docs.rs/tower)
- **`reqwest-middleware`** boxes every middleware call (`#[async_trait]`),
  and its `reqwest-retry` extension loops *inside* a single `handle()`
  call — an outer tracing middleware sees one span covering every retry
  attempt combined. Retry time is folded into one sample, not surfaced.
  (github.com/TrueLayer/reqwest-middleware)

Two lessons carried into the design: first, no tool treats "the hook's time"
as a free pass — every one either counts it, lets the author choose, or is
silent (which this design's rulings explicitly forbid being). Second, the
closest Rust precedent for "retry inside a middleware call"
(`reqwest-middleware` + `reqwest-retry`) hides exactly the thing ruling 2
requires Goose to show, and does so by boxing a recursive continuation
(`Next`) — that shape is both the wrong semantics for Goose and not free.
This design deliberately avoids it; see
[Why no generic "around send" hook](#why-no-generic-around-send-hook).

## Design

### Hook points

| Point | Keep/Drop | Reason |
|---|---|---|
| Before the request is built | Drop | No capability a transaction function doesn't already have: it can mutate `GooseRequest` itself before calling `.get()`/`.post()`/etc. A hook here buys nothing over a line of code, for the cost of another dispatch check on every request. |
| **`before_send`** — after build, before `client.execute` | **Keep** | Only place to apply something to *every* request without touching every transaction: signing, auth, correlation/trace headers, request-side fault injection. Sits inside the already-running `started` clock, so its time counts by ruling 1 with no extra `Instant::now()` call. Takes `&mut GooseUser` (not just the request) so a hook can read session state (e.g. a token an earlier `on_response` hook stashed) and so a hook can scope itself to specific scenarios/transactions by reading `scenario_name`/`transaction_name` off the user, despite registration being attack-wide (see [Dispatch](#dispatch)). Specified to return `FailWith(reason)`, to short-circuit the network call entirely and synthesize a transport error — the fault-injection use case — without Goose ever calling `client.execute`. **Not honored by this branch's prototype**: `call_before_send_hooks` (`src/goose.rs`) logs and otherwise ignores a `FailWith`, sending the request anyway. Fault injection is sound as a design but unimplemented; see [Follow-ups](#follow-ups). |
| Around send (generic wrapping/continuation, à la `tower::Layer` or `reqwest-middleware`'s `Next`) | Drop | See [below](#why-no-generic-around-send-hook). Replaced by a narrower, Goose-driven retry loop off `on_response`'s decision. |
| **`on_response`** — after the response (or transport error) arrives, before success/failure is decided | **Keep** | Covers response checks, status-based retry, and token extraction into session state for the next request. Can ask Goose to retry; Goose's own loop (not the hook) performs the retry and measures its duration itself, so the attempt count and hidden/extra time stay centrally measured and can't be spoofed by a buggy hook (ruling 2; see [Must-fix 1, round 1](#round-1)). **Correction (round 6):** an earlier draft of this row also claimed "response-side fault injection — a hook can treat a 200 as a failure, or vice versa." Neither the `ResponseOutcome` enum (`Accept`/`Retry` only, [Dispatch](#dispatch)) nor the real prototype (`src/goose/hooks.rs`) has any way for a hook to override the status-code-driven success/failure verdict; `error_on_fail`/`expect_status_code` read only the response itself, never a hook's opinion of it. Response-side fault injection is **not** a capability this design provides; a hook wanting to simulate a bad response has to do so via `before_send`'s `FailWith` (itself unimplemented in this prototype, see [Follow-ups](#follow-ups)), not via `on_response`. |
| Per-chunk body hooks | Drop (verdict only, not expanded) | The per-request cost of a hook call on every chunk of every body does not clear the bar for the fraction of load tests that would use it; body-level needs are already served today by `GooseResponse::discard_body`, `.text()`/`.bytes()`/`.json()`, and gzip handling, none of which this design touches. Out of scope per the brief beyond this verdict. |
| `on_metric` — after the metric is finalized, read-only, before it's sent to the parent | Sound, but **cut from 0.20** | Would serve "record traffic to an arbitrary sink" (the one use case the other two hook points don't cover — request/debug/error logs already do fixed-format recording). Costs nothing to the measured times (runs after `response_time` is finalized, same timing position as request-log serialization today). Left out of the 0.20 scope because the review rounds found no concrete load test asking for it yet, and two hook points reviewed faster and cleaner than three; see [Follow-ups](#follow-ups). |
| On error, as a point separate from `on_response` | Drop | `on_response` receives `Result<ResponseParts, &TransportError>` and runs on both branches, so a separate error hook would only split dispatch in two for no new capability. |

### Why no generic "around send" hook

A hook that wraps the whole `client.execute().await` call (tower's `Layer`,
`reqwest-middleware`'s `Middleware::handle` with a `Next` continuation) is
the most expressive shape and the one every existing Rust middleware crate
picked. Three reasons rule it out here:

1. **Measurement.** Ruling 2 requires Goose to measure hidden time and extra
   attempts itself. If an arbitrary closure owns the retry loop, Goose has
   no seam to instrument — it would have to trust the hook to report its own
   attempts honestly, which is exactly the silent-exclusion failure mode
   rulings 1 and 2 exist to prevent. A Goose-owned loop around a narrower
   "retry or accept" decision keeps the measurement in Goose's code, not the
   hook's.
2. **Cost.** A generic wrapping hook that can be sync or async, called zero
   or more times recursively, forces a boxed `dyn Future` at the call site
   even for a hook that never retries — `reqwest-middleware` pays this for
   every single request through every middleware, every time, because the
   continuation itself must be a trait object. The narrower design pays a
   boxed future only on the (expected to be rare) path where a hook actually
   decides to retry.
3. **Non-reqwest client.** A continuation type has to close over "the rest
   of the send," which means closing over whichever client (reqwest today,
   hyper tomorrow) is doing the sending. A narrower "decide from what came
   back" hook only ever sees `http::Request`/`http::Response` parts, so it
   doesn't care which client produced them.

### Dispatch

- **Type erasure, not generics.** `GooseUser`, `GooseAttack`, `Scenario` and
  `Transaction` may not gain a type parameter. Hooks are therefore boxed
  closures behind an `Arc`, stored in `Vec`s — the same shape
  `reqwest-middleware` and most Rust middleware crates land on for the same
  reason (heterogeneous, runtime-registered behavior behind one concrete
  field type), and the same shape Goose itself already uses for
  `GooseAttack::set_worker_data`'s `Fn(&WorkerShare) -> Vec<u8> + Send + Sync + 'static`
  hook (`src/lib.rs`). A closure, not a named trait a user implements, matches
  that existing idiom and everything `test_start`/`test_stop`/`transaction!`
  already do (wrap a plain function or closure, not an `impl Trait`
  boilerplate). Concretely:

  ```rust
  pub type BeforeSendHook =
      Arc<dyn Fn(&mut HookRequestParts, &mut GooseUser) -> BeforeSendOutcome + Send + Sync>;
  pub type OnResponseHook =
      Arc<dyn Fn(Result<&HookResponseParts, &TransportError>, &mut GooseUser) -> ResponseOutcome + Send + Sync>;

  pub enum BeforeSendOutcome {
      Continue,
      /// Skip the network call; synthesize a transport error with this
      /// message instead. The fault-injection use case.
      FailWith(String),
  }

  pub enum ResponseOutcome {
      Accept,
      /// Ask Goose to retry. `hide_this_attempt` only ever *marks* the
      /// attempt Goose is about to measure as excludable; it carries no
      /// duration — see Timing, and round 1's must-fix 1.
      Retry { hide_this_attempt: bool },
  }
  ```

  `HookRequestParts`/`HookResponseParts` wrap `&mut http::HeaderMap` and
  `&http::Method` directly — reqwest re-exports those exact http-crate types
  with no conversion (`pub use http::header`, `pub use http::Method` in
  reqwest's `src/lib.rs`; hyper re-exports the identical types too, so this
  holds for a hyper-backed client as well), so reading or mutating a header
  is a plain reference, zero-cost either client backend. The request/response
  URL is **not** eagerly converted: reqwest stores a `url::Url`, not an
  `http::Uri`, and converting between them is a parse-and-allocate each way
  (verified against reqwest's own `TryFrom` impls). The two structs' sketch —
  note `HookRequestParts` is the one place this prototype does not yet match
  its own target shape, below:

  ```rust
  pub struct HookRequestParts<'a> {
      headers: &'a mut http::HeaderMap,
      method: &'a http::Method,
      url: &'a Url,
      // Written the first time `url()` is called; `before_send` gets `&mut
      // self` already, so a plain field suffices here.
      uri_cache: Option<http::Uri>,
  }
  impl<'a> HookRequestParts<'a> {
      pub fn headers_mut(&mut self) -> &mut http::HeaderMap { self.headers }
      pub fn method(&self) -> &http::Method { self.method }
      pub fn url(&mut self) -> &http::Uri { /* fills uri_cache on first call */ }
  }
  ```

  **What this branch's prototype actually built** (`src/goose/hooks.rs`)
  holds a single `request: &'a mut reqwest::Request` field instead of three
  separate `http`-typed fields, and has every accessor re-borrow through it
  (`self.request.headers_mut()`, `.method()`, and a parse-and-cache `url()`)
  — same public method names and signatures as the sketch above, so a
  hook's own code is unaffected, but internally this prototype is coupled to
  `reqwest::Request`, not decoupled from it the way the "never a reqwest
  type" claim below describes as the goal. `HookResponseParts` has no such
  gap — it already matches its sketch exactly, built from already-extracted
  `StatusCode`/`&HeaderMap`/`&Url`, no reqwest type retained. Reaching the
  sketch above for the request side (so a future hyper-backed client needs
  no change to `HookRequestParts` itself) is a follow-up, not done here; see
  [Follow-ups](#follow-ups).

  ```rust
  pub struct HookResponseParts<'a> {
      status: http::StatusCode,
      headers: &'a http::HeaderMap,
      url: &'a Url,
      // `on_response` only gets `&HookResponseParts` (a shared reference —
      // it must not let a hook mutate a response it didn't produce), so the
      // memo needs interior mutability, unlike the request side above.
      uri_cache: std::cell::OnceCell<http::Uri>,
  }
  impl<'a> HookResponseParts<'a> {
      pub fn status(&self) -> http::StatusCode { self.status }
      pub fn headers(&self) -> &http::HeaderMap { self.headers }
      pub fn url(&self) -> &http::Uri { self.uri_cache.get_or_init(|| /* parse */) }
  }
  ```

  So a header-only hook (signing, trace headers — most of the named use
  cases) never touches the URI conversion at all, and a URL-reading hook pays
  for it once per request, not once per hook — but the mechanism is not
  symmetric between the two structs: `HookRequestParts`' cache is a plain
  field because `before_send` always gets exclusive `&mut` access, while
  `HookResponseParts`' cache needs `OnceCell` because `on_response` is handed
  a shared `&HookResponseParts` (round 2 lane (b) flagged the asymmetry;
  round 1's "memoized" description had only this plain-field case in mind).
- **Registered at the attack level**, the way `test_start`/`test_stop` and
  `GooseDefault` already work — not per-scenario or per-transaction. A
  `GooseAttack` builds the hook set once, wraps it in one `Arc`, and every
  `GooseUser` gets an `Arc::clone` of it (a pointer bump, the same cost
  `GooseUser::clone` already pays for `scenario_name`, `metrics_epoch`, and
  several other `Arc` fields). Per-scenario hook registration is a plausible
  future extension but was not required by any surveyed use case, and the
  real reason it's deferred is implementation complexity, not CPU cost —
  `GooseUser` already resident-carries `scenarios_index`/`transaction_index`
  and reads them on every request for `TransactionDetail`
  (`src/goose.rs:2513-2517`), so indexing a per-scenario hook list would cost
  about the same as the `Vec::is_empty()` check this design already treats
  as free. Since attack-level hooks already take `&mut GooseUser`, a hook
  that only wants to apply to one scenario can read `scenario_name` off the
  user and self-filter — not as clean as real per-scenario registration, but
  enough to avoid forcing a hook-less test author back to copy-pasted code
  for that one case; see [Follow-ups](#follow-ups).
- **Sync is the primary shape.** A hook's main form is a plain closure
  returning a plain enum (`BeforeSendOutcome`/`ResponseOutcome`), not a
  `Future`. Every surveyed use case in the problem statement — signing, trace
  headers, response checks, token extraction into session state, fault
  injection — is CPU-only; none of them need to `.await` inside the hook
  itself.
- **Async is an explicit, separately-costed variant.** A hook that must do
  I/O (e.g. calling an external token-refresh service) is registered through
  a parallel `register_*_hook_async` method, storing
  `Arc<dyn Fn(...) -> BoxFuture<'static, Outcome> + Send + Sync>` in a
  separate `Vec` from the sync hooks. Only calling an async hook allocates a
  boxed future (`futures::future::BoxFuture`, already a transitive dependency
  through `futures`, no new crate needed); a sync hook, and the zero-hook
  path, never do. This is the one place the design pays per-call allocation,
  and only for load tests that register one. **This branch's prototype only
  goes as far as `GooseUser::with_before_send_hook_async`** (test/benchmark
  scaffolding, not an attack-level registration method, and `before_send`
  only — see [Public API](#public-api-and-migration-cost)); an attack-level
  `register_before_send_hook_async`/`register_on_response_hook_async` pair
  is specified here but not built, a follow-up.
  The async `before_send` signature takes an **owned** `http::HeaderMap`
  (`AsyncBeforeSendHook = Arc<dyn Fn(HeaderMap) -> BoxFuture<'static, (HeaderMap, BeforeSendOutcome)> + Send + Sync>`),
  not a borrowed `HookRequestParts`, specifically because a `before_send`
  hook holds `&mut GooseUser`'s borrow across the `.await`, and a borrow
  tied to the caller's `reqwest::Request` cannot cross that `.await` as a
  `'static`-free trait object without a higher-rank lifetime bound. The
  owned-headers clone this needs is a second, real cost the async path pays
  beyond the boxed future — both are deliberately paid only when an async
  hook is registered, never on the sync or zero-hook paths.

### The zero-hook fast path

When the `request-hooks` Cargo feature is off (the default), none of this
code exists in the binary — not a branch, not a field. This is the
strongest possible zero-cost claim and the one the default build gets.

When the feature is on but no hook is registered, `GooseUser` carries one
extra field, `hooks: Arc<HookSet>`, cloned (pointer bump) into every user the
same way several existing `Arc` fields already are. Each of the two kept
hook points is checked with `Vec::is_empty` reads — no allocation, no
virtual call — before falling through to the existing code unchanged. (The
exact count of these checks, and the small amount of unconditional
bookkeeping alongside them, drifted across edits to this document more than
once; [The bar](#the-bar) is the one place that count is kept accurate
against the current code, rather than restated here too, to stop that from
happening again.) This is the arm the benchmark calls "spike with no
hooks"; see [The bar](#the-bar) for what it actually costs.

On the registered-and-called side (not the zero-hook path, so outside
ruling 3's bar, but worth being honest about): calling a hook needs `&mut
GooseUser` passed into the closure while iterating `self.hooks`, which the
borrow checker won't allow directly (can't hold a loan on a field of `self`
while also passing `self` by `&mut`). The call site needs
`let hooks = Arc::clone(&self.hooks)` — one atomic increment/decrement
pair, on the order of a few ns, paid only when at least one hook is
registered. In the prototype, this clone happens once per `before_send`
call and once per loop iteration inside `run_on_response_hooks` — so a
request that retries twice pays three such clones, not one per request as
an earlier draft of this paragraph implied (round 3 lane (b) caught the
discrepancy); each is still only a few ns, and the loop itself only runs
more than once when a hook actually retries. [Round 2 lane (b)](#round-2)
is the review pass that first surfaced the clone's existence; it is folded
into the per-hook-call cost this document attributes to "`Vec::is_empty`,
virtual dispatch, enum match" and does not,
on its own, change the conclusion in [The bar](#the-bar).

A second cost round 3 found unaccounted: the clone `send_request` keeps
ready for a hook-driven retry (described under [Timing](#timing)) was
originally taken whenever any `on_response` hook was registered, full stop
— so a non-idempotent request under the default idempotent-only
`RetryPolicy`, which can never actually be retried, still paid for a real
`reqwest::Request::try_clone()` (a `HeaderMap` clone, a `Url` clone, and a
body clone) on every request. **Fixed in the prototype**: the clone is now
taken only if at least one registered `on_response` hook's `RetryPolicy`
could actually permit retrying this request's method — a cheap
method-comparison per hook, not an allocation, replaces an unconditional
allocation for exactly the requests that could never use it.

Adding this field is a forcing function in its own right: `GooseUser`'s
`Clone` impl is hand-written field-by-field (`src/goose.rs:1424`, no
`#[derive(Clone)]`), so the new `hooks` field must be added there explicitly
or the code won't compile — the same class of compiler-enforced reminder the
Gaggle wire-format changes below rely on.

### Timing

`started = Instant::now()` does not move. `before_send` is called between
build (step 5) and `execute` (step 9) — inside the already-running clock, so
its CPU time is counted in `response_time` by ruling 1 automatically, with no
extra timer call needed to make that true.

`on_response` is called **before** `request_metric.set_response_duration(...)`,
not after — so a hook's own work inspecting or acting on the response also
falls inside the measured window by default, the same way `before_send`
does. This is a deliberate placement choice: the obvious place to put this
hook is *after* TTFB is recorded (so the hook's cost doesn't pollute
`response_time`), but that would silently violate ruling 1 by construction.
Measuring TTFB after the hook runs costs nothing extra when the hook accepts
immediately (the existing `started.elapsed()` call simply happens a few
instructions later than it does today); it only costs an additional
`Instant::now()` pair when a hook actually asks for a retry, see below.

Retry loop, run by Goose, not the hook: `on_response` returns
`ResponseOutcome::Accept` or `ResponseOutcome::Retry { hide_this_attempt: bool }`.
Critically, **the hook never hands Goose a `Duration`** — it only signals a
yes/no policy choice ("is this attempt's time worth calling out separately");
Goose always measures the actual time itself. A hook that lies about how long
something took has nothing to lie *to*: there is no number for it to supply.
On `Retry`, Goose:

1. Checks whether the request's method is idempotent (`is_idempotent()`, the
   same check `src/goose.rs:2559` already uses for resend-on-close) — the
   identical reasoning applies: replaying a `POST` against a live backend can
   duplicate a side effect the first attempt already caused, and Goose can't
   tell whether it did. A hook that returns `Retry` for a non-idempotent
   method is **not honored** by default; Goose logs this at debug level and
   treats the response as `Accept`. A load test that genuinely wants to retry
   a non-idempotent request (e.g. it knows its own endpoint is safe to
   repeat) opts in explicitly when registering the hook — a policy argument
   on registration (`register_on_response_hook(f, RetryPolicy::default().allow_non_idempotent())`,
   not a second same-purpose method with a 56-character name — the first
   draft of this section proposed exactly that, and round 2's lane (c) called
   it out as unscalable) — the risk then is the test author's, stated
   plainly, not a default anyone falls into by accident.
2. **What "re-run the send" actually resends.** The original `RequestBuilder`
   is gone by this point — `RequestBuilder::build()` consumes it
   (`src/goose.rs:2469` already takes it once), so there is no builder
   left to call step 5 on again. A retry resends a **clone of the already-built
   `Request`**, exactly the mechanism the existing resend-on-close path uses
   today (`built_request.try_clone()`, `src/goose.rs:2559-2563`) — which is
   why the idempotency check above matters even before considering a hook's
   involvement, and why it has the same failure mode: `try_clone()` returns
   `None` for a request with a non-buffered (streamed) body, even an
   idempotent one. When that happens, a hook's `Retry` is **not honored**
   either, the same way a non-idempotent method's isn't — logged at debug
   level, treated as `Accept`. One consequence worth being explicit about:
   `before_send` runs again on the clone before it's resent, so a hook can
   still mutate headers on a retry (refresh an auth token and re-sign, the
   named use case) — but it is mutating a clone of the first attempt's
   request, not a fresh one built from the transaction's original call, so it
   cannot change the body or the URL on a retry, only headers.
3. Calls `before_send` again on that clone (a `FailWith` here is handled
   identically to a transport error reaching `on_response` below — see
   point 5), then resends it, bracketing the attempt with Goose's own
   `Instant::now()` pair. If the earlier `on_response` call's outcome had
   `hide_this_attempt: true`, that measured duration — never a hook-supplied
   one — is added to a running `hidden_time_us` total, and `attempts` is
   incremented (this is the *only* place this design allows a hook to mark
   time as excludable, and even then never destructively — see
   [Visibility model](#visibility-model)).
4. Calls `on_response` again on the new result.
5. Repeats until `on_response` returns `Accept`, or until `attempts` reaches a
   hard cap (the cap bounds the total number of attempts including the
   first, so a cap of 3 permits at most 2 retries) to bound worst-case
   latency; reaching the cap behaves as `Accept` with the last response
   obtained. This prototype hardcodes the cap at 3
   (`hooks::MAX_ATTEMPTS`, `src/goose/hooks.rs`) rather than reading it from
   `GooseConfiguration`; making it configurable is a small follow-up, not
   done here. `error_on_fail`, `expect_status_code`,
   and `sticky_follow` all evaluate only this final, accepted response —
   exactly as they read `request_metric` today — never anything about how
   many attempts it took to get there. A `before_send` `FailWith` on *any*
   attempt (the first or a retry) is treated exactly like a transport error
   for that attempt: it skips the network call, is bracketed by that
   attempt's own `Instant::now()` pair same as a real send, and is handed to
   `on_response` as `Err(&TransportError)` — so a `FailWith` can itself
   trigger a further `Retry`, gated by the same idempotency/cap checks as any
   other attempt, and cannot be used to bypass them.

When more than one `on_response` hook is registered, they run in
registration order against the same response. **A hook whose `Retry` is
actually honored short-circuits the rest for that attempt** — a hook
registered after it (e.g. a token-extraction hook meant to inspect every
response) does not run on an attempt a retry is about to replace. A load
test registering several `on_response` hooks, some of which retry, should
order them with that in mind. **Correction (round 6):** an earlier version
of this prototype short-circuited the remaining hooks even when a `Retry`
was asked for and then *denied* (non-idempotent method without opt-in, or
the attempt cap already reached) — in that case there is no replacement
attempt, this response stays final, and every hook registered after the
denied one was silently skipped on it, with nothing in the debug log
pointing at the skip. **Fixed in the prototype**: only an *honored* `Retry`
stops the loop; a denied one logs and continues to the next hook, so every
registered hook still sees whatever response ends up final.

Two related gaps round 6 found and left as documented limitations, not
fixed in code, given this branch's remaining scope: a hook-driven retry's
own `self.client.execute(clone).await` does not get the resend-on-close
protection the very first attempt gets (a connection closed mid-retry
becomes a transport error handed to `on_response`, consuming one of the
capped attempts, rather than being silently retried once more the way the
first attempt would be) — so "a single hook-level attempt can itself
include a connection-close resend underneath it," stated above, is only
guaranteed true of the first attempt. And `hidden_time_us`'s
`Instant::now()` bracket starts *after* `before_send` runs again on a
retried attempt, so a `before_send` hook's own CPU time on that retry is
counted in `response_time` (as ruling 1 requires) but not added to
`hidden_time_us` even when the hook asked for that attempt to be hidden —
a minor precision gap in the derived "excluding hook overhead" figure, not
a ruling violation.

The extra `Instant::now()` pair is paid only inside this loop — i.e. only on
requests a hook actually decided to retry — never on the path where every
hook present simply returns `Accept` on the first attempt. Combined with the
zero-hook path above, the rule is: **you pay for what you use**, at three
granularities (feature compiled out → no hook registered → hook registered
but doesn't retry → hook retries), each one costing more than the last, and
the first three are all expected to be near-zero.

This hook-driven `attempts` count is a different number from the existing
`resends` counter (`src/goose.rs:2610-2611`, incremented when the server closes a
connection before any response arrives and Goose silently resends once,
inside what still counts as a single attempt here). The two are not merged:
a single hook-level attempt can itself include a connection-close resend
underneath it. A report showing both should label them distinctly — the
existing `resends` figure is a run-wide total today (not per-request), and
stays that way; `attempts` is per-request and new.

### Visibility model

**Correction (round 3 found the previous wording of this paragraph false):**
`attempts` and `hidden_time_us` **do exist** in this branch, behind
`request-hooks`: as real fields on `GooseRequestMetric`
(`src/metrics.rs:971-980`, defaulted at `:1010-1013`), populated every
request by the retry loop described in [Timing](#timing)
(`src/goose.rs:2622-2637`, written back into the metric around
`:2643-2644`), and already flowing into the JSON, raw and pretty request
logs for free (verified by the tests around `src/logger.rs:1458-1497` and
`:1750-1767`). What genuinely is **specification only, not implemented in
this branch** — per the brief's explicit out-of-scope list — is everything
*downstream* of the per-request metric: aggregation into
`RequestBatchEntry`/`GooseRequestMetricAggregate`, the CSV log column,
console tables, reports, the dashboard, Gaggle, and coordinated omission's
internal arithmetic. The rest of this section describes that downstream,
unimplemented half; the two fields themselves and their per-request
population are real code, today, in this branch.

Two new fields on `GooseRequestMetric` (and the matching aggregate):

- `attempts: u32`, default `1`. Incremented once per `on_response` retry.
  Never zero; a request Goose never sent (e.g. throttle channel closed) is
  already an error path that doesn't reach this far.
- `hidden_time_us: u64`, default `0`, **microseconds** (consistent with
  every other `_us`-suffixed field in the codebase — `response_time` itself
  is whole milliseconds, see below). The sum of time a hook explicitly asked
  to exclude from "the" response time, across every retried attempt.

Critically: **`response_time` itself is never reduced.** It is always the
full wall-clock time from `started` to the final accepted response,
including every retried attempt and every hook's own CPU time, exactly as
ruling 1 requires by default. `hidden_time_us` is purely additive
information alongside it, not a subtrahend baked into the headline number.
A report, the dashboard, or the console can choose to *also* show a derived
"excluding hook overhead" figure, computed in **matching units** —
`response_time_micros().saturating_sub(hidden_time_us)` (both microseconds;
`response_time_micros()` already exists on `GooseRequestMetric`,
`src/metrics.rs:1018-1020`), never the raw millisecond `response_time` field
minus a microsecond `hidden_time_us` — but the raw field a load test or a CI
threshold already reads today never silently changes meaning. This is the
single most important integrity property of the design, and the one the
correctness review rounds spent the most time on (see
[Critic rounds](#critic-rounds)) — including a round-2 finding that an
earlier draft of this very paragraph wrote `response_time.saturating_sub(hidden_time_us)`,
mixing milliseconds and microseconds, which for almost any real hidden retry
(anything over roughly a millisecond) would have saturated to zero instead
of subtracting out only the hidden portion.

Coordinated omission mitigation's backfill math
(`MetricsProcessor::process_request_metric`, `src/metrics.rs`) subtracts the
triggering request's `response_time` from the stall duration, on the
assumption that `response_time` is real server-side latency. A
hook-inflated `response_time` (from a retry) would corrupt that subtraction
whether or not Goose also reports `hidden_time_us` — the raw number still
goes into the backfill math unless CO mitigation is told to use an
hidden-time-adjusted figure for that one calculation instead of the raw
field. **Specification for report/CO code (out of scope to implement here,
per the brief):** coordinated omission's cadence/backfill arithmetic is
millisecond-denominated throughout (`response_time` itself, `user_cadence`,
`elapsed`), so it should read `response_time.saturating_sub(hidden_time_us / 1_000)`
(hidden microseconds converted down to whole milliseconds, matching the unit
that arithmetic already uses) wherever it reads `response_time` today, so a
hook's deliberately-hidden retry time does not get mistaken for a slow
server. The *reported*, user-facing `response_time` field is unaffected by
this — only CO's internal "what does slow mean" arithmetic reads the
adjusted figure.

A second, more subtle CO interaction, surfaced in round 2: CO's backfill loop
(`process_request_metric`, when `coordinated_omission_elapsed > 0`) clones
the triggering `request_metric` once per synthetic "missed" request it
generates to fill in a stall, and records each clone. A clone represents a
request that was never actually sent — it must not carry the real request's
`attempts`/`hidden_time_us` into the aggregate as if each synthetic entry had
also been retried; whatever accumulation this feature adds must zero both
fields on every CO-synthesized clone before it is recorded, matching the
principle (not literally copied code) `record_status_code_time_us` already
applies by being skipped for `update` requests. Separately, the real
triggering request's own `attempts`/`hidden_time_us` still need to reach the
aggregate through whichever of the three paths below actually carries its
real metric in the CO-triggered case — round 2 found this control flow more
subtle than round 1's spec accounted for, and flagged it as something a
real implementation must trace carefully against `process_request_metric`'s
exact branches on `main` at the time, not assume from this document alone.

How this reaches each surface (specification only; none of this is
implemented in this branch, per the brief's out-of-scope list):

- **Request log**: `GooseRequestMetric` is logged directly for `json`,
  `raw`, and `pretty` formats (`src/logger.rs`), so both new fields appear
  for free with `#[serde(default)]` so old logs still parse. The `csv`
  format lists fields explicitly (`format_csv_row!`, `requests_csv_header()`
  in `src/logger.rs`) and needs both the header and the row macro call
  extended by hand — already flagged in the code as duplicated,
  `@TODO this should be automatically derived from the structure`.
- **Aggregation**: two new accumulators, flat scalars on
  `GooseRequestMetricAggregate` itself (`src/metrics.rs`) —
  `total_attempts: u64` (sum, for an average) and
  `total_hidden_time_us: u64`. These describe real, sent requests only, not
  coordinated omission's synthetic backfilled entries, so they live
  alongside `raw_data`/`coordinated_omission_data`, not duplicated *inside*
  `GooseRequestMetricTimingData` (a response-time histogram with no notion
  of attempts) the way an earlier draft of this document incorrectly
  suggested.

  The non-trivial part: **most requests never reach `record_request_metric`
  at all.** That function (`src/metrics.rs:4216`) is only called from
  `process_request_metric`, which only handles `batch.individual_requests` —
  and `src/goose.rs` is explicit that this path is for **failed and
  CO-affected requests only**. Every successful, non-CO request — including
  the common case this feature exists for, a request a hook retried once and
  then accepted — is pre-aggregated directly into a `RequestBatchEntry` by
  `accumulate_request`/`accumulate_request_by_second`
  (`src/goose.rs:3280, 3364`) and only later folded into the aggregate by
  `merge_batch_entry`, bypassing `record_request_metric` entirely. A spec
  that only touches `record_request_metric` would silently lose `attempts`/
  `hidden_time_us` for the majority of requests — exactly the kind of
  silent exclusion ruling 2 forbids, reappearing one layer down in the
  plumbing meant to report it. The real spec: `RequestBatchEntry`
  (`src/metrics.rs:240-251`) needs its own `attempts_sum`/
  `hidden_time_us_sum` fields, populated in both `accumulate_request` and
  `accumulate_request_by_second`, folded into the aggregate in
  `merge_batch_entry` alongside its existing merges — *in addition to*,
  not instead of, the `record_request_metric` path for the failed/CO case.
  (`src/metrics.rs:4548`.)

  A third path exists besides these two: `GooseUser::set_success`/
  `set_failure` (`src/goose.rs:3755`, `3830`) resend an
  already-recorded `GooseRequestMetric` with `update = true`, which
  `record_request_metric` already special-cases to skip re-recording
  time/status (`src/metrics.rs:4230-4244`) so the first recording isn't double
  counted. Any `attempts`/`hidden_time_us` accumulation added to
  `record_request_metric` needs the same `!request_metric.update` gate, for
  the same reason — round 1's pass over this function didn't name this third
  path; round 2 did, independently, while re-checking round 1's fix.
- **Reports** (`src/report.rs`, `src/report/markdown.rs`): add the two
  fields to `RequestMetric` (`src/report.rs:51-93`), populate them in
  `build_raw`/`build_co` (`src/metrics/common.rs`), and add one column or
  one line per renderer (HTML template, the Markdown row's format string,
  and the JSON report gets both fields automatically since it serializes
  `RequestMetric` directly).
- **Console tables** (`GooseMetrics::fmt_requests`, `write_times_row` in
  `src/metrics.rs`): read straight off the aggregate with no DTO in between;
  add `attempts` and `hidden` to the existing format strings, in the three
  places `write_times_row` is already called (raw/CO/status-code tables).
- **Dashboard / Prometheus** (`src/metrics/dashboard_snapshot.rs`): add the
  two fields to `RequestRow`/`AggregateMetrics`, following the precedent
  already in the struct — `measured_count`/`measured_total_us` are already
  `#[serde(skip)]` fields carried specifically for the Prometheus exporter to
  read without appearing in the JSON dashboard payload the browser gets.
  `src/dashboard/prometheus.rs::render` reads from the already-built
  snapshot, so it gets the new fields for free once the snapshot carries
  them — no separate wiring needed there.
- **Gaggle** (`src/gaggle/delta.rs` `RequestPart`, `src/gaggle/worker.rs`,
  `src/gaggle/merge.rs`): Gaggle does not serialize `GooseRequestMetric` or
  its aggregate directly — it has a hand-maintained wire struct, `RequestPart`,
  encoded with `postcard`. No protobuf schema change and no
  `PROTOCOL_VERSION` bump are needed (the bytes are opaque to the gRPC
  envelope), but `RequestPart` needs the two new fields, `RequestPart::add`
  needs to merge them (with the same overflow-checked `Bounds` the existing
  fields use), and the two exhaustive field-by-field destructurings at the
  worker→delta and delta→manager conversion sites
  (`src/gaggle/worker.rs:289-314`, `src/gaggle/merge.rs:1163-1189`) will fail
  to compile until updated — a forcing function, not a gap, but real work.
  Two more exhaustive destructurings of `GooseRequestMetricAggregate` exist
  and would equally fail to compile unchanged: `ReportRequests::serialize`
  (`src/metrics.rs`, feeds the JSON report's embedded raw metrics) and
  `metrics_diff` in `src/gaggle/compare.rs`, which is explicitly commented
  today as deliberately exhaustive for this exact reason ("a field added to
  any of these fails to compile here until it is compared or left out on
  purpose").

**What this breaks in the existing test suite.** "Entirely additive" is
true for *behavior*, not for *byte-for-byte output a test happens to pin*.
Two tests needed updating once `attempts`/`hidden_time_us` existed on
`GooseRequestMetric` — both **already fixed in this branch**, not a
forward-looking warning: a Debug/JSON exact-string comparison test in
`src/goose.rs` (`request_names_and_log_lines_match_each_naming_mode`) now
routes its expected strings through a small `with_hook_fields` helper that
appends the two new fields' expected text only when `request-hooks` is
compiled in; and `metric_sizes_stay_small` (`src/metrics.rs`) asserts a
different `size_of::<GooseRequestMetric>()` per feature state (272 bytes
without the feature, 288 with it). `standalone_dashboard_snapshot_is_unchanged`
(`src/metrics/golden_tests.rs`) is unaffected in this branch because the
dashboard/report/Gaggle plumbing described above is specification only,
not implemented here — it would need the same `GOOSE_UPDATE_GOLDENS`
treatment only once that plumbing is actually built.

**Documentation.** Two book chapters describe the exact field list a reader
sees today and would need updating once these fields exist:
`src/docs/goose-book/src/logging/requests.md` (documents the request log's
columns, including the CSV header this adds to) and
`src/docs/goose-book/src/getting-started/metrics.md` plus
`src/docs/goose-book/src/coordinated-omission/metrics.md` (document report/
console/CO fields — the latter is exactly where the subtle
`response_time` vs. `response_time - hidden_time_us` distinction from this
section would need to be explained to a reader, not just implemented).

### Built-in verdicts

None of these are changed in this branch (the brief forbids it); each gets a
verdict only.

| Built-in | Verdict | Why |
|---|---|---|
| Throttle | **Stay inline** | It isn't user logic — it's attack-wide infrastructure a load test author never writes a hook for. It already sits exactly at the pre-`started` seam a hook can't usefully improve on, and today's cost is one `bool` check when disabled. Entangling it with a user-ordered hook list adds a failure mode (a user hook racing the throttle) for no benefit. |
| Gzip `Accept-Encoding` header | **Stay inline** | It is, mechanically, exactly what a `before_send` hook does (mutate a header) — but it's `self.gzip`-gated already (a single bool check), and it's entangled with `GooseHttpResponse`'s decoder selection, which reads the same flag. A user `before_send` hook already composes with it correctly as written: Goose's `entry(...).or_insert(...)` only fills the header if a hook (or the test) didn't already set one, so ordering a hook before or after this step produces the same result either way. Converting it to a hook buys composability it already has, at a strictly higher cost than a bool check. |
| Header copying for request/debug/error logs | **Stay inline** | Unrelated to hooks: it is already gated on whether any of those three logs is configured, and a `before_send` hook can read the same headers off the built request directly, for free, before this copy runs. |
| Unread-body draining (`read_unread_bodies`) | **Stay inline / fixed internal step** | Implements reqwest connection-pool hygiene, tightly coupled to an established timing rule already in the codebase (body-read time counts toward the *transaction's* time, never the request's). Turning it into a hook would let a user's hook ordering accidentally defer or skip it, breaking connection reuse in a way that's invisible until throughput drops. |
| Resend-on-close | **Stay inline / fixed internal step** | Needs `reqwest::Request::try_clone()` — a reqwest-specific capability a hyper-backed client may not have the same way, so it can't become a hook without breaking the "hooks never see reqwest types" requirement. It also already has its own visible counter (`resends`, logged at run end); duplicating that accounting inside a generic hook's attempt-count would double-report the same retries under two different names. Its logic (and the throttle's, the gzip header's, and the log-copying's) is byte-for-byte unchanged in this branch, as the brief requires — the one touch nearby is `is_idempotent()` itself gaining `pub(crate)` visibility (from private) so the new retry loop can reuse it; the function's behavior is untouched. |

### Fit with `fetch_all` and a non-reqwest client

Both kept hook points operate on per-call state only — `before_send` reads
and mutates one request's parts, `on_response` reads one response's parts —
**and, as of round 1's fix, both now take `&mut GooseUser`** (for reading or
stashing session data, and for scenario self-filtering; see
[Dispatch](#dispatch)). That is a stronger requirement on `fetch_all` than an
earlier draft of this section implied (when only `on_response` touched
`&mut GooseUser`): a `fetch_all` that runs many **send**s concurrently from
one user now needs to serialize on `&mut GooseUser` access at *two* points
per request, not one, since `GooseUser` isn't naturally `Sync` and a hook
instance can be called from any of several concurrent sends. This doesn't
block the design — attack-level hook closures are `Send + Sync` so they can
be shared across users without trouble, and `fetch_all` already has to
decide how concurrent sends from one user share `&mut GooseUser` for
reasons unrelated to hooks (metrics buffering, `unread_bodies`, session
data) — but it does mean hooks add to, not sidestep, whatever serialization
`fetch_all` needs regardless. How `fetch_all` structures concurrent sends
from one user is a question for that design, not this one — flagged as an
open dependency in [Follow-ups](#follow-ups), since this design was not able
to test against `fetch_all`'s actual shape, which doesn't exist yet.

Both hook points are built from `HookRequestParts`/`HookResponseParts` (see
[Dispatch](#dispatch)), whose **public** surface is `http`-typed accessors
only — a hook's own code never sees `reqwest::Request`/`reqwest::Response`.
`HookResponseParts` already matches that goal internally too (built from
extracted `http` types, no reqwest type retained). `HookRequestParts`, in
this branch's prototype, still holds a `reqwest::Request` reference
internally (see the correction in [Dispatch](#dispatch)), so swapping in a
hyper-backed client would need that one internal change, not a change to
any hook a load test author wrote — the gap is in this prototype's
plumbing, not in the API a hook sees.

### Public API and migration cost

Entirely additive. New methods on `GooseAttack`, taking plain closures (see
[Dispatch](#dispatch)), matching the existing `set_worker_data` idiom
(`pub fn set_worker_data<F>(mut self, hook: F) -> Self where F: Fn(&WorkerShare) -> Vec<u8> + Send + Sync + 'static`,
`src/lib.rs:1343-1349` — the new methods below follow the identical bound
shape):
`register_before_send_hook(impl Fn(&mut HookRequestParts, &mut GooseUser) -> BeforeSendOutcome + Send + Sync + 'static)`,
`register_on_response_hook(impl Fn(Result<&HookResponseParts, &TransportError>, &mut GooseUser) -> ResponseOutcome + Send + Sync + 'static, retry_policy: RetryPolicy)`.
Both exist on `GooseAttack` in this branch's prototype exactly as shown.
**An `_async` sibling for each, on `GooseAttack`, does not exist in this
branch** — only `GooseUser::with_before_send_hook_async` does (see
[Dispatch](#dispatch)), explicitly scoped to tests and this investigation's
benchmark harness, not attack-level registration; there is no async
`on_response` variant anywhere yet, sync or test-only. An attack-level async
registration method for both hook points is specified by the design (the
same owned-data, boxed-future shape `AsyncBeforeSendHook` already uses, see
[Dispatch](#dispatch)) but is a follow-up, not something this prototype
built; see [Follow-ups](#follow-ups). `RetryPolicy`
(default: idempotent methods only) carries the non-idempotent-retry opt-in
described in [Timing](#timing) as a value, not a second same-purpose method —
round 2's review flagged an earlier draft's
`register_on_response_hook_allowing_non_idempotent_retry` name (56
characters, longer than any existing public method, with no parallel
"disallowing" counterpart) as unscalable the moment a second registration
option is ever needed. For example:

```rust
// Wherever the GooseAttack is built, e.g. in main():
attack = attack.register_before_send_hook(|request, _user| {
    request.headers_mut().insert("x-trace-id", trace_header_value());
    BeforeSendOutcome::Continue
});
```

No change to `GooseRequest`, `GooseResponse`, `Transaction`, `Scenario`, the
`transaction!` macro, or any existing method signature.
`GooseUser` does gain new public surface in this branch's prototype beyond
the `GooseAttack` methods above: `with_before_send_hook`,
`with_on_response_hook`, and `with_before_send_hook_async`, all behind
`request-hooks`, documented in their own doc comments as existing for tests
and this investigation's benchmark harness (`examples/bench_hooks.rs`,
`benches/hook_dispatch.rs`) rather than as the load-test-author-facing
entry point — a real load test registers hooks on the `GooseAttack` it
builds, which every one of its users then shares, not on a `GooseUser`
directly. The two new `GooseRequestMetric` fields are
`#[serde(default)]`, so a request log or saved JSON report written before
this feature existed still deserializes — though, as noted above, a few
existing tests that pin exact output byte-for-byte (not deserialization) do
need a one-time update. A load test that registers no hook sees no change in
behavior, output, or timing — this is both a design goal and what the
benchmark in [The bar](#the-bar) verifies. goose-eggs, which is built
entirely on the existing public `Transaction`/`GooseUser` API, needs no
change to keep compiling or behaving the same; a future goose-eggs helper
(e.g. a packaged auth-signing hook) is additive, not migratory.

## Critic rounds

Three fresh reviewer subagents per round, each given this design document and
the relevant source, not told any prior conclusion. Lanes: (a) correctness
and metrics integrity, (b) performance/simplicity/reuse, (c) user-facing
surface and verification. Findings classed must-fix or should-fix; a round
also re-checks the previous round's fixes.

### Round 1

**Lane (a), correctness/metrics integrity:**

- *Must fix:* `hidden_time_us` was specified as a `Duration` the **hook**
  supplies (`Retry { hide_from_response_time: Duration }`), contradicting
  ruling 2's "Goose measures the hidden time" — nothing clamped a hook's
  value to the attempt's real elapsed time, so a buggy or hostile hook could
  report an arbitrary number. **Fixed**: the hook now only signals a `bool`
  (`hide_this_attempt`); Goose always measures the actual duration itself
  with its own `Instant::now()` bracket. See [Timing](#timing).
- *Must fix:* hook-driven retries had no idempotency guard, unlike the
  built-in resend-on-close they sit next to — a retry hook applied to a
  non-idempotent `POST` could silently double-submit against a live backend.
  **Fixed**: `Retry` is only honored for idempotent methods by default
  (mirroring `is_idempotent()`), with an explicit opt-in method for a test
  that knowingly wants otherwise. See [Timing](#timing).
- *Must fix:* the aggregation spec only touched `record_request_metric`,
  which is reached only by failed/CO-affected requests — the majority path
  (a request a hook retried once and then accepted) is pre-aggregated via
  `accumulate_request`/`accumulate_request_by_second` into a
  `RequestBatchEntry` and would never carry the new fields at all. **Fixed**:
  spec now covers `RequestBatchEntry` explicitly. See
  [Visibility model](#visibility-model).
- *Should fix:* an internal inconsistency suggested `coordinated_omission_data`
  (a response-time histogram type) should carry `attempts`/`hidden_time_us`
  fields that don't fit its shape. **Fixed**: clarified these are flat
  scalars on the aggregate itself, describing real requests, not duplicated
  into the CO timing-data object.
- *Should fix:* the retry cap's semantics (does it bound total attempts or
  additional retries?) were ambiguous. **Fixed**: stated explicitly as
  bounding total attempts including the first.
- *Should fix:* the pre-existing `resends` counter and the new `attempts`
  counter could both increment for one underlying flaky request, with no
  guidance on reading them together. **Fixed**: added a clarifying paragraph
  in [Timing](#timing) — they measure different things and are not merged.
- *Should fix:* adding a field to `GooseUser` also requires updating its
  hand-written `Clone` impl, not mentioned as a forcing function alongside
  the Gaggle ones. **Fixed**: noted in [the zero-hook fast path](#the-zero-hook-fast-path).

**Lane (b), performance/simplicity/reuse:**

- *Must fix:* the doc implied hook-visible request/response state would be
  built from real `http::Request`/`http::Response` values, which (verified
  against reqwest's own source) requires a `Url`↔`http::Uri`
  parse-and-allocate conversion reqwest does not do for free — plausibly
  consuming most of the entire 0.3 µs "one trivial hook" budget before the
  hook body even runs. **Fixed**: hook-visible types now borrow
  `http::HeaderMap`/`http::Method` directly (reqwest re-exports these with
  zero conversion) and expose the URL only through a lazily-computed,
  memoized accessor, so a header-only hook never pays for it. See
  [Dispatch](#dispatch).
- *Must fix:* the stated reason for deferring per-scenario hook registration
  ("an extra lookup... on every request even with no hooks") doesn't hold up
  — `GooseUser` already resident-carries and reads `scenarios_index` on every
  request for other purposes, so this would cost about what the already-free
  `is_empty()` check costs. **Fixed**: reasoning corrected to the real
  reason (implementation complexity, no demonstrated need), and noted that
  attack-level hooks can still self-filter by scenario via the `&mut GooseUser`
  they're given.
- *Should fix:* cutting `on_metric` left "recording traffic to a side
  channel" — one of the six use cases in the opening paragraph — entirely
  unserved, which the original framing didn't admit. **Fixed**: reworded the
  decision line and problem statement to say five of six, explicitly, and
  cross-reference the gap to `on_metric`'s follow-up entry.
- *Should fix:* `before_send` lacked `&GooseUser` access that `on_response`
  had, with no way to read session state or self-scope by scenario.
  **Fixed**: `before_send` now takes `&mut GooseUser` too, at the same cost
  (one more reference) `on_response` already pays.

**Lane (c), user-facing surface and verification:**

- *Must fix:* the same aggregation gap lane (a) found, confirmed
  independently: `attempts`/`hidden_time_us` would never appear in the
  console table, reports, or dashboard for the dominant (batched,
  successful) request path. **Fixed**, see lane (a) above.
- *Must fix:* the hook "trait" was never concretely specified (no method
  name, no parameters), `before_send`'s return type was undefined (so
  fault-injection's "short-circuit the send" capability, named in the hook
  points table, had nowhere to attach), and the design didn't address that
  Goose already has a closure-based idiom for this kind of attack-level
  extensibility (`set_worker_data`) that a trait-based API would clash with.
  **Fixed**: switched to closures throughout, with concrete type aliases and
  a `BeforeSendOutcome::FailWith` variant for fault injection. See
  [Dispatch](#dispatch).
- *Should fix:* the forcing-function inventory for Gaggle/report code didn't
  mention two more exhaustive destructuring sites that would also fail to
  compile unchanged (`ReportRequests::serialize`, `gaggle/compare.rs`'s
  `metrics_diff`). **Fixed**: added both.
- *Should fix:* existing tests that pin exact output (a Debug-string
  comparison, a byte-for-byte dashboard snapshot golden) would need updating,
  which "entirely additive" undersold. **Fixed**: named both tests and the
  existing `GOOSE_UPDATE_GOLDENS` mechanism that already handles this class
  of change.
- *Should fix:* no mention of which book chapters would need updating for a
  user who registers a hook. **Fixed**: named the three chapters.

### Round 2

Fresh subagents, same three lanes, given the round-1-revised document and a
direct instruction to independently re-verify round 1's fixes against the
code (not trust round 1's own description of them), plus look for anything
new.

**Lane (a), correctness/metrics integrity:** round 1's fixes all held up
(the hook-never-supplies-a-duration model, the idempotency guard, the
`RequestBatchEntry` aggregation fix, the CO-data-shape correction). Three
new issues, all must-fix:

- The CO backfill arithmetic fix wrote `response_time.saturating_sub(hidden_time_us)`
  — mixing units: `response_time` is whole **milliseconds**,
  `hidden_time_us` is **microseconds**. For almost any real hidden retry
  (anything over ~1-2 ms), this saturates to zero instead of subtracting
  only the hidden portion, corrupting CO's arithmetic in the opposite
  direction from the original bug. **Fixed**: matching-unit arithmetic
  specified throughout (see [Visibility model](#visibility-model)).
- CO's backfill loop clones the triggering `request_metric` once per
  synthetic "missed" request and records each clone — the spec didn't say
  these clones must have `attempts`/`hidden_time_us` zeroed before being
  counted (a synthetic entry never actually retried), nor did it say where
  the *real* triggering request's own attempts contribute in that branch.
  **Fixed**: added the explicit zeroing requirement and flagged the
  remaining control-flow question as something a real implementation must
  re-trace against `main` at the time, rather than assume from this
  document (see [Visibility model](#visibility-model)).
- The retry loop's "re-runs `before_send` and the send again" glossed over
  that `RequestBuilder::build()` already consumed the only builder by the
  time a retry is possible — there is nothing left to "rebuild" from.
  **Fixed**: rewritten to say explicitly that a retry resends a **clone of
  the already-built `Request`** (the same mechanism the existing
  resend-on-close path already uses), with the same `try_clone()`-returns-
  `None` fallback (not honored, logged) that a non-cloneable streamed body
  hits even for an idempotent method. See [Timing](#timing).

One should-fix: `before_send`'s `FailWith` on a *retried* attempt (not the
first) wasn't addressed. **Fixed**: specified as treated exactly like a
transport error for that attempt, gated by the same idempotency/cap checks
as any other attempt (see [Timing](#timing)).

**Lane (b), performance/simplicity:** round 1's lazy-URL-accessor and
corrected per-scenario-deferral reasoning both held up against a fresh check
of reqwest's and hyper's actual re-exports. One must-fix and three should-fix,
all about gaps round 1 didn't reach rather than round 1 being wrong:

- **Must fix**: the "Fit with `fetch_all`" section still said only
  `on_response` takes `&mut GooseUser`, after round 1 had already given
  `before_send` the same access — an internal contradiction with a real
  consequence (a future `fetch_all` serializes on *two* points per request
  sharing one user, not one). **Fixed**: paragraph rewritten to say so
  explicitly.
- Dispatching a hook while iterating `self.hooks` needs `Arc::clone(&self.hooks)`
  before the loop (the borrow checker won't allow holding a loan on
  `self.hooks` while also passing `self` as `&mut` into the hook) — an
  atomic refcount op the cost accounting hadn't named. **Fixed**: named and
  costed (a few ns, paid only when a hook is actually called) in
  [the zero-hook fast path](#the-zero-hook-fast-path).
- The "memoized" URL accessor is asymmetric between the two structs:
  `HookRequestParts` can use a plain field (it always gets `&mut`), but
  `HookResponseParts` only ever gets a shared reference and needs real
  interior mutability (`OnceCell`). **Fixed**: both structs sketched out
  with their actual mechanism in [Dispatch](#dispatch).
- The retry mechanics gap lane (a) found independently.

**Lane (c), user-facing surface:** round 1's closure-based API (verified
against `set_worker_data`'s real signature), the `RequestBatchEntry`
visibility fix, and the test/doc fallout lists all held up on independent
re-verification. One must-fix, two should-fix:

- **Must fix**: the document's own framing overclaimed its state — the
  opening paragraph said "the benchmark that backs the decision... exists
  so these numbers are reproducible," while "The bar" and "Prototype and
  measurement" were still stubs and "Round 2" was an unfilled placeholder,
  at the point this review ran. A reader skimming only the top could believe
  this was a finished, benchmarked call. **Fixed**: this is addressed by
  actually completing the benchmark (below) before finalizing the document;
  the numbers now present are the ones this round asked to see.
- The two new structs (`HookRequestParts`/`HookResponseParts`) were
  described narratively but never given a method sketch, even though the
  worked example already called `headers_mut()`. **Fixed**: added alongside
  the type aliases in [Dispatch](#dispatch).
- The proposed `register_on_response_hook_allowing_non_idempotent_retry`
  method name (56 characters, no parallel "disallowing" counterpart, and it
  doesn't scale to a second registration-time option) was flagged as
  unscalable. **Fixed**: replaced with a `RetryPolicy` value passed to
  `register_on_response_hook` itself (see
  [Public API](#public-api-and-migration-cost)).

Round 2 produced must-fix findings in every lane (3 in lane (a), 1 in lane
(b), 1 in lane (c)), so the stopping rule (first round with zero must-fix
findings) is not met yet. All fixes above were applied; round 3 checks them.

### Round 3

By this round a working prototype existed (`src/goose/hooks.rs`, the hook
call sites in `src/goose.rs`, `examples/bench_hooks.rs`,
`benches/hook_dispatch.rs`), so this round was instructed to check the
document's claims against the **real code**, not just its own internal
consistency — which is exactly where it earned its keep.

**Lane (a), correctness:** round 2's fixes held up. Three new must-fix
findings, all in the same place — the CO-interaction specification had a
second layer of bugs round 2 didn't reach:

- The CO backfill arithmetic fix (round 2) was internally consistent, but a
  **live, reachable version of the same hazard already exists in `main`,
  today, independent of this feature**: `process_request_metric`'s CO
  backfill loop (`src/metrics.rs`) does `let mut co_metric = request_metric.clone();`
  for every synthetic "missed" entry it generates, and `GooseRequestMetric`
  derives `Clone` — so once `attempts`/`hidden_time_us` exist as real
  fields (as of this round, they do; see the Visibility model correction
  below), every synthetic clone silently carries the real triggering
  request's attempt count and hidden time, with nothing in today's code
  zeroing them. It's inert only because no accumulator reads those fields
  off the aggregate yet. **Fixed**: the requirement to zero both fields on
  every CO-synthesized clone is restated as describing an actual, already-
  reachable code path, not a hypothetical one a future implementer should
  watch for.
- `HookRequestParts`, in the actual prototype, holds a `reqwest::Request`
  reference internally rather than the three independent `http`-typed
  fields the Dispatch section's sketch shows — contradicting the "never a
  reqwest type" claim for the request side specifically (the response side
  has no such gap). **Fixed**: both the sketch and the `fetch_all`/
  non-reqwest-client section now say so explicitly, scoped as a follow-up,
  not glossed over. See [Dispatch](#dispatch).
- `before_send`'s `FailWith` is specified as fault injection that skips the
  network call, but the real `call_before_send_hooks` only logs and ignores
  it — the request still goes out. Nothing in the document disclosed this
  for a use case named in the opening paragraph. **Fixed**: the hook-points
  table and the [Dispatch](#dispatch) sketch both now say so, and
  [Follow-ups](#follow-ups) lists it.

**Lane (b), performance:** round 2's fixes (the `fetch_all` consistency,
the `OnceCell`/plain-field asymmetry, the async `HeaderMap`-clone design)
held up. One must-fix, several should-fix, all about costs the document
hadn't actually traced into the code yet:

- **Must fix**: the retry-template clone (`hook_retry_template`) was built
  unconditionally whenever any `on_response` hook was registered, regardless
  of whether that hook's `RetryPolicy` could ever permit retrying the
  request's actual method — so a non-idempotent `POST` under the default
  (idempotent-only) policy paid for a real `HeaderMap`+`Url`+body clone it
  could never use. **Fixed in the code**: `send_request` now only clones
  when at least one registered hook's policy actually permits this
  request's method (`src/goose.rs`), and the cost is described accurately
  in [the zero-hook fast path](#the-zero-hook-fast-path).
- The `Arc::clone(&self.hooks)` cost model described "once per request";
  the real code pays it once per retry-loop iteration, so a twice-retried
  request pays three. **Fixed**: corrected in
  [the zero-hook fast path](#the-zero-hook-fast-path).
- `built_method`'s clone ran unconditionally even on the true zero-hook
  path. **Fixed in the code**: moved inside the same `on_response`-non-empty
  guard as the retry-template clone, so it costs nothing when no
  `on_response` hook is registered at all.
- The `try_clone()`-returns-`None` retry fallback didn't log, unlike the
  idempotency-check fallback the Timing section says is "logged the same
  way." **Fixed in the code**: added the missing `debug!` call
  (`src/goose.rs`).
- This round also found the headline must-fix that applies across all three
  lanes: "The bar" and "Prototype and measurement" were still unfilled
  stubs, and round 2's own log claimed otherwise. **Fixed**: the benchmark
  is actually run and reported below, this time for real — see
  [The bar](#the-bar).

**Lane (c), surface:** round 2's `RetryPolicy`-as-a-value fix and the struct
sketches held up exactly as described, verified against the real
`src/lib.rs`/`src/goose/hooks.rs` signatures. One must-fix shared with the
other two lanes (the Visibility model's false "doesn't exist" claim,
already described above and fixed), one must-fix shared with lane (b) (the
benchmark stub), and should-fixes already covered: the missing mention of
`GooseUser`'s own `with_*` hook methods, and the retry cap being a hardcoded
constant rather than `GooseConfiguration`-driven. Both fixed in their
respective sections above.

No lane produced a must-fix in round 3 that the document's own earlier
claims hadn't already partly set up to find — unlike round 2, which found
bugs in the design's reasoning, round 3 mostly found **gaps between the
design's prose and the prototype's actual code**, which is exactly what
having a prototype to check against is for. Round 4 re-verifies all of the
above against the now-benchmarked, now-corrected document.

### Round 4

Fresh subagents, same three lanes, given the round-3-revised document (with
the real benchmark numbers now filled in, see [The bar](#the-bar)) and
instructed to re-verify round 3's fixes against the code independently.

**Lane (a), correctness:** round 3's fixes (the live CO-clone hazard
correctly scoped, the retry-template gating, the missing debug log) all
held up against a fresh read of the code, including the specific claim that
the CO hazard is inert only because no accumulator currently reads
`attempts`/`hidden_time_us` off the aggregate — verified by grepping all of
`src/metrics.rs` for both names. No must-fix. Three should-fix, all
precision issues rather than defects: several `file:line` citations had
drifted by 10 to ~300 lines as the prototype grew (e.g. `is_idempotent()`'s
citation pointed at line 2442, actually 2559; `set_success`/`set_failure`
pointed nearly 300 lines off). **Fixed**: every citation this round listed
was corrected against the current tree. A parenthetical said the 20-repeat
benchmark run was "200 requests more" than the brief's 10-repeat floor,
which doesn't reconcile (10 extra repeats × 20,000 requests = 200,000, not
200). **Fixed**: reworded. The results table placed CPU-per-request and
wall-clock throughput in the same row with no note that they're different
clocks and not reciprocals. **Fixed**: added a note directly under the
table.

**Lane (b), performance:** round 3's gating fix (`hook_retry_template` only
clones when a registered policy could permit the method, `built_method`
only computed inside the `on_response`-non-empty guard) held up exactly as
described. Two must-fix findings, both about **this document overclaiming
its own precision** rather than a code defect:

- "Arm (a) and arm (b) run through identical compiled code except for two
  `Vec::is_empty` reads" undercounted the real zero-hook-path code: the
  `before_send` guard is actually an `||` of two `is_empty()` checks (one
  for `before_send`, one for `before_send_async`), `on_response` is checked
  at two separate call sites, and several unconditional local declarations
  and two struct-field writes also run. **Fixed**: reworded to describe
  what actually runs (several nanosecond-scale guard checks and
  assignments, not exactly two reads), without changing the conclusion —
  the corrected accounting is still far below the noise floor.
- "The dispatch microbenchmark the brief specifically asked for to cover
  exactly this gap" attributed reasoning to the brief that the brief itself
  doesn't state — it asks for a dispatch microbenchmark as its own
  deliverable, not explicitly because the end-to-end harness would be too
  noisy for the per-hook question. **Fixed**: reworded to own that
  reasoning as this document's, not the brief's.

Three should-fix: the 196 ns dispatch number doesn't include the real call
site's `Arc::clone(&self.hooks)` cost. **Fixed**: noted, with the remaining
margin to the bar stated explicitly. The arm (b) vs (a) statistical
treatment used raw min/max spread instead of the paired, per-cycle analysis
the interleaved design actually supports. **Fixed**: recomputed as a proper
paired analysis (see [The bar](#the-bar)) — a substantively better fix than
a wording change, since the paired standard error (1.83% of baseline) is
itself a cleaner, more defensible way to say "this run can't resolve a 0.5%
effect" than eyeballing a spread. The phrasing "consistent with no added
cost" risked implying more confidence than the data supports. **Fixed**: the
rewritten bar section states the standard error explicitly rather than
leaning on the word "consistent" alone.

**Lane (c), surface:** round 3's Visibility-model correction, the
`GooseUser` `with_*` methods mention, and the hardcoded-retry-cap wording
all held up. Three must-fix: the Follow-ups list was missing three items
the document's body had already flagged as "a follow-up" inline (`FailWith`
unimplemented, no attack-level async registration, the hardcoded retry
cap). **Fixed**: added. This "Round 4" heading itself had an introductory
sentence and no actual findings when this lane's reviewer read it — a
documentation process gap, not a design gap, but a real one. **Fixed**: by
writing this section. No `CHANGELOG.md` entry existed for this work.
**Fixed**: added, under Unreleased.

No lane in round 4 found a correctness or performance defect in the
prototype itself — every finding was the document either overclaiming its
own precision or lagging behind the code/process it describes. Round 5
checks whether these fixes (plus the benchmark section's substantial
rewrite) introduced anything new.

### Round 5

**Lane (a), correctness:** every round 4 citation fix held up against the
real code (spot-checked against `src/goose.rs`, `src/metrics.rs`,
`src/lib.rs`, including re-running `metric_sizes_stay_small` with and
without the feature). No must-fix. Three should-fix: three more citations
— `src/goose.rs:2467` (claimed as the `resends` counter increment; the
increment is actually at `:2610-2611`), `src/goose.rs:2414-2422` (claimed as
where `TransactionDetail` reads `scenarios_index`/`transaction_index`;
that's actually at `:2513-2517`), and `src/metrics.rs:4212` (claimed as
`record_request_metric`'s `update`-skip special case; that's actually at
`:4230-4244`) — had drifted the same way round 4's already found and fixed,
but weren't on round 4's list. **Fixed**: all three corrected.

**Lane (b), performance:** one must-fix — "the zero-hook fast path" section
still said "exactly one check" per hook point, the exact framing round 4
rewrote (more accurately) only in "The bar," leaving the document
self-contradictory between the two sections. **Fixed**: "the zero-hook fast
path" no longer restates a specific count at all; it names the mechanism
and points to "The bar" as the one place the count is kept current, so the
two sections can't drift against each other again. Two should-fix: "The
bar"'s own corrected count said "two guard checks on `before_send`" when
there is exactly one (containing two `is_empty` reads via `||`), and didn't
note that `run_on_response_hooks`'s internal check never executes at all on
the true zero-hook arm, since the function is never entered. **Fixed**:
reworded with the precise four-`is_empty()`-reads accounting, explicit
about which checks actually execute on arm (b). The "over 100 ns of margin"
phrasing for arm (c) stated a specific number against a deliberately
unpinned `Arc::clone` cost estimate ("a few ns"). **Fixed**: restated as
104 ns of raw margin before that cost, with the unpinned estimate shown
explicitly rather than folded into a single overconfident number.

**Lane (c), surface:** no must-fix. CHANGELOG.md, the Follow-ups list, and
the SUMMARY.md/`design/overview.md` linkage all held up exactly as round 4
left them; the decision line was judged still defensible given everything
found (a judgment call, not a finding). One should-fix: the opening
paragraph listed fault injection among the "covered" use cases without
flagging, this early, that it isn't actually honored by the prototype — a
reader stopping at paragraph one could believe otherwise. **Fixed**: the
opening paragraph now says so directly, with a link to
[Follow-ups](#follow-ups).

Round 5 found one must-fix (a self-contradiction round 4's own fix
introduced between two sections) and five should-fix items, all precision
gaps rather than defects in the design or the prototype. Round 6 is the
cap; it checks these fixes and is the last round regardless of outcome.

### Round 6 (final — cap reached)

Round 5's fixes (the three corrected citations, the zero-hook-path section
no longer restating a drifting count, "The bar"'s four-`is_empty()`-reads
accounting and 104 ns margin restatement, the opening paragraph's
fault-injection disclosure) all held up against the real code on
independent re-verification. But this final round's skeptical pass — the
kind every prior round was also asked to do, and did less of as the
document grew — found real issues rounds 1-5 missed:

- **Must fix**: `GooseAttack::register_before_send_hook`'s own doc comment
  (`src/lib.rs`) claimed a hook could "fail it outright without sending it
  (fault injection)" — the one place in the actual source, not just this
  document, where the `FailWith`-isn't-honored gap wasn't disclosed, right
  where a load test author would read it. **Fixed**: the doc comment now
  states plainly that this prototype doesn't honor it yet.
- **Must fix**: the `on_response` hook-points table row claimed "response-side
  fault injection (a hook can treat a 200 as a failure, or vice versa)" — a
  capability that doesn't exist anywhere in the `ResponseOutcome` enum
  (`Accept`/`Retry` only) or the code; `error_on_fail`/`expect_status_code`
  never read a hook's opinion of a response. **Fixed**: the claim is
  removed and corrected in the hook-points table.
- **Must fix**: a `Retry` that was asked for but *denied* (non-idempotent
  method without opt-in, or the attempt cap reached) still short-circuited
  every later-registered `on_response` hook, even though that response was
  about to become final with no replacement — silently skipping hooks the
  design's own "short-circuits the rest for that attempt" framing says
  should only happen when there *is* a replacement. **Fixed in the
  prototype**: `run_on_response_hooks` now only stops the loop when a
  `Retry` is actually honored; a denied one logs and continues to the next
  hook. See [Timing](#timing).
- Should-fix: hook-driven retries don't get the resend-on-close protection
  the first attempt gets, and `hidden_time_us`'s bracket excludes a
  retry's own `before_send` hook time even when that attempt is hidden.
  **Documented as known limitations**, not fixed in code, in
  [Timing](#timing) — both are precision/robustness gaps in the prototype,
  not metrics-integrity violations (ruling 1 and 2 both still hold: nothing
  is silently excluded from `response_time`, and nothing is hidden without
  being measured).
- Should-fix: a dangling reference to a `Hook::Async` type that was never
  defined anywhere in this document. **Fixed**: corrected to name the real
  `AsyncBeforeSendHook` shape.
- Should-fix: the identical `has_before_send`-style condition was
  duplicated verbatim at two call sites in `send_request`/
  `run_on_response_hooks`. **Fixed in the prototype**: factored into one
  `HookSet::has_before_send()` method both sites call.

**This document hit its cap at round 6 with three must-fix findings in
this final round**, not the "first round with zero must-fix" the brief's
stopping rule asks for. All three were fixed, and the fixes were verified
against the actual compiled, tested code (`cargo build`/`cargo test` with
`--features request-hooks` pass after every fix in this round) — but,
honestly: round 6's fixes are **not** re-verified by a round 7, because
there is no round 7. Every prior round found real things the one before it
missed; there is no specific reason to believe round 6 is the first round
that found everything. Treat this document's correctness claims with that
in mind, on top of the shared-host caveat on its performance claims: both
are reasons the [pinned rerun and a fresh maintainer review](#follow-ups)
belong before release, not reasons to distrust the design's direction.

## Prototype and measurement

**The host this ran on is a shared cloud machine (dev job GO360 on `kc1`),
not dedicated hardware.** Every number below carries that caveat; see
[Follow-ups](#follow-ups) for the pinned rerun this investigation could not
do itself.

### Harness

Two separate measurements, for two different questions:

- **End-to-end CPU per request** (`examples/bench_hooks.rs`): a tiny
  hand-rolled HTTP/1.1 server (plain TCP, thread per connection,
  keep-alive, a fixed 2-byte body) runs in a **separate child process**,
  not a thread of the benchmark process, so the client's
  `/proc/self/stat` (user+system CPU time) never includes the server's own
  CPU. This substitutes for "a local hyper server" as the brief specifies —
  a minimal server was written directly rather than pulling `hyper`/`axum`
  into the benchmark's required features, to keep the harness buildable in
  both the feature-on and feature-off configurations without extra feature
  entanglement; this substitution is disclosed here rather than left
  implicit. A `GooseUser::single()`-built user (no full `GooseAttack`, no
  throttle, no multi-threading — isolating `GooseUser::request()`'s own
  cost) runs 50 warm-up requests (outside the measured window, to let
  connection-pool setup happen first), then the process's CPU time and
  wall clock are sampled, 20,000 sequential `GET /` requests run, and both
  are sampled again; CPU per request is the delta divided by 20,000.
- **Dispatch alone, no network** (`benches/hook_dispatch.rs`, a `criterion`
  dev-dependency — no new dependency in any normal build): calls hook
  closures directly against an in-memory `reqwest::Request` built once
  outside the timed loop, isolating the `Vec::is_empty` check, the virtual
  call, and the enum match from any network or connection-pool cost at all.

### Arms

(a) this branch with `request-hooks` off — structurally identical to
`main`'s `request()` for this comparison, since every line this feature
adds is behind `#[cfg(feature = "request-hooks")]`; (b) `request-hooks` on,
no hook registered; (c) one sync `before_send` hook adding a header; (d) one
sync `on_response` hook that always accepts; (e) five hooks (two
`before_send`, three `on_response`, all trivial); (f) one async
`before_send` hook (owned-`HeaderMap` signature, boxed future); (g) one
`on_response` hook that retries once on a 503, against a server that fails
every tenth request.

### End-to-end results

Arms (a)-(g), 20,000 requests per sample, interleaved in the order
a,b,c,d,e,f,g and repeated 20 times (10 more repeats, 200,000 more requests
total per arm, than the brief's "at least 10" floor), median/min/max of the
20 samples' CPU-per-request:

| Arm | Median (µs/req) | Min | Max | Median req/s |
|---|---|---|---|---|
| (a) no feature (≅ `main`) | 145.00 | 126.5 | 171.5 | 4785.6 |
| (b) feature on, no hooks | 145.25 | 125.0 | 160.5 | 4906.2 |
| (c) one sync `before_send` hook | 141.50 | 127.0 | 158.0 | 4797.2 |
| (d) one sync `on_response` hook | 141.75 | 127.5 | 158.0 | 4871.1 |
| (e) five hooks | 145.50 | 130.0 | 167.0 | 4919.6 |
| (f) one async `before_send` hook | 150.25 | 136.5 | 174.5 | 4753.6 |
| (g) one retry-on-503 hook | 164.75 | 141.0 | 195.0 | 4325.2 |

`µs/req` and `req/s` are two different clocks and are not reciprocals of
each other: `µs/req` is this process's own CPU time (`/proc/self/stat`,
user+system), divided by request count; `req/s` is wall-clock throughput
(`Instant::now()` elapsed, divided into the request count). They diverge
because a request spends wall-clock time waiting on the loopback round
trip and the OS scheduler that never shows up as this process's CPU time —
1,000,000 / 145.00 ≈ 6896, not arm (a)'s measured 4785.6 req/s, and that gap
is expected, not an inconsistency.

A sanity check on arm (g)'s mechanism, run separately at low volume: against
the same every-tenth-request-fails server, the 10th, 20th, 30th... request
each showed `status_code=200` (the retry succeeded), `attempts=2`,
`hidden_time_us` a small nonzero value (the measured duration of the
retried attempt) — every other request showed `attempts=1`,
`hidden_time_us=0`. The accounting works end to end, not just in the
design.

### Dispatch-alone results (criterion, release build)

| Benchmark | Median time |
|---|---|
| zero-hook `Vec::is_empty` check | 1.44 ns |
| one sync hook (parses and inserts one header) | 196.0 ns |
| three sync hooks | 576.8 ns |
| five sync hooks | 984.7 ns |
| one async hook (`HeaderMap` clone + boxed future + poll) | 677.3 ns |

Scaling is linear in hook count (≈192-197 ns/hook across 1, 3 and 5), and
the async hook's extra cost over a sync one (≈480 ns) is consistent with
one `HeaderMap` clone plus one heap allocation for the boxed future, exactly
the two costs [Dispatch](#dispatch) says async pays and sync doesn't.

## The bar

The brief's bar: arm (b) must be within noise of arm (a), median difference
under 0.5% of CPU per request; arm (c) must add under 0.3 µs per request.

**Arm (b) vs (a):** comparing the two arms' medians across all 20 samples
independently gives 145.25 µs vs. 145.00 µs, a difference of 0.25 µs
(0.17%). But the harness interleaves a,b,c,...,g within each of the 20
repeats specifically to control for host drift, which sets up a better
analysis than comparing independent medians: the **paired** per-cycle
difference (b − a, computed within each of the 20 interleaved cycles, so
each pair shares whatever drift happened during that cycle). That paired
series has mean **−0.50 µs**, standard deviation 11.84 µs, standard error
of the mean **2.65 µs** (**1.83% of arm (a)'s 145.00 µs baseline**). Two
things follow from that number directly, not from eyeballing a spread:

- The paired mean (−0.50 µs, i.e. arm (b) measured very slightly *faster*)
  is well within one standard error of zero — statistically indistinguishable
  from no difference at all, which is the strongest statement this data
  supports.
- The standard error itself, 1.83%, is **more than three times the 0.5%
  bar**. A measurement whose own uncertainty is larger than the effect size
  it's supposed to bound cannot confirm *or* rule out a 0.5% effect either
  way — this run is genuinely inconclusive at the precision the bar asks
  for, not just noisy-looking.

The architectural argument is independent of this measurement and doesn't
share its imprecision: arm (a) and arm (b) run through the same compiled
`send_request`, differing only in a handful of `#[cfg(feature = "request-hooks")]`
additions gated behind `Vec::is_empty()` checks on the zero-hook path. On
arm (b) specifically (feature on, no hook registered), what actually
executes beyond arm (a)'s code is: one `if` guarding `before_send` (itself
an `||` of two `is_empty()` reads, for the sync and async hook lists), two
more separate `is_empty()` checks on `on_response` (one guarding the
retry-template clone, one guarding the call into `run_on_response_hooks` —
`run_on_response_hooks`'s *own* internal check never executes on this arm,
since the function is never entered), plus a handful of local-variable
declarations, a tuple construction, and two struct-field writes at the end
— four `is_empty()` reads in total, none of which allocate or branch into
any hook-calling code. Each of these individually measures at low
single-digit nanoseconds or less (the `is_empty()` check alone: 1.44 ns, see
the dispatch table) — even summed generously, several times smaller than the
noise floor above, let alone the bar. This is a *plausibility* argument
from counting what the zero-hook path actually executes, not a
measurement of it in isolation; it doesn't resolve a sub-0.5% question any
more precisely than the paired analysis does, it just comes at the question
from a different, corroborating direction.

**Arm (c), one trivial hook:** the end-to-end data (c's median is actually
*below* b's — noise, not a real speedup) can't resolve a sub-microsecond
effect for the same reason arm (b) can't. The dispatch microbenchmark
answers this question directly instead, with no network noise at all: **196
ns for one hook, comfortably under the 300 ns (0.3 µs) bar**, with a clear
linear scaling trend (3 hooks ≈ 3×, 5 hooks ≈ 5×) corroborating that it is
measuring real per-hook cost, not noise. This number is a genuine
substitution for a question the end-to-end harness's noise floor cannot
answer — the brief does separately ask for exactly this microbenchmark
("microbenchmark the dispatch alone with the repo's existing benchmark
tooling, or criterion"), but it asks for it as its own thing, not
explicitly *because* the end-to-end harness would be too noisy to answer
the per-hook question; that reasoning is this document's, stated here
plainly rather than attributed to the brief. One cost the isolated
microbenchmark doesn't include: the real call site's
`Arc::clone(&self.hooks)` before the loop (a few ns, see
[the zero-hook fast path](#the-zero-hook-fast-path)) — folding that in
still leaves comfortable margin under 300 ns.

**Arm (g):** the one effect large enough to be unambiguous against this
host's noise floor: +19.75 µs median (+13.6%) over arm (a), far outside the
±2.65 µs standard error the paired analysis above established as this
harness's resolution. This is the *expected*, *explicable* cost of doing
genuinely more work — 10% of requests pay for a second full network round
trip — not a hidden tax on the other 90%; it demonstrates the
attempts/hidden-time accounting functions correctly under load, not just in
the single-request sanity check above.

**Conclusion on the bar**: arm (c) clears its bar decisively, via the
dispatch microbenchmark: 104 ns of raw margin (300 − 196) before even
counting the real call site's `Arc::clone(&self.hooks)`, which this
document only ever prices as "on the order of a few ns," not a pinned
number — an uncontended `Relaxed` atomic increment is ordinarily 1-3 ns on
this kind of hardware, which would leave essentially all of that margin
intact, but this document doesn't measure that specific op in isolation
and so doesn't claim a tighter number than "comfortable" here. Arm (b) is
**indistinguishable from no added cost** in this run, but the run's own
standard error is too wide, by more than 3×, to confirm it clears the 0.5%
bar with confidence one way or the other — an honest "not yet resolved,"
not a pass. The architectural argument (four `is_empty()` reads and a
handful of trivial assignments, nothing more, on the zero-hook path) is the
strongest evidence available that it should pass, but it is reasoning about
the code, not a replacement for a measurement
precise enough to confirm it. This supports **build smaller in 0.20** on
the strength of the design review, the decisive dispatch-cost result, and
the architectural argument — with the pinned, dedicated-hardware rerun in
[Follow-ups](#follow-ups) as a hard prerequisite before anyone quotes a
zero-hook-cost number, specifically, in a release: this run did not fail
that bar, but it did not have the precision to pass it either.

## Follow-ups

- **A pinned rerun on dedicated (non-shared-cloud) hardware before any
  release claim.** Every number in this document comes from an interleaved
  run on a shared machine; the relative comparisons (arm vs. arm, same run)
  are much more trustworthy than any arm's absolute CPU-per-request number.
- **A fresh maintainer or reviewer pass before this is treated as settled.**
  Round 6, the capped final review round, still found three must-fix
  issues — including two factually wrong capability claims (a misleading
  public doc comment, a hook-points-table row describing a capability the
  code doesn't have) that five prior rounds missed. The review process
  found real things at every single round, including the last one; there's
  no specific reason the next read wouldn't too. Treat this document as
  thoroughly checked, not as exhaustively checked.
- **`on_metric` (traffic-recording hook)**, cut from 0.20 in round 1 of
  review for lack of a concrete use case beyond what request/debug/error
  logs already do. Revisit if a real load test asks for arbitrary-sink
  traffic recording that the fixed-format logs can't serve.
- **Per-scenario or per-transaction hook registration**, beyond the
  attack-level registration this design ships. No surveyed use case needed
  it; revisit if one appears.
- **Coordinated omission mitigation's internal arithmetic** needs the
  `response_time - hidden_time_us` substitution specified in
  [Visibility model](#visibility-model) actually implemented, when and if
  this feature is implemented for real (out of scope for this branch).
- **Report/dashboard/Gaggle plumbing** for the two new fields is specified
  but not implemented, per the brief's scope. The specification in
  [Visibility model](#visibility-model) is detailed enough to implement
  directly from file:line references, but should be re-checked against
  `main` at implementation time in case those files moved.
- **`fetch_all`'s actual shape** doesn't exist on `main` yet; this design's
  claim that both hook points are "per-call, parallel-safe" should be
  re-verified once `fetch_all` is a real, reviewable diff, not a plan.
- **The raw hyper client spike** doesn't exist on `main` yet either; this
  design's `http`-crate-only hook state should be re-verified against that
  spike's actual request/response types once it exists. Relatedly,
  `HookRequestParts` itself should be changed to hold independent
  `http`-typed fields instead of a `reqwest::Request` reference internally
  (see [Dispatch](#dispatch)), so it's actually decoupled from reqwest, not
  just exposing a decoupled-looking public surface over a coupled
  implementation.
- **`before_send`'s `FailWith` (fault injection)** is fully specified
  (short-circuit the network call, synthesize a transport error, let
  `on_response` and the retry loop treat it like a real one) but not
  honored by this prototype — `call_before_send_hooks` logs it and sends
  the request anyway. Implementing it is the main piece of the design left
  unbuilt that a real load test author would notice missing.
- **Attack-level async hook registration** (`register_before_send_hook_async`/
  `register_on_response_hook_async` on `GooseAttack`) is specified in
  [Dispatch](#dispatch) but not built; only `GooseUser::with_before_send_hook_async`
  exists, scoped to tests and this investigation's benchmark harness. There
  is no async `on_response` variant anywhere yet, specified or built.
- **The retry attempt cap** is a hardcoded constant
  (`hooks::MAX_ATTEMPTS = 3`) in this prototype, not read from
  `GooseConfiguration` as earlier drafts of this document implied. Making
  it configurable is a small, low-risk follow-up.
