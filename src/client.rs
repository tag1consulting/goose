//! # Type-Safe HTTP Client Builder for Goose Load Tests
//!
//! This module configures the HTTP clients Goose users make requests with. A
//! [`GooseClientBuilder`] collects client settings, and its type parameter records
//! whether the clients store cookies: [`GooseClientBuilder::new`] starts in
//! [`CookiesEnabled`], [`GooseClientBuilder::without_cookies`] moves to
//! [`CookiesDisabled`], and [`GooseClientBuilder::with_cookies`] moves back. Hand
//! the builder to the [`GooseAttack`](crate::GooseAttack) with one of two methods:
//!
//! - [`set_client_builder_with_cookies`](crate::GooseAttack::set_client_builder_with_cookies)
//!   gives every user its own client, with its own cookie jar.
//! - [`set_client_builder_without_cookies`](crate::GooseAttack::set_client_builder_without_cookies)
//!   builds one client, without a cookie store, before any user starts, and shares
//!   it across every user.
//!
//! The two replace each other, and the last call wins. With neither, each user gets
//! its own client built from the configuration, as before this module existed.
//!
//! ## Which settings win
//!
//! The builder's user agent replaces Goose's `goose/<version>`. The timeout, gzip
//! and invalid certificate settings also have a configuration option, and each comes
//! from the first of: `--timeout`, `--no-gzip` or `--accept-invalid-certs` given on
//! the command line; the builder, when it was given that setting; a
//! [`GooseDefault`](crate::config::GooseDefault) for the option; Goose's built in
//! default (a 60 second timeout, gzip on, certificates checked). The users that run
//! the `test_start` and `test_stop` transactions get the builder's clients too.
//! A transaction that calls [`GooseUser::set_client_builder`](crate::goose::GooseUser::set_client_builder)
//! still replaces its own user's client, and with a shared client that user leaves it
//! for a client of its own.
//!
//! ## Basic Usage
//!
//! ### Default Behavior (No changes needed)
//! ```rust
//! use goose::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     // Each user gets its own client, built from the configuration.
//!     let _metrics = GooseAttack::initialize()?
//!         .register_scenario(scenario!("Test").set_host("http://localhost"))
//!         .set_default(GooseDefault::RunTime, 3)?
//!         .set_default(GooseDefault::Users, 1)?
//!         .execute().await?;
//!     Ok(())
//! }
//! ```
//!
//! ### Individual Clients with Custom Configuration
//!
//! This needs the `cookies` feature, which is on by default; without it
//! `set_client_builder_with_cookies` returns
//! [`GooseError::FeatureNotEnabled`].
//! ```rust
//! use goose::prelude::*;
//! use goose::client::GooseClientBuilder;
//! use std::time::Duration;
//!
//! # #[cfg(feature = "cookies")]
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     // Every user gets its own client, with this timeout and user agent.
//!     let _metrics = GooseAttack::initialize()?
//!         .set_client_builder_with_cookies(
//!             GooseClientBuilder::new()
//!                 .timeout(Duration::from_secs(30))
//!                 .user_agent("my-loadtest/1.0")
//!         )?
//!         .register_scenario(scenario!("Test").set_host("http://localhost"))
//!         .set_default(GooseDefault::RunTime, 3)?
//!         .set_default(GooseDefault::Users, 1)?
//!         .execute().await?;
//!     Ok(())
//! }
//! # #[cfg(not(feature = "cookies"))]
//! # fn main() {}
//! ```
//!
//! ### Shared Client
//! ```rust
//! use goose::prelude::*;
//! use goose::client::GooseClientBuilder;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     // One client, built before any user starts, is shared by every user.
//!     let _metrics = GooseAttack::initialize()?
//!         .set_client_builder_without_cookies(
//!             GooseClientBuilder::new()
//!                 .without_cookies()
//!                 .timeout(Duration::from_secs(15))
//!         )?
//!         .register_scenario(scenario!("Test").set_host("http://localhost"))
//!         .set_default(GooseDefault::RunTime, 3)?
//!         .set_default(GooseDefault::Users, 1)?
//!         .execute().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Type Safety Examples
//!
//! ```rust
//! use goose::client::GooseClientBuilder;
//! use std::time::Duration;
//!
//! // A new builder is in the CookiesEnabled state.
//! let cookies_enabled = GooseClientBuilder::new()
//!     .timeout(Duration::from_secs(30));
//!
//! // The settings are available in both states.
//! let cookies_disabled = GooseClientBuilder::new()
//!     .without_cookies()
//!     .timeout(Duration::from_secs(30))
//!     .user_agent("test");
//!
//! // The state decides which GooseAttack method accepts the builder:
//! // set_client_builder_with_cookies takes only a CookiesEnabled builder, and
//! // set_client_builder_without_cookies only a CookiesDisabled one.
//!
//! // A builder can move between the states.
//! let transitioning = GooseClientBuilder::new()
//!     .without_cookies()
//!     .timeout(Duration::from_secs(20))
//!     .with_cookies()
//!     .timeout(Duration::from_secs(15));
//! ```
//!
//! ## Performance Considerations
//!
//! ### Individual Clients (CookiesEnabled)
//! - **Memory usage**: one client per user, built before the users start
//! - **Cookie support**: a cookie jar per user
//! - **Use cases**: applications that need sessions or other per user state
//!
//! ### Shared Client (CookiesDisabled)
//! - **Memory usage**: one client, and one connection pool, for every user
//! - **Cookie support**: none
//! - **Use cases**: stateless API tests, and load tests with many users
//!
//! ## Migration Guide
//!
//! Load tests that set no client builder behave as before. Add a builder when a load
//! test needs different client settings for every user, or a shared client.

use reqwest::{Client, ClientBuilder};
use rustls::client::danger::ServerCertVerifier;
use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, ConfigBuilder, WantsVerifier};
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::{GooseConfiguration, GooseError};

/// Type state indicating cookies are enabled for the client.
pub struct CookiesEnabled;

/// Type state indicating cookies are disabled for the client.
pub struct CookiesDisabled;

/// Configuration for creating reqwest clients.
#[derive(Clone, Debug)]
pub struct GooseClientConfig {
    /// Request timeout duration.
    pub timeout: Option<Duration>,
    /// User agent string to use for requests.
    pub user_agent: String,
    /// Whether to enable gzip compression.
    pub gzip: bool,
    /// Whether to accept invalid certificates.
    pub accept_invalid_certs: bool,
    /// Whether to enable cookies (determined by type state).
    pub cookies_enabled: bool,
}

impl Default for GooseClientConfig {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_millis(60_000)),
            user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")).to_string(),
            gzip: true,
            accept_invalid_certs: false,
            cookies_enabled: true,
        }
    }
}

impl From<&GooseConfiguration> for GooseClientConfig {
    fn from(config: &GooseConfiguration) -> Self {
        Self {
            timeout: Some(config.request_timeout()),
            user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")).to_string(),
            gzip: !config.no_gzip,
            accept_invalid_certs: config.accept_invalid_certs,
            cookies_enabled: true, // Default to enabled, will be overridden by type state
        }
    }
}

#[cfg(test)]
thread_local! {
    /// How many shared clients this thread has built, so a unit test can confirm
    /// users created after the first ones reuse the client already built. Per
    /// thread, so unit tests running at the same time do not count each other's.
    pub(crate) static SHARED_CLIENT_BUILDS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

impl GooseClientConfig {
    /// A client for each user when cookies are enabled, or else one client, built
    /// now, for every user to share.
    pub(crate) fn into_strategy(self) -> Result<ClientStrategy, GooseError> {
        if self.cookies_enabled {
            Ok(ClientStrategy::Individual(self))
        } else {
            let client =
                create_reqwest_client_without_cookies(&self).map_err(GooseError::Reqwest)?;
            #[cfg(test)]
            SHARED_CLIENT_BUILDS.with(|builds| builds.set(builds.get() + 1));
            Ok(ClientStrategy::Shared(Arc::new(client)))
        }
    }
}

/// Which client options were given on the command line, read before
/// [`GooseConfiguration`]'s defaults are merged in, as these win over a client
/// builder's settings while the defaults do not.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommandLineClientOptions {
    /// `--timeout`.
    timeout: bool,
    /// `--no-gzip`.
    no_gzip: bool,
    /// `--accept-invalid-certs`.
    accept_invalid_certs: bool,
}

impl CommandLineClientOptions {
    /// The client options `configuration` holds before it is configured.
    pub(crate) fn from_command_line(configuration: &GooseConfiguration) -> Self {
        Self {
            timeout: configuration.timeout.is_some(),
            no_gzip: configuration.no_gzip,
            accept_invalid_certs: configuration.accept_invalid_certs,
        }
    }
}

/// Which of the settings that also have a configuration option a
/// [`GooseClientBuilder`] was given a value for.
#[derive(Clone, Copy, Debug, Default)]
struct ExplicitSettings {
    timeout: bool,
    gzip: bool,
    accept_invalid_certs: bool,
}

/// What a [`GooseClientBuilder`] handed to a
/// [`GooseAttack`](crate::GooseAttack) holds: its settings, and which of them it
/// was given.
#[derive(Clone, Debug)]
pub(crate) struct ClientBuilderSettings {
    config: GooseClientConfig,
    explicit: ExplicitSettings,
}

impl ClientBuilderSettings {
    /// The settings every user's client is built with. Each setting that also has a
    /// configuration option comes from the command line when it was given there,
    /// else from the builder when the builder was given it, else from the configured
    /// `configuration`, which holds a [`GooseDefault`](crate::config::GooseDefault)
    /// or Goose's built in default.
    pub(crate) fn resolve(
        &self,
        command_line: CommandLineClientOptions,
        configuration: &GooseConfiguration,
    ) -> GooseClientConfig {
        let mut config = GooseClientConfig {
            user_agent: self.config.user_agent.clone(),
            cookies_enabled: self.config.cookies_enabled,
            ..GooseClientConfig::from(configuration)
        };
        if self.explicit.timeout && !command_line.timeout {
            config.timeout = self.config.timeout;
        }
        if self.explicit.gzip && !command_line.no_gzip {
            config.gzip = self.config.gzip;
        }
        if self.explicit.accept_invalid_certs && !command_line.accept_invalid_certs {
            config.accept_invalid_certs = self.config.accept_invalid_certs;
        }
        config
    }
}

/// Strategy for creating HTTP clients based on cookie configuration.
#[derive(Clone, Debug)]
pub enum ClientStrategy {
    /// Create individual clients for each user (cookies enabled).
    Individual(GooseClientConfig),
    /// Use a shared client for all users (cookies disabled).
    Shared(Arc<Client>),
}

impl ClientStrategy {
    /// A client for one user: a new one for `Individual`, or a clone of the shared
    /// client, which shares its connection pool, for `Shared`.
    pub(crate) fn user_client(&self) -> Result<Client, reqwest::Error> {
        match self {
            ClientStrategy::Individual(config) => create_reqwest_client_with_cookies(config),
            ClientStrategy::Shared(shared) => Ok(Client::clone(shared)),
        }
    }
}

/// Type-safe builder for configuring Goose HTTP clients.
pub struct GooseClientBuilder<State = CookiesEnabled> {
    config: GooseClientConfig,
    explicit: ExplicitSettings,
    _state: PhantomData<State>,
}

impl GooseClientBuilder<CookiesEnabled> {
    /// Create a new client builder with cookies enabled by default.
    pub fn new() -> Self {
        Self {
            config: GooseClientConfig::default(),
            explicit: ExplicitSettings::default(),
            _state: PhantomData,
        }
    }

    /// Create a new client builder from an existing GooseConfiguration.
    ///
    /// Its settings count as not given, so when the builder is handed to a
    /// [`GooseAttack`](crate::GooseAttack) they come from that load test's own
    /// configuration instead.
    pub fn from_configuration(config: &GooseConfiguration) -> Self {
        Self {
            config: GooseClientConfig::from(config),
            explicit: ExplicitSettings::default(),
            _state: PhantomData,
        }
    }

    /// Transition to cookies disabled state.
    pub fn without_cookies(self) -> GooseClientBuilder<CookiesDisabled> {
        GooseClientBuilder {
            config: GooseClientConfig {
                cookies_enabled: false,
                ..self.config
            },
            explicit: self.explicit,
            _state: PhantomData,
        }
    }

    /// Build the client strategy for cookies-enabled configuration.
    pub fn build_strategy(self) -> ClientStrategy {
        ClientStrategy::Individual(self.config)
    }
}

impl GooseClientBuilder<CookiesDisabled> {
    /// Transition back to cookies enabled state.
    pub fn with_cookies(self) -> GooseClientBuilder<CookiesEnabled> {
        GooseClientBuilder {
            config: GooseClientConfig {
                cookies_enabled: true,
                ..self.config
            },
            explicit: self.explicit,
            _state: PhantomData,
        }
    }

    /// Build the client strategy for cookies-disabled configuration.
    pub fn build_strategy(self) -> Result<ClientStrategy, GooseError> {
        // Create a new shared client - still provides the performance benefit
        // of sharing one client across all users vs individual per-user clients
        self.config.into_strategy()
    }
}

// Shared methods available on both states
impl<State> GooseClientBuilder<State> {
    /// The settings collected so far, and which of them were given.
    pub(crate) fn into_settings(self) -> ClientBuilderSettings {
        ClientBuilderSettings {
            config: self.config,
            explicit: self.explicit,
        }
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self.explicit.timeout = true;
        self
    }

    /// Set the user agent string.
    pub fn user_agent<T: Into<String>>(mut self, user_agent: T) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }

    /// Enable or disable gzip compression.
    pub fn gzip(mut self, enabled: bool) -> Self {
        self.config.gzip = enabled;
        self.explicit.gzip = true;
        self
    }

    /// Enable or disable acceptance of invalid certificates.
    pub fn accept_invalid_certs(mut self, enabled: bool) -> Self {
        self.config.accept_invalid_certs = enabled;
        self.explicit.accept_invalid_certs = true;
        self
    }
}

impl Default for GooseClientBuilder<CookiesEnabled> {
    fn default() -> Self {
        Self::new()
    }
}

/// The part of reqwest's default TLS setup that is costly to build, built once and
/// shared by every client Goose builds: the certificate verifier, which on Linux reads
/// and parses every system root certificate when it is built, and the rustls config
/// builder holding the crypto provider and TLS versions it was built with.
struct SharedTls {
    builder: ConfigBuilder<ClientConfig, WantsVerifier>,
    verifier: Arc<dyn ServerCertVerifier>,
}

impl SharedTls {
    /// What reqwest 0.13 builds for each client by default: the crypto provider
    /// installed as the process default, else aws-lc-rs, rustls' TLS 1.3 and 1.2,
    /// and the platform verifier over the system's root certificates.
    fn new() -> Option<Self> {
        let provider = CryptoProvider::get_default()
            .cloned()
            .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
        let verifier = rustls_platform_verifier::Verifier::new(provider.clone()).ok()?;
        Self::with_verifier(provider, Arc::new(verifier))
    }

    /// The TLS versions reqwest offers by default, with `provider` and `verifier`.
    fn with_verifier(
        provider: Arc<CryptoProvider>,
        verifier: Arc<dyn ServerCertVerifier>,
    ) -> Option<Self> {
        let builder = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(rustls::ALL_VERSIONS)
            .ok()?;
        Some(Self { builder, verifier })
    }

    /// A TLS config for one client, set as reqwest sets its default: the shared
    /// verifier, no client certificate, SNI on, HTTP/1.1 offered by ALPN as reqwest
    /// is built without HTTP/2, and its own session resumption store, so no TLS
    /// session is resumed across clients.
    fn client_config(&self) -> ClientConfig {
        let mut config = self
            .builder
            .clone()
            .dangerous()
            .with_custom_certificate_verifier(self.verifier.clone())
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config
    }
}

/// Built the first time a client checks certificates, then shared. `None` if the
/// verifier could not be built, as with no system root certificates; each client
/// then builds its own as before, so reqwest returns the error it always did.
static SHARED_TLS: OnceLock<Option<SharedTls>> = OnceLock::new();

#[cfg(test)]
/// How many times [`SHARED_TLS`] was built in this process.
pub(crate) static SHARED_TLS_BUILDS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn shared_tls() -> Option<&'static SharedTls> {
    SHARED_TLS
        .get_or_init(|| {
            #[cfg(test)]
            SHARED_TLS_BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            SharedTls::new()
        })
        .as_ref()
}

/// A client builder with the settings both kinds of client share, and a TLS config
/// over `shared` unless certificates are not checked, as reqwest then loads no
/// root certificates.
fn client_builder(config: &GooseClientConfig, shared: Option<&SharedTls>) -> ClientBuilder {
    let mut client_builder = Client::builder()
        .user_agent(&config.user_agent)
        .gzip(config.gzip)
        .danger_accept_invalid_certs(config.accept_invalid_certs);

    if !config.accept_invalid_certs {
        if let Some(shared) = shared {
            client_builder = client_builder.tls_backend_preconfigured(shared.client_config());
        }
    }

    if let Some(timeout) = config.timeout {
        client_builder = client_builder.timeout(timeout);
    }

    client_builder
}

/// Create a reqwest client without cookie support.
pub(crate) fn create_reqwest_client_without_cookies(
    config: &GooseClientConfig,
) -> Result<Client, reqwest::Error> {
    // Explicitly do NOT add .cookie_store(true) here
    client_builder(config, shared_tls()).build()
}

/// Create a reqwest client with cookie support.
pub(crate) fn create_reqwest_client_with_cookies(
    config: &GooseClientConfig,
) -> Result<Client, reqwest::Error> {
    let client_builder = client_builder(config, shared_tls());

    // Enable cookie store - this calls reqwest's cookie_store method
    #[cfg(feature = "cookies")]
    let client_builder = client_builder.cookie_store(true);

    client_builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_state_transitions() {
        // Start with cookies enabled
        let builder = GooseClientBuilder::new();

        // Can transition to cookies disabled
        let builder = builder.without_cookies();

        // Can transition back to cookies enabled
        let _builder = builder.with_cookies();
    }

    #[test]
    fn test_shared_methods() {
        let builder = GooseClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .user_agent("test-agent")
            .gzip(false)
            .accept_invalid_certs(true);

        assert_eq!(builder.config.timeout, Some(Duration::from_secs(30)));
        assert_eq!(builder.config.user_agent, "test-agent");
        assert!(!builder.config.gzip);
        assert!(builder.config.accept_invalid_certs);
    }

    #[test]
    fn test_configuration_from_goose_config() {
        use crate::config::GooseConfiguration;

        let args: Vec<&str> = vec![];
        let config = GooseConfiguration::parse_args_default(&args).unwrap();
        let builder = GooseClientBuilder::from_configuration(&config);

        // Should have default values
        assert!(builder.config.cookies_enabled);
        assert!(builder.config.gzip); // Should be true when no_gzip is false
    }

    #[test]
    fn test_client_strategy_individual() {
        let builder = GooseClientBuilder::new().timeout(Duration::from_secs(10));

        let strategy = builder.build_strategy();

        match strategy {
            ClientStrategy::Individual(config) => {
                assert_eq!(config.timeout, Some(Duration::from_secs(10)));
                assert!(config.cookies_enabled);
            }
            ClientStrategy::Shared(_) => panic!("Expected Individual strategy"),
        }
    }

    #[test]
    fn test_client_strategy_shared() {
        let builder = GooseClientBuilder::new()
            .without_cookies()
            .timeout(Duration::from_secs(10));

        let strategy = builder.build_strategy().unwrap();

        match strategy {
            ClientStrategy::Shared(_) => {
                // Success - we got a shared client
            }
            ClientStrategy::Individual(_) => panic!("Expected Shared strategy"),
        }
    }

    /// Answers each HTTPS request on 127.0.0.1 with `200 ok`, as `localhost`, with a
    /// certificate signed by `tests/fixtures/tls/ca.pem`, a CA no system trusts. It
    /// closes every connection, so each request is a new TLS handshake, and sends
    /// whether each handshake that got as far as a request resumed a session.
    fn tls_server() -> (String, std::sync::mpsc::Receiver<rustls::HandshakeKind>) {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use std::io::{Read, Write};

        let certs =
            CertificateDer::pem_slice_iter(include_bytes!("../tests/fixtures/tls/localhost.pem"))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        let key =
            PrivateKeyDer::from_pem_slice(include_bytes!("../tests/fixtures/tls/localhost.key"))
                .unwrap();
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap(),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "https://localhost:{}/",
            listener.local_addr().unwrap().port()
        );
        let (handshakes, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                let mut tls = rustls::StreamOwned::new(connection, stream);
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                if !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    // The client refused the certificate.
                    continue;
                }
                handshakes.send(tls.conn.handshake_kind().unwrap()).unwrap();
                tls.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                )
                .unwrap();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        });
        (url, received)
    }

    /// The crypto provider reqwest 0.13 builds a client's TLS config with.
    fn reqwest_provider() -> Arc<CryptoProvider> {
        CryptoProvider::get_default()
            .cloned()
            .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
    }

    /// Every path that builds a user's client, and the shared client, uses one
    /// verifier, built once in the process however many clients are built.
    #[test]
    fn user_clients_share_one_tls_verifier() {
        let strategy = GooseClientBuilder::new().build_strategy();
        for _ in 0..3 {
            strategy.user_client().unwrap();
        }
        let configuration =
            crate::GooseConfiguration::parse_args_default(&Vec::<&str>::new()).unwrap();
        for _ in 0..2 {
            crate::goose::GooseUser::new(
                0,
                Arc::from(""),
                url::Url::parse("http://localhost/").unwrap(),
                &configuration,
                0,
                None,
            )
            .unwrap();
        }
        GooseClientBuilder::new()
            .without_cookies()
            .build_strategy()
            .unwrap();
        assert_eq!(
            SHARED_TLS_BUILDS.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    /// Each client keeps its own TLS session resumption store: a client resumes its
    /// own session, as with reqwest's default, and a second client sharing the
    /// verifier starts a full handshake.
    #[tokio::test]
    async fn user_clients_do_not_share_tls_sessions() {
        let (url, handshakes) = tls_server();
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(
                rustls::pki_types::pem::PemObject::from_pem_slice(include_bytes!(
                    "../tests/fixtures/tls/ca.pem"
                ))
                .unwrap(),
            )
            .unwrap();
        let provider = reqwest_provider();
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            provider.clone(),
        )
        .build()
        .unwrap();
        let shared = SharedTls::with_verifier(provider, verifier).unwrap();
        let config = GooseClientConfig::default();
        let first = client_builder(&config, Some(&shared)).build().unwrap();
        let second = client_builder(&config, Some(&shared)).build().unwrap();

        let mut kinds = Vec::new();
        for client in [&first, &first, &second, &second] {
            let body = client.get(&url).send().await.unwrap().text().await.unwrap();
            assert_eq!(body, "ok");
            kinds.push(handshakes.recv_timeout(Duration::from_secs(10)).unwrap());
        }
        use rustls::HandshakeKind::{Full, Resumed};
        assert_eq!(kinds, [Full, Resumed, Full, Resumed]);
    }

    /// The shared verifier trusts what reqwest's own default build trusts: the same
    /// platform verifier with the same root certificates (rustls shows how many in
    /// `Debug`, not which), and a certificate from a CA the system does not trust is
    /// refused with the same error as by a client reqwest builds itself, and
    /// accepted by both when certificates are not checked.
    #[tokio::test]
    async fn user_clients_trust_what_reqwest_trusts() {
        let shared = shared_tls().unwrap();
        let reqwest_default = rustls_platform_verifier::Verifier::new(reqwest_provider()).unwrap();
        assert_eq!(
            format!("{:?}", shared.verifier),
            format!("{:?}", reqwest_default)
        );

        fn error_chain(error: &reqwest::Error) -> Vec<String> {
            let mut chain = vec![error.to_string()];
            let mut source = std::error::Error::source(error);
            while let Some(error) = source {
                chain.push(error.to_string());
                source = error.source();
            }
            chain
        }

        let (url, _handshakes) = tls_server();
        let checked = GooseClientConfig::default();
        let goose = create_reqwest_client_with_cookies(&checked).unwrap();
        let reqwest = Client::builder().build().unwrap();
        let goose_error = goose.get(&url).send().await.unwrap_err();
        let reqwest_error = reqwest.get(&url).send().await.unwrap_err();
        assert!(
            error_chain(&goose_error)
                .iter()
                .any(|e| e.contains("UnknownIssuer")),
            "{:?}",
            error_chain(&goose_error)
        );
        assert_eq!(error_chain(&goose_error), error_chain(&reqwest_error));

        let unchecked = GooseClientConfig {
            accept_invalid_certs: true,
            ..GooseClientConfig::default()
        };
        let goose = create_reqwest_client_with_cookies(&unchecked).unwrap();
        let reqwest = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        for client in [&goose, &reqwest] {
            let body = client.get(&url).send().await.unwrap().text().await.unwrap();
            assert_eq!(body, "ok");
        }
    }
}
