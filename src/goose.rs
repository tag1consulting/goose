//! Helpers and objects for building Goose load tests.
//!
//! Goose manages load tests with a series of objects:
//!
//! - [`Scenario`] each user is assigned a scenario, which is a collection of transactions.
//! - [`Transaction`] transactions define one or more web requests and are assigned to scenarios.
//! - [`GooseUser`] a user state responsible for repeatedly running all transactions in the assigned scenario.
//! - [`GooseRequestMetric`] optional metrics collected for each URL/method pair.
//!
//! ## Creating Scenarios
//!
//! A [`Scenario`](./struct.Scenario.html) is created by passing in a `&str` name to the `new` function, for example:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut loadtest_transactions = scenario!("LoadtestTransactions");
//! ```
//!
//! ### Scenario Weight
//!
//! A weight can be applied to a scenario, controlling how often it is assigned to
//! [`GooseUser`](../goose/struct.GooseUser.html) threads. The larger the integer value
//! of weight, the more the scenario will be assigned to user threads. In the following
//! example, `FooTransactions` will be assigned to users twice as often as `Bar`
//! Transactions. We could have just added a weight of `2` to `FooTransactions` and left
//! the default weight of `1` assigned to `BarTransactions` for the same weighting:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     let mut foo_transactions = scenario!("FooTransactions").set_weight(10)?;
//!     let mut bar_transactions = scenario!("BarTransactions").set_weight(5)?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Scenario Host
//!
//! A default host can be assigned to a scenario, which will be used only if the `--host`
//! CLI option is not set at run-time. For example, this can configure your load test to
//! run against your local development environment by default, allowing the `--host` option
//! to override host when you want to load test production. You can also assign different
//! hosts to different scenario if this is desirable:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut foo_transactions = scenario!("FooTransactions").set_host("http://www.local");
//! let mut bar_transactions = scenario!("BarTransactions").set_host("http://www2.local");
//! ```
//!
//! ### Scenario Wait Time
//!
//! Wait time is specified as a low-high Duration range. Each time a transaction completes in the
//! scenario, the user will pause for a random number of milliseconds inclusively between
//! the low and high wait times. In the following example, users loading `foo` transactions will
//! sleep 0 to 2.5 seconds after each transaction completes, and users loading `bar` transactions will
//! always sleep 5 seconds after each transaction completes.
//!
//! ```rust
//! use goose::prelude::*;
//! use std::time::Duration;
//!
//! let mut foo_transactions = scenario!("FooTransactions").set_wait_time(Duration::from_secs(0), Duration::from_millis(2500)).unwrap();
//! let mut bar_transactions = scenario!("BarTransactions").set_wait_time(Duration::from_secs(5), Duration::from_secs(5)).unwrap();
//! ```
//! ## Creating Transactions
//!
//! A [`Transaction`](./struct.Transaction.html) must include a pointer to a function which
//! will be executed each time the transaction is run.
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut a_transaction = transaction!(transaction_function);
//!
//! /// A very simple transaction that loads the front page.
//! async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Transaction Name
//!
//! A name can be assigned to a transaction, and will be displayed in metrics about all requests
//! made by the transaction.
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut a_transaction = transaction!(transaction_function).set_name("a");
//!
//! /// A very simple transaction that loads the front page.
//! async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Transaction Weight
//!
//! Individual transactions can be assigned a weight, controlling how often the transaction runs. The
//! larger the value of weight, the more it will run. In the following example, `a_transaction`
//! runs 3 times as often as `b_transaction`:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), GooseError> {
//!     let mut a_transaction = transaction!(a_transaction_function).set_weight(9)?;
//!     let mut b_transaction = transaction!(b_transaction_function).set_weight(3)?;
//!
//!     Ok(())
//! }
//!
//! /// A very simple transaction that loads the "a" page.
//! async fn a_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("a/").await?;
//!
//!     Ok(())
//! }
//!
//! /// Another very simple transaction that loads the "b" page.
//! async fn b_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("b/").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Transaction Sequence
//!
//! Transactions can also be configured to run in a sequence. For example, a transaction with a
//! sequence value of `1` will always run before a transaction with a sequence value of `2`. Weight can
//! be applied to sequenced transactions, so for example a transaction with a weight of `2` and a sequence
//! of `1` will run two times before a transaction with a sequence of `2`. Scenarios can contain
//! transactions with sequence values and without sequence values, and in this case all transactions with
//! a sequence value will run before transactions without a sequence value. In the following example,
//! `a_transaction` runs before `b_transaction`, which runs before `c_transaction`:
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut a_transaction = transaction!(a_transaction_function).set_sequence(1);
//! let mut b_transaction = transaction!(b_transaction_function).set_sequence(2);
//! let mut c_transaction = transaction!(c_transaction_function);
//!
//! /// A very simple transaction that loads the "a" page.
//! async fn a_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("a/").await?;
//!
//!     Ok(())
//! }
//!
//! /// Another very simple transaction that loads the "b" page.
//! async fn b_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("b/").await?;
//!
//!     Ok(())
//! }
//!
//! /// Another very simple transaction that loads the "c" page.
//! async fn c_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("c/").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Transaction On Start
//!
//! Transactions can be flagged to only run when a user first starts. This can be useful if you'd
//! like your load test to use a logged-in user. It is possible to assign sequences and weights
//! to [`on_start`](./struct.Transaction.html#method.set_on_start) functions if you want to have
//! multiple transactions run in a specific order at start time, and/or the transactions to run multiple times.
//! A transaction can be flagged to run both on start and on stop.
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut a_transaction = transaction!(a_transaction_function).set_sequence(1).set_on_start();
//!
//! /// A very simple transaction that loads the "a" page.
//! async fn a_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("a/").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Transaction On Stop
//!
//! Transactions can be flagged to only run when a user stops. This can be useful if you'd like your
//! load test to simulate a user logging out when it finishes. It is possible to assign sequences
//! and weights to [`on_stop`](./struct.Transaction.html#method.set_on_stop) functions if you want to
//! have multiple transactions run in a specific order at stop time, and/or the transactions to run multiple
//! times. A transaction can be flagged to run both on start and on stop.
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut b_transaction = transaction!(b_transaction_function).set_sequence(2).set_on_stop();
//!
//! /// Another very simple transaction that loads the "b" page.
//! async fn b_transaction_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("b/").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Controlling User
//!
//! When Goose starts, it creates one or more [`GooseUser`](./struct.GooseUser.html)s,
//! assigning a single [`Scenario`](./struct.Scenario.html) to each. This user is
//! then used to generate load. Behind the scenes, Goose is leveraging the
//! [`reqwest::client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html)
//! to load web pages, and Goose can therefor do anything [`reqwest`](https://docs.rs/reqwest/)
//! can do.
//!
//! The most common request types are [`GET`](./struct.GooseUser.html#method.get) and
//! [`POST`](./struct.GooseUser.html#method.post), but [`HEAD`](./struct.GooseUser.html#method.head),
//! PUT, PATCH and [`DELETE`](./struct.GooseUser.html#method.delete) are also supported.
//!
//! GraphQL requests are supported with [`post_graphql`](./struct.GooseUser.html#method.post_graphql)
//! and [`post_graphql_named`](./struct.GooseUser.html#method.post_graphql_named) helper methods.
//!
//! ### GET
//!
//! A helper to make a `GET` request of a path and collect relevant metrics.
//! Automatically prepends the correct host.
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut transaction = transaction!(get_function);
//!
//! /// A very simple transaction that makes a GET request.
//! async fn get_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.get("path/to/foo/").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! The returned response is a [`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)
//! struct. You can use it as you would any Reqwest Response.
//!
//!
//! ### POST
//!
//! A helper to make a `POST` request of a string value to the path and collect relevant
//! metrics. Automatically prepends the correct host. The returned response is a
//! [`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)
//!
//! ```rust
//! use goose::prelude::*;
//!
//! let mut transaction = transaction!(post_function);
//!
//! /// A very simple transaction that makes a POST request.
//! async fn post_function(user: &mut GooseUser) -> TransactionResult {
//!     let _goose = user.post("path/to/foo/", "string value to post").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## License
//!
//! Copyright 2020-2023 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

use downcast_rs::{impl_downcast, Downcast};
use regex::Regex;
use reqwest::{header, Client, ClientBuilder, Method, RequestBuilder, Response};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter, Write as _};
use std::hash::{Hash, Hasher};
use std::str;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use std::{future::Future, pin::Pin, time::Instant};
use url::Url;

use crate::logger::GooseLog;
use crate::metrics::{
    GooseCoordinatedOmissionMitigation, GooseMetric, GooseMetricBatch, GooseRawRequest,
    GooseRequestCounterRegistry, GooseRequestCounters, GooseRequestMetric,
    GooseRequestMetricTimingData, MetricsEpoch, RequestBatchEntry, ScenarioBatchEntry,
    ScenarioMetric, StatusCodeTimingSummary, TransactionBatchEntry, TransactionDetail,
    TransactionMetric, METRICS_BATCH_MAX_AGE, METRICS_BATCH_SIZE,
};
use crate::{GooseConfiguration, GooseError, WeightedTransactions};

/// By default Goose sets the following User-Agent header when making requests.
static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// Atomically decrement `counter` by one, saturating at zero.
///
/// An explicit `compare_exchange_weak` loop with `Relaxed` ordering, equivalent to
/// `fetch_update(Relaxed, Relaxed, |v| Some(v.saturating_sub(1)))`, which is
/// deprecated as of Rust 1.99 in favor of `try_update`.
fn saturating_decrement(counter: &AtomicU64) {
    let mut current = counter.load(AtomicOrdering::Relaxed);
    loop {
        match counter.compare_exchange_weak(
            current,
            current.saturating_sub(1),
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

/// `transaction!(foo)` expands to `Transaction::new(foo)`, but also does some boxing to work around a limitation in the compiler.
#[macro_export]
macro_rules! transaction {
    ($transaction_func:ident) => {
        Transaction::new(std::sync::Arc::new(move |s| {
            std::boxed::Box::pin($transaction_func(s))
        }))
    };
}

/// `scenario!("foo")` expands to `Scenario::new("foo")`.
#[macro_export]
macro_rules! scenario {
    ($name:tt) => {
        Scenario::new($name)
    };
}

/// Goose transactions return a result, which is empty on success, or contains a boxed
/// [`TransactionError`](./enum.TransactionError.html) on error.
pub type TransactionResult = Result<(), Box<TransactionError>>;

/// An enumeration of all errors a [`Transaction`](./struct.Transaction.html) can return.
#[derive(Debug)]
pub enum TransactionError {
    /// Wraps a [`reqwest::Error`](https://docs.rs/reqwest/*/reqwest/struct.Error.html).
    Reqwest(reqwest::Error),
    /// Wraps a [`url::ParseError`](https://docs.rs/url/*/url/enum.ParseError.html).
    Url(url::ParseError),
    /// A generic, custom error for cases where the [`TransactionFunction`] cannot use any of the standard error types.
    ///
    /// # Examples
    /// ```rust
    /// use goose::goose::TransactionError;
    ///
    /// // Direct creation
    /// let error = TransactionError::Custom("User validation failed".to_string());
    ///
    /// // Using From<&str> (most convenient)
    /// let error: TransactionError = "Database connection timeout".into();
    /// ```
    Custom(String),
    /// The request failed.
    RequestFailed {
        /// The [`GooseRequestMetric`](./struct.GooseRequestMetric.html) that failed.
        raw_request: Box<GooseRequestMetric>,
    },
    /// The request was canceled. This happens when the throttle is enabled and the load
    /// test finishes.
    RequestCanceled {
        /// Wraps a [`flume::SendError`](https://docs.rs/flume/*/flume/struct.SendError.html),
        /// a [`GooseRequestMetric`](./struct.GooseRequestMetric.html) has not yet been constructed.
        source: flume::SendError<bool>,
    },
    /// There was an error sending the metrics for a request to the parent thread.
    MetricsFailed {
        /// Wraps a [`flume::SendError`](https://docs.rs/flume/*/flume/struct.SendError.html),
        /// which contains the [`GooseMetric`](../metrics/enum.GooseMetric.html) that wasn't sent.
        source: flume::SendError<GooseMetric>,
    },
    /// There was an error sending debug information to the logger thread.
    LoggerFailed {
        /// Wraps a [`flume::SendError`](https://docs.rs/flume/*/flume/struct.SendError.html),
        /// which contains the [`GooseDebug`](./struct.GooseDebug.html) that wasn't sent.
        source: Box<flume::SendError<Option<GooseLog>>>,
    },
    /// Attempted an unrecognized HTTP request method.
    InvalidMethod {
        /// The unrecognized HTTP request method.
        method: Method,
    },
}
/// Implement a helper to provide a text description of all possible types of errors.
impl TransactionError {
    fn describe(&self) -> &str {
        match *self {
            TransactionError::Reqwest(_) => "reqwest::Error",
            TransactionError::Url(_) => "url::ParseError",
            TransactionError::Custom(_) => "custom error",
            TransactionError::RequestFailed { .. } => "request failed",
            TransactionError::RequestCanceled { .. } => {
                "request canceled because throttled load test ended"
            }
            TransactionError::MetricsFailed { .. } => "failed to send metrics to parent thread",
            TransactionError::LoggerFailed { .. } => "failed to send log message to logger thread",
            TransactionError::InvalidMethod { .. } => "unrecognized HTTP request method",
        }
    }
}

/// Implement format trait to allow displaying errors.
impl fmt::Display for TransactionError {
    // Implement display of error with `{}` marker.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TransactionError::Reqwest(ref source) => {
                write!(f, "TransactionError: {} ({})", self.describe(), source)
            }
            TransactionError::Url(ref source) => {
                write!(f, "TransactionError: {} ({})", self.describe(), source)
            }
            TransactionError::RequestCanceled { ref source } => {
                write!(f, "TransactionError: {} ({})", self.describe(), source)
            }
            TransactionError::MetricsFailed { ref source } => {
                write!(f, "TransactionError: {} ({})", self.describe(), source)
            }
            TransactionError::LoggerFailed { ref source } => {
                write!(f, "TransactionError: {} ({})", self.describe(), source)
            }
            TransactionError::Custom(ref message) => {
                write!(f, "TransactionError: {} ({})", self.describe(), message)
            }
            _ => write!(f, "TransactionError: {}", self.describe()),
        }
    }
}

// Define the lower level source of this error, if any.
impl std::error::Error for TransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            TransactionError::Reqwest(ref source) => Some(source),
            TransactionError::Url(ref source) => Some(source),
            TransactionError::RequestCanceled { ref source } => Some(source),
            TransactionError::MetricsFailed { ref source } => Some(source),
            TransactionError::LoggerFailed { ref source } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// Auto-convert Reqwest errors.
impl From<reqwest::Error> for TransactionError {
    fn from(err: reqwest::Error) -> TransactionError {
        TransactionError::Reqwest(err)
    }
}

/// Auto-convert Reqwest errors to boxed TransactionError.
impl From<reqwest::Error> for Box<TransactionError> {
    fn from(value: reqwest::Error) -> Self {
        Box::new(TransactionError::Reqwest(value))
    }
}

/// Auto-convert Url errors.
impl From<url::ParseError> for TransactionError {
    fn from(err: url::ParseError) -> TransactionError {
        TransactionError::Url(err)
    }
}

/// When the throttle is enabled and the load test ends, the throttle channel is
/// shut down. This causes a
/// [`flume::SendError`](https://docs.rs/flume/*/flume/struct.SendError.html),
/// which gets automatically converted to `RequestCanceled`.
/// [`RequestCanceled`](./enum.TransactionError.html#variant.RequestCanceled)
impl From<flume::SendError<bool>> for TransactionError {
    fn from(source: flume::SendError<bool>) -> TransactionError {
        TransactionError::RequestCanceled { source }
    }
}

/// Attempt to send metrics to the parent thread failed.
impl From<flume::SendError<GooseMetric>> for TransactionError {
    fn from(source: flume::SendError<GooseMetric>) -> TransactionError {
        TransactionError::MetricsFailed { source }
    }
}

/// Attempt to send logs to the logger thread failed.
impl From<flume::SendError<Option<GooseLog>>> for TransactionError {
    fn from(source: flume::SendError<Option<GooseLog>>) -> TransactionError {
        TransactionError::LoggerFailed {
            source: Box::new(source),
        }
    }
}

/// Auto-convert String errors to Custom TransactionError.
impl From<String> for TransactionError {
    fn from(err: String) -> TransactionError {
        TransactionError::Custom(err)
    }
}

/// Auto-convert &str errors to Custom TransactionError.
impl From<&str> for TransactionError {
    fn from(err: &str) -> TransactionError {
        TransactionError::Custom(err.to_string())
    }
}

/// Auto-convert String errors to boxed TransactionError.
impl From<String> for Box<TransactionError> {
    fn from(value: String) -> Self {
        Box::new(TransactionError::Custom(value))
    }
}

/// Auto-convert &str errors to boxed TransactionError.
impl From<&str> for Box<TransactionError> {
    fn from(value: &str) -> Self {
        Box::new(TransactionError::Custom(value.to_string()))
    }
}

/// An individual scenario.
#[derive(Clone, Hash)]
pub struct Scenario {
    /// The name of the scenario.
    pub name: Arc<str>,
    /// Auto-generated machine name of the scenario.
    pub machine_name: Arc<str>,
    /// An integer reflecting where this scenario lives in the internal
    /// [`GooseAttack`](../struct.GooseAttack.html)`.scenarios` vector.
    pub scenarios_index: usize,
    /// An integer value that controls the frequency that this scenario will be assigned to a user.
    pub weight: usize,
    /// A [`Duration`](https://doc.rust-lang.org/std/time/struct.Duration.html) range defining the
    /// minimum and maximum time a [`GooseUser`] should sleep after running a transaction.
    pub transaction_wait: Option<(Duration, Duration)>,
    /// A vector containing one copy of each [`Transaction`](./struct.Transaction.html) that will
    /// run by users running this scenario.
    pub transactions: Vec<Transaction>,
    /// A fully scheduled and weighted vector of integers (pointing to
    /// [`Transaction`](./struct.Transaction.html)s and [`Transaction`](./struct.Transaction.html) names.
    pub weighted_transactions: WeightedTransactions,
    /// A vector of vectors of integers, controlling the sequence and order
    /// [`on_start`](./struct.Transaction.html#method.set_on_start)
    /// [`Transaction`](./struct.Transaction.html)s are run when the user first starts.
    pub weighted_on_start_transactions: WeightedTransactions,
    /// A vector of vectors of integers, controlling the sequence and order
    /// [`on_stop`](./struct.Transaction.html#method.set_on_stop)
    /// [`Transaction`](./struct.Transaction.html)s are run when the user first starts.
    pub weighted_on_stop_transactions: WeightedTransactions,
    /// An optional default host to run this `Scenario` against.
    pub host: Option<String>,
}
impl Scenario {
    /// Creates a new [`Scenario`](./struct.Scenario.html). Once created, a
    /// [`Transaction`](./struct.Transaction.html) must be assigned to it, and finally it must
    /// be registered with the [`GooseAttack`](../struct.GooseAttack.html) object. The
    /// returned object must be stored in a mutable value.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut example_transactions = scenario!("ExampleTransactions");
    /// ```
    pub fn new(name: &str) -> Self {
        trace!("new scenario: name: {}", name);
        Scenario {
            name: Arc::from(name),
            machine_name: Arc::from(Scenario::get_machine_name(name)),
            scenarios_index: usize::MAX,
            weight: 1,
            transaction_wait: None,
            transactions: Vec::new(),
            weighted_transactions: Vec::new(),
            weighted_on_start_transactions: Vec::new(),
            weighted_on_stop_transactions: Vec::new(),
            host: None,
        }
    }

    /// An internal helper to convert a Scenario name to a machine name which is only
    /// alphanumerics.
    fn get_machine_name(name: &str) -> String {
        // Remove all non-alphanumeric characters.
        let re = Regex::new("[^a-zA-Z0-9]+").unwrap();
        let alphanumeric = re.replace_all(name, "");
        // Convert to lower case.
        alphanumeric.to_lowercase()
    }

    /// Registers a [`Transaction`](./struct.Transaction.html) with a
    /// [`Scenario`](./struct.Scenario.html), where it is stored in the
    /// [`Scenario`](./struct.Scenario.html)`.transactions` vector. The function
    /// associated with the transaction will be run during the load test.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut example_transactions = scenario!("ExampleTransactions");
    /// example_transactions.register_transaction(transaction!(a_transaction_function));
    ///
    /// /// A very simple transaction that loads the "a" page.
    /// async fn a_transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("a/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn register_transaction(mut self, mut transaction: Transaction) -> Self {
        trace!(
            "{} register_transaction: {}",
            self.name,
            transaction.name.name_for_transaction()
        );
        transaction.transactions_index = self.transactions.len();
        self.transactions.push(transaction);
        self
    }

    /// Sets a weight on a scenario. The larger the value of weight, the more often the scenario will
    /// be assigned to users. For example, if you have scenario foo with a weight of 3, and scenario
    /// bar with a weight of 1, and you spin up a load test with 8 users, 6 of them will be running
    /// the foo scenario, and 2 will be running the bar scenario.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     let mut example_transactions = scenario!("ExampleTransactions").set_weight(3)?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Result<Self, GooseError> {
        trace!("{} set_weight: {}", self.name, weight);
        if weight == 0 {
            return Err(GooseError::InvalidWeight {
                weight,
                detail: ("Weight must be set to at least 1.".to_string()),
            });
        }
        self.weight = weight;

        Ok(self)
    }

    /// Set a default host for the scenario. If no `--host` flag is set when running the load test, this
    /// host will be pre-pended on all requests. For example, this can configure your load test to run
    /// against your local development environment by default, and the `--host` option could be used to
    /// override host when running the load test against production.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut example_transactions = scenario!("ExampleTransactions").set_host("http://10.1.1.42");
    /// ```
    pub fn set_host(mut self, host: &str) -> Self {
        trace!("{} set_host: {}", self.name, host);
        // Host validation happens in main() at startup.
        self.host = Some(host.to_string());
        self
    }

    /// Configure a senario to to pause after running each transaction. The length of the pause will be randomly
    /// selected from `min_wait` to `max_wait` inclusively.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     scenario!("ExampleTransactions").set_wait_time(Duration::from_secs(0), Duration::from_secs(1))?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_wait_time(
        mut self,
        min_wait: Duration,
        max_wait: Duration,
    ) -> Result<Self, GooseError> {
        trace!(
            "{} set_wait time: min: {:?} max: {:?}",
            self.name,
            min_wait,
            max_wait
        );
        if min_wait.as_millis() > max_wait.as_millis() {
            return Err(GooseError::InvalidWaitTime {
                min_wait,
                max_wait,
                detail:
                    "The min_wait option can not be set to a larger value than the max_wait option."
                        .to_string(),
            });
        }
        self.transaction_wait = Some((min_wait, max_wait));

        Ok(self)
    }
}

/// Commands sent from the parent thread to the user threads, and from the manager to the
/// worker processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GooseUserCommand {
    /// Tell worker process to pause load test.
    Wait,
    /// Tell worker process to start load test.
    Run,
    /// Tell user thread or worker process to exit.
    Exit,
}

/// Supported request methods, including HTTP methods and [`GooseMethod::Custom`] for non-HTTP protocols.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Ord, PartialOrd, Hash)]
pub enum GooseMethod {
    Delete,
    Get,
    Head,
    Patch,
    Post,
    Put,
    /// Used for non-HTTP protocols (TCP, UDP, WebSocket, etc.) when recording
    /// custom request metrics via [`GooseUser::record_custom_request`].
    Custom,
}
/// Display method in upper case.
impl fmt::Display for GooseMethod {
    // Implement display of `GooseMethod` with `{}` marker.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GooseMethod::Delete => write!(f, "DELETE"),
            GooseMethod::Get => write!(f, "GET"),
            GooseMethod::Head => write!(f, "HEAD"),
            GooseMethod::Patch => write!(f, "PATCH"),
            GooseMethod::Post => write!(f, "POST"),
            GooseMethod::Put => write!(f, "PUT"),
            GooseMethod::Custom => write!(f, "CUSTOM"),
        }
    }
}

/// Convert [`http::method::Method`](https://docs.rs/http/*/http/method/struct.Method.html)
/// to [`GooseMethod`](./enum.GooseMethod.html).
pub fn goose_method_from_method(method: Method) -> Result<GooseMethod, Box<TransactionError>> {
    Ok(match method {
        Method::DELETE => GooseMethod::Delete,
        Method::GET => GooseMethod::Get,
        Method::HEAD => GooseMethod::Head,
        Method::PATCH => GooseMethod::Patch,
        Method::POST => GooseMethod::Post,
        Method::PUT => GooseMethod::Put,
        _ => {
            return Err(Box::new(TransactionError::InvalidMethod { method }));
        }
    })
}

/// The response to a [`GooseRequestMetric`].
#[derive(Debug)]
pub struct GooseResponse {
    /// The request that this is a response to.
    pub request: GooseRequestMetric,
    /// The response.
    pub response: Result<Response, reqwest::Error>,
}
impl GooseResponse {
    pub fn new(request: GooseRequestMetric, response: Result<Response, reqwest::Error>) -> Self {
        GooseResponse { request, response }
    }
}

/// Object created by [`log_debug()`](struct.GooseUser.html#method.log_debug) and written
/// to log to assist in debugging.
#[derive(Debug, Deserialize, Serialize)]
pub struct GooseDebug {
    /// String to identify the source of the log message.
    pub tag: String,
    /// Optional request made.
    pub request: Option<GooseRequestMetric>,
    /// Optional headers returned by server.
    pub header: Option<String>,
    /// Optional body text returned by server.
    pub body: Option<String>,
}
impl GooseDebug {
    fn new(
        tag: &str,
        request: Option<&GooseRequestMetric>,
        header: Option<&header::HeaderMap>,
        body: Option<&str>,
    ) -> Self {
        GooseDebug {
            // Convert tag from &str to string.
            tag: tag.to_string(),
            // If request is defined, clone it.
            request: request.cloned(),
            // If header is defined, convert it to a string.
            header: header.map(|h| format!("{h:?}")),
            // If header is defined, convert from &str to string.
            body: body.map(|b| b.to_string()),
        }
    }
}

/// Used internally by Coordinated Omission Mitigation, tracks the cadence between when the same request
/// is made as Goose loops through a Scenario.
#[derive(Debug, Clone)]
struct GooseRequestCadence {
    /// The last time this GooseUser lopped through its Transactions.
    last_time: std::time::Instant,
    /// Total milliseconds of delays followed each Transaction. This has to be substracted out as it's
    /// not impacted by the upstream server and it can change randomly affecting the cadence.
    delays_since_last_time: u64,
    /// How many times this GooseUser has looped through all of its Transactions.
    counter: u64,
    /// The minimum time taken to loop through all Transactions.
    minimum_cadence: u64,
    /// The maximum time taken to loop through all Transactions.
    maximum_cadence: u64,
    /// Average amount of time taken to loop through all Transactions.
    average_cadence: u64,
    /// Total amount of time spent processing Transactions.
    total_elapsed: u64,
    /// If non-zero, the length of the server slowdown detected by the Goose Coordinated
    /// Omission Mitigation in milliseconds.
    coordinated_omission_mitigation: u64,
    /// The expected cadence to loop through all Transactions.
    user_cadence: u64,
    /// If -1 coordinated_omission_mitigation was never enabled. Otherwise is a counter of how
    /// many times the mitigation triggered.
    coordinated_omission_counter: isize,
}
impl GooseRequestCadence {
    // Return a new, empty RequestCadence object.
    fn new() -> GooseRequestCadence {
        GooseRequestCadence {
            last_time: std::time::Instant::now(),
            delays_since_last_time: 0,
            counter: 0,
            minimum_cadence: 0,
            maximum_cadence: 0,
            average_cadence: 0,
            total_elapsed: 0,
            coordinated_omission_mitigation: 0,
            user_cadence: 0,
            coordinated_omission_counter: -1,
        }
    }
}

/// A marker trait representing user data of any type
/// ([generic](https://doc.rust-lang.org/rust-by-example/generics.html)) that can
/// be added to any [`GooseUser`](../goose/struct.GooseUser.html). The format of
/// the data stored in `GooseUserData` must be defined in your load test, and by
/// default supports any type that supports
/// [`Send`](https://doc.rust-lang.org/std/marker/trait.Send.html) and
/// [`Sync`](https://doc.rust-lang.org/std/marker/trait.Sync.html).
///
/// Stored in the [`GooseUser`] object in a private `session_data` field. Per-user
/// session data is stored by invoking [`GooseUser::set_session_data`]. The session
/// data can be accessed by invoking [`GooseUser::get_session_data`],
/// [`GooseUser::get_session_data_mut`], [`GooseUser::get_session_data_unchecked`],
/// or [`GooseUser::get_session_data_unchecked_mut`].
///
/// For an example, see
/// [`examples/simple_with_session`](https://github.com/tag1consulting/goose/blob/main/examples/simple_with_session.rs).
pub trait GooseUserData: Downcast + Send + Sync + 'static {
    fn clone_goose_user_data(&self) -> Box<dyn GooseUserData>;
}
impl_downcast!(GooseUserData);

impl<T> GooseUserData for T
where
    T: Clone + Send + Sync + 'static,
{
    fn clone_goose_user_data(&self) -> Box<dyn GooseUserData> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn GooseUserData> {
    fn clone(&self) -> Self {
        (**self).clone_goose_user_data()
    }
}

/// An individual user state, repeatedly running all [`Transaction`](./struct.Transaction.html)s
/// in a specific [`Scenario`](./struct.Scenario.html).
#[derive(Debug)]
pub struct GooseUser {
    /// The Instant the load test started, shared by every `GooseUser` and kept
    /// across a metrics reset; `elapsed` in request, error, transaction and
    /// scenario metrics counts from it. A `GooseUser` built outside a load test
    /// keeps the Instant it was built.
    pub started: Instant,
    /// How many iterations of the scenario this GooseUser has run.
    pub(crate) iterations: usize,
    /// An index into the internal [`GooseAttack`](../struct.GooseAttack.html)`.scenarios`
    /// vector, indicating which [`Scenario`](./struct.Scenario.html) is running.
    pub(crate) scenarios_index: usize,
    /// The Scenario this GooseUser is running.
    pub(crate) scenario_name: Arc<str>,
    /// An optional index into [`Scenario`]`.transaction`, indicating which transaction this is.
    pub(crate) transaction_index: Option<usize>,
    /// Current transaction name, if set.
    pub(crate) transaction_name: Option<TransactionName>,
    /// Client used to make requests, managing sessions and cookies.
    pub client: Client,
    /// The base URL to prepend to all relative paths.
    pub base_url: Url,
    /// A local copy of the global [`GooseConfiguration`](../struct.GooseConfiguration.html).
    pub config: GooseConfiguration,
    /// Channel to logger.
    pub logger: Option<flume::Sender<Option<GooseLog>>>,
    /// Channel to throttle.
    pub throttle: Option<flume::Sender<bool>>,
    /// Normal transactions are optionally throttled,
    /// [`test_start`](../struct.GooseAttack.html#method.test_start) and
    /// [`test_stop`](../struct.GooseAttack.html#method.test_stop) transactions are not.
    pub is_throttled: bool,
    /// Channel for sending metrics to the parent for aggregation.
    pub metrics_channel: Option<flume::Sender<GooseMetric>>,
    /// Channel for notifying the parent when thread shuts down.
    pub shutdown_channel: Option<flume::Sender<usize>>,
    /// An index into the internal [`GooseAttack`](../struct.GooseAttack.html)`.weighted_users`
    /// vector, indicating which weighted `GooseUser` is running.
    pub weighted_users_index: usize,
    /// Load test hash.
    pub load_test_hash: u64,
    /// Tracks the cadence that this user is looping through all Transactions, used by Coordinated
    /// Omission Mitigation.
    request_cadence: GooseRequestCadence,
    /// Tracks how much time is spent sleeping during a loop through all transactions.
    pub(crate) slept: u64,
    /// Buffered request metric awaiting combination with transaction/scenario metrics.
    /// When a scenario iteration completes with exactly one pending request and one
    /// pending transaction, all three are sent as a single [`GooseMetric::All`] message.
    pub(crate) pending_request: Option<GooseRequestMetric>,
    /// Buffered transaction metric awaiting combination with scenario metric.
    pub(crate) pending_transaction: Option<TransactionMetric>,
    /// Pre-aggregated metrics accumulator for batch sending. Reduces channel pressure
    /// from `O(RPS)` to `O(RPS / batch_size)`. `None` when metrics are disabled.
    pub(crate) metrics_batch: Option<GooseMetricBatch>,
    /// Number of request metrics accumulated in the current batch (both
    /// pre-aggregated and individual). Used as the size-based flush trigger;
    /// transactions and scenarios are not counted here because they are
    /// low-frequency and do not drive batch sizing.
    pub(crate) batch_item_count: usize,
    /// When the current batch started accumulating (`None` if batch is empty).
    pub(crate) batch_start: Option<std::time::Instant>,
    /// Whether graph data is collected: `--report-file` is set or `--dashboard` is on
    /// (cached to avoid repeated string checks).
    pub(crate) record_graph_data: bool,
    /// Whether the dashboard's heatmap keeps response times by second
    /// ([`GooseConfiguration::records_latency_series`]): only then does a
    /// batch hold requests from one second and the next. Without it, batches
    /// are built as they always were.
    pub(crate) record_latency_series: bool,
    /// Shared epoch counter for batch validity across metrics resets.
    pub(crate) metrics_epoch: Option<MetricsEpoch>,
    /// Optional per-user session data of a generic type implementing the
    /// [`GooseUserData`] trait.
    session_data: Option<Box<dyn GooseUserData>>,
    /// Shared registry of atomic request counters, keyed by `"METHOD path"`.
    /// User threads lazily insert counters on first use and then increment atomically.
    pub(crate) request_counters: Option<GooseRequestCounterRegistry>,
    /// Local cache of `Arc<GooseRequestCounters>` handles to avoid repeated mutex lookups.
    request_counter_cache: HashMap<String, Arc<GooseRequestCounters>>,
    /// Reusable buffer for formatting counter keys, avoiding per-request heap allocation.
    counter_key_buf: String,
}

impl Clone for GooseUser {
    fn clone(&self) -> Self {
        Self {
            started: self.started,
            iterations: self.iterations,
            scenarios_index: self.scenarios_index,
            scenario_name: self.scenario_name.clone(),
            transaction_index: self.transaction_index,
            transaction_name: self.transaction_name.clone(),
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            config: self.config.clone(),
            logger: self.logger.clone(),
            throttle: self.throttle.clone(),
            is_throttled: self.is_throttled,
            metrics_channel: self.metrics_channel.clone(),
            shutdown_channel: self.shutdown_channel.clone(),
            weighted_users_index: self.weighted_users_index,
            load_test_hash: self.load_test_hash,
            request_cadence: self.request_cadence.clone(),
            slept: self.slept,
            pending_request: self.pending_request.clone(),
            pending_transaction: self.pending_transaction.clone(),
            metrics_batch: self.metrics_batch.as_ref().map(|_| {
                let epoch = self
                    .metrics_epoch
                    .as_ref()
                    .map_or(0, |e| e.load(std::sync::atomic::Ordering::Relaxed));
                GooseMetricBatch::new(epoch)
            }),
            batch_item_count: 0,
            batch_start: None,
            record_graph_data: self.record_graph_data,
            record_latency_series: self.record_latency_series,
            metrics_epoch: self.metrics_epoch.clone(),
            session_data: self.session_data.clone(),
            request_counters: self.request_counters.clone(),
            request_counter_cache: HashMap::new(),
            counter_key_buf: String::new(),
        }
    }
}

impl Debug for dyn GooseUserData {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
        write!(fmt, "GooseUserData")
    }
}

impl GooseUser {
    /// Create a new user state.
    pub fn new(
        scenarios_index: usize,
        scenario_name: Arc<str>,
        base_url: Url,
        configuration: &GooseConfiguration,
        load_test_hash: u64,
        reqwest_client: Option<Client>,
    ) -> Result<Self, GooseError> {
        trace!("new GooseUser");

        let client = match reqwest_client {
            Some(c) => c,
            // Use the new unified client creation function for consistency
            None => {
                let config = crate::client::GooseClientConfig::from(configuration);
                crate::client::create_reqwest_client_with_cookies(&config)?
            }
        };

        let record_graph_data = configuration.records_graph_data();
        let record_latency_series = configuration.records_latency_series();

        Ok(GooseUser {
            started: Instant::now(),
            iterations: 0,
            scenarios_index,
            scenario_name,
            transaction_index: None,
            transaction_name: None,
            client,
            base_url,
            config: configuration.clone(),
            logger: None,
            throttle: None,
            is_throttled: true,
            metrics_channel: None,
            shutdown_channel: None,
            // A value of max_value() indicates this user isn't fully initialized yet.
            weighted_users_index: usize::MAX,
            load_test_hash,
            request_cadence: GooseRequestCadence::new(),
            slept: 0,
            pending_request: None,
            pending_transaction: None,
            metrics_batch: None,
            batch_item_count: 0,
            batch_start: None,
            record_graph_data,
            record_latency_series,
            metrics_epoch: None,
            session_data: None,
            request_counters: None,
            request_counter_cache: HashMap::new(),
            counter_key_buf: String::new(),
        })
    }

    /// Create a new single-use user.
    pub fn single(base_url: Url, configuration: &GooseConfiguration) -> Result<Self, GooseError> {
        let mut single_user = GooseUser::new(0, Arc::from(""), base_url, configuration, 0, None)?;
        // Only one user, so index is 0.
        single_user.weighted_users_index = 0;
        // Do not throttle [`test_start`](../struct.GooseAttack.html#method.test_start) (setup) and
        // [`test_stop`](../struct.GooseAttack.html#method.test_stop) (teardown) transactions.
        single_user.is_throttled = false;

        Ok(single_user)
    }

    /// Returns the number of iterations this GooseUser has run through it's
    /// assigned [`Scenario`].
    pub fn get_iterations(&self) -> usize {
        self.iterations
    }

    /// Returns an optional reference to per-[`GooseUser`] session data.
    ///
    /// Leaves the session data in-place, returning an optional reference to the
    /// original session data if existing and of the correct type. Returns [`None`]
    /// if no session data has been set or the session data set is not of type `T`.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct Foo(String);
    ///
    /// let mut transaction = transaction!(get_session_data_function);
    ///
    /// /// A very simple transaction that makes a GET request.
    /// async fn get_session_data_function(user: &mut GooseUser) -> TransactionResult {
    ///     let foo = user.get_session_data::<Foo>().expect("Missing session data!");
    ///     println!("Session data: {}", foo.0);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn get_session_data<T: GooseUserData>(&self) -> Option<&T> {
        match &self.session_data {
            Some(data) => data.downcast_ref::<T>(),
            None => None,
        }
    }

    /// Returns a reference to per-[`GooseUser`] session data, without doing any
    /// validation that the session data exists and is of the correct type.
    ///
    /// Leaves the session data in-place, returning a reference to the original
    /// session data. Calling this method on a [`GooseUser`] object without
    /// session data or with a different type `T` will panic.
    ///
    /// For a safe alternative see [`GooseUser::get_session_data`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct Foo(String);
    ///
    /// let mut transaction = transaction!(get_session_data_unchecked_function);
    ///
    /// /// A very simple transaction that makes a GET request.
    /// async fn get_session_data_unchecked_function(user: &mut GooseUser) -> TransactionResult {
    ///     let foo = user.get_session_data_unchecked::<Foo>();
    ///     println!("Session data: {}", foo.0);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn get_session_data_unchecked<T: GooseUserData>(&self) -> &T {
        let session_data = self.session_data.as_deref().expect("Missing session data!");

        session_data
            .downcast_ref::<T>()
            .expect("Invalid session data!")
    }

    /// Returns an optional mutable reference to per-[`GooseUser`] session data.
    ///
    /// Leaves the session data in-place, returning an optional mutable reference
    /// to the original session data if existing and of the correct type. Returns
    /// [`None`] if no session data has been set or the session data set is not of
    /// type `T`.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct Foo(String);
    ///
    /// let mut transaction = transaction!(get_session_data_mut_function);
    ///
    /// /// A very simple transaction that makes a GET request.
    /// async fn get_session_data_mut_function(user: &mut GooseUser) -> TransactionResult {
    ///     let foo = user.get_session_data_mut::<Foo>().expect("Missing session data!");
    ///     foo.0 = "Bar".to_owned();
    ///     Ok(())
    /// }
    /// ```
    pub fn get_session_data_mut<T: GooseUserData>(&mut self) -> Option<&mut T> {
        match &mut self.session_data {
            Some(data) => data.downcast_mut::<T>(),
            None => None,
        }
    }

    /// Returns a mutable reference to per-[`GooseUser`] session data, without
    /// doing any validation that the session data exists and is of the correct
    /// type.
    ///
    /// Leaves the session data in-place, returning a mutable reference to the
    /// original session data. Calling this method on a [`GooseUser`] object
    /// without session data or with a different type `T` will panic.
    ///
    /// For a safe alternative see [`GooseUser::get_session_data_mut`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct Foo(String);
    ///
    /// let mut transaction = transaction!(get_session_data_unchecked_mut_function);
    ///
    /// /// A very simple transaction that makes a GET request.
    /// async fn get_session_data_unchecked_mut_function(user: &mut GooseUser) -> TransactionResult {
    ///     let foo = user.get_session_data_unchecked_mut::<Foo>();
    ///     foo.0 = "Bar".to_owned();
    ///     Ok(())
    /// }
    /// ```
    pub fn get_session_data_unchecked_mut<T: GooseUserData>(&mut self) -> &mut T {
        let session_data = self
            .session_data
            .as_deref_mut()
            .expect("Missing session data!");
        session_data
            .downcast_mut::<T>()
            .expect("Invalid session data!")
    }

    /// Sets session data for the current [`GooseUser`].
    ///
    /// If session data already exists for the current [`GooseUser`], it will be
    /// replaced. Session data must be of a type implementing the
    /// [`GooseUserData`] trait.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct Foo(String);
    ///
    /// let mut transaction = transaction!(set_session_data_function);
    ///
    /// /// A very simple transaction that makes a GET request.
    /// async fn set_session_data_function(user: &mut GooseUser) -> TransactionResult {
    ///     user.set_session_data(Foo("Foo".to_string()));
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_session_data<T: GooseUserData>(&mut self, data: T) {
        self.session_data.replace(Box::new(data));
    }

    /// A helper that prepends a `base_url` to all relative paths.
    ///
    /// A `base_url` is determined per user thread, using the following order
    /// of precedence:
    ///  1. `--host` (host specified on the command line when running load test)
    ///  2. [`Scenario`](./struct.Scenario.html)`.host` (default host defined for the
    ///     current scenario)
    ///  3. [`GooseDefault::Host`](../config/enum.GooseDefault.html#variant.Host) (default host
    ///     defined for the current load test)
    pub fn build_url(&self, path: &str) -> Result<String, Box<TransactionError>> {
        // If URL includes a host, simply use it.
        if let Ok(parsed_path) = Url::parse(path) {
            if let Some(_host) = parsed_path.host() {
                return Ok(path.to_string());
            }
        }

        // Otherwise use the `base_url`.
        match self.base_url.join(path) {
            Ok(u) => Ok(u.to_string()),
            Err(e) => Err(Box::new(e.into())),
        }
    }

    /// A helper to make a `GET` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `get()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// GET a URL.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(get_function);
    ///
    /// /// A very simple transaction that makes a GET request.
    /// async fn get_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("path/to/foo/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn get(&mut self, path: &str) -> Result<GooseResponse, Box<TransactionError>> {
        // GET path.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Get)
            .path(path)
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a named `GET` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `get_named()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// GET a URL and name the request in collected metrics.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(get_function);
    ///
    /// /// A very simple transaction that makes a GET request, naming it for metrics.
    /// async fn get_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get_named("path/to/foo/", "foo").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_named(
        &mut self,
        path: &str,
        name: &str,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // GET path named.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Get)
            .path(path)
            .name(name)
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a `POST` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `post()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// POST an arbitrary body.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(post_function);
    ///
    /// /// A very simple transaction that makes a POST request.
    /// async fn post_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.post("path/to/foo/", "BODY BEING POSTED").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post<T: Into<reqwest::Body>>(
        &mut self,
        path: &str,
        body: T,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // Build a Reqwest RequestBuilder object.
        let url = self.build_url(path)?;
        let reqwest_request_builder = self.client.post(url);

        // POST request.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Post)
            .path(path)
            .set_request_builder(reqwest_request_builder.body(body))
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a `POST` request of a form on a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `post_form()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// POST a form.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(post_function);
    ///
    /// /// A very simple transaction that POSTs form parameters.
    /// async fn post_function(user: &mut GooseUser) -> TransactionResult {
    ///     let params = [("foo", "bar"), ("foo2", "bar2")];
    ///     let _goose = user.post_form("path/to/foo/", &params).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post_form<T: Serialize + ?Sized>(
        &mut self,
        path: &str,
        form: &T,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // Build a Reqwest RequestBuilder object.
        let url = self.build_url(path)?;
        let reqwest_request_builder = self.client.post(url);

        // POST form request.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Post)
            .path(path)
            .set_request_builder(reqwest_request_builder.form(&form))
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a `POST` request of json on a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `post_json()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// POST an arbitrary JSON object.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(post_function);
    ///
    /// /// A very simple transaction that POSTs an arbitrary json object.
    /// async fn post_function(user: &mut GooseUser) -> TransactionResult {
    ///     let json = &serde_json::json!({
    ///         "foo": "bar",
    ///         "foo2": "bar2"
    ///     });
    ///     let _goose = user.post_json("path/to/foo/", &json).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post_json<T: Serialize + ?Sized>(
        &mut self,
        path: &str,
        json: &T,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // Build a Reqwest RequestBuilder object.
        let url = self.build_url(path)?;
        let reqwest_request_builder = self.client.post(url);

        // POST json request.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Post)
            .path(path)
            .set_request_builder(reqwest_request_builder.json(&json))
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a GraphQL request and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `post_graphql()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// Make a GraphQL query.
    /// ```rust
    /// use goose::prelude::*;
    /// use serde_json::json;
    ///
    /// let mut transaction = transaction!(graphql_function);
    ///
    /// /// A simple transaction that makes a GraphQL query.
    /// async fn graphql_function(user: &mut GooseUser) -> TransactionResult {
    ///     let query = json!({
    ///         "query": "{ user(id: 1) { id name email } }"
    ///     });
    ///     let _goose = user.post_graphql("/graphql", &query).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post_graphql<T: Serialize + ?Sized>(
        &mut self,
        path: &str,
        query: &T,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // Build a Reqwest RequestBuilder object.
        let url = self.build_url(path)?;
        let reqwest_request_builder = self.client.post(url);

        // POST GraphQL request.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Post)
            .path(path)
            .set_request_builder(reqwest_request_builder.json(&query))
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a named GraphQL request and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `post_graphql_named()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// Make a named GraphQL query for better metrics tracking.
    /// ```rust
    /// use goose::prelude::*;
    /// use serde_json::json;
    ///
    /// let mut transaction = transaction!(graphql_function);
    ///
    /// /// A simple transaction that makes a named GraphQL query.
    /// async fn graphql_function(user: &mut GooseUser) -> TransactionResult {
    ///     let query = json!({
    ///         "query": "{ user(id: 1) { id name email } }"
    ///     });
    ///     let _goose = user.post_graphql_named("/graphql", &query, "get user").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn post_graphql_named<T: Serialize + ?Sized>(
        &mut self,
        path: &str,
        query: &T,
        name: &str,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // Build a Reqwest RequestBuilder object.
        let url = self.build_url(path)?;
        let reqwest_request_builder = self.client.post(url);

        // POST GraphQL request with custom name.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Post)
            .path(path)
            .name(name)
            .set_request_builder(reqwest_request_builder.json(&query))
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a `HEAD` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `head()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// Make a HEAD request.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(head_function);
    ///
    /// /// A very simple transaction that makes a HEAD request.
    /// async fn head_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.head("path/to/foo/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn head(&mut self, path: &str) -> Result<GooseResponse, Box<TransactionError>> {
        // HEAD request.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Head)
            .path(path)
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// A helper to make a `DELETE` request of a path and collect relevant metrics.
    /// Automatically prepends the correct host.
    ///
    /// Calls to `delete()` return a [`GooseResponse`](./struct.GooseResponse.html) object which
    /// contains a copy of the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)),
    /// and the response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// If you need to set headers, change timeouts, or otherwise make use of the
    /// [`reqwest::RequestBuilder`](https://docs.rs/reqwest/*/reqwest/struct.RequestBuilder.html)
    /// object, refer to [`GooseUser::get_request_builder`].
    ///
    /// # Example
    /// Make a DELETE request.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(delete_function);
    ///
    /// /// A very simple transaction that makes a DELETE request.
    /// async fn delete_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.delete("path/to/foo/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete(&mut self, path: &str) -> Result<GooseResponse, Box<TransactionError>> {
        // DELETE request.
        let goose_request = GooseRequest::builder()
            .method(GooseMethod::Delete)
            .path(path)
            .build();

        // Make the request and return the GooseResponse.
        self.request(goose_request).await
    }

    /// Used to get a [`reqwest::RequestBuilder`] object. If no [`reqwest::RequestBuilder`] is
    /// already defined in the [`GooseRequest`] passed to [`GooseUser::request`] it will automatically
    /// invoke this function.
    ///
    /// The HTTP request method must be defined as a [`GooseMethod`], and the path that will be requested
    /// must be defined as a [`&str`].
    ///
    /// It is possible to use this function to directly interact with the [`reqwest::RequestBuilder`]
    /// object and the [`GooseRequest`] object during load tests. In the following example, we set a
    /// timeout on the Request, and tell Goose to expect a 404 HTTP response status code.
    ///
    /// # Example
    /// Request a non-existent page, timing out after 500 milliseconds.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(test_404);
    ///
    /// async fn test_404(user: &mut GooseUser) -> TransactionResult {
    ///     use std::time::Duration;
    ///
    ///     // Manually interact with the Reqwest RequestBuilder object.
    ///     let request_builder = user.get_request_builder(&GooseMethod::Get, "no/such/path")?
    ///         // Configure the request to timeout if it takes longer than 500 milliseconds.
    ///         .timeout(Duration::from_millis(500));
    ///
    ///     // Manually build a GooseRequest.
    ///     let goose_request = GooseRequest::builder()
    ///         // Manually add our custom RequestBuilder object.
    ///         .set_request_builder(request_builder)
    ///         // Tell Goose to expect a 404 status code.
    ///         .expect_status_code(404)
    ///         // Turn the GooseRequestBuilder object into a GooseRequest.
    ///         .build();
    ///
    ///     // Finally make the actual request with our custom GooseRequest object.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// async fn validate_response(user: &mut GooseUser) -> TransactionResult {
    ///     let response = user.get("/api/data").await?;
    ///     let response_data = response.response?;
    ///     let text = response_data.text().await?;
    ///
    ///     if !text.contains("expected_content") {
    ///         return Err("Missing expected content in response".into());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// async fn business_logic_check(user: &mut GooseUser) -> TransactionResult {
    ///     let response = user.post("/login", "").await?;
    ///     let response_data = response.response?;
    ///
    ///     if !response_data.status().is_success() {
    ///         return Err(format!("Login failed with status: {}", response_data.status()).into());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn get_request_builder(
        &self,
        method: &GooseMethod,
        path: &str,
    ) -> Result<RequestBuilder, Box<TransactionError>> {
        // Prepend the `base_url` to all relative paths.
        let url = self.build_url(path)?;

        // Invoke appropriate Reqwest convenience function to generate an
        // appropriate RequestBuilder.
        Ok(match method {
            GooseMethod::Delete => self.client.delete(&url),
            GooseMethod::Get => self.client.get(&url),
            GooseMethod::Head => self.client.head(&url),
            GooseMethod::Patch => self.client.patch(&url),
            GooseMethod::Post => self.client.post(&url),
            GooseMethod::Put => self.client.put(&url),
            GooseMethod::Custom => {
                return Err(Box::new(TransactionError::Custom(
                    "GooseMethod::Custom cannot be used with HTTP request builders; \
                     use record_custom_request() instead"
                        .to_string(),
                )))
            }
        })
    }

    /// Makes a request for the provided [`GooseRequest`] object, and if metrics are enabled
    /// captures relevant metrics.
    ///
    /// Calls to `request()` return a [`Result`] containing a [`GooseResponse`] on success, and a
    /// [`flume::SendError`](https://docs.rs/flume/*/flume/struct.SendError.html)`<bool>`,
    /// on failure. Failure only happens when `--throttle-requests` is enabled and the load test
    /// completes. The [`GooseResponse`](./struct.GooseResponse.html) object contains a copy of
    /// the request you made ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)), and the
    /// response ([`reqwest::Response`](https://docs.rs/reqwest/*/reqwest/struct.Response.html)).
    ///
    /// # Example
    /// Make a GET request.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(get_function);
    ///
    /// /// A simple transaction that makes a GET request.
    /// async fn get_function(user: &mut GooseUser) -> TransactionResult {
    ///     let goose_request = GooseRequest::builder()
    ///       // Goose will prepend a host name to this path.
    ///       .path("path/to/loadtest")
    ///       // GET is the default method, this is not necessary.
    ///       .method(GooseMethod::Get)
    ///       // Assemble the `GooseRequestBuilder` into a `GooseRequest.
    ///       .build();
    ///     let goose = user.request(goose_request).await?;
    ///
    ///     // Do stuff with goose.request and/or goose.response here.
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn request(
        &mut self,
        mut request: GooseRequest<'_>,
    ) -> Result<GooseResponse, Box<TransactionError>> {
        // If the RequestBuilder is already defined in the GooseRequest use it.
        let request_builder = if request.request_builder.is_some() {
            request.request_builder.take().unwrap()
        // Otherwise get a new RequestBuilder.
        } else {
            self.get_request_builder(&request.method, request.path)?
        };

        // Determine the name for this request.
        let request_name = self.get_request_name(&request);

        // If throttle-requests is enabled...
        if self.is_throttled && self.throttle.is_some() {
            // ...wait until there's room to add a token to the throttle channel before proceeding.
            debug!(
                "[user {}]: waiting on throttle",
                self.weighted_users_index + 1
            );
            // Will result in TransactionError::RequestCanceled if this fails.
            if let Err(e) = self.throttle.clone().unwrap().send_async(true).await {
                return Err(Box::new(e.into()));
            }
        };

        // Once past the throttle, the request is officially started.
        let started = Instant::now();

        // Create a Reqwest Request object from the RequestBuilder.
        let built_request = match request_builder.build() {
            Ok(r) => r,
            Err(e) => return Err(Box::new(e.into())),
        };

        // Grab a copy of any headers set by this request, included in the request log,
        // the debug log and the error log. Skip the formatting when none of those logs
        // is enabled, as nothing else reads them.
        let headers: Vec<String> = if self.config.request_log().is_some()
            || self.config.debug_log().is_some()
            || self.config.error_log().is_some()
        {
            let mut headers = Vec::with_capacity(built_request.headers().len());
            for header in built_request.headers() {
                headers.push(format!("{header:?}"));
            }
            headers
        } else {
            Vec::new()
        };

        // If enabled, grab a copy of the request body, included in the request log and
        // the debug log.
        let body = if self.config.request_body {
            // Get a bytes representation of the body, if any.
            let body_bytes = match built_request.body() {
                Some(b) => b.as_bytes().unwrap_or(b""),
                None => b"",
            };
            // Convert the bytes into a &str if valid utf8.
            str::from_utf8(body_bytes).unwrap_or("")
        } else {
            ""
        };

        // Record the complete client request, included in the request log and the debug log.
        let raw_request = GooseRawRequest::new(
            goose_method_from_method(built_request.method().clone())?,
            built_request.url().as_str(),
            headers,
            body,
        );

        // Provide details about the current transaction for logging.
        let transaction_detail = TransactionDetail {
            scenario_index: self.scenarios_index,
            scenario_name: self.scenario_name.clone(),
            transaction_index: self.transaction_index,
            transaction_name: self
                .transaction_name
                .clone()
                .unwrap_or(TransactionName::default_value()),
        };

        // Record information about the request.
        let mut request_metric = GooseRequestMetric::new(
            raw_request,
            transaction_detail,
            request_name,
            self.started.elapsed().as_millis(),
            self.weighted_users_index,
        );

        // Make the actual request.
        let response = self.client.execute(built_request).await;
        // Record Time to First Byte (TTFB): client.execute() returns when response headers
        // are received, not when the complete response body is downloaded.
        request_metric.set_response_duration(started.elapsed());

        // Determine if the request suceeded or failed.
        match &response {
            Ok(r) => {
                let status_code = r.status();
                debug!(
                    "{:?}: status_code {}",
                    request_log_path(&request_metric.raw.url),
                    status_code
                );

                // Update the request_metric object.
                request_metric.set_status_code(Some(status_code));
                request_metric.set_final_url(r.url().as_str());

                // Check if we were expecting a specific status code.
                if let Some(expect_status_code) = request.expect_status_code {
                    // Record a failure if the expected status code was not returned.
                    if status_code != expect_status_code {
                        request_metric.success = false;
                        request_metric.error = format!("{status_code}: {}", request_metric.name);
                    }
                // Otherwise record a failure if the returned status code was not a success.
                } else if !status_code.is_success() {
                    request_metric.success = false;
                    request_metric.error = format!("{status_code}: {}", request_metric.name);
                }

                // Load test user was redirected.
                if self.config.sticky_follow && request_metric.raw.url != request_metric.final_url {
                    let base_url = self.base_url.to_string();
                    // Check if the URL redirected started with the load test base_url.
                    if !request_metric.final_url.starts_with(&base_url) {
                        let redirected_url = match Url::parse(&request_metric.final_url) {
                            Ok(u) => u,
                            Err(e) => return Err(Box::new(e.into())),
                        };
                        let redirected_base_url =
                            redirected_url[..url::Position::BeforePath].to_string();
                        info!(
                            "base_url for user {} redirected from {} to {}",
                            self.weighted_users_index + 1,
                            base_url,
                            redirected_base_url
                        );
                        let _ = self.set_base_url(&redirected_base_url);
                    }
                }
            }
            Err(e) => {
                // @TODO: what can we learn from a reqwest error?
                warn!("{:?}: {}", request_log_path(&request_metric.raw.url), e);
                request_metric.success = false;
                request_metric.set_status_code(None);
                request_metric.error = clean_reqwest_error(e, &request_metric.name);
            }
        };

        // If enabled, track the cadence between each time the same request is made while
        // this GooseUser is running. If requests are blocked by the upstream server, this
        // allows Goose to backfill the requests that should have been made based on
        // cadence statistics.
        request_metric.user_cadence = self
            .coordinated_omission_mitigation(&request_metric)
            .await?;

        // Send a copy of the raw request object to the parent process if
        // we're tracking metrics.
        if !self.config.no_metrics {
            self.send_request_metric_to_parent(request_metric.clone())?;
            // Atomic counter increment happens at actual send time (in
            // flush_pending_request or record_scenario's GooseMetric::All path),
            // not here at buffer time, so that metrics reset can't zero the counter
            // before the parent processes the buffered request.
        }

        if request.error_on_fail && !request_metric.success {
            error!(
                "{:?} {}",
                request_log_path(&request_metric.raw.url),
                request_metric.error
            );
            return Err(Box::new(TransactionError::RequestFailed {
                raw_request: Box::new(request_metric),
            }));
        }

        Ok(GooseResponse::new(request_metric, response))
    }

    /// Tracks the time it takes for the current GooseUser to loop through all Transactions
    /// if Coordinated Omission Mitigation is enabled.
    pub(crate) async fn update_request_cadence(&mut self, thread_number: usize) {
        if let Some(co_mitigation) = self.config.co_mitigation.as_ref() {
            // Return immediately if coordinated omission mitigation is disabled.
            if co_mitigation == &GooseCoordinatedOmissionMitigation::Disabled {
                return;
            }

            // Grab the current timestamp to calculate the difference since the last
            // time through the loop.
            let now = std::time::Instant::now();

            // Swap out the `slept` counter, which is the total time the GooseUser slept
            // between transactions, a potentially randomly changing value. Reset to 0 for the
            // next loop through all Transactions.
            self.request_cadence.delays_since_last_time = self.slept;
            self.slept = 0;

            // How much time passed since the last time this GooseUser looped through all
            // transactions, accounting for time waiting between Transactions due to `set_wait_time`.
            let elapsed = (now - self.request_cadence.last_time).as_millis() as u64
                - self.request_cadence.delays_since_last_time;

            // Update `minimum_cadence` if this was the fastest seen.
            if elapsed < self.request_cadence.minimum_cadence
                || self.request_cadence.minimum_cadence == 0
            {
                self.request_cadence.minimum_cadence = elapsed;
            // Update `maximum_cadence` if this was the slowest seen.
            } else if elapsed > self.request_cadence.maximum_cadence {
                self.request_cadence.maximum_cadence = elapsed;
            }

            // Update request_cadence metrics based on the timing of the current request.
            self.request_cadence.counter += 1;
            self.request_cadence.total_elapsed += elapsed;
            self.request_cadence.last_time = now;
            self.request_cadence.average_cadence =
                self.request_cadence.total_elapsed / self.request_cadence.counter;

            if self.request_cadence.counter > 3 {
                if self.request_cadence.coordinated_omission_counter < 0 {
                    debug!("user {thread_number} enabled coordinated omission mitigation");
                    self.request_cadence.coordinated_omission_counter += 1;
                }
                // Calculate the expected cadence for this Transaction request.
                let cadence = match co_mitigation {
                    // Expected cadence is the average time between requests.
                    GooseCoordinatedOmissionMitigation::Average => {
                        self.request_cadence.average_cadence
                    }
                    // Expected cadence is the maximum time between requests.
                    GooseCoordinatedOmissionMitigation::Maximum => {
                        self.request_cadence.maximum_cadence
                    }
                    // Expected cadence is the minimum time between requests.
                    GooseCoordinatedOmissionMitigation::Minimum => {
                        self.request_cadence.minimum_cadence
                    }
                    // This is not possible as we would have exited already if coordinated
                    // omission mitigation was disabled.
                    GooseCoordinatedOmissionMitigation::Disabled => unreachable!(),
                };
                if elapsed > (cadence * 2) {
                    debug!(
                        "user {thread_number}: coordinated_omission_mitigation: elapsed({elapsed}) > cadence({cadence})"
                    );
                    self.request_cadence.coordinated_omission_counter += 1;
                    self.request_cadence.coordinated_omission_mitigation = elapsed;
                } else {
                    self.request_cadence.coordinated_omission_mitigation = 0;
                }
                // Always track the expected cadence.
                self.request_cadence.user_cadence = cadence;
            }
        }
        // Without a setting, as for a user made outside a load test, there
        // is no mitigation: a load test always sets one, Average by default.
    }

    /// If Coordinated Omission Mitigation is enabled, compares how long has passed since the last
    /// loop through all Transactions by the current GooseUser. Through this mechanism, Goose is
    /// able to detect stalls on the upstream server being load tested, backfilling requests based
    /// on what statistically should have happened. Can be disabled with `--co-mitigation disabled`.
    async fn coordinated_omission_mitigation(
        &mut self,
        request_metric: &GooseRequestMetric,
    ) -> Result<u64, Box<TransactionError>> {
        if let Some(co_mitigation) = self.config.co_mitigation.as_ref() {
            // Return immediately if coordinated omission mitigation is disabled.
            if co_mitigation == &GooseCoordinatedOmissionMitigation::Disabled {
                return Ok(0);
            }

            // Generate an info level alert if this specific request took longer than the normal
            // cadence, as that means this specific request will likely trigger Coordinated
            // Omission Mitigation.
            if self.request_cadence.counter > 3
                && request_metric.response_time > self.request_cadence.user_cadence
            {
                let transaction_name = if let Some(transaction_name) = &self.transaction_name {
                    format!(
                        ", transaction name: \"{}\"",
                        transaction_name.name_for_transaction()
                    )
                } else {
                    "".to_string()
                };
                info!(
                    "{:.3}s into goose attack: \"{} {}\" [{}] took abnormally long ({} ms){}",
                    request_metric.elapsed as f64 / 1_000.0,
                    request_metric.raw.method_label(),
                    request_metric.raw.url,
                    request_metric.status_code,
                    request_metric.response_time,
                    transaction_name,
                );
            }

            // Check if Coordinated Omission Mitigation has been triggered.
            if self.request_cadence.coordinated_omission_mitigation > 0 {
                // Base our coordinated omission generated request metric on the actual
                // metric that triggered this logic.
                let mut coordinated_omission_request_metric = request_metric.clone();
                // Record data points specific to coordinated_omission.
                coordinated_omission_request_metric.coordinated_omission_elapsed =
                    self.request_cadence.coordinated_omission_mitigation;
                // Record data points specific to coordinated_omission.
                coordinated_omission_request_metric.user_cadence =
                    self.request_cadence.user_cadence;
                // Send the coordinated omission mitigation generated metrics to the parent.
                self.send_request_metric_to_parent(coordinated_omission_request_metric)?;
            }
            Ok(self.request_cadence.user_cadence)
        } else {
            // Without a setting, as for a user made outside a load test, there
            // is no mitigation: a load test always sets one, Average by default.
            Ok(0)
        }
    }

    /// Send a request metric immediately on the metrics channel (not buffered).
    fn send_request_metric_now(&self, request_metric: GooseRequestMetric) -> TransactionResult {
        if let Some(metrics_channel) = self.metrics_channel.clone() {
            if let Err(e) = metrics_channel.send(GooseMetric::Request(Box::new(request_metric))) {
                return Err(Box::new(e.into()));
            }
        }
        Ok(())
    }

    /// Accumulate any pending request metric into the batch (or send directly
    /// if batching is not active).
    pub(crate) fn flush_pending_request(&mut self) -> TransactionResult {
        if let Some(request) = self.pending_request.take() {
            // Save fields needed for atomic counter before moving request.
            let method = request.raw.method;
            let name = request.name.clone();
            let success = request.success;

            if self.metrics_batch.is_some() {
                // Discard stale batch if a metrics reset changed the epoch.
                self.ensure_batch_current();
                // Batch path: accumulate instead of sending individually.
                if request.coordinated_omission_elapsed > 0 || !request.error.is_empty() {
                    // CO-affected and failed requests need per-request processing.
                    self.accumulate_individual_request(request);
                } else if self.record_latency_series {
                    self.accumulate_request_by_second(&request)?;
                } else {
                    self.accumulate_request(&request);
                }
                if self.batch_start.is_none() {
                    self.batch_start = Some(std::time::Instant::now());
                }
                self.maybe_flush_batch()?;
            } else {
                // Non-batch path: send directly (e.g., during test_start/test_stop).
                self.send_request_metric_now(request)?;
            }
            // Increment at actual send/accumulate time so metrics reset can't zero
            // the counter before the parent processes this request.
            self.increment_request_counter(&method, &name, success);
        }
        Ok(())
    }

    /// Accumulate any pending transaction metric into the batch (or send
    /// directly if batching is not active).
    pub(crate) fn flush_pending_transaction(&mut self) {
        if let Some(transaction) = self.pending_transaction.take() {
            if self.metrics_batch.is_some() {
                self.ensure_batch_current();
                self.accumulate_transaction(&transaction);
            } else if let Some(metrics_channel) = self.metrics_channel.clone() {
                let _ = metrics_channel.send(GooseMetric::Transaction(transaction));
            }
        }
    }

    /// Flush all pending metrics (request and transaction) individually.
    pub(crate) fn flush_pending_metrics(&mut self) -> TransactionResult {
        self.flush_pending_request()?;
        self.flush_pending_transaction();
        Ok(())
    }

    fn send_request_metric_to_parent(
        &mut self,
        request_metric: GooseRequestMetric,
    ) -> TransactionResult {
        // If requests-file is enabled, send a copy of the raw request to the logger thread.
        if self.config.request_log().is_some() {
            if let Some(logger) = self.logger.as_ref() {
                if let Err(e) = logger.send(Some(GooseLog::Request(request_metric.clone()))) {
                    return Err(Box::new(e.into()));
                }
            }
        }

        // If there's already a pending request, flush it before buffering the new one.
        self.flush_pending_request()?;

        // Buffer this request metric instead of sending immediately. It will be
        // combined with the transaction and scenario metrics into a single
        // GooseMetric::All message when the scenario iteration completes.
        if self.metrics_channel.is_some() {
            self.pending_request = Some(request_metric);
        }

        Ok(())
    }

    /// Atomically increment the shared request counter for the given method/name.
    ///
    /// Uses the per-user cache to avoid repeated mutex lookups on the shared registry.
    /// Formats the key into a reusable buffer to avoid per-request heap allocation.
    pub(crate) fn increment_request_counter(
        &mut self,
        method: &GooseMethod,
        name: &str,
        success: bool,
    ) {
        if let Some(registry) = &self.request_counters {
            self.counter_key_buf.clear();
            let _ = write!(self.counter_key_buf, "{} {}", method, name);
            // Fast path: check the local cache with a borrowed key (no allocation).
            if !self
                .request_counter_cache
                .contains_key(self.counter_key_buf.as_str())
            {
                let mut map = registry
                    .lock()
                    .expect("request_counter_registry mutex poisoned");
                let arc = map
                    .entry(self.counter_key_buf.clone())
                    .or_insert_with(|| Arc::new(GooseRequestCounters::new()))
                    .clone();
                self.request_counter_cache
                    .insert(self.counter_key_buf.clone(), arc);
            }
            let counter = self
                .request_counter_cache
                .get(self.counter_key_buf.as_str())
                .unwrap();
            if success {
                counter.success.fetch_add(1, AtomicOrdering::Relaxed);
            } else {
                counter.failure.fetch_add(1, AtomicOrdering::Relaxed);
            }
        }
    }

    /// Atomically update the shared request counter for a retroactive correction
    /// from `set_success()` or `set_failure()`.
    ///
    /// Takes `&self` (no cache mutation) so it can be called from the non-mut
    /// `set_success()` / `set_failure()` methods. These update calls are rare,
    /// so the extra mutex lookup and key allocation are acceptable.
    fn update_request_counter(&self, method: &GooseMethod, name: &str, success: bool) {
        if let Some(registry) = &self.request_counters {
            let key = format!("{} {}", method, name);
            let map = registry
                .lock()
                .expect("request_counter_registry mutex poisoned");
            if let Some(counter) = map.get(&key) {
                if success {
                    // Was failure, now success: increment success, saturating-decrement failure.
                    counter.success.fetch_add(1, AtomicOrdering::Relaxed);
                    saturating_decrement(&counter.failure);
                } else {
                    // Was success, now failure: saturating-decrement success, increment failure.
                    saturating_decrement(&counter.success);
                    counter.failure.fetch_add(1, AtomicOrdering::Relaxed);
                }
            }
        }
    }

    /// Accumulate a successful, non-CO request metric into the local batch.
    ///
    /// Pre-aggregates timing data, status codes, and status-code timings into
    /// compact batch entries. Failed and CO-affected requests are handled by
    /// [`Self::accumulate_individual_request`] instead.
    ///
    /// # Invariants
    ///
    /// This method is only called for requests with an empty error and no CO
    /// elapsed time (`flush_pending_request` routes all other requests to
    /// `accumulate_individual_request`). Consequently, `request_metric.success`
    /// is always `true` and `request_metric.error` is always empty here.
    fn accumulate_request(&mut self, request_metric: &GooseRequestMetric) {
        debug_assert!(
            request_metric.success,
            "accumulate_request called with unsuccessful request"
        );
        debug_assert!(
            request_metric.error.is_empty(),
            "accumulate_request called with non-empty error"
        );

        let batch = match self.metrics_batch.as_mut() {
            Some(b) => b,
            None => return,
        };

        let method_label = request_metric.raw.method_label();
        // Format key into reusable buffer to avoid per-request allocation.
        self.counter_key_buf.clear();
        let _ = std::fmt::Write::write_fmt(
            &mut self.counter_key_buf,
            format_args!("{} {}", method_label, request_metric.name),
        );

        let entry = batch
            .requests
            .entry(self.counter_key_buf.clone())
            .or_insert_with(|| RequestBatchEntry {
                path: request_metric.name.to_string(),
                method: request_metric.raw.method,
                custom_method: request_metric.raw.custom_method.clone(),
                raw_data: GooseRequestMetricTimingData::new(None),
                status_code_counts: HashMap::new(),
                status_code_timings: HashMap::new(),
            });

        // Only non-update requests reach accumulate_request; updates go through
        // set_success/set_failure → send_request_metric_now directly.
        debug_assert!(
            !request_metric.update,
            "accumulate_request called with update request"
        );

        // Pre-aggregate timing data using the same bucketing logic.
        let us = request_metric.response_time_micros();
        entry.raw_data.record_time_us_unsorted(us);

        if !self.config.no_status_codes {
            // Increment status-code count.
            *entry
                .status_code_counts
                .entry(request_metric.status_code)
                .or_insert(0) += 1;

            // Record status-code timing summary.
            let summary = entry
                .status_code_timings
                .entry(request_metric.status_code)
                .or_insert_with(StatusCodeTimingSummary::default);
            summary.record_us(us);
        }

        // Track graph data if a report file is configured or the dashboard is on.
        if self.record_graph_data {
            let second = (request_metric.elapsed / 1000) as usize;
            let graph_entry = batch
                .graph_request_data
                .entry((self.counter_key_buf.clone(), second))
                .or_default();
            graph_entry.count += 1;
            graph_entry.total_time += us as f64 / 1000.0;
        }

        self.batch_item_count += 1;
    }

    /// [`Self::accumulate_request`] when the dashboard's heatmap keeps
    /// response times by second: the request's time goes in the batch's
    /// `raw_data` when it started in the batch's second, or in its
    /// `next_second` when it started in the second after, after
    /// [`Self::flush_batch_outside_seconds`] has sent a batch it can not
    /// join. The batch records no `graph_request_data`: the metrics thread
    /// counts requests per second from the batch's second. Kept out of line,
    /// so the path without the heatmap stays as it was.
    #[inline(never)]
    fn accumulate_request_by_second(
        &mut self,
        request_metric: &GooseRequestMetric,
    ) -> TransactionResult {
        debug_assert!(
            request_metric.success,
            "accumulate_request_by_second called with unsuccessful request"
        );
        debug_assert!(
            request_metric.error.is_empty(),
            "accumulate_request_by_second called with non-empty error"
        );
        // Only non-update requests get here; updates go through
        // set_success/set_failure → send_request_metric_now directly.
        debug_assert!(
            !request_metric.update,
            "accumulate_request_by_second called with update request"
        );

        self.flush_batch_outside_seconds(request_metric.elapsed)?;

        let batch = match self.metrics_batch.as_mut() {
            Some(b) => b,
            None => return Ok(()),
        };

        let method_label = request_metric.raw.method_label();
        // Format key into reusable buffer to avoid per-request allocation.
        self.counter_key_buf.clear();
        let _ = std::fmt::Write::write_fmt(
            &mut self.counter_key_buf,
            format_args!("{} {}", method_label, request_metric.name),
        );

        let second = (request_metric.elapsed / 1000) as usize;
        let next_second = match batch.second {
            None => {
                batch.second = Some(second);
                false
            }
            Some(held) => {
                debug_assert!(
                    second == held || second == held + 1,
                    "a batch holds requests from more than two seconds"
                );
                second != held
            }
        };

        // One lookup by reference when the batch has seen the name; the key
        // is only copied the first time it does.
        let key = self.counter_key_buf.as_str();
        let entry = match batch.requests.get_mut(key) {
            Some(entry) => entry,
            None => batch
                .requests
                .entry(key.to_string())
                .or_insert_with(|| RequestBatchEntry {
                    path: request_metric.name.to_string(),
                    method: request_metric.raw.method,
                    custom_method: request_metric.raw.custom_method.clone(),
                    raw_data: GooseRequestMetricTimingData::new(None),
                    status_code_counts: HashMap::new(),
                    status_code_timings: HashMap::new(),
                }),
        };

        // Pre-aggregate timing data using the same bucketing logic.
        let us = request_metric.response_time_micros();
        if next_second {
            match batch.next_second.get_mut(key) {
                Some(next) => next.record_time_us_unsorted(us),
                None => {
                    let mut next = GooseRequestMetricTimingData::new(None);
                    next.record_time_us_unsorted(us);
                    batch.next_second.insert(key.to_string(), next);
                }
            }
        } else {
            entry.raw_data.record_time_us_unsorted(us);
        }

        if !self.config.no_status_codes {
            // Increment status-code count.
            *entry
                .status_code_counts
                .entry(request_metric.status_code)
                .or_insert(0) += 1;

            // Record status-code timing summary.
            let summary = entry
                .status_code_timings
                .entry(request_metric.status_code)
                .or_insert_with(StatusCodeTimingSummary::default);
            summary.record_us(us);
        }

        self.batch_item_count += 1;
        Ok(())
    }

    /// Buffer an individual request metric in the batch without pre-aggregating.
    ///
    /// Used for failed requests (which need per-request error logging on the
    /// processor side) and CO-affected requests (which need per-request backfill).
    fn accumulate_individual_request(&mut self, request_metric: GooseRequestMetric) {
        if let Some(batch) = self.metrics_batch.as_mut() {
            batch.individual_requests.push(request_metric);
            self.batch_item_count += 1;
        }
    }

    /// Accumulate a transaction metric into the local batch.
    pub(crate) fn accumulate_transaction(&mut self, transaction: &TransactionMetric) {
        let batch = match self.metrics_batch.as_mut() {
            Some(b) => b,
            None => return,
        };

        let key = (transaction.scenario_index, transaction.transaction_index);
        let entry = batch
            .transactions
            .entry(key)
            .or_insert_with(|| TransactionBatchEntry {
                run_times: GooseRequestMetricTimingData::default(),
                success_count: 0,
                fail_count: 0,
            });

        entry
            .run_times
            .record_time_us_unsorted(transaction.run_time_micros());
        if transaction.success {
            entry.success_count += 1;
        } else {
            entry.fail_count += 1;
        }

        // Track graph data.
        if self.record_graph_data {
            let second = (transaction.elapsed / 1000) as usize;
            *batch.graph_tps.entry(second).or_insert(0) += 1;
        }
    }

    /// Accumulate a scenario metric into the local batch.
    pub(crate) fn accumulate_scenario(&mut self, scenario: &ScenarioMetric) {
        let batch = match self.metrics_batch.as_mut() {
            Some(b) => b,
            None => return,
        };

        let entry = batch
            .scenarios
            .entry(scenario.index)
            .or_insert_with(|| ScenarioBatchEntry {
                run_times: GooseRequestMetricTimingData::default(),
                user: scenario.user,
            });

        entry
            .run_times
            .record_time_us_unsorted(scenario.run_time_micros());

        // Track graph data.
        if self.record_graph_data {
            let second = (scenario.elapsed / 1000) as usize;
            *batch.graph_sps.entry(second).or_insert(0) += 1;
        }
    }

    /// Check if the current batch is stale (epoch changed due to metrics reset)
    /// and discard it if so, creating a fresh batch with the current epoch.
    ///
    /// This must be called before accumulating any new metrics, so that
    /// post-reset requests go into a fresh batch and maintain consistency
    /// between atomic counters (which were reset) and batch data.
    fn ensure_batch_current(&mut self) {
        if let (Some(batch), Some(epoch_arc)) = (&self.metrics_batch, &self.metrics_epoch) {
            let current_epoch = epoch_arc.load(AtomicOrdering::Relaxed);
            if batch.epoch < current_epoch {
                // Discard stale batch data from before the metrics reset.
                // The atomic counters for these metrics were already reset to 0,
                // so discarding the batch data maintains consistency.
                self.metrics_batch = Some(GooseMetricBatch::new(current_epoch));
                self.batch_item_count = 0;
                self.batch_start = None;
            }
        }
    }

    /// Check if the current batch should be flushed, and flush if so.
    ///
    /// The batch is flushed when either:
    /// - The number of accumulated requests reaches [`METRICS_BATCH_SIZE`]
    /// - The batch has been accumulating for longer than [`METRICS_BATCH_MAX_AGE`]
    pub(crate) fn maybe_flush_batch(&mut self) -> TransactionResult {
        if self.batch_item_count >= METRICS_BATCH_SIZE {
            return self.flush_batch();
        }
        if let Some(start) = self.batch_start {
            if start.elapsed() >= METRICS_BATCH_MAX_AGE {
                return self.flush_batch();
            }
        }
        Ok(())
    }

    /// When the heatmap is on, flush the batch first if a request
    /// that started at `elapsed` ms can not join it. A batch holds requests
    /// from its first request's second and the next one (see
    /// [`GooseMetricBatch::second`]); a request from before that second, or
    /// two or more seconds after it, starts a new batch. Batches flush as
    /// [`Self::maybe_flush_batch`] says, which checks a batch's age only
    /// after adding a request, so this happens when a user adds a request
    /// after idling a second or more with a batch open, or when a slow
    /// request, added when it finished, started well before the requests
    /// that follow it.
    ///
    /// Main would have added the request to the open batch and sent the two
    /// together when the batch reached its count or age. The new batch keeps
    /// the open one's start and count, so the request leaves on the flush
    /// main would use, never later: a user that idled past the batch's age
    /// sends it at once instead of holding it.
    fn flush_batch_outside_seconds(&mut self, elapsed: u64) -> TransactionResult {
        let second = (elapsed / 1000) as usize;
        let outside = self
            .metrics_batch
            .as_ref()
            .and_then(|batch| batch.second)
            .is_some_and(|held| second != held && second != held + 1);
        if outside {
            let (start, count) = (self.batch_start, self.batch_item_count);
            self.flush_batch()?;
            self.batch_start = start;
            self.batch_item_count = count;
        }
        Ok(())
    }

    /// Whether the batch holds any request, transaction or scenario.
    fn batch_has_data(&self) -> bool {
        self.batch_item_count > 0
            || self
                .metrics_batch
                .as_ref()
                .is_some_and(|b| !b.transactions.is_empty() || !b.scenarios.is_empty())
    }

    /// Before a wait of at least [`METRICS_BATCH_MAX_AGE`], add the pending
    /// request and transaction to the batch and send it. A batch's age is
    /// checked only when something is added to it, so it would otherwise be
    /// held through the wait.
    pub(crate) fn flush_batch_before_wait(&mut self) -> TransactionResult {
        if self.metrics_batch.is_none() {
            return Ok(());
        }
        self.flush_pending_metrics()?;
        self.flush_batch()
    }

    /// Flush the current batch to the metrics channel.
    ///
    /// Sends the accumulated batch as a single [`GooseMetric::Batch`] message
    /// and resets the accumulator for the next batch.
    pub(crate) fn flush_batch(&mut self) -> TransactionResult {
        if !self.batch_has_data() {
            return Ok(());
        }
        let current_epoch = self
            .metrics_epoch
            .as_ref()
            .map_or(0, |e| e.load(AtomicOrdering::Relaxed));
        if let Some(batch) = self.metrics_batch.take() {
            if let Some(metrics_channel) = self.metrics_channel.as_ref() {
                if let Err(e) = metrics_channel.send(GooseMetric::Batch(Box::new(batch))) {
                    self.metrics_batch = Some(GooseMetricBatch::new(current_epoch));
                    self.batch_item_count = 0;
                    self.batch_start = None;
                    return Err(Box::new(e.into()));
                }
            }
            // Replace with a fresh batch stamped with the current epoch.
            self.metrics_batch = Some(GooseMetricBatch::new(current_epoch));
            self.batch_item_count = 0;
            self.batch_start = None;
        }
        Ok(())
    }

    /// If `request_name` is set, unwrap and use this. Otherwise, if the Transaction has a name
    /// set use it. Otherwise use the path.
    ///
    /// A name inherited from the Transaction is shared with it, not copied.
    fn get_request_name(&self, request: &GooseRequest) -> Arc<str> {
        match request.name {
            // If a request.name is set, unwrap and return it.
            Some(rn) => Arc::from(rn),
            None => {
                // Otherwise determine if the current Transaction is named, and if so return it.
                if let Some(TransactionName::InheritNameByRequests(request_name)) =
                    &self.transaction_name
                {
                    if !request_name.is_empty() {
                        return request_name.clone();
                    }
                }
                Arc::from(request.path)
            }
        }
    }

    /// Manually mark a request as a success.
    ///
    /// Goose determines if a request was successful based on the the HTTP response status
    /// code. By default, it uses [`reqwest::StatusCode::is_success`]. If an alternative
    /// HTTP response code is expected, use [`GooseRequestBuilder::expect_status_code`]. If
    /// validation requires additional logic, you can use set_success().
    ///
    /// A copy of your original request is returned with the response, and a mutable copy
    /// must be included when setting a request as a success.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(get_function);
    ///
    /// /// A simple transaction that makes a GET request.
    /// async fn get_function(user: &mut GooseUser) -> TransactionResult {
    ///     let mut goose = user.get("404").await?;
    ///
    ///     if let Ok(response) = &goose.response {
    ///         // We expect a 404 here.
    ///         if response.status() == 404 {
    ///             return user.set_success(&mut goose.request);
    ///         }
    ///     }
    ///
    ///     Err(Box::new(TransactionError::RequestFailed {
    ///         raw_request: Box::new(goose.request.clone()),
    ///     }))
    /// }
    /// ````
    pub fn set_success(&mut self, request: &mut GooseRequestMetric) -> TransactionResult {
        // Only send update if this was previously not a success.
        if !request.success {
            request.success = true;
            request.update = true;
            // Flush any pending request metric to maintain ordering before the update.
            self.flush_pending_request()?;
            // Flush the batch so the original metric arrives before the update.
            self.flush_batch()?;
            self.send_request_metric_now(request.clone())?;
            // Atomically correct the shared counters (success +1, failure -1).
            // Done after the channel send so counts stay consistent if the send fails.
            self.update_request_counter(&request.raw.method, &request.name, true);
        }

        Ok(())
    }

    /// Manually mark a request as a failure.
    ///
    /// By default, Goose will consider any response with a 2xx status code as a success.
    /// You may require more advanced logic, in which a 2xx status code is actually a
    /// failure. A copy of your original request is returned with the response, and a
    /// mutable copy must be included when setting a request as a failure.
    ///
    /// Calls to `set_failure` must include four parameters. The first, `tag`, is an
    /// arbitrary string identifying the reason for the failure, used when logging. The
    /// second, `request`, is a mutable reference to the
    /// ([`GooseRequestMetric`](./struct.GooseRequestMetric.html)) object of the request being
    /// identified as a failure (the contained `success` field will be set to `false`,
    /// and the `update` field will be set to `true`). The last two parameters, `header`
    /// and `body`, are optional and used to provide more detail in logs.
    ///
    /// The value of `tag` will normally be collected into the errors summary table if
    /// metrics are being displayed. However, if `set_failure` is called multiple times,
    /// or is called on a request that was already an error, only the first error will
    /// be collected.
    ///
    /// This also calls [`GooseUser::log_debug`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(loadtest_index_page);
    ///
    /// async fn loadtest_index_page(user: &mut GooseUser) -> TransactionResult {
    ///     let mut goose = user.get("").await?;
    ///
    ///     if let Ok(response) = goose.response {
    ///         // We only need to check pages that returned a success status code.
    ///         if response.status().is_success() {
    ///             match response.text().await {
    ///                 Ok(text) => {
    ///                     // If the expected string doesn't exist, this page load
    ///                     // was a failure.
    ///                     if !text.contains("this string must exist") {
    ///                         // As this is a named request, pass in the name not the URL
    ///                         return user.set_failure("string missing", &mut goose.request, None, None);
    ///                     }
    ///                 }
    ///                 // Empty page, this is a failure.
    ///                 Err(_) => {
    ///                     return user.set_failure("empty page", &mut goose.request, None, None);
    ///                 }
    ///             }
    ///         }
    ///     };
    ///
    ///     Ok(())
    /// }
    /// ````
    pub fn set_failure(
        &mut self,
        tag: &str,
        request: &mut GooseRequestMetric,
        headers: Option<&header::HeaderMap>,
        body: Option<&str>,
    ) -> TransactionResult {
        // Only send update if this was previously a success.
        if request.success {
            request.success = false;
            request.update = true;
            request.error = tag.to_string();
            // Flush any pending request metric to maintain ordering before the update.
            self.flush_pending_request()?;
            // Flush the batch so the original metric arrives before the update.
            self.flush_batch()?;
            self.send_request_metric_now(request.clone())?;
            // Atomically correct the shared counters (success -1, failure +1).
            // Done after the channel send so counts stay consistent if the send fails.
            self.update_request_counter(&request.raw.method, &request.name, false);
        }
        // Write failure to log, converting `&mut request` to `&request` as needed by `log_debug()`.
        self.log_debug(tag, Some(&*request), headers, body)?;

        // Print log to stdout.
        info!("set_failure: {tag}");

        Err(Box::new(TransactionError::RequestFailed {
            raw_request: Box::new(request.clone()),
        }))
    }

    /// Record a metric for a custom (non-HTTP) request.
    ///
    /// Goose is primarily designed for HTTP load testing, but this method allows you to
    /// record request metrics for any protocol — TCP, UDP, WebSocket, gRPC, etc. The
    /// recorded metrics are aggregated alongside HTTP request metrics and appear in all
    /// Goose reports under the method label you provide.
    ///
    /// # Parameters
    /// - `method`: A short label identifying the protocol (e.g. `"TCP"`, `"GRPC"`, `"WS"`).
    ///   Displayed in the method column of all metrics output. Must not contain whitespace.
    /// - `name`: An arbitrary string identifying the operation (used as the request name
    ///   in metrics output, e.g. `"send_file"` or `"tcp_echo"`).
    /// - `response_time`: How long the operation took, **in milliseconds**. Use
    ///   `started.elapsed().as_millis() as u64`. Note that `as_millis()` truncates to whole
    ///   milliseconds: sub-millisecond operations will be recorded as `0 ms`. To keep
    ///   the microseconds, as Goose does for HTTP requests, call
    ///   [`record_custom_request_duration`](Self::record_custom_request_duration) instead.
    /// - `success`: Whether the operation succeeded. Unlike HTTP requests where Goose
    ///   infers success from the status code, for custom protocols you control this directly.
    /// - `status_code`: A protocol-specific status or result code. Pass `None` when the
    ///   protocol has no concept of status codes; pass `Some(code)` to record a meaningful
    ///   code (e.g. a gRPC status code). Internally, `None` is stored as `0`. If status code
    ///   reporting is enabled, `0` will appear in status code tables for these requests.
    /// - `error`: An optional error tag describing why the operation failed. Ignored if
    ///   `success` is `true`.
    ///
    /// # Return value
    ///
    /// This method always returns `Ok(())`, even when `success` is `false`. Recording a
    /// failure and propagating a failure as a [`TransactionError`] are independent: if you
    /// want the enclosing transaction to fail, return a `TransactionError` after calling
    /// this method.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    /// use std::time::Instant;
    ///
    /// let mut transaction = transaction!(tcp_upload);
    ///
    /// /// Simulate sending a file over TCP and record the result as a Goose metric.
    /// async fn tcp_upload(user: &mut GooseUser) -> TransactionResult {
    ///     let started = Instant::now();
    ///
    ///     // Perform the actual TCP operation here.
    ///     // let result = send_file_over_tcp(...);
    ///     let success = true;
    ///
    ///     user.record_custom_request(
    ///         "TCP",
    ///         "tcp_upload",
    ///         started.elapsed().as_millis() as u64,
    ///         success,
    ///         None, // TCP has no status codes
    ///         None,
    ///     ).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn record_custom_request(
        &mut self,
        method: &str,
        name: &str,
        response_time: u64,
        success: bool,
        status_code: Option<u16>,
        error: Option<&str>,
    ) -> TransactionResult {
        self.record_custom(
            method,
            name,
            response_time.saturating_mul(1000),
            success,
            status_code,
            error,
        )
        .await
    }

    /// [`record_custom_request`](Self::record_custom_request) with the time the
    /// operation took as a [`Duration`], such as
    /// `started.elapsed()`, so a sub millisecond operation keeps its microseconds
    /// as an HTTP request does.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    /// use std::time::Instant;
    ///
    /// let mut transaction = transaction!(udp_ping);
    ///
    /// async fn udp_ping(user: &mut GooseUser) -> TransactionResult {
    ///     let started = Instant::now();
    ///     // Send a datagram and wait for the answer here.
    ///     user.record_custom_request_duration("UDP", "ping", started.elapsed(), true, None, None)
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn record_custom_request_duration(
        &mut self,
        method: &str,
        name: &str,
        response_time: Duration,
        success: bool,
        status_code: Option<u16>,
        error: Option<&str>,
    ) -> TransactionResult {
        self.record_custom(
            method,
            name,
            crate::metrics::duration_micros(response_time),
            success,
            status_code,
            error,
        )
        .await
    }

    /// Record a custom request that took `response_time_us` microseconds.
    async fn record_custom(
        &mut self,
        method: &str,
        name: &str,
        response_time_us: u64,
        success: bool,
        status_code: Option<u16>,
        error: Option<&str>,
    ) -> TransactionResult {
        // The aggregation key uses "{method} {name}", so both must be non-empty
        // and the method label must not contain whitespace.
        if method.is_empty() || method.contains(char::is_whitespace) {
            return Err(Box::new(TransactionError::Custom(
                "method label must not be empty and must not contain whitespace".to_string(),
            )));
        }
        if name.is_empty() || name.trim().is_empty() {
            return Err(Box::new(TransactionError::Custom(
                "name must not be empty or whitespace-only".to_string(),
            )));
        }

        let raw_request = GooseRawRequest::new_custom(name, method);

        let transaction_detail = TransactionDetail {
            scenario_index: self.scenarios_index,
            scenario_name: self.scenario_name.clone(),
            transaction_index: self.transaction_index,
            transaction_name: self
                .transaction_name
                .clone()
                .unwrap_or(TransactionName::default_value()),
        };

        let mut request_metric = GooseRequestMetric::new(
            raw_request,
            transaction_detail,
            name,
            self.started.elapsed().as_millis(),
            self.weighted_users_index,
        );

        request_metric.set_response_micros(response_time_us);
        // For custom protocol requests there is no redirect URL; reuse `name` so that
        // per-request log entries and report displays have something meaningful to show.
        request_metric.final_url = name.to_string();
        request_metric.status_code = status_code.unwrap_or(0);
        request_metric.success = success;
        if !success {
            request_metric.error = error
                .map(|e| e.to_string())
                .unwrap_or_else(|| format!("{method} {name}: request failed"));
        }

        // Track cadence for Coordinated Omission Mitigation, mirroring the HTTP request path.
        request_metric.user_cadence = self
            .coordinated_omission_mitigation(&request_metric)
            .await?;

        if !self.config.no_metrics {
            self.send_request_metric_to_parent(request_metric)?;
        }

        Ok(())
    }

    /// Write to [`debug_file`](../struct.GooseConfiguration.html#structfield.debug_file)
    /// if enabled.
    ///
    /// This function provides a mechanism for optional debug logging when a load test
    /// is running. This can be especially helpful when writing a load test. Each entry
    /// must include a tag, which is an arbitrary string identifying the debug message.
    /// It may also optionally include references to the GooseRequestMetric made, the headers
    /// returned by the server, and the response body returned by the server,
    ///
    /// As the response body can be large, the `--no-debug-body` option (or
    /// [`GooseDefault::NoDebugBody`](../config/enum.GooseDefault.html#variant.NoDebugBody) default)
    /// can be set to prevent the debug log from including the response body. When this option
    /// is enabled, the body will always show up as `null` in the debug log.
    ///
    /// Calls to [`GooseUser::set_failure`] automatically invoke `log_debug`.
    ///
    /// To enable the debug log, a load test must be run with the `--debug-log-file=foo`
    /// option set, where `foo` is either a relative or an absolute path of the log file
    /// to create. Any existing file will be overwritten.
    ///
    /// In the following example, we are logging debug messages whenever there are errors.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut transaction = transaction!(loadtest_index_page);
    ///
    /// async fn loadtest_index_page(user: &mut GooseUser) -> TransactionResult {
    ///     let mut goose = user.get("").await?;
    ///
    ///     match goose.response {
    ///         Ok(response) => {
    ///             // Grab a copy of the headers so we can include them when logging errors.
    ///             let headers = &response.headers().clone();
    ///             // We only need to check pages that returned a success status code.
    ///             if !response.status().is_success() {
    ///                 match response.text().await {
    ///                     Ok(html) => {
    ///                         // Server returned an error code, log everything.
    ///                         user.log_debug(
    ///                             "error loading /",
    ///                             Some(&goose.request),
    ///                             Some(headers),
    ///                             Some(&html),
    ///                         );
    ///                     },
    ///                     Err(e) => {
    ///                         // No body was returned, log everything else.
    ///                         user.log_debug(
    ///                             &format!("error loading /: {}", e),
    ///                             Some(&goose.request),
    ///                             Some(headers),
    ///                             None,
    ///                         );
    ///                     }
    ///                 }
    ///             }
    ///         },
    ///         // No response from server.
    ///         Err(e) => {
    ///             user.log_debug(
    ///                 "no response from server when loading /",
    ///                 Some(&goose.request),
    ///                 None,
    ///                 None,
    ///             );
    ///         }
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ````
    pub fn log_debug(
        &self,
        tag: &str,
        request: Option<&GooseRequestMetric>,
        headers: Option<&header::HeaderMap>,
        body: Option<&str>,
    ) -> TransactionResult {
        if self.config.debug_log().is_some() {
            // Logger is not defined when running
            // [`test_start`](../struct.GooseAttack.html#method.test_start),
            // [`test_stop`](../struct.GooseAttack.html#method.test_stop), and during testing.
            if let Some(logger) = self.logger.clone() {
                if self.config.no_debug_body {
                    if let Err(e) = logger.send(Some(GooseLog::Debug(GooseDebug::new(
                        tag, request, headers, None,
                    )))) {
                        return Err(Box::new(e.into()));
                    }
                } else if let Err(e) = logger.send(Some(GooseLog::Debug(GooseDebug::new(
                    tag, request, headers, body,
                )))) {
                    return Err(Box::new(e.into()));
                }
            }
        }

        Ok(())
    }

    /// Manually build a
    /// [`reqwest::Client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html).
    ///
    /// By default, Goose configures the following options when building a
    /// [`reqwest::Client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html):
    ///  - reports itself as the
    ///    [`user_agent`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.user_agent)
    ///    requesting web pages (ie `goose/0.15.2`);
    ///  - [stores cookies](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.cookie_store),
    ///    generally necessary if you aim to simulate logged in users;
    ///  - enables
    ///    [`gzip`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.gzip) compression;
    ///  - sets a 60 second [`timeout`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.timeout) all
    ///    on all requests.
    ///
    /// # Default configuration:
    ///
    /// ```rust
    /// use reqwest::Client;
    /// use core::time::Duration;
    ///
    /// static APP_USER_AGENT: &str = "goose/0.18.0";
    ///
    /// let mut builder = Client::builder()
    ///   .user_agent(APP_USER_AGENT)
    ///   .gzip(true)
    ///   .timeout(Duration::from_secs(60));
    ///
    /// #[cfg(feature = "cookies")]
    /// {
    ///     builder = builder.cookie_store(true);  // Enable cookies by default
    /// }
    /// ```
    ///
    /// Alternatively, you can use this function to manually build a
    /// [`reqwest::Client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html).
    /// with custom configuration. Available options are found in the
    /// [`reqwest::ClientBuilder`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html)
    /// documentation.
    ///
    /// When manually building a
    /// [`reqwest::Client`](https://docs.rs/reqwest/*/reqwest/struct.Client.html),
    /// there are a few things to be aware of:
    ///  - Manually building a client in [`test_start`](../struct.GooseAttack.html#method.test_start)
    ///    will only affect requests made during test setup;
    ///  - Manually building a client in [`test_stop`](../struct.GooseAttack.html#method.test_stop)
    ///    will only affect requests made during test teardown;
    ///  - A manually built client is specific to a single Goose thread -- if you are
    ///    generating a large load test with many users, each will need to manually build their
    ///    own client (typically you'd do this in a Transaction that is registered with
    ///    [`Transaction::set_on_start()`] in each Scenario requiring a custom client;
    ///  - Manually building a client will completely replace the automatically built client
    ///    with a brand new one, so any configuration, cookies or headers set in the previously
    ///    built client will be gone;
    ///  - You must include all desired configuration, as you are completely replacing Goose
    ///    defaults. For example, if you want Goose clients to store cookies, you will have to
    ///    include
    ///    [`.cookie_store(true)`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.cookie_store).
    ///
    /// In the following example, the Goose client is configured with a different user agent,
    /// sets a default header on every request, stores cookies, supports gzip compression, and
    /// times out requests after 30 seconds.
    ///
    /// ## Example
    /// ```rust
    /// use goose::prelude::*;
    /// use core::time::Duration;
    ///
    /// transaction!(setup_custom_client).set_on_start();
    ///
    /// async fn setup_custom_client(user: &mut GooseUser) -> TransactionResult {
    ///     use reqwest::{Client, header};
    ///
    ///     // Build a custom HeaderMap to include with all requests made by this client.
    ///     let mut headers = header::HeaderMap::new();
    ///     headers.insert("X-Custom-Header", header::HeaderValue::from_str("custom value").unwrap());
    ///
    ///     // Build a custom client.
    ///     let mut builder = Client::builder()
    ///         .default_headers(headers)
    ///         .user_agent("custom user agent");
    ///
    ///     #[cfg(feature = "cookies")]
    ///     {
    ///         builder = builder.cookie_store(true);
    ///     }
    ///
    ///     let builder = builder
    ///         .gzip(true)
    ///         .timeout(Duration::from_secs(30));
    ///
    ///     // Assign the custom client to this GooseUser.
    ///     user.set_client_builder(builder).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Alternative Compression Algorithms
    /// Reqwest also supports
    /// [`brotli`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.brotli) and
    /// [`deflate`](https://docs.rs/reqwest/*/reqwest/struct.ClientBuilder.html#method.deflate) compression.
    ///
    /// To enable either, you must enable the features in your load test's `Cargo.toml`, for example:
    /// ```text
    /// reqwest = { version = "^0.11.4",  default-features = false, features = [
    ///     "brotli",
    ///     "cookies",
    ///     "deflate",
    ///     "gzip",
    ///     "json",
    /// ] }
    /// ```
    ///
    /// Once enabled, you can add `.brotli(true)` and/or `.deflate(true)` to your custom
    /// [`reqwest::Client::builder()`], following the documentation above.
    ///
    /// # Custom Cookies
    /// Custom cookies can also be manually set when building a custom [`reqwest::Client`]. This requires
    /// loading the [`GooseUser::base_url`] being load tested in order to properly build the cookie. Then
    /// a custom [`reqwest::cookie::Jar`] is created and the custom cookie is added with
    /// [`reqwest::cookie::Jar::add_cookie_str`]. Finally, the new cookie jar must be specified as the
    /// [`reqwest::ClientBuilder::cookie_provider`] for the custom client.
    ///
    /// ## Example
    /// ```rust
    /// # #[cfg(feature = "cookies")]
    /// # {
    /// use reqwest::{cookie::Jar, Client};
    /// use std::sync::Arc;
    ///
    /// use goose::prelude::*;
    ///
    /// transaction!(custom_cookie_with_custom_client).set_on_start();
    ///
    /// async fn custom_cookie_with_custom_client(user: &mut GooseUser) -> TransactionResult {
    ///     // Prepare the contents of a custom cookie.
    ///     let cookie = "my-custom-cookie=custom-value";
    ///
    ///     // Pre-load one or more cookies into a custom cookie jar to use with this client.
    ///     let jar = Jar::default();
    ///     jar.add_cookie_str(
    ///         cookie,
    ///         &user.base_url,
    ///     );
    ///
    ///     // Build a custom client.
    ///     let mut builder = Client::builder()
    ///         .user_agent("example-loadtest");
    ///
    ///     #[cfg(feature = "cookies")]
    ///     {
    ///         builder = builder.cookie_store(true).cookie_provider(Arc::new(jar));
    ///     }
    ///
    ///     let builder = builder.gzip(true);
    ///
    ///     // Assign the custom client to this GooseUser.
    ///     user.set_client_builder(builder).await?;
    ///
    ///     Ok(())
    /// }
    /// # }
    /// ```
    pub async fn set_client_builder(
        &mut self,
        builder: ClientBuilder,
    ) -> Result<(), TransactionError> {
        self.client = builder.build()?;

        Ok(())
    }

    /// Some websites use multiple domains to serve traffic, redirecting depending on
    /// the user's roll. For this reason, Goose needs to respect a redirect of the
    /// `base_url` and subsequent paths should be built from the redirect domain.
    ///
    /// For example, if the `base_url` (ie `--host`) is set to `foo.example.com` and the
    /// load test requests `/login`, thereby loading `http://foo.example.com/login` and
    /// this request gets redirected by the server to `http://foo-secure.example.com/`,
    /// subsequent requests made by this user need to be against the new
    /// `foo-secure.example.com domain`. (Further, if the `base_url` is again redirected,
    /// such as when loading `http://foo-secure.example.com/logout`, the user should
    /// again follow for subsequent requests, perhaps in this case back to
    /// `foo.example.com`.)
    ///
    /// Load tests can also request absolute URLs, and if these URLs are redirected
    /// it does not affect the `base_url` of the load test. For example, if
    /// `foo.example.com` is the base url, and the load test requests
    /// `http://bar.example.com` (a different domain) and this request gets redirected
    /// to `http://other.example.com`, subsequent relative requests would still be made
    /// against `foo.example.com`.
    ///
    /// This functionality is used internally by Goose to follow redirects of the
    /// `base_url` when `--sticky-follow` is specified at run time, or
    /// [`set_default`](../struct.GooseAttack.html#method.set_default)
    /// `(`[`GooseDefault::StickyFollow`](../config/enum.GooseDefault.html#variant.StickyFollow)
    /// `, true)` is enabled. It is also
    /// available to be manually invoked from a load test such as in the following
    /// example.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     let _goose_metrics = GooseAttack::initialize()?
    ///         .register_scenario(scenario!("LoadtestTransactions")
    ///             .set_host("http://foo.example.com/")
    ///             .set_wait_time(Duration::from_secs(0), Duration::from_secs(3))?
    ///             .register_transaction(transaction!(transaction_foo).set_weight(10)?)
    ///             .register_transaction(transaction!(transaction_bar))
    ///         )
    ///         // Set a default run time so this test runs to completion.
    ///         .set_default(GooseDefault::RunTime, 1)?
    ///         .execute()
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn transaction_foo(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn transaction_bar(user: &mut GooseUser) -> TransactionResult {
    ///     // Before this transaction runs, all requests are being made against
    ///     // http://foo.example.com, after this transaction runs all subsequent
    ///     // requests are made against http://bar.example.com/.
    ///     user.set_base_url("http://bar.example.com/");
    ///     let _goose = user.get("").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_base_url(&mut self, host: &str) -> Result<(), Box<TransactionError>> {
        self.base_url = match Url::parse(host) {
            Ok(u) => u,
            Err(e) => return Err(Box::new(e.into())),
        };
        Ok(())
    }
}

/// Internal helper function to create the default GooseUser reqwest client
pub(crate) fn create_reqwest_client(
    configuration: &GooseConfiguration,
) -> Result<Client, reqwest::Error> {
    let client_builder = Client::builder()
        .user_agent(APP_USER_AGENT)
        .timeout(configuration.request_timeout())
        // Enable gzip unless `--no-gzip` flag is enabled.
        .gzip(!configuration.no_gzip)
        // Validate https certificates unless `--accept-invalid-certs` is enabled.
        .danger_accept_invalid_certs(configuration.accept_invalid_certs);

    #[cfg(feature = "cookies")]
    let client_builder = client_builder.cookie_store(true);

    client_builder.build()
}

/// Defines the HTTP requests that Goose makes.
///
/// Can be manually created and configured with [`GooseRequest::builder`], but it's typically
/// more convenient to use the [`GooseUser::get`], [`GooseUser::get_named`], [`GooseUser::post`],
/// [`GooseUser::post_form`], [`GooseUser::post_json`], [`GooseUser::head`] and
/// [`GooseUser::delete`] helpers.
///
/// For complete instructions review [`GooseRequestBuilder`].
#[derive(Debug)]
pub struct GooseRequest<'a> {
    // Defaults to `""`.
    path: &'a str,
    // Defaults to [`GooseMethod::Get`].
    method: GooseMethod,
    // Defaults to [`None`].
    name: Option<&'a str>,
    // Defaults to [`None`].
    expect_status_code: Option<u16>,
    // Defaults to [`false`].
    error_on_fail: bool,
    // Defaults to [`None`].
    request_builder: Option<RequestBuilder>,
}
impl<'a> GooseRequest<'a> {
    /// Convenience function to bring [`GooseRequestBuilder`] into scope.
    pub fn builder() -> GooseRequestBuilder<'a> {
        GooseRequestBuilder::new()
    }
}

/// Used to build a [`GooseRequest`] object, necessary to make a request with Goose.
///
/// It's only necessary to build manually if the [`GooseUser::get`], [`GooseUser::get_named`],
/// [`GooseUser::post`], [`GooseUser::post_form`], [`GooseUser::post_json`], [`GooseUser::head`]
/// and [`GooseUser::delete`] helpers don't provide you with enough flexibility.
///
/// # Example
/// ```rust
/// use goose::prelude::*;
///
/// let mut a_transaction = transaction!(transaction_function);
///
/// // A simple transaction that loads a relative path.
/// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
///     // Manually create a GooseRequestBuilder object.
///     let goose_request = GooseRequest::builder()
///         // Set a relative path to request.
///         .path("about")
///         // Name the request in the metircs.
///         .name("About page")
///         // Build the GooseRequest object.
///         .build();
///
///     // Make the configured request.
///     let _goose = user.request(goose_request).await?;
///
///     Ok(())
/// }
/// ```
pub struct GooseRequestBuilder<'a> {
    path: &'a str,
    method: GooseMethod,
    name: Option<&'a str>,
    expect_status_code: Option<u16>,
    error_on_fail: bool,
    request_builder: Option<RequestBuilder>,
}
impl<'a> GooseRequestBuilder<'a> {
    // Internal method to build a [`GooseRequest`] from a [`GooseRequestBuilder`].
    fn new() -> Self {
        Self {
            path: "",
            method: GooseMethod::Get,
            name: None,
            expect_status_code: None,
            error_on_fail: false,
            request_builder: None,
        }
    }

    /// Set the path to request.
    ///
    /// Typically is a relative path allowing Goose to append a configurable base_url.
    ///
    /// Defaults to `""` (the main index).
    ///
    /// # Example
    /// This can be implemented in a simpler way using the [`GooseUser::get`] helper function.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut a_transaction = transaction!(transaction_function);
    ///
    /// // A simple transaction that loads a relative path.
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     // Manually create a GooseRequestBuilder object.
    ///     let goose_request = GooseRequest::builder()
    ///         // Set a relative path to request.
    ///         .path("a/relative/path")
    ///         // Build the GooseRequest object.
    ///         .build();
    ///
    ///     // Make the configured request.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn path(mut self, path: impl Into<&'a str>) -> Self {
        self.path = path.into();
        self
    }

    /// Set the method of the request.
    ///
    /// Must be set to a [`GooseMethod`].
    ///
    /// Defaults to [`GooseMethod::Get`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut a_transaction = transaction!(transaction_function);
    ///
    /// // Make a DELETE request.
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     // Manually create a GooseRequestBuilder object.
    ///     let goose_request = GooseRequest::builder()
    ///         // Set a relative path to request.
    ///         .path("path/to/delete")
    ///         // Set the method to DELETE.
    ///         .method(GooseMethod::Delete)
    ///         // Build the GooseRequest object.
    ///         .build();
    ///
    ///     // Make the configured DELETE request.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn method(mut self, method: GooseMethod) -> Self {
        self.method = method;
        self
    }

    /// Set a name for the request, affecting how it shows up in metrics.
    ///
    /// Must be set to a [`GooseMethod`].
    ///
    /// Defaults to [`GooseMethod::Get`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut a_transaction = transaction!(transaction_function);
    ///
    /// // Make a named request.
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     // Manually create a GooseRequestBuilder object.
    ///     let goose_request = GooseRequest::builder()
    ///         // Set a relative path to request.
    ///         .path("path/to/request")
    ///         // Name the request in the metrics.
    ///         .name("custom name")
    ///         // Build the GooseRequest object.
    ///         .build();
    ///
    ///     // Make the configured request.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn name(mut self, name: impl Into<&'a str>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Manually configure the expected HTTP response status code.
    ///
    /// Defaults to [`reqwest::StatusCode::is_success`].
    ///
    /// # Example
    /// Intentionally request a 404 page, and do not trigger an error.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut a_transaction = transaction!(transaction_function);
    ///
    /// // Make a named request.
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     // Manually create a GooseRequestBuilder object.
    ///     let goose_request = GooseRequest::builder()
    ///         // Set a relative path to request.
    ///         .path("no/such/path")
    ///         // Tell Goose to expect a 404 HTTP response status code.
    ///         .expect_status_code(404)
    ///         // Build the GooseRequest object.
    ///         .build();
    ///
    ///     // Make the configured request.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn expect_status_code(mut self, status_code: u16) -> Self {
        self.expect_status_code = Some(status_code);
        self
    }

    /// Configure whether the request should return on error when it
    /// fails
    ///
    /// Defaults to [`false`].
    ///
    /// # Example
    /// Intentionally request a 404 page, and do not trigger an error.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut a_transaction = transaction!(transaction_function);
    ///
    /// // Make a named request.
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     // Manually create a GooseRequestBuilder object.
    ///     let goose_request = GooseRequest::builder()
    ///         // Set a relative path to request.
    ///         .path("no/such/path")
    ///         // Tell Goose to expect a 404 HTTP response status code.
    ///         .expect_status_code(404)
    ///         // Tell Goose to return an error if the status code is
    ///         // not a 404
    ///         .error_on_fail()
    ///         // Build the GooseRequest object.
    ///         .build();
    ///
    ///     // Make the configured request.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn error_on_fail(mut self) -> Self {
        self.error_on_fail = true;
        self
    }

    /// Manually create the [`reqwest::RequestBuilder`] used to make a request.
    ///
    /// # Example
    /// Manually create a `RequestBuilder` in order to set a timeout.
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let mut a_transaction = transaction!(transaction_function);
    ///
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     use std::time::Duration;
    ///
    ///     // Manually interact with the Reqwest RequestBuilder object.
    ///     let request_builder = user.get_request_builder(&GooseMethod::Get, "no/such/path")?
    ///         // Configure the request to timeout if it takes longer than 500 milliseconds.
    ///         .timeout(Duration::from_millis(500));
    ///
    ///     // Manually build a GooseRequest in order to set our custom RequestBuilder.
    ///     let goose_request = GooseRequest::builder()
    ///         // Manually add our custom RequestBuilder object.
    ///         .set_request_builder(request_builder)
    ///         // Turn the GooseRequestBuilder object into a GooseRequest.
    ///         .build();
    ///
    ///     // Finally make the actual request with our custom GooseRequest object.
    ///     let _goose = user.request(goose_request).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_request_builder(mut self, request_builder: RequestBuilder) -> Self {
        self.request_builder = Some(request_builder);
        self
    }

    /// Build the [`GooseRequest`] object which is then passed to [`GooseUser::request`].
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// // Build the default "GET /".
    /// let goose_request = GooseRequest::builder().build();
    /// ```
    pub fn build(self) -> GooseRequest<'a> {
        let Self {
            path,
            method,
            name,
            expect_status_code,
            error_on_fail,
            request_builder,
        } = self;
        GooseRequest {
            path,
            method,
            name,
            expect_status_code,
            error_on_fail,
            request_builder,
        }
    }
}

/// Remove path from Reqwest error to avoid having a lot of distincts error
/// when path parameters are used.
fn clean_reqwest_error(e: &reqwest::Error, request_name: &str) -> String {
    let kind = if e.is_builder() {
        "builder error"
    } else if e.is_request() {
        "error sending request"
    } else if e.is_body() {
        "request or response body error"
    } else if e.is_decode() {
        "error decoding response body"
    } else if e.is_redirect() {
        "error following redirect"
    } else {
        "Http status"
    };

    if let Some(ref e) = std::error::Error::source(e) {
        format!("{kind} {request_name}: {e}")
    } else {
        format!("{kind} {request_name}")
    }
}

/// Extract the path of a request URL for the request log lines in `request()`, or
/// an empty path if the URL does not parse. Only called from inside the log macro
/// arguments, which are evaluated after the level check, so the parse and the
/// allocation are skipped when the line is filtered out.
fn request_log_path(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) => u.path().to_string(),
        Err(e) => {
            error!("failed to parse url: {e}");
            "".to_string()
        }
    }
}

/// A helper to determine which host should be prepended to relative load test
/// paths in this Scenario.
///
/// The first of these defined will be returned as the prepended host:
///  1. `--host` (host specified on the command line when running load test)
///  2. [`Scenario`](./struct.Scenario.html)`.host` (default host defined
///     for the current scenario)
///  3. [`GooseDefault::Host`](../config/enum.GooseDefault.html#variant.Host) (default
///     host defined for the current load test)
pub fn get_base_url(
    config_host: Option<String>,
    scenario_host: Option<String>,
    default_host: Option<String>,
) -> Result<Url, GooseError> {
    // If the `--host` CLI option is set, build the URL with it.
    match config_host {
        Some(host) => Ok(
            Url::parse(&host).map_err(|parse_error| GooseError::InvalidHost {
                host,
                detail: "There was a failure parsing the host specified with --host.".to_string(),
                parse_error,
            })?,
        ),
        None => {
            match scenario_host {
                // Otherwise, if `Scenario.host` is defined, usee this
                Some(host) => {
                    Ok(
                        Url::parse(&host).map_err(|parse_error| GooseError::InvalidHost {
                            host,
                            detail: "There was a failure parsing the host specified with the Scenario.set_host() function.".to_string(),
                            parse_error,
                        })?,
                    )
                }
                // Otherwise, use global `GooseAttack.host`, refused when
                // there is none: a Start checks for a host before it runs,
                // so this only answers a caller that did not.
                None => {
                    let Some(default_host) = default_host else {
                        return Err(GooseError::InvalidOption {
                            option: "--host".to_string(),
                            value: "".to_string(),
                            detail: "A host must be defined via the --host option, the GooseAttack.set_default() function, or the Scenario.set_host() function.".to_string(),
                        });
                    };
                    Ok(
                        Url::parse(&default_host).map_err(|parse_error| GooseError::InvalidHost {
                            host: default_host.to_string(),
                            detail: "There was a failure parsing the host specified globally with the GooseAttack.set_default() function.".to_string(),
                            parse_error,
                        })?,
                    )
                }
            }
        }
    }
}

/// The function type of a goose transaction function.
pub type TransactionFunction = Arc<
    dyn for<'r> Fn(
            &'r mut GooseUser,
        ) -> Pin<Box<dyn Future<Output = TransactionResult> + Send + 'r>>
        + Send
        + Sync,
>;

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub enum TransactionName {
    InheritNameByRequests(Arc<str>),
    TransactionOnly(Arc<str>),
}

impl Serialize for TransactionName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            TransactionName::InheritNameByRequests(s) => serializer.serialize_newtype_variant(
                "TransactionName",
                0,
                "InheritNameByRequests",
                s.as_ref(),
            ),
            TransactionName::TransactionOnly(s) => serializer.serialize_newtype_variant(
                "TransactionName",
                1,
                "TransactionOnly",
                s.as_ref(),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for TransactionName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename = "TransactionName")]
        enum TransactionNameHelper {
            InheritNameByRequests(String),
            TransactionOnly(String),
        }

        /// Goose 0.18 and earlier wrote a transaction's name as a plain
        /// string, the name its requests took too.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum WrittenName {
            Named(TransactionNameHelper),
            Plain(String),
        }

        Ok(match WrittenName::deserialize(deserializer)? {
            WrittenName::Named(TransactionNameHelper::InheritNameByRequests(s))
            | WrittenName::Plain(s) => {
                TransactionName::InheritNameByRequests(Arc::from(s.as_str()))
            }
            WrittenName::Named(TransactionNameHelper::TransactionOnly(s)) => {
                TransactionName::TransactionOnly(Arc::from(s.as_str()))
            }
        })
    }
}

impl TransactionName {
    pub fn name_for_transaction(&self) -> &str {
        match self {
            TransactionName::InheritNameByRequests(v) => v.as_ref(),
            TransactionName::TransactionOnly(v) => v.as_ref(),
        }
    }
    pub fn name_for_transaction_arc(&self) -> Arc<str> {
        match self {
            TransactionName::InheritNameByRequests(v) | TransactionName::TransactionOnly(v) => {
                v.clone()
            }
        }
    }
    pub fn name_for_request(&self) -> &str {
        match self {
            TransactionName::InheritNameByRequests(v) => v.as_ref(),
            TransactionName::TransactionOnly(_) => "",
        }
    }
    pub fn default_value() -> Self {
        // One shared empty name, so an unnamed transaction does not allocate.
        static EMPTY: OnceLock<Arc<str>> = OnceLock::new();
        Self::TransactionOnly(EMPTY.get_or_init(|| Arc::from("")).clone())
    }
}

/// An individual transaction within a [`Scenario`](./struct.Scenario.html).
#[derive(Clone)]
pub struct Transaction {
    /// An index into [`Scenario`](./struct.Scenario.html)`.transaction`, indicating which
    /// transaction this is.
    pub transactions_index: usize,
    /// An optional name for the transaction, used when displaying metrics.
    pub name: TransactionName,
    /// An integer value that controls the frequency that this transaction will be run.
    pub weight: usize,
    /// An integer value that controls when this transaction runs compared to other transactions in the same
    /// [`Scenario`](./struct.Scenario.html).
    pub sequence: usize,
    /// A flag indicating that this transaction runs when the user starts.
    pub on_start: bool,
    /// A flag indicating that this transaction runs when the user stops.
    pub on_stop: bool,
    /// A required function that is executed each time this transaction runs.
    pub function: TransactionFunction,
}
impl Transaction {
    pub fn new(function: TransactionFunction) -> Self {
        trace!("new transaction");
        Transaction {
            transactions_index: usize::MAX,
            name: TransactionName::default_value(),
            weight: 1,
            sequence: 0,
            on_start: false,
            on_stop: false,
            function,
        }
    }

    /// Sets a custom name for this transaction only.
    ///
    /// This name will be used to identify the transaction in metrics and reports,
    /// but will not affect the names of individual requests within the transaction.
    ///
    /// # Example
    ///
    /// ```
    /// use goose::prelude::*;
    ///
    /// let transaction = transaction!(example_transaction)
    ///     .set_name("Login Flow")
    ///     .set_on_start();
    ///
    /// async fn example_transaction(user: &mut GooseUser) -> TransactionResult {
    ///     // Transaction logic here
    ///     Ok(())
    /// }
    /// ```
    pub fn set_name(mut self, name: &str) -> Self {
        trace!("[{}] set_name: {}", self.transactions_index, name);
        self.name = TransactionName::TransactionOnly(Arc::from(name));
        self
    }

    /// Sets a custom name for both the transaction and all its requests.
    ///
    /// This name will be used as a prefix for all requests within the transaction,
    /// allowing you to group related requests under a common namespace in metrics.
    ///
    /// # Example
    ///
    /// ```
    /// use goose::prelude::*;
    ///
    /// let transaction = transaction!(checkout_flow)
    ///     .set_name_for_transaction_and_requests("Checkout");
    ///
    /// async fn checkout_flow(user: &mut GooseUser) -> TransactionResult {
    ///     // All requests will be prefixed with "Checkout"
    ///     Ok(())
    /// }
    /// ```
    pub fn set_name_for_transaction_and_requests(mut self, name: &str) -> Self {
        trace!(
            "[{}] set_name (for transaction only): {}",
            self.transactions_index,
            name
        );
        self.name = TransactionName::InheritNameByRequests(Arc::from(name));
        self
    }

    /// Set an optional flag indicating that this transaction should be run when
    /// a user first starts. This could be used to log the user in, and
    /// so all subsequent transaction are done as a logged in user. A transaction
    /// with this flag set will only run at start time (and optionally at stop
    /// time as well, if that flag is also set).
    ///
    /// On-start transactions can be sequenced and weighted. Sequences allow
    /// multiple on-start transactions to run in a controlled order. Weights allow
    /// on-start transactions to run multiple times when a user starts.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// transaction!(my_on_start_function).set_on_start();
    ///
    /// async fn my_on_start_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_on_start(mut self) -> Self {
        trace!(
            "{} [{}] set_on_start transaction",
            self.name.name_for_transaction(),
            self.transactions_index
        );
        self.on_start = true;
        self
    }

    /// Set an optional flag indicating that this transaction should be run when
    /// a user stops. This could be used to log a user out when the user
    /// finishes its load test. A transaction with this flag set will only run at
    /// stop time (and optionally at start time as well, if that flag is
    /// also set).
    ///
    /// On-stop transactions can be sequenced and weighted. Sequences allow
    /// multiple on-stop transactions to run in a controlled order. Weights allow
    /// on-stop transactions to run multiple times when a user stops.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// transaction!(my_on_stop_function).set_on_stop();
    ///
    /// async fn my_on_stop_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_on_stop(mut self) -> Self {
        trace!(
            "{} [{}] set_on_stop transaction",
            self.name.name_for_transaction(),
            self.transactions_index
        );
        self.on_stop = true;
        self
    }

    /// Sets a weight on an individual transaction. The larger the value of weight, the more often it will be run
    /// in the Scenario. For example, if one transaction has a weight of 3 and another transaction has a weight of
    /// 1, the first transaction will run 3 times as often.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     transaction!(transaction_function).set_weight(3)?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_weight(mut self, weight: usize) -> Result<Self, GooseError> {
        trace!(
            "{} [{}] set_weight: {}",
            self.name.name_for_transaction(),
            self.transactions_index,
            weight
        );
        if weight == 0 {
            return Err(GooseError::InvalidWeight {
                weight,
                detail: "Weight must be set to at least 1.".to_string(),
            });
        }
        self.weight = weight;

        Ok(self)
    }

    /// Defines the sequence value of an individual transactions. Transactions are run in order of their
    /// sequence value, so a transaction with a sequence value of 1 will run before a transaction with a
    /// sequence value of 2. Transactions with no sequence value (or a sequence value of 0) will run last,
    /// after all transactions with positive sequence values.
    ///
    /// All transactions with the same sequence value will run in a random order. Transactions can be
    /// assigned both squence values and weights.
    ///
    /// # Examples
    /// In this first example, the variable names indicate the order the transactions will be run in:
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// let runs_first = transaction!(first_transaction_function).set_sequence(3);
    /// let runs_second = transaction!(second_transaction_function).set_sequence(5835);
    /// let runs_last = transaction!(third_transaction_function);
    ///
    /// async fn first_transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("1").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn second_transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("2").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn third_transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("3").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// In the following example, the `runs_first` transactions runs two times, then one instance of `runs_second`
    /// and two instances of `also_runs_second` are all three run. The user will do this over and over
    /// the entire time it runs, with `runs_first` always running first, then the other transactions being
    /// run in a random and weighted order:
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     let runs_first = transaction!(first_transaction_function).set_sequence(1).set_weight(2)?;
    ///     let runs_second = transaction!(second_transaction_function_a).set_sequence(2);
    ///     let also_runs_second = transaction!(second_transaction_function_b).set_sequence(2).set_weight(2)?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn first_transaction_function(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("1").await?;
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn second_transaction_function_a(user: &mut GooseUser) -> TransactionResult {
    ///     let _goose = user.get("2a").await?;
    ///
    ///     Ok(())
    /// }
    ///
    ///     async fn second_transaction_function_b(user: &mut GooseUser) -> TransactionResult {
    ///       let _goose = user.get("2b").await?;
    ///
    ///       Ok(())
    ///     }
    /// ```
    pub fn set_sequence(mut self, sequence: usize) -> Self {
        trace!(
            "{} [{}] set_sequence: {}",
            self.name.name_for_transaction(),
            self.transactions_index,
            sequence
        );
        if sequence < 1 {
            info!(
                "setting sequence to 0 for transaction {} is unnecessary, sequence disabled",
                self.name.name_for_transaction()
            );
        }
        self.sequence = sequence;
        self
    }
}
impl Hash for Transaction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.transactions_index.hash(state);
        self.name.hash(state);
        self.weight.hash(state);
        self.sequence.hash(state);
        self.on_start.hash(state);
        self.on_stop.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use httpmock::{
        Method::{GET, POST},
        MockServer,
    };

    const EMPTY_ARGS: Vec<&str> = vec![];

    fn setup_user(server: &MockServer) -> Result<GooseUser, GooseError> {
        let mut configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        configuration.co_mitigation = Some(GooseCoordinatedOmissionMitigation::Average);
        let base_url = get_base_url(Some(server.url("/")), None, None).unwrap();
        GooseUser::single(base_url, &configuration)
    }

    #[test]
    fn goose_scenario() {
        // Simplistic test transaction functions.
        async fn test_function_a(user: &mut GooseUser) -> TransactionResult {
            let _goose = user.get("/a/").await?;

            Ok(())
        }

        async fn test_function_b(user: &mut GooseUser) -> TransactionResult {
            let _goose = user.get("/b/").await?;

            Ok(())
        }

        let mut scenario = scenario!("foo");
        assert_eq!(&*scenario.name, "foo");
        assert_eq!(scenario.scenarios_index, usize::MAX);
        assert_eq!(scenario.weight, 1);
        assert_eq!(scenario.transaction_wait, None);
        assert!(scenario.host.is_none());
        assert_eq!(scenario.transactions.len(), 0);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.weighted_on_start_transactions.len(), 0);
        assert_eq!(scenario.weighted_on_stop_transactions.len(), 0);

        // Registering a transaction adds it to transactions, but doesn't update weighted_transactions.
        scenario = scenario.register_transaction(transaction!(test_function_a));
        assert_eq!(scenario.transactions.len(), 1);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.scenarios_index, usize::MAX);
        assert_eq!(scenario.weight, 1);
        assert_eq!(scenario.transaction_wait, None);
        assert!(scenario.host.is_none());

        // Different transactions can be registered.
        scenario = scenario.register_transaction(transaction!(test_function_b));
        assert_eq!(scenario.transactions.len(), 2);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.scenarios_index, usize::MAX);
        assert_eq!(scenario.weight, 1);
        assert_eq!(scenario.transaction_wait, None);
        assert!(scenario.host.is_none());

        // Same transactions can be registered again.
        scenario = scenario.register_transaction(transaction!(test_function_a));
        assert_eq!(scenario.transactions.len(), 3);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.scenarios_index, usize::MAX);
        assert_eq!(scenario.weight, 1);
        assert_eq!(scenario.transaction_wait, None);
        assert!(scenario.host.is_none());

        // Setting weight only affects weight field.
        scenario = scenario.set_weight(50).unwrap();
        assert_eq!(scenario.weight, 50);
        assert_eq!(scenario.transactions.len(), 3);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.scenarios_index, usize::MAX);
        assert_eq!(scenario.transaction_wait, None);
        assert!(scenario.host.is_none());

        // Weight can be changed.
        scenario = scenario.set_weight(5).unwrap();
        assert_eq!(scenario.weight, 5);

        // Setting host only affects host field.
        scenario = scenario.set_host("http://foo.example.com/");
        assert_eq!(scenario.host, Some("http://foo.example.com/".to_string()));
        assert_eq!(scenario.weight, 5);
        assert_eq!(scenario.transactions.len(), 3);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.scenarios_index, usize::MAX);
        assert_eq!(scenario.transaction_wait, None);

        // Host field can be changed.
        scenario = scenario.set_host("https://bar.example.com/");
        assert_eq!(scenario.host, Some("https://bar.example.com/".to_string()));

        // Wait time only affects wait time fields.
        scenario = scenario
            .set_wait_time(Duration::from_secs(1), Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            scenario.transaction_wait,
            Some((Duration::from_secs(1), Duration::from_secs(10)))
        );
        assert_eq!(scenario.host, Some("https://bar.example.com/".to_string()));
        assert_eq!(scenario.weight, 5);
        assert_eq!(scenario.transactions.len(), 3);
        assert_eq!(scenario.weighted_transactions.len(), 0);
        assert_eq!(scenario.scenarios_index, usize::MAX);

        // Wait time can be changed.
        scenario = scenario
            .set_wait_time(Duration::from_secs(3), Duration::from_secs(9))
            .unwrap();
        assert_eq!(
            scenario.transaction_wait,
            Some((Duration::from_secs(3), Duration::from_secs(9)))
        );
    }

    #[test]
    fn goose_transaction() {
        // Simplistic test transaction functions.
        async fn test_function_a(user: &mut GooseUser) -> TransactionResult {
            let _goose = user.get("/a/").await?;

            Ok(())
        }

        // Initialize scenario.
        let mut transaction = transaction!(test_function_a);
        assert_eq!(transaction.transactions_index, usize::MAX);
        assert_eq!(transaction.name.name_for_transaction(), "".to_string());
        assert_eq!(transaction.weight, 1);
        assert_eq!(transaction.sequence, 0);
        assert!(!transaction.on_start);
        assert!(!transaction.on_stop);

        // Name can be set, without affecting other fields.
        transaction = transaction.set_name("foo");
        assert_eq!(transaction.name.name_for_transaction(), "foo".to_string());
        assert_eq!(transaction.weight, 1);
        assert_eq!(transaction.sequence, 0);
        assert!(!transaction.on_start);
        assert!(!transaction.on_stop);

        // Name can be set multiple times.
        transaction = transaction.set_name("bar");
        assert_eq!(transaction.name.name_for_transaction(), "bar".to_string());

        // On start flag can be set, without affecting other fields.
        transaction = transaction.set_on_start();
        assert!(transaction.on_start);
        assert_eq!(transaction.name.name_for_transaction(), "bar".to_string());
        assert_eq!(transaction.weight, 1);
        assert_eq!(transaction.sequence, 0);
        assert!(!transaction.on_stop);

        // Setting on start flag twice doesn't change anything.
        transaction = transaction.set_on_start();
        assert!(transaction.on_start);

        // On stop flag can be set, without affecting other fields.
        // It's possible to set both on_start and on_stop for same transaction.
        transaction = transaction.set_on_stop();
        assert!(transaction.on_stop);
        assert!(transaction.on_start);
        assert_eq!(transaction.name.name_for_transaction(), "bar".to_string());
        assert_eq!(transaction.weight, 1);
        assert_eq!(transaction.sequence, 0);

        // Setting on stop flag twice doesn't change anything.
        transaction = transaction.set_on_stop();
        assert!(transaction.on_stop);

        // Setting weight doesn't change anything else.
        transaction = transaction.set_weight(2).unwrap();
        assert_eq!(transaction.weight, 2);
        assert!(transaction.on_stop);
        assert!(transaction.on_start);
        assert_eq!(transaction.name.name_for_transaction(), "bar".to_string());
        assert_eq!(transaction.sequence, 0);

        // Weight field can be changed multiple times.
        transaction = transaction.set_weight(3).unwrap();
        assert_eq!(transaction.weight, 3);

        // Setting sequence doesn't change anything else.
        transaction = transaction.set_sequence(4);
        assert_eq!(transaction.sequence, 4);
        assert_eq!(transaction.weight, 3);
        assert!(transaction.on_stop);
        assert!(transaction.on_start);
        assert_eq!(transaction.name.name_for_transaction(), "bar".to_string());

        // Sequence field can be changed multiple times.
        transaction = transaction.set_sequence(8);
        assert_eq!(transaction.sequence, 8);
    }

    #[tokio::test]
    async fn goose_user() {
        const HOST: &str = "http://example.com/";
        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let base_url = get_base_url(Some(HOST.to_string()), None, None).unwrap();
        let user = GooseUser::new(0, Arc::from(""), base_url, &configuration, 0, None).unwrap();
        assert_eq!(user.scenarios_index, 0);
        assert_eq!(user.weighted_users_index, usize::MAX);

        // Confirm the URLs are correctly built using the default_host.
        let url = user.build_url("/foo").unwrap();
        assert_eq!(&url, &[HOST, "foo"].concat());
        let url = user.build_url("bar/").unwrap();
        assert_eq!(&url, &[HOST, "bar/"].concat());
        let url = user.build_url("/foo/bar").unwrap();
        assert_eq!(&url, &[HOST, "foo/bar"].concat());

        // Confirm the URLs are built with their own specified host.
        let url = user.build_url("https://example.com/foo").unwrap();
        assert_eq!(url, "https://example.com/foo");
        let url = user
            .build_url("https://www.example.com/path/to/resource")
            .unwrap();
        assert_eq!(url, "https://www.example.com/path/to/resource");

        // Create a second user, this time setting a scenario_host.
        let base_url = get_base_url(
            None,
            Some("http://www2.example.com/".to_string()),
            Some("http://www.example.com/".to_string()),
        )
        .unwrap();
        let user2 = GooseUser::new(0, Arc::from(""), base_url, &configuration, 0, None).unwrap();

        // Confirm the URLs are correctly built using the scenario_host.
        let url = user2.build_url("/foo").unwrap();
        assert_eq!(url, "http://www2.example.com/foo");

        // Confirm URLs are still built with their own specified host.
        let url = user2.build_url("https://example.com/foo").unwrap();
        assert_eq!(url, "https://example.com/foo");

        // Confirm Goose can build a base_url that includes a path.
        const HOST_WITH_PATH: &str = "http://example.com/with/path/";
        let base_url = get_base_url(Some(HOST_WITH_PATH.to_string()), None, None).unwrap();
        let user = GooseUser::new(0, Arc::from(""), base_url, &configuration, 0, None).unwrap();

        // Confirm the URLs are correctly built using the default_host that includes a path.
        let url = user.build_url("foo").unwrap();
        assert_eq!(&url, &[HOST_WITH_PATH, "foo"].concat());
        let url = user.build_url("bar/").unwrap();
        assert_eq!(&url, &[HOST_WITH_PATH, "bar/"].concat());
        let url = user.build_url("foo/bar").unwrap();
        assert_eq!(&url, &[HOST_WITH_PATH, "foo/bar"].concat());

        // Confirm that URLs are correctly re-written if an absolute path is used.
        let url = user.build_url("/foo").unwrap();
        assert_eq!(&url, &[HOST, "foo"].concat());
    }

    #[tokio::test]
    async fn manual_requests() {
        let server = MockServer::start();

        let mut user = setup_user(&server).unwrap();

        // Set up a mock http server endpoint.
        const INDEX_PATH: &str = "/";
        let index = server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        });

        // Make a GET request to the mock http server and confirm we get a 200 response.
        assert_eq!(index.calls(), 0);
        let goose = user
            .get(INDEX_PATH)
            .await
            .expect("get returned unexpected error");
        let status = goose.response.unwrap().status();
        assert_eq!(status, 200);
        assert_eq!(goose.request.raw.method, GooseMethod::Get);
        assert_eq!(goose.request.name, INDEX_PATH.into());
        assert!(goose.request.success);
        assert!(!goose.request.update);
        assert_eq!(goose.request.status_code, 200);
        assert_eq!(index.calls(), 1);

        const NO_SUCH_PATH: &str = "/no/such/path";
        // Set up a mock http server endpoint.
        let not_found = server.mock(|when, then| {
            when.method(GET).path(NO_SUCH_PATH);
            then.status(404);
        });

        // Make an invalid GET request to the mock http server and confirm we get a 404 response.
        assert_eq!(not_found.calls(), 0);
        let goose = user
            .get(NO_SUCH_PATH)
            .await
            .expect("get returned unexpected error");
        let status = goose.response.unwrap().status();
        assert_eq!(status, 404);
        assert_eq!(goose.request.raw.method, GooseMethod::Get);
        assert_eq!(goose.request.name, NO_SUCH_PATH.into());
        assert!(!goose.request.success);
        assert!(!goose.request.update);
        assert_eq!(goose.request.status_code, 404,);
        not_found.assert_calls(1);

        // Set up a mock http server endpoint.
        const COMMENT_PATH: &str = "/comment";
        let comment = server.mock(|when, then| {
            when.method(POST).path(COMMENT_PATH).body("foo");
            then.status(200).body("foo");
        });

        // Make a POST request to the mock http server and confirm we get a 200 OK response.
        assert_eq!(comment.calls(), 0);
        let goose = user
            .post(COMMENT_PATH, "foo")
            .await
            .expect("post returned unexpected error");
        let unwrapped_response = goose.response.unwrap();
        let status = unwrapped_response.status();
        assert_eq!(status, 200);
        let body = unwrapped_response.text().await.unwrap();
        assert_eq!(body, "foo");
        assert_eq!(goose.request.raw.method, GooseMethod::Post);
        assert!(goose.request.success);
        assert!(!goose.request.update);
        assert_eq!(goose.request.status_code, 200);
        comment.assert_calls(1);
    }

    #[test]
    fn test_set_session_data() {
        #[derive(Debug, PartialEq, Eq, Clone)]
        struct CustomSessionData {
            data: String,
        }

        let session_data = CustomSessionData {
            data: "foo".to_owned(),
        };

        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

        user.set_session_data(session_data.clone());

        let session = user.get_session_data::<CustomSessionData>();
        assert!(session.is_some());
        assert_eq!(session.unwrap(), &session_data);

        let session = user.get_session_data_unchecked::<CustomSessionData>();
        assert_eq!(session, &session_data);
    }

    #[test]
    fn test_get_mut_session_data() {
        #[derive(Debug, Clone)]
        struct CustomSessionData {
            data: String,
        }

        let session_data = CustomSessionData {
            data: "foo".to_owned(),
        };

        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

        user.set_session_data(session_data);

        if let Some(session) = user.get_session_data_mut::<CustomSessionData>() {
            "bar".clone_into(&mut session.data);
        }

        let session = user.get_session_data_unchecked::<CustomSessionData>();
        assert_eq!(session.data, "bar".to_string());

        let session = user.get_session_data_unchecked_mut::<CustomSessionData>();
        "foo".clone_into(&mut session.data);
        let session = user.get_session_data_unchecked::<CustomSessionData>();
        assert_eq!(session.data, "foo".to_string());
    }

    #[test]
    fn test_set_session_data_override() {
        #[derive(Debug, Clone)]
        struct CustomSessionData {
            data: String,
        }

        let mut session_data = CustomSessionData {
            data: "foo".to_owned(),
        };

        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

        user.set_session_data(session_data.clone());

        "bar".clone_into(&mut session_data.data);
        user.set_session_data(session_data);

        let session = user.get_session_data_unchecked::<CustomSessionData>();
        assert_eq!(session.data, "bar".to_string());
    }

    #[test]
    fn test_transaction_name_request_naming() {
        let configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

        // Test 1: TransactionOnly variant should NOT use transaction name for requests
        user.transaction_name = Some(TransactionName::TransactionOnly(Arc::from(
            "my_transaction",
        )));

        // Create a request without a name
        let request = GooseRequest::builder().path("/test/path").build();

        // When TransactionOnly is used, get_request_name should return the path
        let request_name = user.get_request_name(&request);
        assert_eq!(
            &*request_name, "/test/path",
            "TransactionOnly should not inherit name to requests"
        );

        // Test 2: InheritNameByRequests variant SHOULD use transaction name for requests
        user.transaction_name = Some(TransactionName::InheritNameByRequests(Arc::from(
            "inherited_name",
        )));

        let request_name = user.get_request_name(&request);
        assert_eq!(
            &*request_name, "inherited_name",
            "InheritNameByRequests should inherit name to requests"
        );

        // Test 3: Request with explicit name should always use its own name
        let named_request = GooseRequest::builder()
            .path("/test/path")
            .name("explicit_request_name")
            .build();

        let request_name = user.get_request_name(&named_request);
        assert_eq!(
            &*request_name, "explicit_request_name",
            "Explicitly named request should use its own name"
        );

        // Test 4: No transaction name should fall back to path
        user.transaction_name = None;

        let request_name = user.get_request_name(&request);
        assert_eq!(
            &*request_name, "/test/path",
            "Without transaction name, should use path"
        );
    }

    /// Build a minimal GooseRequestMetric for buffering tests.
    fn make_test_request_metric(name: &str) -> GooseRequestMetric {
        use crate::metrics::{GooseRawRequest, TransactionDetail};
        use std::sync::Arc;

        let raw = GooseRawRequest::new(GooseMethod::Get, "http://localhost/", vec![], "");
        GooseRequestMetric::new(
            raw,
            TransactionDetail {
                scenario_index: 0,
                scenario_name: Arc::from("TestScenario"),
                transaction_index: Some(0),
                transaction_name: TransactionName::InheritNameByRequests(Arc::from("tx")),
            },
            name,
            0,
            0,
        )
    }

    #[test]
    fn pending_request_buffers_and_flushes() {
        let server = MockServer::start();
        let mut user = setup_user(&server).unwrap();

        // Wire up a metrics channel so buffering is active.
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);

        let metric = make_test_request_metric("/a");

        // send_request_metric_to_parent should buffer, not send.
        user.send_request_metric_to_parent(metric).unwrap();
        assert!(user.pending_request.is_some());
        assert!(rx.try_recv().is_err(), "should not be on channel yet");

        // flush_pending_request should send and clear.
        user.flush_pending_request().unwrap();
        assert!(user.pending_request.is_none());
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, GooseMetric::Request(_)));
    }

    #[test]
    fn buffering_second_request_flushes_first() {
        let server = MockServer::start();
        let mut user = setup_user(&server).unwrap();

        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);

        let first = make_test_request_metric("/first");
        let second = make_test_request_metric("/second");

        // Buffer the first request.
        user.send_request_metric_to_parent(first).unwrap();
        assert!(rx.try_recv().is_err());

        // Buffering the second should flush the first onto the channel.
        user.send_request_metric_to_parent(second).unwrap();
        let msg = rx.try_recv().unwrap();
        match msg {
            GooseMetric::Request(r) => assert_eq!(r.name.as_ref(), "/first"),
            _ => panic!("expected Request"),
        }

        // The second is still pending.
        assert!(user.pending_request.is_some());
    }

    #[test]
    fn flush_pending_metrics_sends_both() {
        let server = MockServer::start();
        let mut user = setup_user(&server).unwrap();

        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);

        // Buffer a request and a transaction.
        user.pending_request = Some(make_test_request_metric("/x"));
        user.pending_transaction = Some(crate::metrics::TransactionMetric::new(
            0,
            0,
            0,
            "tx".into(),
            0,
        ));

        user.flush_pending_metrics().unwrap();
        assert!(user.pending_request.is_none());
        assert!(user.pending_transaction.is_none());

        // Two messages on the channel.
        let m1 = rx.try_recv().unwrap();
        let m2 = rx.try_recv().unwrap();
        assert!(matches!(m1, GooseMetric::Request(_)));
        assert!(matches!(m2, GooseMetric::Transaction(_)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn custom_method_display() {
        assert_eq!(GooseMethod::Custom.to_string(), "CUSTOM");
    }

    #[test]
    fn custom_get_request_builder_returns_error() {
        let server = MockServer::start();
        let user = setup_user(&server).unwrap();
        let result = user.get_request_builder(&GooseMethod::Custom, "/any/path");
        assert!(
            result.is_err(),
            "get_request_builder with GooseMethod::Custom must return Err"
        );
        match result.unwrap_err().as_ref() {
            TransactionError::Custom(msg) => {
                assert!(
                    msg.contains("record_custom_request"),
                    "error message should mention record_custom_request(), got: {}",
                    msg
                );
            }
            other => panic!("expected TransactionError::Custom, got: {:?}", other),
        }
    }

    // Creates a GooseUser with a live flume channel attached, returning both the user
    // and the receiving end of the channel so tests can inspect emitted metrics.
    fn setup_user_with_channel() -> (GooseUser, flume::Receiver<GooseMetric>) {
        let mut configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        configuration.co_mitigation = Some(GooseCoordinatedOmissionMitigation::Average);
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);
        (user, rx)
    }

    // Helper to extract a GooseRequestMetric from the channel, asserting a single
    // Request variant was sent.
    fn recv_request_metric(rx: &flume::Receiver<GooseMetric>) -> GooseRequestMetric {
        let metric = rx
            .try_recv()
            .expect("expected exactly one metric on the channel");
        assert!(
            rx.try_recv().is_err(),
            "expected no further metrics on the channel"
        );
        match metric {
            GooseMetric::Request(boxed) => *boxed,
            other => panic!("expected GooseMetric::Request, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn record_custom_request_success_metric_fields() {
        let (mut user, rx) = setup_user_with_channel();

        user.record_custom_request("TCP", "my_op", 42, true, None, None)
            .await
            .expect("record_custom_request should return Ok on success");

        // Flush the pending buffer to the channel before inspecting.
        user.flush_pending_request().unwrap();

        let m = recv_request_metric(&rx);
        assert_eq!(m.raw.method, GooseMethod::Custom);
        assert_eq!(m.raw.custom_method, "TCP");
        assert_eq!(m.raw.method_label(), "TCP");
        assert_eq!(m.raw.url, "my_op");
        assert_eq!(m.name.as_ref(), "my_op");
        assert_eq!(m.final_url, "my_op");
        assert_eq!(m.response_time, 42);
        assert_eq!(m.status_code, 0);
        assert!(m.success, "success flag should be true");
        assert!(m.error.is_empty(), "error should be empty on success");
    }

    #[tokio::test]
    async fn record_custom_request_failure_metric_fields() {
        let (mut user, rx) = setup_user_with_channel();

        // Explicit error message is preserved verbatim.
        user.record_custom_request("TCP", "my_op", 500, false, None, Some("connection refused"))
            .await
            .expect("record_custom_request should return Ok even when recording a failure");

        user.flush_pending_request().unwrap();

        let m = recv_request_metric(&rx);
        assert!(!m.success, "success flag should be false");
        assert_eq!(m.error, "connection refused");
        assert_eq!(m.response_time, 500);
    }

    #[tokio::test]
    async fn record_custom_request_failure_default_error() {
        let (mut user, rx) = setup_user_with_channel();

        // When error is None, the fallback message must be used (not an empty string).
        user.record_custom_request("TCP", "my_op", 500, false, None, None)
            .await
            .expect("record_custom_request should return Ok with None error");

        user.flush_pending_request().unwrap();

        let m = recv_request_metric(&rx);
        assert!(!m.success);
        assert_eq!(m.error, "TCP my_op: request failed");
    }

    #[tokio::test]
    async fn record_custom_request_status_code() {
        let (mut user, rx) = setup_user_with_channel();

        user.record_custom_request("GRPC", "rpc_call", 100, true, Some(200), None)
            .await
            .expect("record_custom_request should accept a status_code");

        user.flush_pending_request().unwrap();

        let m = recv_request_metric(&rx);
        assert_eq!(m.status_code, 200);
        assert_eq!(m.raw.custom_method, "GRPC");
        assert_eq!(m.raw.method_label(), "GRPC");
    }

    #[tokio::test]
    async fn record_custom_request_whitespace_method_rejected() {
        let (mut user, _rx) = setup_user_with_channel();

        let result = user
            .record_custom_request("MY METHOD", "my_op", 42, true, None, None)
            .await;
        assert!(
            result.is_err(),
            "method label with whitespace must return Err"
        );
    }

    #[tokio::test]
    async fn record_custom_request_empty_method_rejected() {
        let (mut user, _rx) = setup_user_with_channel();

        let result = user
            .record_custom_request("", "my_op", 42, true, None, None)
            .await;
        assert!(result.is_err(), "empty method label must return Err");
    }

    #[tokio::test]
    async fn record_custom_request_empty_name_rejected() {
        let (mut user, _rx) = setup_user_with_channel();

        let result = user
            .record_custom_request("TCP", "", 42, true, None, None)
            .await;
        assert!(result.is_err(), "empty name must return Err");
    }

    #[tokio::test]
    async fn record_custom_request_whitespace_only_name_rejected() {
        let (mut user, _rx) = setup_user_with_channel();

        let result = user
            .record_custom_request("TCP", "   ", 42, true, None, None)
            .await;
        assert!(result.is_err(), "whitespace-only name must return Err");
    }

    #[tokio::test]
    async fn record_custom_request_no_metrics_skips_channel() {
        let mut configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        configuration.co_mitigation = Some(GooseCoordinatedOmissionMitigation::Average);
        configuration.no_metrics = true;
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);

        user.record_custom_request("TCP", "my_op", 42, true, None, None)
            .await
            .expect("record_custom_request should return Ok when no_metrics is true");

        assert!(
            rx.try_recv().is_err(),
            "no metric should be sent to the channel when no_metrics is true"
        );
    }

    #[test]
    fn request_log_path_extracts_path() {
        // Plain path.
        assert_eq!(request_log_path("http://10.0.0.1/foo"), "/foo");
        // The query is not part of the path.
        assert_eq!(request_log_path("http://example.com/foo?x=1&y=2"), "/foo");
        // Nor is the port.
        assert_eq!(request_log_path("http://10.0.0.1:8080/a/b/"), "/a/b/");
        assert_eq!(request_log_path("http://10.0.0.1:8080"), "/");
        // An unparsable URL gives an empty path, as before.
        assert_eq!(request_log_path("not a url"), "");
        assert_eq!(request_log_path(""), "");

        // The status line built from it keeps its text, including the Debug quoting.
        assert_eq!(
            format!(
                "{:?}: status_code {}",
                request_log_path("http://127.0.0.1:8080/foo?x=1"),
                reqwest::StatusCode::OK
            ),
            "\"/foo\": status_code 200 OK"
        );
    }

    // --- Latency by second (#713) ---

    /// A successful request for `name` that started `elapsed` ms into the run.
    fn latency_request_metric(name: &str, elapsed: u64, response_time: u64) -> GooseRequestMetric {
        let raw = GooseRawRequest::new(GooseMethod::Get, "http://localhost/", vec![], "");
        let mut metric = GooseRequestMetric::new(
            raw,
            TransactionDetail {
                scenario_index: 0,
                scenario_name: Arc::from("LatencyScenario"),
                transaction_index: Some(0),
                transaction_name: TransactionName::InheritNameByRequests(Arc::from("tx")),
            },
            name,
            elapsed as u128,
            0,
        );
        metric.response_time = response_time;
        metric.status_code = 200;
        metric.success = true;
        metric
    }

    /// Each request's time is recorded once, in the batch's timing data for
    /// its name, whatever the dashboard keeps; the graph entry only counts it.
    #[test]
    fn accumulate_request_records_each_time_once() {
        let metric = latency_request_metric("/x", 3_400, 77);
        for args in [
            vec!["--dashboard"],
            vec!["--dashboard", "--no-percentiles-over-time"],
            vec!["--report-file", "report.html"],
            vec!["--report-file", "--no-percentiles-over-time"],
        ] {
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            let mut user =
                GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration)
                    .unwrap();
            user.metrics_batch = Some(GooseMetricBatch::new(0));
            if user.record_latency_series {
                user.accumulate_request_by_second(&metric).unwrap();
            } else {
                user.accumulate_request(&metric);
            }
            let batch = user.metrics_batch.take().unwrap();
            let mut expected = GooseRequestMetricTimingData::new(None);
            expected.record_time(77);
            assert_eq!(batch.requests["GET /x"].raw_data, expected, "{args:?}");
            if user.record_latency_series {
                // The batch's second, which its timing data stands for.
                assert_eq!(batch.second, Some(3), "{args:?}");
                assert!(batch.graph_request_data.is_empty(), "{:?}", args);
            } else {
                // Counted by second as on main.
                assert_eq!(batch.second, None, "{args:?}");
                let graph = &batch.graph_request_data[&("GET /x".to_string(), 3)];
                assert_eq!((graph.count, graph.total_time), (1, 77.0), "{args:?}");
            }
        }
    }

    /// A real stall, no wait time: the copy coordinated omission mitigation
    /// sends puts each synthetic time in the second its request would have
    /// started, inside the stall.
    #[tokio::test]
    async fn synthetic_latency_lands_in_the_stall() {
        let mut configuration = GooseConfiguration::parse_args_default(&["--dashboard"]).unwrap();
        configuration.co_mitigation = Some(GooseCoordinatedOmissionMitigation::Average);
        let mut user =
            GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);

        // Four quick iterations set the cadence, then one stalls.
        for _ in 0..4 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            user.update_request_cadence(0).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2_500)).await;
        user.update_request_cadence(0).await;

        // The first request of the iteration after the stall.
        let s = user.started.elapsed().as_millis() as u64;
        let request = latency_request_metric("/x", s, 5);
        user.coordinated_omission_mitigation(&request)
            .await
            .unwrap();
        user.flush_pending_request().unwrap();
        let copy = match rx.try_recv().expect("coordinated omission copy") {
            GooseMetric::Request(copy) => *copy,
            other => panic!("expected a request metric, got {:?}", other),
        };
        let stall = copy.coordinated_omission_elapsed;
        let cadence = copy.user_cadence;
        assert!(stall >= 2_500, "stall {}", stall);
        assert!(cadence > 0 && stall > 2 * cadence);

        let (_metrics_tx, metrics_rx) = flume::unbounded();
        let (_cmd_tx, cmd_rx) = flume::unbounded();
        let mut metrics = crate::metrics::GooseMetrics::default();
        metrics.coordinated_omission_metrics =
            Some(crate::metrics::CoordinatedOmissionMetrics::new(
                GooseCoordinatedOmissionMitigation::Average,
            ));
        let mut processor = crate::metrics::MetricsProcessor::new(
            metrics_rx,
            cmd_rx,
            metrics,
            crate::graph::GraphData::for_metrics_processor(
                configuration.records_latency_series(),
                configuration.records_synthetic_latency(),
                configuration.records_latency_columns(),
            ),
            configuration.clone(),
            Vec::new(),
            crate::config::GooseDefaults::default(),
            Arc::new(std::sync::Mutex::new(HashMap::new())),
            None,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            false,
        );
        processor.process_request_metric_for_test(&request);
        processor.process_request_metric_for_test(&copy);

        // Expected: t_k = E - k*c - r while t_k > r, in second (s - t_k) / 1000.
        let r = request.response_time;
        let mut expected: HashMap<usize, u64> = HashMap::new();
        let mut k = 1;
        while stall as i64 - (k * cadence) as i64 - r as i64 > r as i64 {
            let t = stall - k * cadence - r;
            assert!(s - t >= s.saturating_sub(stall) && s - t <= s);
            *expected.entry(((s - t) / 1000) as usize).or_default() += 1;
            k += 1;
        }
        assert!(
            !expected.is_empty(),
            "the stall must produce synthetic times"
        );

        let end = (s / 1000) as usize + 3;
        let window = processor
            .graph_data_for_test()
            .export_series_window_with_heatmap(300, Some(end), true, None);
        let series = processor
            .graph_data_for_test()
            .export_request_series("GET".into(), "/x".into(), 300, Some(end), true, None)
            .expect("series for /x");
        for heatmap in [window.heatmap.as_ref().expect("heatmap"), &series.heatmap] {
            for second in 0..=end as u64 {
                let (measured, synthetic) = heatmap.counts(second);
                assert_eq!(
                    synthetic,
                    expected.get(&(second as usize)).copied().unwrap_or(0),
                    "second {second}"
                );
                // The request itself is measured in its own second, and only
                // there.
                let own = u64::from(second == s / 1000);
                assert_eq!(measured, own, "second {second}");
            }
        }
    }

    // --- One recording per request, batches within one second (#713) ---

    /// One request of a seeded run: the user that sent it, its name, when it
    /// started, how long it took, and whether it failed.
    #[derive(Clone, Debug)]
    struct SeededRequest {
        user: usize,
        name: String,
        elapsed: u64,
        response_time: u64,
        failed: bool,
    }

    /// A seeded run: `users` users each sending requests one after another
    /// for `seconds` seconds over twelve names, with fast, slow and very slow
    /// responses (past the 60 second buckets), responses under a millisecond,
    /// and failures. Ordered by when each response arrived, as the metrics
    /// processor sees them.
    fn seeded_requests(seed: u64, users: usize, seconds: u64) -> Vec<SeededRequest> {
        let mut state = seed;
        let mut next = move |modulus: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % modulus
        };
        let mut requests = Vec::new();
        for user in 0..users {
            let mut elapsed = next(997);
            while elapsed < seconds * 1000 {
                let response_time = match next(200) {
                    0..=1 => 0,
                    2 => 61_000 + next(1_000),
                    3..=19 => 1_500 + next(6_500),
                    20..=59 => 60 + next(1_440),
                    _ => 1 + next(60),
                };
                requests.push(SeededRequest {
                    user,
                    name: format!("/p{}", next(12)),
                    elapsed,
                    response_time,
                    failed: next(100) < 3,
                });
                elapsed += response_time + next(30);
            }
        }
        requests.sort_by_key(|request| (request.elapsed + request.response_time, request.user));
        requests
    }

    fn seeded_metric(request: &SeededRequest) -> GooseRequestMetric {
        let mut metric =
            latency_request_metric(&request.name, request.elapsed, request.response_time);
        metric.user = request.user;
        if request.failed {
            metric.success = false;
            metric.status_code = 500;
            metric.error = "500 Internal Server Error".to_string();
        }
        metric
    }

    /// `count` users whose batches go to one channel.
    fn batching_users(
        configuration: &GooseConfiguration,
        count: usize,
    ) -> (Vec<GooseUser>, flume::Receiver<GooseMetric>) {
        // Building a client loads the root certificates, which on Linux dominated the audit,
        // and the audit never sends a request, so every user shares one client.
        static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
        let client = CLIENT.get_or_init(Client::new);
        let (tx, rx) = flume::unbounded();
        let users = (0..count)
            .map(|_| {
                let mut user = GooseUser::new(
                    0,
                    Arc::from(""),
                    "http://localhost:8080".parse().unwrap(),
                    configuration,
                    0,
                    Some(client.clone()),
                )
                .unwrap();
                user.weighted_users_index = 0;
                user.is_throttled = false;
                user.metrics_channel = Some(tx.clone());
                user.metrics_batch = Some(GooseMetricBatch::new(0));
                user
            })
            .collect();
        (users, rx)
    }

    /// A user's batch clock in a test: when its batch started, in
    /// milliseconds of a simulated run, so batches flush by count and age as
    /// they would at those times.
    #[derive(Default)]
    struct BatchClock {
        /// When the batch started, and the `batch_start` last given the user.
        started: Option<(u64, std::time::Instant)>,
    }

    impl BatchClock {
        /// Runs `step` with `user`'s batch start set so that its age reads as
        /// at `now` ms, then notes what it became: kept, cleared by a flush,
        /// or started at `now` by a new batch.
        fn at(&mut self, user: &mut GooseUser, now: u64, step: impl FnOnce(&mut GooseUser)) {
            if let Some((started, _)) = self.started {
                let instant = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(now - started))
                    .expect("monotonic clock far enough from its origin");
                user.batch_start = Some(instant);
                self.started = Some((started, instant));
            }
            step(user);
            self.started = match (user.batch_start, self.started) {
                (None, _) => None,
                (Some(start), Some((started, given))) if start == given => Some((started, given)),
                (Some(start), _) => Some((now, start)),
            };
        }

        /// Adds `metric` to `user`'s batch when its response arrives, at `now` ms.
        fn add(&mut self, user: &mut GooseUser, now: u64, metric: GooseRequestMetric) {
            self.at(user, now, |user| {
                user.pending_request = Some(metric);
                user.flush_pending_request().unwrap();
            });
        }

        /// The check a user thread makes while it has nothing to send, at `now` ms.
        fn check(&mut self, user: &mut GooseUser, now: u64) {
            self.at(user, now, |user| user.maybe_flush_batch().unwrap());
        }
    }

    fn received_batches(rx: &flume::Receiver<GooseMetric>) -> Vec<GooseMetricBatch> {
        rx.drain()
            .map(|metric| match metric {
                GooseMetric::Batch(batch) => *batch,
                other => panic!("expected a batch, got {:?}", other),
            })
            .collect()
    }

    /// Sends each request through its user's batch path when its response
    /// arrives, as user threads do, each user's batch flushing by count and
    /// age at the simulated times, then flushes every user's batch. Returns
    /// the batches in the order they were sent.
    fn batch_through_users(
        configuration: &GooseConfiguration,
        users: usize,
        requests: &[SeededRequest],
    ) -> Vec<GooseMetricBatch> {
        let (mut users, rx) = batching_users(configuration, users);
        let mut clocks: Vec<BatchClock> = users.iter().map(|_| BatchClock::default()).collect();
        for request in requests {
            clocks[request.user].add(
                &mut users[request.user],
                request.elapsed + request.response_time,
                seeded_metric(request),
            );
        }
        for user in users.iter_mut() {
            user.flush_batch().unwrap();
        }
        received_batches(&rx)
    }

    fn test_processor(configuration: &GooseConfiguration) -> crate::metrics::MetricsProcessor {
        let (_metrics_tx, metrics_rx) = flume::unbounded();
        let (_cmd_tx, cmd_rx) = flume::unbounded();
        crate::metrics::MetricsProcessor::new(
            metrics_rx,
            cmd_rx,
            crate::metrics::GooseMetrics::default(),
            crate::graph::GraphData::for_metrics_processor(
                configuration.records_latency_series(),
                configuration.records_synthetic_latency(),
                configuration.records_latency_columns(),
            ),
            configuration.clone(),
            Vec::new(),
            crate::config::GooseDefaults::default(),
            Arc::new(std::sync::Mutex::new(HashMap::new())),
            None,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            false,
        )
    }

    /// Seeded requests with, for one more user, a request ending exactly on
    /// each second boundary and one starting on it.
    fn seeded_requests_with_boundaries(
        seed: u64,
        users: usize,
        seconds: u64,
    ) -> Vec<SeededRequest> {
        let mut requests = seeded_requests(seed, users, seconds);
        for second in 1..seconds {
            for (elapsed, response_time) in [(second * 1000 - 1, 1), (second * 1000, 2)] {
                requests.push(SeededRequest {
                    user: users,
                    name: format!("/p{}", second % 12),
                    elapsed,
                    response_time,
                    failed: false,
                });
            }
        }
        requests.sort_by_key(|request| (request.elapsed + request.response_time, request.user));
        requests
    }

    /// The per second rings built from batches equal an independent
    /// reference built from each request's own time and second, for every
    /// second, all requests and each name: buckets, count, minimum, maximum
    /// and total, and the requests per second counts.
    #[test]
    fn per_second_rings_match_a_reference() {
        let users = 8;
        let configuration = GooseConfiguration::parse_args_default(&["--dashboard"]).unwrap();
        let requests = seeded_requests_with_boundaries(47, users, 40);
        let batches = batch_through_users(&configuration, users + 1, &requests);
        // Batches reach the next second, and slow requests start new ones.
        assert!(batches.iter().any(|batch| !batch.next_second.is_empty()));
        let mut processor = test_processor(&configuration);
        for batch in batches {
            processor.process_batch_for_test(batch);
        }

        let mut reference: HashMap<(Option<String>, usize), crate::graph::SecondHistogram> =
            HashMap::new();
        let mut counts: HashMap<(String, usize), u32> = HashMap::new();
        for request in &requests {
            let key = format!("GET {}", request.name);
            let second = (request.elapsed / 1000) as usize;
            for name in [None, Some(key.clone())] {
                reference
                    .entry((name, second))
                    .or_default()
                    .record(request.response_time);
            }
            *counts.entry((key, second)).or_default() += 1;
        }

        let graph = processor.graph_data_for_test();
        let last = requests.iter().map(|r| r.elapsed / 1000).max().unwrap() as usize;
        for second in 0..=last + 2 {
            assert_eq!(
                graph.latency_second_for_test(None, second),
                reference.get(&(None, second)).cloned(),
                "all requests, second {second}"
            );
            for name in 0..12 {
                let key = format!("GET /p{name}");
                assert_eq!(
                    graph.latency_second_for_test(Some(&key), second),
                    reference.get(&(Some(key.clone()), second)).cloned(),
                    "{key}, second {second}"
                );
                assert_eq!(
                    graph.requests_in_second_for_test(&key, second),
                    counts.get(&(key.clone(), second)).copied().unwrap_or(0),
                    "{key}, second {second}"
                );
            }
        }
        assert!(reference.len() > 300);
    }

    /// The seconds of the requests in a batch: its own second for `raw_data`
    /// and the next for `next_second`, by request count, and each individual
    /// request's.
    fn batch_seconds(batch: &GooseMetricBatch) -> Vec<(usize, usize)> {
        let mut seconds: std::collections::BTreeMap<usize, usize> = Default::default();
        if let Some(second) = batch.second {
            for entry in batch.requests.values() {
                *seconds.entry(second).or_default() += entry.raw_data.counter;
            }
            for next in batch.next_second.values() {
                *seconds.entry(second + 1).or_default() += next.counter;
            }
        }
        seconds
            .into_iter()
            .filter(|&(_, count)| count > 0)
            .collect()
    }

    /// Without the heatmap, every batch a user sends is the batch main's
    /// count and age rule sends for the same requests: the same boundaries
    /// and contents, no second, and, when per second graphs are kept, the
    /// same requests counted by second. At the end the user holds unsent
    /// exactly the requests main holds.
    fn assert_batches_follow_the_count_and_age_rule(configuration: &GooseConfiguration) {
        assert!(!configuration.records_latency_series());
        // One user for a minute: 10 s of responses under 5 ms, so batches
        // fill to 100, then mostly fast responses with slow ones between,
        // so batches go by age, and an idle gap of 1.6 s at 30 s. Times are
        // multiples of 5 ms, so no age sits within a millisecond of the
        // 250 ms limit.
        let mut state = 1_017u64;
        let mut next = move |modulus: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % modulus
        };
        let mut requests: Vec<SeededRequest> = Vec::new();
        let mut elapsed = 0;
        let mut idled = false;
        while elapsed < 60_000 {
            let response_time = match (elapsed < 10_000, next(20)) {
                (true, _) => 0,
                (false, 0) => 5 * (60 + next(400)),
                (false, _) => 5 * next(8),
            };
            requests.push(SeededRequest {
                user: 0,
                name: format!("/p{}", next(4)),
                elapsed,
                response_time,
                failed: next(50) == 0,
            });
            elapsed += response_time + 5 * next(2);
            if !idled && elapsed >= 30_000 {
                elapsed += 1_600;
                idled = true;
            }
        }
        let (mut users, rx) = batching_users(configuration, 1);
        let mut clock = BatchClock::default();
        for request in &requests {
            clock.add(
                &mut users[0],
                request.elapsed + request.response_time,
                seeded_metric(request),
            );
        }
        let sent = received_batches(&rx);
        let held = users[0].batch_item_count;
        users[0].flush_batch().unwrap();
        let mut batches: Vec<GooseMetricBatch> = sent
            .into_iter()
            .chain(received_batches(&rx))
            .filter(|batch| !batch.requests.is_empty() || !batch.individual_requests.is_empty())
            .collect();

        // The rule: a batch goes when an added request makes it 100, or
        // when its first request was added 250 ms or more before.
        let mut expected: Vec<Vec<&SeededRequest>> = Vec::new();
        let mut current: Vec<&SeededRequest> = Vec::new();
        let mut started: Option<u64> = None;
        for request in &requests {
            let now = request.elapsed + request.response_time;
            current.push(request);
            let start = *started.get_or_insert(now);
            if current.len() >= METRICS_BATCH_SIZE
                || now - start >= METRICS_BATCH_MAX_AGE.as_millis() as u64
            {
                expected.push(std::mem::take(&mut current));
                started = None;
            }
        }
        // Main holds what is left; the user holds the same.
        assert_eq!(held, current.len());
        if !current.is_empty() {
            expected.push(current);
        }

        assert_eq!(batches.len(), expected.len());
        assert!(batches.len() > 80);
        // Both limits sent batches.
        assert!(expected
            .iter()
            .any(|batch| batch.len() == METRICS_BATCH_SIZE));
        assert!(expected
            .iter()
            .any(|batch| batch.len() < METRICS_BATCH_SIZE / 2));
        for (batch, expected) in batches.iter_mut().zip(&expected) {
            assert_eq!(batch.second, None);
            assert!(batch.next_second.is_empty());
            let mut times: HashMap<String, GooseRequestMetricTimingData> = HashMap::new();
            let mut graph: HashMap<(String, usize), (u32, f64)> = HashMap::new();
            let mut failed: Vec<u64> = Vec::new();
            for request in expected {
                if request.failed {
                    failed.push(request.elapsed);
                } else {
                    let key = format!("GET {}", request.name);
                    times
                        .entry(key.clone())
                        .or_default()
                        .record_time(request.response_time);
                    if configuration.records_graph_data() {
                        let entry = graph
                            .entry((key, (request.elapsed / 1000) as usize))
                            .or_default();
                        entry.0 += 1;
                        entry.1 += request.response_time as f64;
                    }
                }
            }
            assert_eq!(batch.requests.len(), times.len());
            for (key, entry) in &batch.requests {
                // A batch keeps its buckets in the order the times came;
                // merged, they are the same timing data.
                let mut merged = GooseRequestMetricTimingData::new(None);
                merged.merge(&entry.raw_data);
                assert_eq!(merged, times[key], "{key}");
                assert_eq!(
                    entry.status_code_counts,
                    HashMap::from([(200, times[key].counter)])
                );
            }
            let counted: HashMap<(String, usize), (u32, f64)> = batch
                .graph_request_data
                .drain()
                .map(|(key, data)| (key, (data.count, data.total_time)))
                .collect();
            assert_eq!(counted, graph);
            let individual: Vec<u64> = batch
                .individual_requests
                .iter()
                .map(|request| request.elapsed)
                .collect();
            assert_eq!(individual, failed);
        }
    }

    /// With per second graphs off, batches are main's.
    #[test]
    fn batches_follow_the_count_and_age_rule_without_graphs() {
        let configuration = GooseConfiguration::parse_args_default::<&str>(&[]).unwrap();
        assert!(!configuration.records_graph_data());
        assert_batches_follow_the_count_and_age_rule(&configuration);
    }

    /// With a report, where per second graphs are kept, or the dashboard,
    /// but no heatmap, batches are main's too: the heatmaps alone split
    /// them by second (#713, #746).
    #[test]
    fn batches_follow_the_count_and_age_rule_without_the_heatmap() {
        for args in [
            vec!["--report-file", "report.html", "--no-percentiles-over-time"],
            vec!["--dashboard", "--no-percentiles-over-time"],
        ] {
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            assert!(configuration.records_graph_data());
            assert_batches_follow_the_count_and_age_rule(&configuration);
        }
    }

    /// The requests a user has sent so far, from the batches on `rx`.
    fn requests_sent(rx: &flume::Receiver<GooseMetric>) -> usize {
        received_batches(rx)
            .iter()
            .map(|batch| {
                batch
                    .requests
                    .values()
                    .map(|entry| entry.raw_data.counter)
                    .chain(batch.next_second.values().map(|next| next.counter))
                    .sum::<usize>()
                    + batch.individual_requests.len()
            })
            .sum()
    }

    /// With either heatmap on, the dashboard's or, at default flags, the
    /// report's, a user's last request in a quiet second leaves on the same
    /// flush main would use: after every request, the user has sent as many
    /// requests as main's count and age rule sends, so it holds none main
    /// would not (#713, #746).
    #[test]
    fn last_request_in_a_quiet_second_leaves_with_mains_flush() {
        for args in [vec!["--dashboard"], vec!["--report-file"]] {
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            last_request_in_a_quiet_second_leaves_with_mains_flush_for(&configuration);
        }
    }

    fn last_request_in_a_quiet_second_leaves_with_mains_flush_for(
        configuration: &GooseConfiguration,
    ) {
        assert!(configuration.records_latency_series());
        let (mut users, rx) = batching_users(configuration, 1);
        let mut clock = BatchClock::default();
        let mut sent = 0;
        // Main's rule: the open batch goes when an added request makes it
        // 100, or when its first request was added 250 ms or more before.
        let (mut main_open, mut main_started, mut main_sent) = (0, None, 0);
        // One request every few seconds, as a user with a wait time sends
        // them, then a burst across a second boundary, a slow request, and
        // quiet seconds again. (start, response time)
        let requests: [(u64, u64); 12] = [
            (1_000, 20),
            (6_400, 15),
            (12_900, 30),
            (13_100, 5),
            (13_980, 10),
            (14_005, 5),
            (14_100, 4_000),
            (18_120, 5),
            (21_500, 5),
            (21_900, 5),
            (27_000, 5),
            (35_000, 5),
        ];
        for (i, &(elapsed, response_time)) in requests.iter().enumerate() {
            let now = elapsed + response_time;
            clock.add(
                &mut users[0],
                now,
                latency_request_metric("/x", elapsed, response_time),
            );
            sent += requests_sent(&rx);
            main_open += 1;
            let start = *main_started.get_or_insert(now);
            if main_open >= METRICS_BATCH_SIZE
                || now - start >= METRICS_BATCH_MAX_AGE.as_millis() as u64
            {
                main_sent += main_open;
                main_open = 0;
                main_started = None;
            }
            assert_eq!(sent, main_sent, "after request {i}");
        }
        // Each quiet request went out with the next one, as on main.
        assert_eq!(main_sent, requests.len() - 1);
    }

    /// A batch never holds requests from more than two seconds: not after a
    /// user idles for 2 s, with or without a check while idle, and not after
    /// a slow request whose response came seconds after it started.
    #[test]
    fn no_batch_spans_more_than_two_seconds() {
        for args in [vec!["--dashboard"], vec!["--report-file"]] {
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            assert!(configuration.records_latency_series());
            no_batch_spans_more_than_two_seconds_for(&configuration);
        }
    }

    fn no_batch_spans_more_than_two_seconds_for(configuration: &GooseConfiguration) {
        let (mut users, rx) = batching_users(configuration, 1);
        let user = &mut users[0];
        let mut clock = BatchClock::default();
        let mut send = |user: &mut GooseUser, elapsed: u64, response_time: u64| {
            clock.add(
                user,
                elapsed + response_time,
                latency_request_metric("/x", elapsed, response_time),
            );
        };

        // Idle for 2 s with no check in between: the next request sends the
        // old batch before it is added, then goes itself, as main sends the
        // two together once the batch is past its age.
        send(user, 9_000, 5);
        assert!(rx.is_empty());
        send(user, 11_000, 5);
        assert_eq!(
            received_batches(&rx)
                .iter()
                .map(batch_seconds)
                .collect::<Vec<_>>(),
            [vec![(9, 1)], vec![(11, 1)]]
        );
        user.flush_batch().unwrap();
        assert!(rx.is_empty());

        // Idle for 2 s with checks: the batch goes 250 ms after it started.
        let mut clock = BatchClock::default();
        clock.add(user, 20_905, latency_request_metric("/x", 20_900, 5));
        clock.check(user, 21_100);
        assert!(rx.is_empty());
        clock.check(user, 21_160);
        assert_eq!(
            received_batches(&rx)
                .iter()
                .map(batch_seconds)
                .collect::<Vec<_>>(),
            [vec![(20, 1)]]
        );
        clock.add(user, 22_905, latency_request_metric("/x", 22_900, 5));
        user.flush_batch().unwrap();
        assert_eq!(
            received_batches(&rx)
                .iter()
                .map(batch_seconds)
                .collect::<Vec<_>>(),
            [vec![(22, 1)]]
        );

        // Across a second boundary within 250 ms: one batch, two seconds.
        let mut clock = BatchClock::default();
        clock.add(user, 30_905, latency_request_metric("/x", 30_900, 5));
        clock.add(user, 31_010, latency_request_metric("/x", 31_000, 10));
        user.flush_batch().unwrap();
        assert_eq!(
            received_batches(&rx)
                .iter()
                .map(batch_seconds)
                .collect::<Vec<_>>(),
            [vec![(30, 1), (31, 1)]]
        );

        // A slow request is added when it ends, 5 s after it started: the
        // request after it starts a new batch.
        let mut clock = BatchClock::default();
        clock.add(user, 46_000, latency_request_metric("/x", 41_000, 5_000));
        clock.add(user, 46_020, latency_request_metric("/x", 46_005, 15));
        user.flush_batch().unwrap();
        assert_eq!(
            received_batches(&rx)
                .iter()
                .map(batch_seconds)
                .collect::<Vec<_>>(),
            [vec![(41, 1)], vec![(46, 1)]]
        );

        // Seeded users: every batch holds at most two seconds, and every
        // request is in one.
        let requests = seeded_requests_with_boundaries(713, 8, 40);
        let batches = batch_through_users(configuration, 9, &requests);
        let mut sent = 0;
        for batch in &batches {
            let seconds = batch_seconds(batch);
            assert!(seconds.len() <= 2, "a batch holds seconds {:?}", seconds);
            sent += seconds.iter().map(|&(_, count)| count).sum::<usize>()
                + batch.individual_requests.len();
        }
        assert_eq!(sent, requests.len());
    }

    /// The report graphs' requests per second and average response time per
    /// second, which batches give from each name's timing data for the
    /// batch's second and the next, equal the requests themselves: every request counted in
    /// the second it started, and the mean of their times.
    #[test]
    fn per_second_graphs_come_from_the_batch() {
        // With the recorder on, and with it off.
        for args in [
            vec!["--report-file", "audit.html"],
            vec!["--report-file", "audit.html", "--no-percentiles-over-time"],
        ] {
            let configuration = GooseConfiguration::parse_args_default(&args).unwrap();
            per_second_graphs_come_from_the_batch_for(&configuration);
        }
    }

    fn per_second_graphs_come_from_the_batch_for(configuration: &GooseConfiguration) {
        assert!(configuration.records_graph_data());
        let requests = seeded_requests(2027, 8, 30);
        let mut processor = test_processor(configuration);
        for batch in batch_through_users(configuration, 8, &requests) {
            processor.process_batch_for_test(batch);
        }
        let graph = processor.graph_data_for_test();
        let mut by_second: HashMap<(String, usize), Vec<u64>> = HashMap::new();
        for request in &requests {
            by_second
                .entry((
                    format!("GET {}", request.name),
                    (request.elapsed / 1000) as usize,
                ))
                .or_default()
                .push(request.response_time);
        }
        assert!(by_second.len() > 200);
        for ((key, second), times) in &by_second {
            assert_eq!(
                graph.requests_in_second_for_test(key, *second) as usize,
                times.len(),
                "{} second {}",
                key,
                second
            );
            let mean = times.iter().sum::<u64>() as f64 / times.len() as f64;
            let average = graph
                .average_in_second_for_test(key, *second)
                .expect("an average") as f64;
            assert!(
                (average - mean).abs() <= mean.max(1.0) * 1e-5,
                "{} second {}: {} against {}",
                key,
                second,
                average,
                mean
            );
        }
    }

    /// Through the batch path, as users send them: after every batch the
    /// report heatmap's columns stay within their budgets, widening as the
    /// run goes on, and at the end they hold every request once, in the
    /// column of the second it started (#746).
    #[test]
    fn batches_keep_the_columns_within_their_budgets() {
        let configuration = GooseConfiguration::parse_args_default(&["--report-file"]).unwrap();
        assert!(configuration.records_latency_columns());
        // Only successes, so every time goes through the batches.
        let mut requests = seeded_requests(746, 8, 40);
        for request in requests.iter_mut() {
            request.failed = false;
        }
        let mut processor = test_processor(&configuration);
        *processor.graph_data_for_test() =
            crate::graph::GraphData::with_latency_budgets(false, 4, 2);
        for batch in batch_through_users(&configuration, 8, &requests) {
            processor.process_batch_for_test(batch);
            let graph = processor.graph_data_for_test();
            for (key, budget) in [(None, 4), (Some("GET /p0"), 2)] {
                if let Some((_, counts)) = graph.latency_columns_for_test(key) {
                    assert!(
                        counts.iter().all(|&(column, _)| column < budget),
                        "{:?} {:?}",
                        key,
                        counts
                    );
                }
            }
        }
        let graph = processor.graph_data_for_test();
        graph.finish_latency_columns();
        let (width, counts) = graph.latency_columns_for_test(None).unwrap();
        let last = requests
            .iter()
            .map(|request| request.elapsed / 1000)
            .max()
            .unwrap() as usize;
        assert!(
            width > 1 && last / width < 4 && last / (width / 2) >= 4,
            "width {} for second {}",
            width,
            last
        );
        let mut expected: std::collections::BTreeMap<usize, usize> = Default::default();
        for request in &requests {
            *expected
                .entry(request.elapsed as usize / 1000 / width)
                .or_default() += 1;
        }
        assert_eq!(counts, expected.into_iter().collect::<Vec<_>>());
    }

    /// A batch holding a request from the second after its own widens the
    /// columns for that second before it writes it (#746).
    #[test]
    fn a_batch_fits_its_next_second_in_the_columns() {
        let configuration = GooseConfiguration::parse_args_default(&["--report-file"]).unwrap();
        let requests = [(3_900, 20), (4_100, 20)].map(|(elapsed, response_time)| SeededRequest {
            user: 0,
            name: "/a".to_string(),
            elapsed,
            response_time,
            failed: false,
        });
        let batches = batch_through_users(&configuration, 1, &requests);
        assert_eq!(batches.len(), 1);
        assert_eq!(batch_seconds(&batches[0]), [(3, 1), (4, 1)]);
        let mut processor = test_processor(&configuration);
        *processor.graph_data_for_test() =
            crate::graph::GraphData::with_latency_budgets(false, 4, 4);
        for batch in batches {
            processor.process_batch_for_test(batch);
        }
        let graph = processor.graph_data_for_test();
        graph.finish_latency_columns();
        assert_eq!(
            graph.latency_columns_for_test(None),
            Some((2, vec![(1, 1), (2, 1)]))
        );
    }

    /// Whole run results (every request's aggregate, errors, and all requests
    /// together with their status codes) are the same with percentiles over
    /// time collected, with per second graphs but no percentiles, with no
    /// graphs at all, and with every request processed one at a time.
    #[test]
    fn whole_run_results_match_with_collection_on_and_off() {
        let users = 8;
        let requests = seeded_requests_with_boundaries(2026, users, 40);
        let batched = |args: &[&str]| {
            let configuration = GooseConfiguration::parse_args_default(args).unwrap();
            let mut processor = test_processor(&configuration);
            for batch in batch_through_users(&configuration, users + 1, &requests) {
                processor.process_batch_for_test(batch);
            }
            processor
        };
        let on = batched(&["--dashboard"]);
        let others = [
            batched(&["--dashboard", "--no-percentiles-over-time"]),
            batched(&["--report-file", "audit.html", "--no-percentiles-over-time"]),
            // A report: the report's recorder and columns, no dashboard.
            batched(&["--report-file"]),
            batched(&[]),
            {
                let plain = GooseConfiguration::parse_args_default::<&str>(&[]).unwrap();
                let mut processor = test_processor(&plain);
                for request in &requests {
                    processor.process_request_metric_for_test(&seeded_metric(request));
                }
                processor
            },
        ];

        let aggregate = |processor: &crate::metrics::MetricsProcessor| {
            let mut times = GooseRequestMetricTimingData::new(None);
            let mut status_codes: HashMap<u16, usize> = HashMap::new();
            for request in processor.metrics_for_test().requests.values() {
                times.merge(&request.raw_data);
                for (code, count) in &request.status_code_counts {
                    *status_codes.entry(*code).or_default() += count;
                }
            }
            (times, status_codes)
        };
        let metrics = on.metrics_for_test();
        assert_eq!(metrics.requests.len(), 12);
        assert_eq!(aggregate(&on).0.counter, requests.len());
        for other in &others {
            assert_eq!(metrics.requests, other.metrics_for_test().requests);
            assert_eq!(metrics.errors, other.metrics_for_test().errors);
            assert_eq!(aggregate(&on), aggregate(other));
        }
    }

    // --- Whole run figures against an independent reference (#713) ---

    /// Every percentile Goose prints (console: 50, 75, 98, 99, 99.9 and
    /// 99.99), reports (50, 60, 70, 80, 90, 95, 99 and 100) or shows on the
    /// dashboard (50, 95 and 99), as an exact fraction and as Goose uses it.
    const AUDIT_PERCENTS: [(usize, usize, f32); 12] = [
        (50, 100, 0.5),
        (60, 100, 0.6),
        (70, 100, 0.7),
        (75, 100, 0.75),
        (80, 100, 0.8),
        (90, 100, 0.9),
        (95, 100, 0.95),
        (98, 100, 0.98),
        (99, 100, 0.99),
        (999, 1000, 0.999),
        (9999, 10000, 0.9999),
        (100, 100, 1.0),
    ];

    /// The audit's reference, written apart from Goose's code: nearest rank
    /// over the times as their buckets report them. The rank is `p` times
    /// the count, rounded half away from zero and at least 1. The first rank
    /// gives the fastest measured time and the last the slowest; any other
    /// rank gives its time clamped to them. The datasets make every time a
    /// whole number of milliseconds. Each is stored in microseconds
    /// and counts as the middle of the bucket that holds it.
    struct Reference {
        rounded: Vec<u64>,
        minimum: u64,
        maximum: u64,
        total: u64,
    }

    impl Reference {
        fn new(times: &[u64]) -> Self {
            let mut rounded: Vec<u64> = times
                .iter()
                .map(|&t| crate::metrics::buckets::bucket_value(t * 1000))
                .collect();
            rounded.sort_unstable();
            Reference {
                rounded,
                minimum: times.iter().copied().min().unwrap_or(0) * 1000,
                maximum: times.iter().copied().max().unwrap_or(0) * 1000,
                total: times.iter().sum::<u64>() * 1000,
            }
        }

        fn count(&self) -> usize {
            self.rounded.len()
        }

        /// The `numerator / denominator` percentile in microseconds, 0
        /// without times.
        fn percentile(&self, numerator: usize, denominator: usize) -> u64 {
            let n = self.rounded.len();
            if n == 0 {
                return 0;
            }
            // round(n * p), half away from zero, in integers.
            let rank = ((2 * n * numerator + denominator) / (2 * denominator)).max(1);
            // The fastest and slowest are kept exactly.
            if rank >= n {
                return self.maximum;
            }
            if rank == 1 {
                return self.minimum;
            }
            self.rounded[rank - 1].clamp(self.minimum, self.maximum)
        }

        /// The mean in milliseconds, 0 without times.
        fn average(&self) -> f64 {
            if self.count() == 0 {
                return 0.0;
            }
            self.total as f64 / self.count() as f64 / 1000.0
        }
    }

    /// A seeded source of the audit's datasets.
    struct AuditRng(u64);

    impl AuditRng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        /// Uniform in (0, 1].
        fn unit(&mut self) -> f64 {
            (self.next() as f64 + 1.0) / (1u64 << 53) as f64
        }

        /// A lognormal time with this median and sigma, in whole
        /// milliseconds as Goose measures them, at most 60 s.
        fn lognormal(&mut self, median: f64, sigma: f64) -> u64 {
            let z =
                (-2.0 * self.unit().ln()).sqrt() * (2.0 * std::f64::consts::PI * self.unit()).cos();
            (median * (sigma * z).exp()).min(60_000.0) as u64
        }
    }

    /// One request of an audit dataset.
    struct AuditRequest {
        user: usize,
        name: usize,
        time: u64,
        failed: bool,
    }

    /// The figures for one request name (or all of them) that the audit
    /// compares, from one place Goose prints or reports them, as that place
    /// shows them.
    #[derive(Debug, PartialEq)]
    struct Figures {
        percentiles: Vec<(String, String)>,
        minimum: String,
        maximum: String,
        average: String,
    }

    /// The reference's figures, each time in microseconds shown by `show`
    /// as the place compared shows it.
    fn figures(
        reference: &Reference,
        percents: &[(usize, usize, f32)],
        average: String,
        show: impl Fn(u64) -> String,
    ) -> Figures {
        Figures {
            percentiles: percents
                .iter()
                .map(|&(numerator, denominator, _)| {
                    (
                        format!("{}/{}", numerator, denominator),
                        show(reference.percentile(numerator, denominator)),
                    )
                })
                .collect(),
            minimum: show(reference.minimum),
            maximum: show(reference.maximum),
            average,
        }
    }

    /// Whole milliseconds, truncated, as the dashboard's `_ms` fields hold
    /// them.
    fn whole_ms(us: u64) -> String {
        (us / 1000).to_string()
    }

    /// Microseconds, as the reports keep them.
    fn micros(us: u64) -> String {
        us.to_string()
    }

    /// As the console prints a time, without thousands separators: the
    /// percentile table prints none.
    fn as_printed(us: u64) -> String {
        crate::metrics::format_micros(us, false)
    }

    /// A printed cell without thousands separators.
    fn unseparated(cell: &str) -> String {
        cell.replace(',', "")
    }

    fn goose_figures(
        percentiles: &[String],
        percents: &[(usize, usize, f32)],
        minimum: String,
        maximum: String,
        average: String,
    ) -> Figures {
        Figures {
            percentiles: percents
                .iter()
                .zip(percentiles)
                .map(|(&(numerator, denominator, _), value)| {
                    (format!("{}/{}", numerator, denominator), value.clone())
                })
                .collect(),
            minimum,
            maximum,
            average,
        }
    }

    /// One figure set that differs from the reference.
    struct Mismatch {
        what: String,
    }

    /// The console's response time and percentile tables, as printed.
    struct ConsoleTables<'a>(&'a crate::metrics::GooseMetrics);

    impl std::fmt::Display for ConsoleTables<'_> {
        fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fmt_response_times(fmt)?;
            self.0.fmt_percentiles(fmt)
        }
    }

    /// The printed rows of the table with this header, by name, each cell
    /// trimmed: the measured rows, or with `adjusted` the rows under
    /// "Adjusted for Coordinated Omission".
    fn console_rows(text: &str, header: &str, adjusted: bool) -> HashMap<String, Vec<String>> {
        const ADJUSTED: &str = "Adjusted for Coordinated Omission";
        let mut rows = HashMap::new();
        let mut inside = false;
        // A table prints its header once for measured times and, with
        // coordinated omission data, once more for adjusted ones.
        let mut seen = 0;
        for line in text.lines() {
            if line.contains(header) {
                inside = seen == usize::from(adjusted);
                seen += 1;
                continue;
            }
            if line.contains(ADJUSTED) {
                if inside {
                    break;
                }
                continue;
            }
            if !inside {
                continue;
            }
            if line.trim().is_empty() || line.contains("====") {
                break;
            }
            let cells: Vec<String> = line
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect();
            // The next table begins.
            if line.contains("Slowest page load") || (cells[0] == "Name" && !rows.is_empty()) {
                break;
            }
            if cells.len() < 2 || cells[0].starts_with('-') || cells[0] == "Name" {
                continue;
            }
            rows.insert(cells[0].clone(), cells[1..].to_vec());
        }
        rows
    }

    /// Feeds a dataset through users' batches (`accumulate_request` and
    /// `process_batch`), as user threads and the metrics processor do, and
    /// returns the processor and the synthetic times coordinated omission
    /// mitigation should add for each name.
    fn audit_run(
        requests: &[AuditRequest],
        names: usize,
        users: usize,
        stalls: &[(usize, usize, u64, u64)],
        rng: &mut AuditRng,
        collect: bool,
    ) -> (crate::metrics::MetricsProcessor, Vec<Vec<u64>>) {
        let args: &[&str] = if collect { &["--dashboard"] } else { &[] };
        let mut configuration = GooseConfiguration::parse_args_default(args).unwrap();
        let co = !stalls.is_empty();
        if co {
            configuration.co_mitigation = Some(GooseCoordinatedOmissionMitigation::Average);
        }
        let registry: crate::metrics::GooseRequestCounterRegistry =
            Arc::new(std::sync::Mutex::new(HashMap::new()));
        let (_metrics_tx, metrics_rx) = flume::unbounded();
        let (_cmd_tx, cmd_rx) = flume::unbounded();
        let mut metrics = crate::metrics::GooseMetrics::default();
        if co {
            metrics.coordinated_omission_metrics =
                Some(crate::metrics::CoordinatedOmissionMetrics::new(
                    GooseCoordinatedOmissionMitigation::Average,
                ));
        }
        let mut processor = crate::metrics::MetricsProcessor::new(
            metrics_rx,
            cmd_rx,
            metrics,
            crate::graph::GraphData::for_metrics_processor(
                configuration.records_latency_series(),
                configuration.records_synthetic_latency(),
                configuration.records_latency_columns(),
            ),
            configuration.clone(),
            Vec::new(),
            crate::config::GooseDefaults::default(),
            registry.clone(),
            None,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            false,
        );
        let (mut batching, rx) = batching_users(&configuration, users);
        for user in batching.iter_mut() {
            user.request_counters = Some(registry.clone());
        }
        let mut limits: Vec<usize> = (0..users).map(|_| 1 + rng.below(100) as usize).collect();
        let mut synthetic: Vec<Vec<u64>> = vec![Vec::new(); names];
        let feed = |user: &mut GooseUser,
                    metric: GooseRequestMetric,
                    limit: &mut usize,
                    rng: &mut AuditRng| {
            let (method, name, success) = (metric.raw.method, metric.name.clone(), metric.success);
            if metric.error.is_empty() && metric.coordinated_omission_elapsed == 0 {
                if user.record_latency_series {
                    user.accumulate_request_by_second(&metric).unwrap();
                } else {
                    user.accumulate_request(&metric);
                }
            } else {
                user.accumulate_individual_request(metric);
            }
            user.increment_request_counter(&method, &name, success);
            if user.batch_item_count >= *limit {
                user.flush_batch().unwrap();
                *limit = 1 + rng.below(100) as usize;
            }
        };
        for (i, request) in requests.iter().enumerate() {
            // Requests start 3 ms apart, so batches hold several seconds.
            let elapsed = 1_000 + 3 * i as u64;
            let mut metric =
                latency_request_metric(&format!("/audit{}", request.name), elapsed, request.time);
            if request.failed {
                metric.success = false;
                metric.status_code = 500;
                metric.error = "500 Internal Server Error".to_string();
            }
            let user = request.user;
            feed(&mut batching[user], metric, &mut limits[user], rng);
            // After some requests, a stall: the copy coordinated omission
            // mitigation sends for the next request.
            for &(after, name, elapsed, cadence) in stalls {
                if after == i {
                    let r = requests[i].time;
                    let mut copy = latency_request_metric(&format!("/audit{}", name), elapsed, r);
                    copy.coordinated_omission_elapsed = elapsed;
                    copy.user_cadence = cadence;
                    // Synthetic times as the book documents mitigation:
                    // the stall minus each missed cadence, less this
                    // request's own time, while longer than it.
                    let mut t = elapsed as i64 - cadence as i64 - r as i64;
                    while t > r as i64 {
                        synthetic[name].push(t as u64);
                        t -= cadence as i64;
                    }
                    feed(&mut batching[user], copy, &mut limits[user], rng);
                }
            }
        }
        for user in batching.iter_mut() {
            user.flush_batch().unwrap();
        }
        for batch in received_batches(&rx) {
            processor.process_batch_for_test(batch);
        }
        processor.sync_atomic_counters_for_test();
        (processor, synthetic)
    }

    /// For 200 seeded datasets of up to 1,000 requests, split across users
    /// and batches, every percentile Goose prints or reports, and the
    /// minimum, maximum and average, per request and for all requests, equal
    /// an independent reference; with coordinated omission mitigation, the
    /// adjusted ones equal the reference over measured and synthetic times
    /// together. Run with percentiles over time collected and not.
    #[test]
    fn whole_run_figures_match_a_reference() {
        for collect in [false, true] {
            audit_whole_run(&[1, 2, 3, 10, 1_000], 200, collect);
        }
    }

    /// The same audit with 100,000 request datasets added: 200 datasets,
    /// 3,333,531 requests. Too slow for every `cargo test`; run it with
    /// `cargo test --release --lib whole_run_figures_match_a_reference_full -- --ignored`.
    #[test]
    #[ignore]
    fn whole_run_figures_match_a_reference_full() {
        for collect in [false, true] {
            audit_whole_run(&[1, 2, 3, 10, 1_000, 100_000], 200, collect);
        }
    }

    /// Runs `datasets` seeded datasets, cycling through `sizes`, and checks
    /// every whole run figure on the dashboard, in the reports and on the
    /// console against an independent reference.
    fn audit_whole_run(sizes: &[usize], datasets: usize, collect: bool) {
        let mut rng = AuditRng(0x713);
        let mut checked = 0usize;
        let mut mismatches: Vec<Mismatch> = Vec::new();
        for dataset in 0..datasets {
            let size = sizes[dataset % sizes.len()];
            let distribution = dataset / sizes.len() % 3;
            let users = 1 + rng.below(8) as usize;
            let names = 1 + rng.below(3) as usize;
            let requests: Vec<AuditRequest> = (0..size)
                .map(|_| {
                    let time = match distribution {
                        0 => rng.below(60_001),
                        1 => rng.lognormal(50.0, 1.2),
                        _ if rng.below(2) == 0 => rng.lognormal(5.0, 0.5),
                        _ => rng.lognormal(800.0, 0.3),
                    };
                    AuditRequest {
                        user: rng.below(users as u64) as usize,
                        name: rng.below(names as u64) as usize,
                        time,
                        failed: rng.below(50) == 0,
                    }
                })
                .collect();
            // One dataset in four has stalls that mitigation backfills, each
            // with the copy of the request it follows.
            let stalls: Vec<(usize, usize, u64, u64)> = if dataset % 4 == 3 {
                (0..1 + size / 1_000)
                    .map(|_| {
                        let cadence = 20 + rng.below(200);
                        let after = rng.below(size as u64) as usize;
                        (
                            after,
                            requests[after].name,
                            cadence * (3 + rng.below(40)),
                            cadence,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let (processor, synthetic) =
                audit_run(&requests, names, users, &stalls, &mut rng, collect);
            let metrics = processor.metrics_for_test();

            // The references: each name's measured times, all of them, and
            // with stalls, measured and synthetic together.
            let mut by_name: Vec<Vec<u64>> = vec![Vec::new(); names];
            for request in &requests {
                by_name[request.name].push(request.time);
            }
            let all: Vec<u64> = requests.iter().map(|r| r.time).collect();
            let mut adjusted = all.clone();
            for times in &synthetic {
                adjusted.extend(times);
            }

            let check = |what: String,
                         goose: Figures,
                         reference: Figures,
                         mismatches: &mut Vec<Mismatch>| {
                if goose != reference {
                    mismatches.push(Mismatch {
                        what: format!(
                            "dataset {} (size {}, distribution {}, users {}, stalls {}, collect {}): {}\n  goose     {:?}\n  reference {:?}",
                            dataset, size, distribution, users, stalls.len(), collect, what, goose, reference
                        ),
                    });
                }
            };

            // 1. The dashboard snapshot.
            let snapshot = crate::metrics::dashboard_snapshot::build_dashboard_snapshot(
                crate::metrics::dashboard_snapshot::DashboardSnapshotInput {
                    metrics,
                    series: crate::metrics::dashboard_snapshot::SeriesWindow::empty(),
                    active_users: users,
                    maximum_users: users,
                    target_users: users,
                    total_users: users,
                    phase: "maintain".to_string(),
                    stopping: false,
                    series_window_secs: 300,
                    no_status_codes: false,
                    metrics_disabled: false,
                    no_transaction_metrics: false,
                    no_scenario_metrics: false,
                    report: crate::metrics::dashboard_snapshot::SnapshotReport::default(),
                    settings: crate::metrics::dashboard_snapshot::SnapshotSettings::idle(),
                    percentiles_over_time: false,
                    series_keys: std::collections::HashSet::new(),
                },
            );
            let dashboard_percents = [AUDIT_PERCENTS[0], AUDIT_PERCENTS[6], AUDIT_PERCENTS[8]];
            for row in &snapshot.requests {
                let name: usize = row.name.trim_start_matches("/audit").parse().unwrap();
                let reference = Reference::new(&by_name[name]);
                let p = &row.percentile_ms;
                check(
                    format!("dashboard row {}", row.name),
                    goose_figures(
                        &[p.p50, p.p95, p.p99].map(|ms| ms.to_string()),
                        &dashboard_percents,
                        row.response_time_min_ms.to_string(),
                        row.response_time_max_ms.to_string(),
                        format!("{:.6}", row.response_time_avg_ms),
                    ),
                    figures(
                        &reference,
                        &dashboard_percents,
                        format!("{:.6}", reference.average()),
                        whole_ms,
                    ),
                    &mut mismatches,
                );
            }
            let aggregate = &snapshot.aggregate;
            let reference = Reference::new(&all);
            let p = &aggregate.percentile_ms;
            check(
                "dashboard aggregate".to_string(),
                goose_figures(
                    &[p.p50, p.p95, p.p99].map(|ms| ms.to_string()),
                    &dashboard_percents,
                    aggregate.response_time_min_ms.to_string(),
                    aggregate.response_time_max_ms.to_string(),
                    format!("{:.6}", aggregate.response_time_avg_ms),
                ),
                figures(
                    &reference,
                    &dashboard_percents,
                    format!("{:.6}", reference.average()),
                    whole_ms,
                ),
                &mut mismatches,
            );
            if let Some(co) = aggregate.co_adjusted.as_ref() {
                let reference = Reference::new(&adjusted);
                let p = &co.percentile_ms;
                let mut goose = goose_figures(
                    &[p.p50, p.p95, p.p99].map(|ms| ms.to_string()),
                    &dashboard_percents,
                    String::new(),
                    co.response_time_max_ms.to_string(),
                    format!("{:.6}", co.response_time_avg_ms),
                );
                let mut expected = figures(
                    &reference,
                    &dashboard_percents,
                    format!("{:.6}", reference.average()),
                    whole_ms,
                );
                // The adjusted figures show no minimum.
                goose.minimum = String::new();
                expected.minimum = String::new();
                check(
                    "dashboard adjusted aggregate".to_string(),
                    goose,
                    expected,
                    &mut mismatches,
                );
            } else if !synthetic.iter().all(Vec::is_empty) {
                mismatches.push(Mismatch {
                    what: format!(
                        "dataset {}: no adjusted aggregate on the dashboard",
                        dataset
                    ),
                });
            }

            // 2. The reports (HTML, Markdown, JSON and PDF share this data).
            let report = crate::metrics::prepare_data_with_baseline(
                crate::metrics::AuditReportOptions {
                    no_transaction_metrics: true,
                    no_scenario_metrics: true,
                    no_status_codes: false,
                },
                metrics,
                None,
            );
            let report_percents = [
                AUDIT_PERCENTS[0],
                AUDIT_PERCENTS[1],
                AUDIT_PERCENTS[2],
                AUDIT_PERCENTS[4],
                AUDIT_PERCENTS[5],
                AUDIT_PERCENTS[6],
                AUDIT_PERCENTS[8],
                AUDIT_PERCENTS[11],
            ];
            let report_values = |m: &crate::report::ResponseMetric| -> Vec<String> {
                [
                    &m.percentile_50,
                    &m.percentile_60,
                    &m.percentile_70,
                    &m.percentile_80,
                    &m.percentile_90,
                    &m.percentile_95,
                    &m.percentile_99,
                    &m.percentile_100,
                ]
                .iter()
                .map(|value| micros(value.value().0))
                .collect()
            };
            for (response, request) in report.raw_response_metrics.iter().zip(
                report
                    .raw_request_metrics
                    .iter()
                    .filter(|r| !r.is_breakdown),
            ) {
                let reference = if response.name == "Aggregated" {
                    Reference::new(&all)
                } else {
                    let name: usize = response.name.trim_start_matches("/audit").parse().unwrap();
                    Reference::new(&by_name[name])
                };
                check(
                    format!("report {}", response.name),
                    goose_figures(
                        &report_values(response),
                        &report_percents,
                        micros(request.response_time_minimum.value().0),
                        micros(request.response_time_maximum.value().0),
                        // The mean, to the hundredth.
                        format!("{:.2}", request.response_time_average.value().0),
                    ),
                    figures(
                        &reference,
                        &report_percents,
                        format!("{:.2}", reference.average()),
                        micros,
                    ),
                    &mut mismatches,
                );
            }
            if let Some(co) = report.co_response_metrics.as_ref() {
                let aggregated = co.iter().find(|m| m.name == "Aggregated").unwrap();
                let reference = Reference::new(&adjusted);
                let mut goose = goose_figures(
                    &report_values(aggregated),
                    &report_percents,
                    String::new(),
                    String::new(),
                    String::new(),
                );
                let mut expected = figures(&reference, &report_percents, String::new(), micros);
                expected.minimum = String::new();
                expected.maximum = String::new();
                goose.minimum = String::new();
                check(
                    "report adjusted aggregate".to_string(),
                    goose,
                    expected,
                    &mut mismatches,
                );
            }

            // 3. The console tables, as printed.
            // The percentile table prints with the final metrics.
            let mut final_metrics = metrics.clone();
            final_metrics.final_metrics = true;
            let printed = ConsoleTables(&final_metrics).to_string();
            let times = console_rows(&printed, "Avg (ms)", false);
            let percentiles = console_rows(&printed, "50%", false);
            let console_percents = [
                AUDIT_PERCENTS[0],
                AUDIT_PERCENTS[3],
                AUDIT_PERCENTS[7],
                AUDIT_PERCENTS[8],
                AUDIT_PERCENTS[9],
                AUDIT_PERCENTS[10],
            ];
            let mut console_names: Vec<(String, Reference)> = (0..names)
                .filter(|&name| !by_name[name].is_empty())
                .map(|name| {
                    (
                        format!("GET /audit{}", name),
                        Reference::new(&by_name[name]),
                    )
                })
                .collect();
            if console_names.len() > 1 {
                console_names.push(("Aggregated".to_string(), Reference::new(&all)));
            }
            // The console's average, shown as every time is: to the
            // microsecond, then half up to the hundredth under 10 ms and to
            // whole milliseconds from there, with separators.
            let console_average = |reference: &Reference| {
                let us = (reference.average() * 1000.0).round() as u64;
                if (us + 5) / 10 < 1000 {
                    let text = format!("{:.2}", ((us + 5) / 10) as f64 / 100.0);
                    text.trim_end_matches('0').trim_end_matches('.').to_string()
                } else {
                    num_format::ToFormattedString::to_formatted_string(
                        &((us + 500) / 1000),
                        &num_format::Locale::en,
                    )
                }
            };
            for (name, reference) in &console_names {
                let (Some(time), Some(percentile)) = (times.get(name), percentiles.get(name))
                else {
                    mismatches.push(Mismatch {
                        what: format!("dataset {}: console has no row {}", dataset, name),
                    });
                    continue;
                };
                let mut values: Vec<String> = percentile.iter().map(|c| unseparated(c)).collect();
                // The response time table's median is a 50th percentile too.
                values.push(unseparated(&time[3]));
                let mut percents = console_percents.to_vec();
                percents.push(AUDIT_PERCENTS[0]);
                check(
                    format!("console {}", name),
                    goose_figures(
                        &values,
                        &percents,
                        unseparated(&time[1]),
                        unseparated(&time[2]),
                        time[0].clone(),
                    ),
                    figures(reference, &percents, console_average(reference), as_printed),
                    &mut mismatches,
                );
            }
            let has_synthetic = !synthetic.iter().all(Vec::is_empty);
            if has_synthetic && metrics.requests.len() > 1 {
                let times = console_rows(&printed, "Avg (ms)", true);
                let percentiles = console_rows(&printed, "50%", true);
                let reference = Reference::new(&adjusted);
                match (times.get("Aggregated"), percentiles.get("Aggregated")) {
                    (Some(time), Some(percentile)) => {
                        let mut values: Vec<String> =
                            percentile.iter().map(|c| unseparated(c)).collect();
                        values.push(unseparated(&time[3]));
                        let mut percents = console_percents.to_vec();
                        percents.push(AUDIT_PERCENTS[0]);
                        // The adjusted table shows a standard deviation where
                        // the measured one shows the minimum.
                        let mut expected = figures(
                            &reference,
                            &percents,
                            console_average(&reference),
                            as_printed,
                        );
                        expected.minimum = String::new();
                        check(
                            "console adjusted aggregate".to_string(),
                            goose_figures(
                                &values,
                                &percents,
                                String::new(),
                                unseparated(&time[2]),
                                time[0].clone(),
                            ),
                            expected,
                            &mut mismatches,
                        );
                    }
                    _ => mismatches.push(Mismatch {
                        what: format!(
                            "dataset {}: console has no adjusted Aggregated row",
                            dataset
                        ),
                    }),
                }
            }
            checked += size;
        }
        assert!(
            mismatches.is_empty(),
            "{} mismatches over {} requests:\n{}",
            mismatches.len(),
            checked,
            mismatches
                .iter()
                .map(|m| m.what.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    // --- Request names (#641) ---

    /// The request metric a request for `/a` records, named `request_name`
    /// if given, while `transaction` runs, with the fields that change from
    /// run to run fixed so its log lines can be compared.
    async fn request_metric_named(
        transaction: Option<TransactionName>,
        request_name: Option<&str>,
    ) -> GooseRequestMetric {
        request_metric_named_status(transaction, request_name, 200, None).await
    }

    /// [`request_metric_named`] when the server returns `status` and the
    /// request expects `expect`, if given.
    async fn request_metric_named_status(
        transaction: Option<TransactionName>,
        request_name: Option<&str>,
        status: u16,
        expect: Option<u16>,
    ) -> GooseRequestMetric {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/a");
            then.status(status);
        });
        let mut user = setup_user(&server).unwrap();
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);
        user.scenario_name = Arc::from("Scenario One");
        user.transaction_name = transaction;
        user.transaction_index = user.transaction_name.as_ref().map(|_| 3);
        let mut builder = GooseRequest::builder().path("/a");
        if let Some(name) = request_name {
            builder = builder.name(name);
        }
        if let Some(code) = expect {
            builder = builder.expect_status_code(code);
        }
        user.request(builder.build()).await.unwrap();
        user.flush_pending_request().unwrap();
        mock.assert();
        let mut metric = recv_request_metric(&rx);
        metric.elapsed = 1234;
        metric.response_time = 5;
        // The real request's microseconds vary from run to run.
        metric.response_time_fraction_us = 0;
        metric.raw.url = "http://target/a".to_string();
        metric.final_url = "http://target/a".to_string();
        metric
    }

    /// The request log line for `metric` in each format.
    fn request_log_lines(metric: &GooseRequestMetric) -> Vec<String> {
        use crate::logger::{GooseLogFormat, GooseLogger};
        [
            GooseLogFormat::Json,
            GooseLogFormat::Raw,
            GooseLogFormat::Csv,
        ]
        .iter()
        .map(|format| {
            let mut configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
            configuration.request_format = Some(format.clone());
            configuration.format_message(metric.clone())
        })
        .collect()
    }

    /// For each way a request is named, the request metric's names and its
    /// request log line in each format are what they were before the name
    /// was shared with the transaction.
    #[tokio::test]
    async fn request_names_and_log_lines_match_each_naming_mode() {
        // The mode, the transaction, the request's own name, the name it
        // records, and its JSON, raw and CSV request log lines.
        type NamingMode<'a> = (
            &'a str,
            Option<TransactionName>,
            Option<&'a str>,
            &'a str,
            [&'a str; 3],
        );
        let modes: Vec<NamingMode> = vec![
            (
                "inheriting",
                Some(TransactionName::InheritNameByRequests(Arc::from("Front Page"))),
                None,
                "Front Page",
                [
                    "{\"coordinated_omission_elapsed\":0,\"elapsed\":1234,\"error\":\"\",\"final_url\":\"http://target/a\",\"name\":\"Front Page\",\"raw\":{\"body\":\"\",\"custom_method\":\"\",\"headers\":[],\"method\":\"Get\",\"url\":\"http://target/a\"},\"redirected\":false,\"response_time\":5,\"response_time_fraction_us\":0,\"scenario_index\":0,\"scenario_name\":\"Scenario One\",\"status_code\":200,\"success\":true,\"transaction_index\":3,\"transaction_name\":{\"InheritNameByRequests\":\"Front Page\"},\"update\":false,\"user\":0,\"user_cadence\":0}",
                    "GooseRequestMetric { elapsed: 1234, scenario_index: 0, scenario_name: \"Scenario One\", transaction_index: 3, transaction_name: InheritNameByRequests(\"Front Page\"), raw: GooseRawRequest { method: Get, url: \"http://target/a\", headers: [], body: \"\", custom_method: \"\" }, name: \"Front Page\", final_url: \"http://target/a\", redirected: false, response_time: 5, status_code: 200, success: true, update: false, user: 0, error: \"\", coordinated_omission_elapsed: 0, user_cadence: 0, response_time_fraction_us: 0 }",
                    "1234,0,Scenario One,3,Front Page,\"GooseRawRequest { method: Get, url: \"\"http://target/a\"\", headers: [], body: \"\"\"\", custom_method: \"\"\"\" }\",Front Page,http://target/a,false,5,200,true,false,0,,0,0,5000",
                ],
            ),
            (
                "transaction only",
                Some(TransactionName::TransactionOnly(Arc::from("Front Page"))),
                None,
                "/a",
                [
                    "{\"coordinated_omission_elapsed\":0,\"elapsed\":1234,\"error\":\"\",\"final_url\":\"http://target/a\",\"name\":\"/a\",\"raw\":{\"body\":\"\",\"custom_method\":\"\",\"headers\":[],\"method\":\"Get\",\"url\":\"http://target/a\"},\"redirected\":false,\"response_time\":5,\"response_time_fraction_us\":0,\"scenario_index\":0,\"scenario_name\":\"Scenario One\",\"status_code\":200,\"success\":true,\"transaction_index\":3,\"transaction_name\":{\"TransactionOnly\":\"Front Page\"},\"update\":false,\"user\":0,\"user_cadence\":0}",
                    "GooseRequestMetric { elapsed: 1234, scenario_index: 0, scenario_name: \"Scenario One\", transaction_index: 3, transaction_name: TransactionOnly(\"Front Page\"), raw: GooseRawRequest { method: Get, url: \"http://target/a\", headers: [], body: \"\", custom_method: \"\" }, name: \"/a\", final_url: \"http://target/a\", redirected: false, response_time: 5, status_code: 200, success: true, update: false, user: 0, error: \"\", coordinated_omission_elapsed: 0, user_cadence: 0, response_time_fraction_us: 0 }",
                    "1234,0,Scenario One,3,,\"GooseRawRequest { method: Get, url: \"\"http://target/a\"\", headers: [], body: \"\"\"\", custom_method: \"\"\"\" }\",/a,http://target/a,false,5,200,true,false,0,,0,0,5000",
                ],
            ),
            (
                "request named",
                Some(TransactionName::InheritNameByRequests(Arc::from("Front Page"))),
                Some("named request"),
                "named request",
                [
                    "{\"coordinated_omission_elapsed\":0,\"elapsed\":1234,\"error\":\"\",\"final_url\":\"http://target/a\",\"name\":\"named request\",\"raw\":{\"body\":\"\",\"custom_method\":\"\",\"headers\":[],\"method\":\"Get\",\"url\":\"http://target/a\"},\"redirected\":false,\"response_time\":5,\"response_time_fraction_us\":0,\"scenario_index\":0,\"scenario_name\":\"Scenario One\",\"status_code\":200,\"success\":true,\"transaction_index\":3,\"transaction_name\":{\"InheritNameByRequests\":\"Front Page\"},\"update\":false,\"user\":0,\"user_cadence\":0}",
                    "GooseRequestMetric { elapsed: 1234, scenario_index: 0, scenario_name: \"Scenario One\", transaction_index: 3, transaction_name: InheritNameByRequests(\"Front Page\"), raw: GooseRawRequest { method: Get, url: \"http://target/a\", headers: [], body: \"\", custom_method: \"\" }, name: \"named request\", final_url: \"http://target/a\", redirected: false, response_time: 5, status_code: 200, success: true, update: false, user: 0, error: \"\", coordinated_omission_elapsed: 0, user_cadence: 0, response_time_fraction_us: 0 }",
                    "1234,0,Scenario One,3,Front Page,\"GooseRawRequest { method: Get, url: \"\"http://target/a\"\", headers: [], body: \"\"\"\", custom_method: \"\"\"\" }\",named request,http://target/a,false,5,200,true,false,0,,0,0,5000",
                ],
            ),
            (
                "path only",
                Some(TransactionName::TransactionOnly(Arc::from(""))),
                None,
                "/a",
                [
                    "{\"coordinated_omission_elapsed\":0,\"elapsed\":1234,\"error\":\"\",\"final_url\":\"http://target/a\",\"name\":\"/a\",\"raw\":{\"body\":\"\",\"custom_method\":\"\",\"headers\":[],\"method\":\"Get\",\"url\":\"http://target/a\"},\"redirected\":false,\"response_time\":5,\"response_time_fraction_us\":0,\"scenario_index\":0,\"scenario_name\":\"Scenario One\",\"status_code\":200,\"success\":true,\"transaction_index\":3,\"transaction_name\":{\"TransactionOnly\":\"\"},\"update\":false,\"user\":0,\"user_cadence\":0}",
                    "GooseRequestMetric { elapsed: 1234, scenario_index: 0, scenario_name: \"Scenario One\", transaction_index: 3, transaction_name: TransactionOnly(\"\"), raw: GooseRawRequest { method: Get, url: \"http://target/a\", headers: [], body: \"\", custom_method: \"\" }, name: \"/a\", final_url: \"http://target/a\", redirected: false, response_time: 5, status_code: 200, success: true, update: false, user: 0, error: \"\", coordinated_omission_elapsed: 0, user_cadence: 0, response_time_fraction_us: 0 }",
                    "1234,0,Scenario One,3,,\"GooseRawRequest { method: Get, url: \"\"http://target/a\"\", headers: [], body: \"\"\"\", custom_method: \"\"\"\" }\",/a,http://target/a,false,5,200,true,false,0,,0,0,5000",
                ],
            ),
            (
                "inheriting empty",
                Some(TransactionName::InheritNameByRequests(Arc::from(""))),
                None,
                "/a",
                [
                    "{\"coordinated_omission_elapsed\":0,\"elapsed\":1234,\"error\":\"\",\"final_url\":\"http://target/a\",\"name\":\"/a\",\"raw\":{\"body\":\"\",\"custom_method\":\"\",\"headers\":[],\"method\":\"Get\",\"url\":\"http://target/a\"},\"redirected\":false,\"response_time\":5,\"response_time_fraction_us\":0,\"scenario_index\":0,\"scenario_name\":\"Scenario One\",\"status_code\":200,\"success\":true,\"transaction_index\":3,\"transaction_name\":{\"InheritNameByRequests\":\"\"},\"update\":false,\"user\":0,\"user_cadence\":0}",
                    "GooseRequestMetric { elapsed: 1234, scenario_index: 0, scenario_name: \"Scenario One\", transaction_index: 3, transaction_name: InheritNameByRequests(\"\"), raw: GooseRawRequest { method: Get, url: \"http://target/a\", headers: [], body: \"\", custom_method: \"\" }, name: \"/a\", final_url: \"http://target/a\", redirected: false, response_time: 5, status_code: 200, success: true, update: false, user: 0, error: \"\", coordinated_omission_elapsed: 0, user_cadence: 0, response_time_fraction_us: 0 }",
                    "1234,0,Scenario One,3,,\"GooseRawRequest { method: Get, url: \"\"http://target/a\"\", headers: [], body: \"\"\"\", custom_method: \"\"\"\" }\",/a,http://target/a,false,5,200,true,false,0,,0,0,5000",
                ],
            ),
            (
                "no transaction",
                None,
                None,
                "/a",
                [
                    "{\"coordinated_omission_elapsed\":0,\"elapsed\":1234,\"error\":\"\",\"final_url\":\"http://target/a\",\"name\":\"/a\",\"raw\":{\"body\":\"\",\"custom_method\":\"\",\"headers\":[],\"method\":\"Get\",\"url\":\"http://target/a\"},\"redirected\":false,\"response_time\":5,\"response_time_fraction_us\":0,\"scenario_index\":0,\"scenario_name\":\"Scenario One\",\"status_code\":200,\"success\":true,\"transaction_index\":18446744073709551615,\"transaction_name\":{\"TransactionOnly\":\"\"},\"update\":false,\"user\":0,\"user_cadence\":0}",
                    "GooseRequestMetric { elapsed: 1234, scenario_index: 0, scenario_name: \"Scenario One\", transaction_index: 18446744073709551615, transaction_name: TransactionOnly(\"\"), raw: GooseRawRequest { method: Get, url: \"http://target/a\", headers: [], body: \"\", custom_method: \"\" }, name: \"/a\", final_url: \"http://target/a\", redirected: false, response_time: 5, status_code: 200, success: true, update: false, user: 0, error: \"\", coordinated_omission_elapsed: 0, user_cadence: 0, response_time_fraction_us: 0 }",
                    "1234,0,Scenario One,18446744073709551615,,\"GooseRawRequest { method: Get, url: \"\"http://target/a\"\", headers: [], body: \"\"\"\", custom_method: \"\"\"\" }\",/a,http://target/a,false,5,200,true,false,0,,0,0,5000",
                ],
            ),
        ];
        for (mode, transaction, request_name, name, lines) in modes {
            let metric = request_metric_named(transaction.clone(), request_name).await;
            assert_eq!(metric.name.as_ref(), name, "{mode}");
            assert_eq!(metric.scenario_name.as_ref(), "Scenario One", "{mode}");
            assert_eq!(
                metric.transaction_name,
                transaction.unwrap_or(TransactionName::default_value()),
                "{mode}"
            );
            assert_eq!(request_log_lines(&metric), lines, "{mode}");
        }
    }

    /// A request named after its transaction shares the transaction's name
    /// rather than copying it.
    #[tokio::test]
    async fn inherited_request_name_shares_the_transaction_name() {
        let name: Arc<str> = Arc::from("Front Page");
        let metric = request_metric_named(
            Some(TransactionName::InheritNameByRequests(name.clone())),
            None,
        )
        .await;
        assert!(Arc::ptr_eq(&metric.name, &name));
    }

    /// Every unnamed transaction shares one empty name, which serializes as
    /// it always has.
    #[test]
    fn default_transaction_name_is_shared() {
        let (TransactionName::TransactionOnly(first), TransactionName::TransactionOnly(second)) = (
            TransactionName::default_value(),
            TransactionName::default_value(),
        ) else {
            panic!("the default transaction name is TransactionOnly");
        };
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            serde_json::json!(TransactionName::default_value()).to_string(),
            r#"{"TransactionOnly":""}"#
        );
    }

    /// The error strings `request()` writes name the request as it is
    /// recorded, for a bad status, an unexpected status and a transport error.
    #[tokio::test]
    async fn request_error_strings_use_the_request_name() {
        let inherit = || {
            Some(TransactionName::InheritNameByRequests(Arc::from(
                "Front Page",
            )))
        };
        let m = request_metric_named_status(inherit(), None, 500, None).await;
        assert_eq!(m.error, "500 Internal Server Error: Front Page");
        let m = request_metric_named_status(inherit(), None, 200, Some(201)).await;
        assert_eq!(m.error, "200 OK: Front Page");
        let m = request_metric_named_status(inherit(), Some("named request"), 500, None).await;
        assert_eq!(m.error, "500 Internal Server Error: named request");
        let only = Some(TransactionName::TransactionOnly(Arc::from("Front Page")));
        let m = request_metric_named_status(only, None, 500, None).await;
        assert_eq!(m.error, "500 Internal Server Error: /a");

        // Nothing listens on a port once its listener is dropped.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut configuration = GooseConfiguration::parse_args_default(&EMPTY_ARGS).unwrap();
        configuration.co_mitigation = Some(GooseCoordinatedOmissionMitigation::Average);
        let base_url = format!("http://127.0.0.1:{port}/").parse().unwrap();
        let mut user = GooseUser::single(base_url, &configuration).unwrap();
        let (tx, rx) = flume::unbounded();
        user.metrics_channel = Some(tx);
        user.transaction_name = inherit();
        let _ = user
            .request(GooseRequest::builder().path("/a").build())
            .await;
        user.flush_pending_request().unwrap();
        let m = recv_request_metric(&rx);
        assert!(!m.success);
        assert!(
            m.error.starts_with("error sending request Front Page"),
            "{}",
            m.error
        );
    }

    /// A custom request inside an inheriting transaction keeps its own name.
    #[tokio::test]
    async fn record_custom_request_in_inheriting_transaction() {
        let (mut user, rx) = setup_user_with_channel();
        let transaction = TransactionName::InheritNameByRequests(Arc::from("Front Page"));
        user.transaction_name = Some(transaction.clone());
        user.record_custom_request("TCP", "my_op", 42, true, None, None)
            .await
            .unwrap();
        user.flush_pending_request().unwrap();
        let m = recv_request_metric(&rx);
        assert_eq!(m.name.as_ref(), "my_op");
        assert_eq!(m.transaction_name, transaction);
    }
}
