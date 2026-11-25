//! An experimental request-hook layer, off by default behind the
//! `request-hooks` Cargo feature. Exists to prototype and benchmark the
//! design at `src/docs/goose-book/src/design/request-hooks.md`; not a
//! supported public API yet. Every type here may change or disappear
//! without a deprecation cycle while the feature is unreleased.

use std::cell::OnceCell;
use std::sync::Arc;

use http::{HeaderMap, Method, StatusCode};
use url::Url;

use crate::goose::GooseUser;

/// The error a transport (today, reqwest; tomorrow, maybe a raw hyper
/// client) returned instead of a response. A thin wrapper, not
/// `reqwest::Error` itself, so a hook never has to name a specific client's
/// error type.
#[derive(Debug, Clone)]
pub struct TransportError(pub(crate) String);

impl TransportError {
    pub(crate) fn new(e: &reqwest::Error) -> Self {
        TransportError(e.to_string())
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a `before_send` hook sees and can mutate: one `&mut` borrow of the
/// request actually about to be sent, re-borrowed per accessor call so a
/// hook can call `headers_mut()` and `method()`/`url()` in any order
/// without the borrow checker treating them as held simultaneously.
///
/// `url()` is the one accessor that costs anything beyond a header lookup:
/// converting reqwest's `url::Url` into an `http::Uri` is a parse, done at
/// most once per request no matter how many hooks call it, and never done
/// at all for a hook that only touches headers.
pub struct HookRequestParts<'a> {
    request: &'a mut reqwest::Request,
    uri_cache: Option<http::Uri>,
}

impl<'a> HookRequestParts<'a> {
    pub fn new(request: &'a mut reqwest::Request) -> Self {
        HookRequestParts {
            request,
            uri_cache: None,
        }
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.request.headers_mut()
    }

    pub fn method(&self) -> &Method {
        self.request.method()
    }

    /// The request's URL as an `http::Uri`, parsed and cached on first call.
    pub fn url(&mut self) -> &http::Uri {
        if self.uri_cache.is_none() {
            self.uri_cache = Some(
                self.request
                    .url()
                    .as_str()
                    .parse()
                    .expect("a built request's URL is already a valid URI"),
            );
        }
        self.uri_cache.as_ref().unwrap()
    }
}

/// What an `on_response` hook sees: borrowed response parts, never the
/// body. `url()` is lazily converted the same way `HookRequestParts::url`
/// is, but through a `OnceCell` rather than a plain field, since this side
/// only ever hands out a shared reference.
pub struct HookResponseParts<'a> {
    status: StatusCode,
    headers: &'a HeaderMap,
    url: &'a Url,
    uri_cache: OnceCell<http::Uri>,
}

impl<'a> HookResponseParts<'a> {
    pub(crate) fn new(status: StatusCode, headers: &'a HeaderMap, url: &'a Url) -> Self {
        HookResponseParts {
            status,
            headers,
            url,
            uri_cache: OnceCell::new(),
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        self.headers
    }

    pub fn url(&self) -> &http::Uri {
        self.uri_cache.get_or_init(|| {
            self.url
                .as_str()
                .parse()
                .expect("a received response's URL is already a valid URI")
        })
    }
}

/// What a `before_send` hook returns.
#[derive(Debug, Clone)]
pub enum BeforeSendOutcome {
    /// Send the request as built (after any mutation the hook made).
    Continue,
    /// Skip the network call entirely and treat this attempt as if the
    /// given transport error had come back instead. The fault-injection
    /// use case.
    FailWith(String),
}

/// What an `on_response` hook returns.
#[derive(Debug, Clone, Copy)]
pub enum ResponseOutcome {
    /// Use this response (or error) as the request's final result.
    Accept,
    /// Ask Goose to retry. Honored only when the retry's registration
    /// allows it for this request's method (see [`RetryPolicy`]) and the
    /// attempt cap hasn't been reached; otherwise treated as `Accept`.
    /// `hide_this_attempt` only marks the retried attempt as one whose time
    /// Goose should also add to `hidden_time_us` — it carries no duration;
    /// Goose always measures the attempt itself.
    Retry { hide_this_attempt: bool },
}

/// Whether an `on_response` hook's `Retry` is honored for a non-idempotent
/// method (default: no — replaying a `POST` against a live backend can
/// duplicate a side effect the first attempt already caused, and Goose has
/// no way to tell whether it did).
#[derive(Debug, Clone, Copy, Default)]
pub struct RetryPolicy {
    allow_non_idempotent: bool,
}

impl RetryPolicy {
    pub fn allow_non_idempotent(mut self) -> Self {
        self.allow_non_idempotent = true;
        self
    }

    pub(crate) fn permits(&self, method: &Method) -> bool {
        self.allow_non_idempotent || super::is_idempotent(method)
    }
}

/// A `before_send` hook: `Send + Sync` so it can be shared across every
/// user's thread, taking `&mut GooseUser` so it can read or write session
/// data (e.g. a token an `on_response` hook stashed on an earlier request).
pub type BeforeSendHook =
    Arc<dyn Fn(&mut HookRequestParts, &mut GooseUser) -> BeforeSendOutcome + Send + Sync>;

/// An `on_response` hook; see [`BeforeSendHook`] for why it takes
/// `&mut GooseUser`.
pub type OnResponseHook = Arc<
    dyn Fn(Result<&HookResponseParts, &TransportError>, &mut GooseUser) -> ResponseOutcome
        + Send
        + Sync,
>;

/// An async `before_send` hook, for a hook that must do I/O (e.g. an
/// external token refresh). Takes an owned clone of the request's headers
/// rather than `HookRequestParts`' borrow: a `before_send` hook holds `&mut
/// GooseUser`'s borrow across the `.await`, and a borrow tied to a
/// `reqwest::Request` living on the caller's stack cannot cross that
/// `.await` as a `'static`-free trait object without a lifetime higher-rank
/// bound. The clone is the one piece of this prototype's async path that
/// costs more than the sync path would for the same mutation; the boxed
/// future is the other. Both are deliberately paid only when an async hook
/// is registered.
pub type AsyncBeforeSendHook = Arc<
    dyn Fn(HeaderMap) -> futures::future::BoxFuture<'static, (HeaderMap, BeforeSendOutcome)>
        + Send
        + Sync,
>;

/// Every hook a `GooseAttack` registered, shared read-only (behind one
/// `Arc`, cloned — a pointer bump — into every `GooseUser`) once the attack
/// starts.
#[derive(Default, Clone)]
pub struct HookSet {
    pub(crate) before_send: Vec<BeforeSendHook>,
    pub(crate) before_send_async: Vec<AsyncBeforeSendHook>,
    pub(crate) on_response: Vec<(OnResponseHook, RetryPolicy)>,
}

impl HookSet {
    /// Whether any `before_send` hook, sync or async, is registered — the
    /// one check both call sites in `GooseUser::send_request` need, kept in
    /// one place so they can't drift apart.
    pub(crate) fn has_before_send(&self) -> bool {
        !self.before_send.is_empty() || !self.before_send_async.is_empty()
    }
}

/// A closure isn't `Debug`; `GooseUser`'s derived `Debug` only needs to know
/// how many of each hook are registered.
impl std::fmt::Debug for HookSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookSet")
            .field("before_send", &self.before_send.len())
            .field("before_send_async", &self.before_send_async.len())
            .field("on_response", &self.on_response.len())
            .finish()
    }
}

/// How many times Goose will resend a request an `on_response` hook asks to
/// retry, including the first attempt. A hook's `Retry` past this cap is
/// treated as `Accept` with the response already in hand.
pub(crate) const MAX_ATTEMPTS: u32 = 3;
