//! # Type-Safe HTTP Client Builder for Goose Load Tests
//!
//! This module provides a type-safe way to configure HTTP clients for Goose load tests
//! using the type state pattern. The builder ensures cookie-related methods are only
//! available when cookies are enabled, preventing configuration errors at compile time.
//!
//! ## Key Features
//!
//! - **Compile-time safety**: Cookie methods only available on appropriate states
//! - **Performance optimization**: Shared clients when cookies disabled for high user counts
//! - **Zero breaking changes**: All existing APIs continue to work unchanged
//! - **Clean architecture**: Eliminates conditional compilation complexity
//! - **Easy migration**: Opt-in adoption with clear upgrade path
//!
//! ## Basic Usage
//!
//! ### Default Behavior (No changes needed)
//! ```rust
//! use goose::prelude::*;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     // Existing code continues to work exactly as before
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
//! ```rust
//! use goose::prelude::*;
//! use goose::client::GooseClientBuilder;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
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
//! ```
//!
//! ### Optimized Performance (Shared Client)
//! ```rust
//! use goose::prelude::*;
//! use goose::client::GooseClientBuilder;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     // Use shared client for better performance with high user counts
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
//! // ✅ This compiles - cookies are enabled by default on CookiesEnabled state
//! let cookies_enabled = GooseClientBuilder::new()
//!     .timeout(Duration::from_secs(30));
//!
//! // ✅ This compiles - shared methods available on both states  
//! let cookies_disabled = GooseClientBuilder::new()
//!     .without_cookies()
//!     .timeout(Duration::from_secs(30))  // Available
//!     .user_agent("test");               // Available
//!
//! // ❌ This would NOT compile - cookie methods not available on CookiesDisabled
//! // let invalid = GooseClientBuilder::new()
//! //     .without_cookies()
//! //     .cookie_store(true);  // Error: method not found
//!
//! // ✅ State transitions work seamlessly\
//! let transitioning = GooseClientBuilder::new()  // Cookies enabled by default\
//!     .without_cookies()       // Transition to CookiesDisabled\
//!     .timeout(Duration::from_secs(20))  // Available on both\
//!     .with_cookies()          // Transition back to CookiesEnabled\
//!     .timeout(Duration::from_secs(15)); // Continue configuring
//! ```
//!
//! ## Performance Considerations
//!
//! ### Individual Clients (CookiesEnabled)
//! - **Memory usage**: Higher (one client per user)
//! - **Cookie support**: Full cookie jar per user
//! - **Performance**: Standard
//! - **Use cases**: Applications requiring session management, user-specific state
//!
//! ### Shared Client (CookiesDisabled)  
//! - **Memory usage**: Lower (single shared client)
//! - **Cookie support**: None
//! - **Performance**: Optimized for high user counts (1000+ users)
//! - **Use cases**: Stateless API testing, high-scale load tests
//!
//! ## Migration Guide
//!
//! The new client builder system is fully backward compatible:
//!
//! 1. **No changes needed**: Existing code continues to work unchanged
//! 2. **Opt-in optimization**: Add client builder configuration when ready  
//! 3. **Incremental adoption**: Can be adopted gradually across different scenarios

use reqwest::Client;
use std::marker::PhantomData;
use std::sync::Arc;
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
        // Either use manually configured timeout, or default.
        let timeout = if config.timeout.is_some() {
            match crate::util::get_float_from_string(config.timeout.clone()) {
                Some(f) => Some(Duration::from_millis(f as u64 * 1_000)),
                None => Some(Duration::from_millis(60_000)),
            }
        } else {
            Some(Duration::from_millis(60_000))
        };

        Self {
            timeout,
            user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")).to_string(),
            gzip: !config.no_gzip,
            accept_invalid_certs: config.accept_invalid_certs,
            cookies_enabled: true, // Default to enabled, will be overridden by type state
        }
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

/// Type-safe builder for configuring Goose HTTP clients.
pub struct GooseClientBuilder<State = CookiesEnabled> {
    config: GooseClientConfig,
    _state: PhantomData<State>,
}

impl GooseClientBuilder<CookiesEnabled> {
    /// Create a new client builder with cookies enabled by default.
    pub fn new() -> Self {
        Self {
            config: GooseClientConfig::default(),
            _state: PhantomData,
        }
    }

    /// Create a new client builder from an existing GooseConfiguration.
    pub fn from_configuration(config: &GooseConfiguration) -> Self {
        Self {
            config: GooseClientConfig::from(config),
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
            _state: PhantomData,
        }
    }

    /// Build the client strategy for cookies-disabled configuration.
    pub fn build_strategy(self) -> Result<ClientStrategy, GooseError> {
        // Create a new shared client - still provides the performance benefit
        // of sharing one client across all users vs individual per-user clients
        let client =
            create_reqwest_client_without_cookies(&self.config).map_err(GooseError::Reqwest)?;

        Ok(ClientStrategy::Shared(Arc::new(client)))
    }
}

// Shared methods available on both states
impl<State> GooseClientBuilder<State> {
    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
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
        self
    }

    /// Enable or disable acceptance of invalid certificates.
    pub fn accept_invalid_certs(mut self, enabled: bool) -> Self {
        self.config.accept_invalid_certs = enabled;
        self
    }
}

impl Default for GooseClientBuilder<CookiesEnabled> {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a reqwest client without cookie support.
pub(crate) fn create_reqwest_client_without_cookies(
    config: &GooseClientConfig,
) -> Result<Client, reqwest::Error> {
    let mut client_builder = Client::builder()
        .user_agent(&config.user_agent)
        .gzip(config.gzip)
        .danger_accept_invalid_certs(config.accept_invalid_certs);

    if let Some(timeout) = config.timeout {
        client_builder = client_builder.timeout(timeout);
    }

    // Explicitly do NOT add .cookie_store(true) here
    client_builder.build()
}

/// Create a reqwest client with cookie support.
#[allow(dead_code)]
pub(crate) fn create_reqwest_client_with_cookies(
    config: &GooseClientConfig,
) -> Result<Client, reqwest::Error> {
    let mut client_builder = Client::builder()
        .user_agent(&config.user_agent)
        .gzip(config.gzip)
        .danger_accept_invalid_certs(config.accept_invalid_certs);

    if let Some(timeout) = config.timeout {
        client_builder = client_builder.timeout(timeout);
    }

    // Enable cookie store - this calls reqwest's cookie_store method
    #[cfg(feature = "cookies")]
    {
        client_builder = client_builder.cookie_store(true);
    }

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
        use gumdrop::Options;

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
}
