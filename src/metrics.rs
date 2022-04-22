//! Optional metrics collected and aggregated during load tests.
//!
//! By default, Goose collects a large number of metrics while performing a load test.
//! When [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) completes
//! it returns a [`GooseMetrics`] object.
//!
//! When the [`GooseMetrics`] object is viewed with [`std::fmt::Display`], the
//! contained [`TransactionMetrics`], [`GooseRequestMetrics`], and
//! [`GooseErrorMetrics`] are displayed in tables.

use crate::config::GooseDefaults;
use crate::goose::{get_base_url, GooseMethod, Scenario};
use crate::logger::GooseLog;
use crate::report;
use crate::test_plan::{TestPlanHistory, TestPlanStepAction};
use crate::util;
#[cfg(feature = "gaggle")]
use crate::worker::{self, GaggleMetrics};
use crate::{AttackMode, GooseAttack, GooseAttackRunState, GooseConfiguration, GooseError};
use chrono::prelude::*;
use http::StatusCode;
use itertools::Itertools;
use num_format::{Locale, ToFormattedString};
use regex::RegexSet;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::{f32, fmt};
use tokio::io::AsyncWriteExt;

/// Used to send metrics from [`GooseUser`](../goose/struct.GooseUser.html) threads
/// to the parent Goose process.
///
/// [`GooseUser`](../goose/struct.GooseUser.html) threads send these metrics to the
/// Goose parent process using an
/// [`unbounded Flume channel`](https://docs.rs/flume/*/flume/fn.unbounded.html).
///
/// The parent process will spend up to 80% of its time receiving and aggregating
/// these metrics. The parent process aggregates [`GooseRequestMetric`]s into
/// [`GooseRequestMetricAggregate`], [`TransactionMetric`]s into [`TransactionMetricAggregate`],
/// and [`GooseErrorMetric`]s into [`GooseErrorMetricAggregate`]. Aggregation happens in the
/// parent process so the individual [`GooseUser`](../goose/struct.GooseUser.html) threads
/// can spend all their time generating and validating load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GooseMetric {
    Request(GooseRequestMetric),
    Transaction(TransactionMetric),
}

/// THIS IS AN EXPERIMENTAL FEATURE, DISABLED BY DEFAULT. Optionally mitigate the loss of data
/// (coordinated omission) due to stalls on the upstream server.
///
/// Stalling can happen for many reasons, for example: garbage collection, a cache stampede,
/// even unrelated load on the same server. Without any mitigation, Goose loses
/// statistically relevant information as [`GooseUser`](../goose/struct.GooseUser.html)
/// threads are unable to make additional requests while they are blocked by an upstream stall.
/// Goose mitigates this by backfilling the requests that would have been made during that time.
/// Backfilled requests show up in the `--request-file` if enabled, though they were not actually
/// sent to the server.
///
/// Goose can be configured to backfill requests based on the expected
/// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence). The expected
/// cadence can be automatically calculated with any of the following configuration options.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum GooseCoordinatedOmissionMitigation {
    /// Backfill based on the average
    /// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence) for this
    /// [`GooseUser`](../goose/struct.GooseUser.html).
    Average,
    /// Backfill based on the maximum
    /// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence) for this
    /// [`GooseUser`](../goose/struct.GooseUser.html).
    Maximum,
    /// Backfill based on the minimum
    /// [`user_cadence`](struct.GooseRequestMetric.html#structfield.user_cadence) for this
    /// [`GooseUser`](../goose/struct.GooseUser.html).
    Minimum,
    /// Completely disable coordinated omission mitigation (default).
    Disabled,
}
/// Allow `--co-mitigation` from the command line using text variations on supported
/// `GooseCoordinatedOmissionMitigation`s by implementing [`FromStr`].
impl FromStr for GooseCoordinatedOmissionMitigation {
    type Err = GooseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Use a [`RegexSet`] to match string representations of `GooseCoordinatedOmissionMitigation`,
        // returning the appropriate enum value. Also match a wide range of abbreviations and synonyms.
        let co_mitigation = RegexSet::new(&[
            r"(?i)^(average|ave|aver|avg|mean)$",
            r"(?i)^(maximum|ma|max|maxi)$",
            r"(?i)^(minimum|mi|min|mini)$",
            r"(?i)^(disabled|di|dis|disable|none|no)$",
        ])
        .expect("failed to compile co_mitigation RegexSet");
        let matches = co_mitigation.matches(s);
        if matches.matched(0) {
            Ok(GooseCoordinatedOmissionMitigation::Average)
        } else if matches.matched(1) {
            Ok(GooseCoordinatedOmissionMitigation::Maximum)
        } else if matches.matched(2) {
            Ok(GooseCoordinatedOmissionMitigation::Minimum)
        } else if matches.matched(3) {
            Ok(GooseCoordinatedOmissionMitigation::Disabled)
        } else {
            Err(GooseError::InvalidOption {
                option: format!("GooseCoordinatedOmissionMitigation::{:?}", s),
                value: s.to_string(),
                detail:
                    "Invalid co_mitigation, expected: average, disabled, maximum, median, or minimum"
                        .to_string(),
            })
        }
    }
}

/// All requests made during a load test.
///
/// Goose optionally tracks metrics about requests made during a load test. The
/// metrics can be disabled with the `--no-metrics` run-time option, or with
/// [`GooseDefault::NoMetrics`](../config/enum.GooseDefault.html#variant.NoMetrics).
///
/// Aggregated requests ([`GooseRequestMetricAggregate`]) are stored in a HashMap
/// with they key `method request-name`, for example `GET /`.
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`GooseRequestMetrics`] are displayed in
/// a table:
/// ```text
/// === PER REQUEST METRICS ===
/// ------------------------------------------------------------------------------
/// Name                     |        # reqs |        # fails |    req/s |  fail/s
/// ------------------------------------------------------------------------------
/// GET (Anon) front page    |           438 |         0 (0%) |    43.80 |    0.00
/// GET (Anon) node page     |           296 |         0 (0%) |    29.60 |    0.00
/// GET (Anon) user page     |            90 |         0 (0%) |     9.00 |    0.00
/// GET (Auth) comment form  |            19 |         0 (0%) |     1.90 |    0.00
/// GET (Auth) front page    |           108 |         0 (0%) |    10.80 |    0.00
/// GET (Auth) node page     |            74 |         0 (0%) |     7.40 |    0.00
/// GET (Auth) user page     |            19 |         0 (0%) |     1.90 |    0.00
/// GET static asset         |         3,288 |         0 (0%) |   328.80 |    0.00
/// POST (Auth) comment form |            20 |         0 (0%) |     2.00 |    0.00
/// -------------------------+---------------+----------------+----------+--------
/// Aggregated               |         4,352 |         0 (0%) |   435.20 |    0.00
/// ------------------------------------------------------------------------------
/// Name                     |    Avg (ms) |        Min |        Max |      Median
/// ------------------------------------------------------------------------------
/// GET (Anon) front page    |       14.22 |          2 |         211 |         14
/// GET (Anon) node page     |       53.26 |          3 |          96 |         53
/// GET (Anon) user page     |       32.97 |         17 |         221 |         30
/// GET (Auth) comment form  |       54.32 |         36 |          88 |         50
/// GET (Auth) front page    |       39.02 |         25 |         232 |         38
/// GET (Auth) node page     |       52.08 |         36 |          81 |         51
/// GET (Auth) user page     |       31.21 |         25 |          40 |         31
/// GET static asset         |       11.55 |          3 |         217 |          8
/// POST (Auth) comment form |       54.30 |         41 |          73 |         52
/// -------------------------+-------------+------------+-------------+-----------
/// Aggregated               |       16.94 |          2 |         232 |         10
/// ------------------------------------------------------------------------------
/// Slowest page load within specified percentile of requests (in ms):
/// ------------------------------------------------------------------------------
/// Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
/// ------------------------------------------------------------------------------
/// GET (Anon) front page    |     14 |     18 |     30 |     43 |    210 |    210
/// GET (Anon) node page     |     53 |     62 |     78 |     86 |     96 |     96
/// GET (Anon) user page     |     30 |     33 |     43 |     53 |    220 |    220
/// GET (Auth) comment form  |     50 |     65 |     88 |     88 |     88 |     88
/// GET (Auth) front page    |     38 |     43 |     58 |     59 |    230 |    230
/// GET (Auth) node page     |     51 |     58 |     72 |     72 |     81 |     81
/// GET (Auth) user page     |     31 |     33 |     40 |     40 |     40 |     40
/// GET static asset         |      8 |     16 |     30 |     36 |    210 |    210
/// POST (Auth) comment form |     52 |     59 |     73 |     73 |     73 |     73
/// -------------------------+--------+--------+--------+--------+--------+-------
/// Aggregated               |     10 |     20 |     64 |     71 |    210 |    230
/// ```
pub type GooseRequestMetrics = HashMap<String, GooseRequestMetricAggregate>;

/// All transactions executed during a load test.
///
/// Goose optionally tracks metrics about transactions executed during a load test. The
/// metrics can be disabled with either the `--no-transaction-metrics` or the `--no-metrics`
/// run-time option, or with either
/// [`GooseDefault::NoTransactionMetrics`](../config/enum.GooseDefault.html#variant.NoTransactionMetrics) or
/// [`GooseDefault::NoMetrics`](../config/enum.GooseDefault.html#variant.NoMetrics).
///
/// Aggregated transactions ([`TransactionMetricAggregate`]) are stored in a Vector of Vectors
/// keyed to the order the transaction is created in the load test.
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`TransactionMetrics`] are displayed in
/// a table:
/// ```text
///  === PER TRANSACTION METRICS ===
/// ------------------------------------------------------------------------------
/// Name                     |   # times run |        # fails |  trans/s |  fail/s
/// ------------------------------------------------------------------------------
/// 1: AnonBrowsingUser      |
///   1: (Anon) front page   |           440 |         0 (0%) |    44.00 |    0.00
///   2: (Anon) node page    |           296 |         0 (0%) |    29.60 |    0.00
///   3: (Anon) user page    |            90 |         0 (0%) |     9.00 |    0.00
/// 2: AuthBrowsingUser      |
///   1: (Auth) login        |             0 |         0 (0%) |     0.00 |    0.00
///   2: (Auth) front page   |           109 |         0 (0%) |    10.90 |    0.00
///   3: (Auth) node page    |            74 |         0 (0%) |     7.40 |    0.00
///   4: (Auth) user page    |            19 |         0 (0%) |     1.90 |    0.00
///   5: (Auth) comment form |            20 |         0 (0%) |     2.00 |    0.00
/// -------------------------+---------------+----------------+----------+--------
/// Aggregated               |         1,048 |         0 (0%) |   104.80 |    0.00
/// ------------------------------------------------------------------------------
/// Name                     |    Avg (ms) |        Min |         Max |     Median
/// ------------------------------------------------------------------------------
/// 1: AnonBrowsingUser      |
///   1: (Anon) front page   |       94.41 |         59 |         294 |         88
///   2: (Anon) node page    |       53.29 |          3 |          96 |         53
///   3: (Anon) user page    |       33.02 |         17 |         221 |         30
/// 2: AuthBrowsingUser      |
///   1: (Auth) login        |        0.00 |          0 |           0 |          0
///   2: (Auth) front page   |      119.45 |         84 |         307 |        110
///   3: (Auth) node page    |       52.16 |         37 |          81 |         51
///   4: (Auth) user page    |       31.21 |         25 |          40 |         31
///   5: (Auth) comment form |      135.10 |        107 |         175 |        130
/// -------------------------+-------------+------------+-------------+-----------
/// Aggregated               |       76.78 |          3 |         307 |         74
/// ```
pub type TransactionMetrics = Vec<Vec<TransactionMetricAggregate>>;

/// All errors detected during a load test.
///
/// By default Goose tracks all errors detected during the load test. Each error is stored
/// as a [`GooseErrorMetricAggregate`](./struct.GooseErrorMetricAggregate.html), and they
/// are all stored together within a `BTreeMap` which is returned by
/// [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load test
/// completes.
///
/// `GooseErrorMetrics` can be disabled with the `--no-error-summary` run-time option, or with
/// [GooseDefault::NoErrorSummary](../config/enum.GooseDefault.html#variant.NoErrorSummary).
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`GooseErrorMetrics`] are displayed in
/// a table:
/// ```text
///  === ERRORS ===
/// ------------------------------------------------------------------------------
/// Count       | Error
/// ------------------------------------------------------------------------------
/// 924           GET (Auth) front page: 503 Service Unavailable: /
/// 715           POST (Auth) front page: 503 Service Unavailable: /user
/// 36            GET (Anon) front page: error sending request for url (http://example.com/): connection closed before message completed
/// ```
pub type GooseErrorMetrics = BTreeMap<String, GooseErrorMetricAggregate>;

/// For tracking and logging requests made during a load test.
///
/// The raw request that the GooseClient is making. Is included in the [`GooseRequestMetric`]
/// when metrics are enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseRawRequest {
    /// The method being used (ie, Get, Post, etc).
    pub method: GooseMethod,
    /// The full URL that was requested.
    pub url: String,
    /// Any headers set by the client when making the request.
    pub headers: Vec<String>,
    /// The body of the request made, if `--request-body` is enabled.
    pub body: String,
}
impl GooseRawRequest {
    pub(crate) fn new(
        method: GooseMethod,
        url: &str,
        headers: Vec<String>,
        body: &str,
    ) -> GooseRawRequest {
        GooseRawRequest {
            method,
            url: url.to_string(),
            headers,
            body: body.to_string(),
        }
    }
}

/// For tracking and counting requests made during a load test.
///
/// The request that Goose is making. User threads send this data to the parent thread
/// when metrics are enabled. This request object must be provided to calls to
/// [`set_success`](../goose/struct.GooseUser.html#method.set_success) or
/// [`set_failure`](../goose/struct.GooseUser.html#method.set_failure) so Goose
/// knows which request is being updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseRequestMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// The raw request that the GooseClient made.
    pub raw: GooseRawRequest,
    /// The optional name of the request.
    pub name: String,
    /// The final full URL that was requested, after redirects.
    pub final_url: String,
    /// How many milliseconds the request took.
    pub redirected: bool,
    /// How many milliseconds the request took.
    pub response_time: u64,
    /// The HTTP response code (optional).
    pub status_code: u16,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Whether or not we're updating a previous request, modifies how the parent thread records it.
    pub update: bool,
    /// Which [`GooseUser`](../goose/struct.GooseUser.html) thread processed the request.
    pub user: usize,
    /// The optional error caused by this request.
    pub error: String,
    /// If non-zero, Coordinated Omission Mitigation detected an abnormally long response time on
    /// the upstream server, blocking requests from being made.
    pub coordinated_omission_elapsed: u64,
    /// If non-zero, the calculated cadence of looping through all
    /// [`Transaction`](../goose/struct.Transaction.html)s by this
    /// [`GooseUser`](../goose/struct.GooseUser.html) thread.
    pub user_cadence: u64,
}
impl GooseRequestMetric {
    pub(crate) fn new(raw: GooseRawRequest, name: &str, elapsed: u128, user: usize) -> Self {
        GooseRequestMetric {
            elapsed: elapsed as u64,
            raw,
            name: name.to_string(),
            final_url: "".to_string(),
            redirected: false,
            response_time: 0,
            status_code: 0,
            success: true,
            update: false,
            user,
            error: "".to_string(),
            coordinated_omission_elapsed: 0,
            user_cadence: 0,
        }
    }

    // Record the final URL returned.
    pub(crate) fn set_final_url(&mut self, final_url: &str) {
        self.final_url = final_url.to_string();
        if self.final_url != self.raw.url {
            self.redirected = true;
        }
    }

    // Record how long the `response_time` took.
    pub(crate) fn set_response_time(&mut self, response_time: u128) {
        self.response_time = response_time as u64;
    }

    // Record the returned `status_code`.
    pub(crate) fn set_status_code(&mut self, status_code: Option<StatusCode>) {
        self.status_code = match status_code {
            Some(status_code) => status_code.as_u16(),
            None => 0,
        };
    }
}

/// Metrics collected about a method-path pair, (for example `GET /index`).
///
/// [`GooseRequestMetric`]s are sent by [`GooseUser`](../goose/struct.GooseUser.html)
/// threads to the Goose parent process where they are aggregated together into this
/// structure, and stored in [`GooseMetrics::requests`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GooseRequestMetricAggregate {
    /// The request path for which metrics are being collected.
    ///
    /// For example: "/".
    pub path: String,
    /// The method for which metrics are being collected.
    ///
    /// For example: [`GooseMethod::Get`].
    pub method: GooseMethod,
    /// The raw data seen from actual requests.
    pub raw_data: GooseRequestMetricTimingData,
    /// Combines the raw data with statistically generated Coordinated Omission Metrics.
    pub coordinated_omission_data: Option<GooseRequestMetricTimingData>,
    /// Per-status-code counters, tracking how often each response code was returned for this request.
    pub status_code_counts: HashMap<u16, usize>,
    /// Total number of times this path-method request resulted in a successful (2xx) status code.
    ///
    /// A count of how many requests resulted in a 2xx status code.
    pub success_count: usize,
    /// Total number of times this path-method request resulted in a non-successful (non-2xx) status code.
    ///
    /// A count of how many requests resulted in a non-2xx status code.
    pub fail_count: usize,
    /// Load test hash.
    ///
    /// The hash is primarily used when running a distributed Gaggle, allowing the Manager to confirm
    /// that all Workers are running the same load test plan.
    pub load_test_hash: u64,
}
impl GooseRequestMetricAggregate {
    /// Create a new GooseRequestMetricAggregate object.
    pub(crate) fn new(path: &str, method: GooseMethod, load_test_hash: u64) -> Self {
        trace!("new request");
        GooseRequestMetricAggregate {
            path: path.to_string(),
            method,
            raw_data: GooseRequestMetricTimingData::new(None),
            coordinated_omission_data: None,
            status_code_counts: HashMap::new(),
            success_count: 0,
            fail_count: 0,
            load_test_hash,
        }
    }

    pub(crate) fn record_time(&mut self, time_elapsed: u64, coordinated_omission_mitigation: bool) {
        // Only add time_elapsed to raw_data if the time wasn't generated by Coordinated
        // Omission Mitigation.
        if !coordinated_omission_mitigation {
            self.raw_data.record_time(time_elapsed);
        }

        // A Coordinated Omission data object already exists, add a new time into the data.
        if let Some(coordinated_omission_data) = self.coordinated_omission_data.as_mut() {
            coordinated_omission_data.record_time(time_elapsed);
        }
        // Create a new Coordinated Omission data object by cloning the raw data.
        else if coordinated_omission_mitigation {
            // If this time_elapsed was generated by Coordinated Omission Mitigation, it doesn't
            // exist in the raw_data, so add it.
            let mut coordinated_omission_data = self.raw_data.clone();
            coordinated_omission_data.record_time(time_elapsed);
            self.coordinated_omission_data = Some(coordinated_omission_data);
        }
    }

    /// Increment counter for status code, creating new counter if first time seeing status code.
    pub(crate) fn set_status_code(&mut self, status_code: u16) {
        let counter = match self.status_code_counts.get(&status_code) {
            // We've seen this status code before, increment counter.
            Some(c) => {
                debug!("got {:?} counter: {}", status_code, c);
                *c + 1
            }
            // First time we've seen this status code, initialize counter.
            None => {
                debug!("no match for counter: {}", status_code);
                1
            }
        };
        self.status_code_counts.insert(status_code, counter);
        debug!("incremented {} counter: {}", status_code, counter);
    }
}

/// Implement equality for GooseRequestMetricAggregate. We can't simply derive since
/// we have floats in the struct.
///
/// `Eq` trait has no functions on it - it is there only to inform the compiler
/// that this is an equivalence rather than a partial equivalence.
///
/// See <https://doc.rust-lang.org/std/cmp/trait.Eq.html> for more information.
impl Eq for GooseRequestMetricAggregate {}

/// Implement ordering for GooseRequestMetricAggregate.
impl Ord for GooseRequestMetricAggregate {
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.method, &self.path).cmp(&(&other.method, &other.path))
    }
}
/// Implement partial-ordering for GooseRequestMetricAggregate.
impl PartialOrd for GooseRequestMetricAggregate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Collects per-request timing metrics.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GooseRequestMetricTimingData {
    /// Per-response-time counters, tracking how often pages are returned with this response time.
    ///
    /// All response times between 1 and 100ms are stored without any rounding. Response times between
    /// 100 and 500ms are rounded to the nearest 10ms and then stored. Response times betwee 500 and
    /// 1000ms are rounded to the nearest 100ms. Response times larger than 1000ms are rounded to the
    /// nearest 1000ms.
    pub times: BTreeMap<usize, usize>,
    /// The shortest response time seen so far.
    ///
    /// For example a `min_response_time` of `3` means the quickest response for this method-path
    /// pair returned in 3 milliseconds. This value is not rounded.
    pub minimum_time: usize,
    /// The longest response time seen so far.
    ///
    /// For example a `max_response_time` of `2013` means the slowest response for this method-path
    /// pair returned in 2013 milliseconds. This value is not rounded.
    pub maximum_time: usize,
    /// Total combined response times seen so far.
    ///
    /// A running total of all response times returned for this method-path pair.
    pub total_time: usize,
    /// Total number of response times seen so far.
    ///
    /// A count of how many requests have been tracked for this method-path pair.
    pub counter: usize,
}
impl GooseRequestMetricTimingData {
    /// Create a new GooseRequestMetricAggregate object.
    pub(crate) fn new(metric_data: Option<&GooseRequestMetricTimingData>) -> Self {
        trace!("new GooseRequestMetricTimingData");
        // Simply clone the exiting metric_data.
        if let Some(data) = metric_data {
            data.clone()
        // Create a new empty metric_data.
        } else {
            GooseRequestMetricTimingData {
                times: BTreeMap::new(),
                minimum_time: 0,
                maximum_time: 0,
                total_time: 0,
                counter: 0,
            }
        }
    }

    /// Record a new time.
    pub(crate) fn record_time(&mut self, time_elapsed: u64) {
        // Perform this conversin only once, then re-use throughout this funciton.
        let time = time_elapsed as usize;

        // Update minimum if this one is fastest yet.
        if time > 0 && (self.minimum_time == 0 || time < self.minimum_time) {
            self.minimum_time = time;
        }

        // Update maximum if this one is slowest yet.
        if time > self.maximum_time {
            self.maximum_time = time;
        }

        // Update total time, adding in this one.
        self.total_time += time;

        // Each time we store a new time, increment counter by one.
        self.counter += 1;

        // Round the time so we can combine similar times together and
        // minimize required memory to store and push upstream to the parent.
        // No rounding for 1-100ms times.
        let rounded_time = if time_elapsed < 100 {
            time
        }
        // Round to nearest 10 for 100-500ms times.
        else if time_elapsed < 500 {
            ((time_elapsed as f64 / 10.0).round() * 10.0) as usize
        }
        // Round to nearest 100 for 500-1000ms times.
        else if time_elapsed < 1000 {
            ((time_elapsed as f64 / 100.0).round() * 100.0) as usize
        }
        // Round to nearest 1000 for all larger times.
        else {
            ((time_elapsed as f64 / 1000.0).round() * 1000.0) as usize
        };

        let counter = match self.times.get(&rounded_time) {
            // We've seen this elapsed time before, increment counter.
            Some(c) => {
                debug!("got {:?} counter: {}", rounded_time, c);
                *c + 1
            }
            // First time we've seen this elapsed time, initialize counter.
            None => {
                debug!("no match for counter: {}", rounded_time);
                1
            }
        };
        debug!("incremented {} counter: {}", rounded_time, counter);
        self.times.insert(rounded_time, counter);
    }
}

/// The per-transaction metrics collected each time a transaction is invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// An index into [`GooseAttack`]`.scenarios`, indicating which transaction set this is.
    pub scenario_index: usize,
    /// An index into [`Scenario`]`.transaction`, indicating which transaction this is.
    pub transaction_index: usize,
    /// The optional name of the transaction.
    pub name: String,
    /// How long transaction ran.
    pub run_time: u64,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Which GooseUser thread processed the request.
    pub user: usize,
}
impl TransactionMetric {
    /// Create a new TransactionMetric metric.
    pub(crate) fn new(
        elapsed: u128,
        scenario_index: usize,
        transaction_index: usize,
        name: String,
        user: usize,
    ) -> Self {
        TransactionMetric {
            elapsed: elapsed as u64,
            scenario_index,
            transaction_index,
            name,
            run_time: 0,
            success: true,
            user,
        }
    }

    /// Update a TransactionMetric metric.
    pub(crate) fn set_time(&mut self, time: u128, success: bool) {
        self.run_time = time as u64;
        self.success = success;
    }
}

/// Aggregated per-transaction metrics updated each time a transaction is invoked.
///
/// [`TransactionMetric`]s are sent by [`GooseUser`](../goose/struct.GooseUser.html)
/// threads to the Goose parent process where they are aggregated together into this
/// structure, and stored in [`GooseMetrics::transactions`].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionMetricAggregate {
    /// An index into [`GooseAttack`](../struct.GooseAttack.html)`.scenarios`,
    /// indicating which scenario this is.
    pub scenario_index: usize,
    /// The scenario name.
    pub scenario_name: String,
    /// An index into [`Scenario`](../goose/struct.Scenario.html)`.transaction`,
    /// indicating which transaction this is.
    pub transaction_index: usize,
    /// An optional name for the transaction.
    pub transaction_name: String,
    /// Per-run-time counters, tracking how often transactions take a given time to complete.
    pub times: BTreeMap<usize, usize>,
    /// The shortest run-time for this transaction.
    pub min_time: usize,
    /// The longest run-time for this transaction.
    pub max_time: usize,
    /// Total combined run-times for this transaction.
    pub total_time: usize,
    /// Total number of times transaction has run.
    pub counter: usize,
    /// Total number of times transaction has run successfully.
    pub success_count: usize,
    /// Total number of times transaction has failed.
    pub fail_count: usize,
}
impl TransactionMetricAggregate {
    /// Create a new TransactionMetricAggregate.
    pub(crate) fn new(
        scenario_index: usize,
        scenario_name: &str,
        transaction_index: usize,
        transaction_name: &str,
    ) -> Self {
        TransactionMetricAggregate {
            scenario_index,
            scenario_name: scenario_name.to_string(),
            transaction_index,
            transaction_name: transaction_name.to_string(),
            times: BTreeMap::new(),
            min_time: 0,
            max_time: 0,
            total_time: 0,
            counter: 0,
            success_count: 0,
            fail_count: 0,
        }
    }

    /// Track transaction function elapsed time in milliseconds.
    pub(crate) fn set_time(&mut self, time: u64, success: bool) {
        // Perform this conversion only once, then re-use throughout this function.
        let time_usize = time as usize;

        // Update minimum if this one is fastest yet.
        if self.min_time == 0 || time_usize < self.min_time {
            self.min_time = time_usize;
        }

        // Update maximum if this one is slowest yet.
        if time_usize > self.max_time {
            self.max_time = time_usize;
        }

        // Update total_time, adding in this one.
        self.total_time += time_usize;

        // Each time we store a new time, increment counter by one.
        self.counter += 1;

        if success {
            self.success_count += 1;
        } else {
            self.fail_count += 1;
        }

        // Round the time so we can combine similar times together and
        // minimize required memory to store and push upstream to the parent.
        let rounded_time = match time {
            // No rounding for times 0-100 ms.
            0..=100 => time_usize,
            // Round to nearest 10 for times 100-500 ms.
            101..=500 => ((time as f64 / 10.0).round() * 10.0) as usize,
            // Round to nearest 100 for times 500-1000 ms.
            501..=1000 => ((time as f64 / 100.0).round() * 10.0) as usize,
            // Round to nearest 1000 for larger times.
            _ => ((time as f64 / 1000.0).round() * 10.0) as usize,
        };

        let counter = match self.times.get(&rounded_time) {
            // We've seen this time before, increment counter.
            Some(c) => *c + 1,
            // First time we've seen this time, initialize counter.
            None => 1,
        };
        self.times.insert(rounded_time, counter);
        debug!("incremented {} counter: {}", rounded_time, counter);
    }
}
/// All metrics optionally collected during a Goose load test.
///
/// By default, Goose collects metrics during a load test in a `GooseMetrics` object
/// that is returned by
/// [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load
/// test finishes.
///
/// # Example
/// ```rust
/// use goose::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<(), GooseError> {
///     let goose_metrics: GooseMetrics = GooseAttack::initialize()?
///         .register_scenario(scenario!("ExampleUsers")
///             .register_transaction(transaction!(example_transaction))
///         )
///         // Set a default host so the load test will start.
///         .set_default(GooseDefault::Host, "http://localhost/")?
///         // Set a default run time so this test runs to completion.
///         .set_default(GooseDefault::RunTime, 1)?
///         .execute()
///         .await?;
///
///     // It is now possible to do something with the metrics collected by Goose.
///     // For now, we'll just pretty-print the entire object.
///     println!("{:#?}", goose_metrics);
///
///     /**
///     // For example:
///     $ cargo run -- -H http://example.com -u1 -t1
///     GooseMetrics {
///         hash: 0,
///         started: Some(
///             2021-06-15T09:32:49.888147+02:00,
///         ),
///         duration: 1,
///         users: 1,
///         requests: {
///             "GET /": GooseRequestMetricAggregate {
///                 path: "/",
///                 method: Get,
///                 response_times: {
///                     3: 14,
///                     4: 163,
///                     5: 36,
///                     6: 8,
///                 },
///                 min_response_time: 3,
///                 max_response_time: 6,
///                 total_response_time: 922,
///                 response_time_counter: 221,
///                 status_code_counts: {},
///                 success_count: 0,
///                 fail_count: 221,
///                 load_test_hash: 0,
///             },
///         },
///         transactions: [
///             [
///                 TransactionMetricAggregate {
///                     scenario_index: 0,
///                     scenario_name: "ExampleUsers",
///                     transaction_index: 0,
///                     transaction_name: "",
///                     times: {
///                         3: 14,
///                         4: 161,
///                         5: 38,
///                         6: 8,
///                     },
///                     min_time: 3,
///                     max_time: 6,
///                     total_time: 924,
///                     counter: 221,
///                     success_count: 221,
///                     fail_count: 0,
///                 },
///             ],
///         ],
///         errors: {
///             "503 Service Unavailable: /.GET./": GooseErrorMetric {
///                 method: Get,
///                 name: "/",
///                 error: "503 Service Unavailable: /",
///                 occurrences: 221,
///             },
///         },
///         final_metrics: true,
///         display_status_codes: false,
///         display_metrics: true,
///     }
///     **/
///
///     Ok(())
/// }
///
/// async fn example_transaction(user: &mut GooseUser) -> TransactionResult {
///     let _goose = user.get("/").await?;
///
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct GooseMetrics {
    /// A hash of the load test, primarily used to validate all Workers in a Gaggle
    /// are running the same load test.
    pub hash: u64,
    /// A vector recording the history of each load test step.
    pub history: Vec<TestPlanHistory>,
    /// Total number of seconds the load test ran.
    pub duration: usize,
    /// Total number of users simulated during this load test.
    ///
    /// This value may be smaller than what was configured at start time if the test
    /// didn't run long enough for all configured users to start.
    pub users: usize,
    /// Tracks details about each request made during the load test.
    ///
    /// Can be disabled with the `--no-metrics` run-time option, or with
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub requests: GooseRequestMetrics,
    /// Transactions details about each transaction that is invoked during the load test.
    ///
    /// Can be disabled with either the `--no-transaction-metrics` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoTransactionMetrics](../config/enum.GooseDefault.html#variant.NoTransactionMetrics) or
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub transactions: TransactionMetrics,
    /// Tracks and counts each time an error is detected during the load test.
    ///
    /// Can be disabled with either the `--no-error-summary` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoErrorSummary](../config/enum.GooseDefault.html#variant.NoErrorSummary) or
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub errors: GooseErrorMetrics,
    /// Tracks all hosts that the load test is run against.
    pub hosts: HashSet<String>,
    /// Flag indicating whether or not these are the final metrics, used to determine
    /// which metrics should be displayed. Defaults to false.
    pub(crate) final_metrics: bool,
    /// Flag indicating whether or not to display status_codes. Defaults to false.
    pub(crate) display_status_codes: bool,
    /// Flag indicating whether or not to display metrics. This defaults to false on
    /// Workers, otherwise true.
    pub(crate) display_metrics: bool,
}
impl GooseMetrics {
    /// Initialize the transaction_metrics vector, and determine which hosts are being
    /// load tested to display when printing metrics.
    pub(crate) fn initialize_transaction_metrics(
        &mut self,
        scenarios: &[Scenario],
        config: &GooseConfiguration,
        defaults: &GooseDefaults,
    ) -> Result<(), GooseError> {
        self.transactions = Vec::new();
        for scenario in scenarios {
            // Don't initialize transaction metrics if metrics or transaction_metrics are disabled.
            if !config.no_metrics {
                if !config.no_transaction_metrics {
                    let mut transaction_vector = Vec::new();
                    for transaction in &scenario.transactions {
                        transaction_vector.push(TransactionMetricAggregate::new(
                            scenario.scenarios_index,
                            &scenario.name,
                            transaction.transactions_index,
                            &transaction.name,
                        ));
                    }
                    self.transactions.push(transaction_vector);
                }

                // The host is not needed on the Worker, metrics are only printed on
                // the Manager.
                if !config.worker {
                    // Determine the base_url for this transaction based on which of the following
                    // are configured so metrics can be printed.
                    self.hosts.insert(
                        get_base_url(
                            // Determine if --host was configured.
                            if !config.host.is_empty() {
                                Some(config.host.to_string())
                            } else {
                                None
                            },
                            // Determine if the scenario defines a host.
                            scenario.host.clone(),
                            // Determine if there is a default host.
                            defaults.host.clone(),
                        )?
                        .to_string(),
                    );
                }
            }
        }

        Ok(())
    }

    /// Displays metrics while a load test is running.
    ///
    /// This function is invoked one time immediately after all GooseUsers are
    /// started, unless the `--no-reset-metrics` run-time option is enabled. It
    /// is invoked at regular intervals if the `--running-metrics` run-time
    /// option is enabled.
    pub(crate) fn print_running(&self) {
        if self.display_metrics {
            info!(
                "printing running metrics after {} seconds...",
                self.duration
            );

            // Include a blank line after printing running metrics.
            println!("{}", self);
        }
    }

    /// Optionally prepares a table of requests and fails.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_requests(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.requests.is_empty() {
            return Ok(());
        }

        // Display metrics from merged HashMap
        writeln!(
            fmt,
            "\n === PER REQUEST METRICS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>13} | {:>14} | {:>8} | {:>7}",
            "Name", "# reqs", "# fails", "req/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        for (request_key, request) in self.requests.iter().sorted() {
            let total_count = request.success_count + request.fail_count;
            let fail_percent = if request.fail_count > 0 {
                request.fail_count as f32 / total_count as f32 * 100.0
            } else {
                0.0
            };
            let (reqs, fails) =
                per_second_calculations(self.duration, total_count, request.fail_count);
            let reqs_precision = determine_precision(reqs);
            let fails_precision = determine_precision(fails);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if fail_percent as usize == 100 || fail_percent as usize == 0 {
                let fail_and_percent = format!(
                    "{} ({}%)",
                    request.fail_count.to_formatted_string(&Locale::en),
                    fail_percent as usize
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    util::truncate_string(request_key, 24),
                    total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                let fail_and_percent = format!(
                    "{} ({:.1}%)",
                    request.fail_count.to_formatted_string(&Locale::en),
                    fail_percent
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    util::truncate_string(request_key, 24),
                    total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            }
            aggregate_total_count += total_count;
            aggregate_fail_count += request.fail_count;
        }
        if self.requests.len() > 1 {
            let aggregate_fail_percent = if aggregate_fail_count > 0 {
                aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
            } else {
                0.0
            };
            writeln!(
                fmt,
                " -------------------------+---------------+----------------+----------+--------"
            )?;
            let (reqs, fails) =
                per_second_calculations(self.duration, aggregate_total_count, aggregate_fail_count);
            let reqs_precision = determine_precision(reqs);
            let fails_precision = determine_precision(fails);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
                let fail_and_percent = format!(
                    "{} ({}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent as usize
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                let fail_and_percent = format!(
                    "{} ({:.1}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of transactions.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_transactions(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.transactions.is_empty() || !self.display_metrics {
            return Ok(());
        }

        // Display metrics from transactions Vector
        writeln!(
            fmt,
            "\n === PER TRANSACTION METRICS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>13} | {:>14} | {:>8} | {:>7}",
            "Name", "# times run", "# fails", "trans/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        let mut transaction_count = 0;
        for scenario in &self.transactions {
            let mut displayed_scenario = false;
            for transaction in scenario {
                transaction_count += 1;
                let total_count = transaction.success_count + transaction.fail_count;
                let fail_percent = if transaction.fail_count > 0 {
                    transaction.fail_count as f32 / total_count as f32 * 100.0
                } else {
                    0.0
                };
                let (runs, fails) =
                    per_second_calculations(self.duration, total_count, transaction.fail_count);
                let runs_precision = determine_precision(runs);
                let fails_precision = determine_precision(fails);

                // First time through display name of scenario.
                if !displayed_scenario {
                    writeln!(
                        fmt,
                        " {:24 } |",
                        util::truncate_string(
                            &format!(
                                "{}: {}",
                                transaction.scenario_index + 1,
                                &transaction.scenario_name
                            ),
                            60
                        ),
                    )?;
                    displayed_scenario = true;
                }

                if fail_percent as usize == 100 || fail_percent as usize == 0 {
                    let fail_and_percent = format!(
                        "{} ({}%)",
                        transaction.fail_count.to_formatted_string(&Locale::en),
                        fail_percent as usize
                    );
                    writeln!(
                        fmt,
                        " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                        util::truncate_string(
                            &format!(
                                "  {}: {}",
                                transaction.transaction_index + 1,
                                transaction.transaction_name
                            ),
                            24
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        fail_and_percent,
                        runs,
                        fails,
                        runs_p = runs_precision,
                        fails_p = fails_precision,
                    )?;
                } else {
                    let fail_and_percent = format!(
                        "{} ({:.1}%)",
                        transaction.fail_count.to_formatted_string(&Locale::en),
                        fail_percent
                    );
                    writeln!(
                        fmt,
                        " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                        util::truncate_string(
                            &format!(
                                "  {}: {}",
                                transaction.transaction_index + 1,
                                transaction.transaction_name
                            ),
                            24
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        fail_and_percent,
                        runs,
                        fails,
                        runs_p = runs_precision,
                        fails_p = fails_precision,
                    )?;
                }
                aggregate_total_count += total_count;
                aggregate_fail_count += transaction.fail_count;
            }
        }
        if transaction_count > 1 {
            let aggregate_fail_percent = if aggregate_fail_count > 0 {
                aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
            } else {
                0.0
            };
            writeln!(
                fmt,
                " -------------------------+---------------+----------------+----------+--------"
            )?;
            let (runs, fails) =
                per_second_calculations(self.duration, aggregate_total_count, aggregate_fail_count);
            let runs_precision = determine_precision(runs);
            let fails_precision = determine_precision(fails);

            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
                let fail_and_percent = format!(
                    "{} ({}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent as usize
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    runs,
                    fails,
                    runs_p = runs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                let fail_and_percent = format!(
                    "{} ({:.1}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent
                );
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    fail_and_percent,
                    runs,
                    fails,
                    runs_p = runs_precision,
                    fails_p = fails_precision,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of transaction times.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_transaction_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.transactions.is_empty() || !self.display_metrics {
            return Ok(());
        }

        let mut aggregate_transaction_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_total_transaction_time: usize = 0;
        let mut aggregate_transaction_time_counter: usize = 0;
        let mut aggregate_min_transaction_time: usize = 0;
        let mut aggregate_max_transaction_time: usize = 0;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut transaction_count = 0;
        for scenario in &self.transactions {
            let mut displayed_scenario = false;
            for transaction in scenario {
                transaction_count += 1;
                // First time through display name of scenario.
                if !displayed_scenario {
                    writeln!(
                        fmt,
                        " {:24 } |",
                        util::truncate_string(
                            &format!(
                                "{}: {}",
                                transaction.scenario_index + 1,
                                &transaction.scenario_name
                            ),
                            60
                        ),
                    )?;
                    displayed_scenario = true;
                }

                // Iterate over user transaction times, and merge into global transaction times.
                aggregate_transaction_times =
                    merge_times(aggregate_transaction_times, transaction.times.clone());

                // Increment total transaction time counter.
                aggregate_total_transaction_time += &transaction.total_time;

                // Increment counter tracking individual transaction times seen.
                aggregate_transaction_time_counter += &transaction.counter;

                // If user had new fastest transaction time, update global fastest transaction time.
                aggregate_min_transaction_time =
                    update_min_time(aggregate_min_transaction_time, transaction.min_time);

                // If user had new slowest transaction` time, update global slowest transaction` time.
                aggregate_max_transaction_time =
                    update_max_time(aggregate_max_transaction_time, transaction.max_time);

                let average = match transaction.counter {
                    0 => 0.00,
                    _ => transaction.total_time as f32 / transaction.counter as f32,
                };
                let average_precision = determine_precision(average);

                writeln!(
                    fmt,
                    " {:<24} | {:>11.avg_precision$} | {:>10} | {:>11} | {:>10}",
                    util::truncate_string(
                        &format!(
                            "  {}: {}",
                            transaction.transaction_index + 1,
                            transaction.transaction_name
                        ),
                        24
                    ),
                    average,
                    format_number(transaction.min_time),
                    format_number(transaction.max_time),
                    format_number(util::median(
                        &transaction.times,
                        transaction.counter,
                        transaction.min_time,
                        transaction.max_time
                    )),
                    avg_precision = average_precision,
                )?;
            }
        }
        if transaction_count > 1 {
            let average = match aggregate_transaction_time_counter {
                0 => 0.00,
                _ => {
                    aggregate_total_transaction_time as f32
                        / aggregate_transaction_time_counter as f32
                }
            };
            let average_precision = determine_precision(average);

            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;
            writeln!(
                fmt,
                " {:<24} | {:>11.avg_precision$} | {:>10} | {:>11} | {:>10}",
                "Aggregated",
                average,
                format_number(aggregate_min_transaction_time),
                format_number(aggregate_max_transaction_time),
                format_number(util::median(
                    &aggregate_transaction_times,
                    aggregate_transaction_time_counter,
                    aggregate_min_transaction_time,
                    aggregate_max_transaction_time
                )),
                avg_precision = average_precision,
            )?;
        }

        Ok(())
    }

    /// Optionally prepares a table of response times.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_response_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.requests.is_empty() {
            return Ok(());
        }

        let mut aggregate_raw_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_raw_total_time: usize = 0;
        let mut aggregate_raw_counter: usize = 0;
        let mut aggregate_raw_min_time: usize = 0;
        let mut aggregate_raw_max_time: usize = 0;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // First display the raw data, as it always exists.
        let mut co_data = false;
        for (request_key, request) in self.requests.iter().sorted() {
            if !co_data && request.coordinated_omission_data.is_some() {
                co_data = true;
            }

            let raw_average = match request.raw_data.counter {
                0 => 0.0,
                _ => request.raw_data.total_time as f32 / request.raw_data.counter as f32,
            };
            let raw_average_precision = determine_precision(raw_average);

            // Merge in all times from this request into an aggregate.
            aggregate_raw_times = merge_times(aggregate_raw_times, request.raw_data.times.clone());
            // Increment total response time counter.
            aggregate_raw_total_time += &request.raw_data.total_time;
            // Increment counter tracking individual response times seen.
            aggregate_raw_counter += &request.raw_data.counter;
            // If user had new fastest response time, update global fastest response time.
            aggregate_raw_min_time =
                update_min_time(aggregate_raw_min_time, request.raw_data.minimum_time);
            // If user had new slowest response time, update global slowest response time.
            aggregate_raw_max_time =
                update_max_time(aggregate_raw_max_time, request.raw_data.maximum_time);

            writeln!(
                fmt,
                " {:<24} | {:>11.raw_avg_precision$} | {:>10} | {:>11} | {:>10}",
                util::truncate_string(request_key, 24),
                raw_average,
                format_number(request.raw_data.minimum_time),
                format_number(request.raw_data.maximum_time),
                format_number(util::median(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                )),
                raw_avg_precision = raw_average_precision,
            )?;
        }

        let raw_average = match aggregate_raw_counter {
            0 => 0.0,
            _ => aggregate_raw_total_time as f32 / aggregate_raw_counter as f32,
        };
        let raw_average_precision = determine_precision(raw_average);

        // Display aggregated data if there was more than one request.
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;
            writeln!(
                fmt,
                " {:<24} | {:>11.avg_precision$} | {:>10} | {:>11} | {:>10}",
                "Aggregated",
                raw_average,
                format_number(aggregate_raw_min_time),
                format_number(aggregate_raw_max_time),
                format_number(util::median(
                    &aggregate_raw_times,
                    aggregate_raw_counter,
                    aggregate_raw_min_time,
                    aggregate_raw_max_time
                )),
                avg_precision = raw_average_precision,
            )?;
        }

        // Nothing more to display if there was no Coordinated Omission data collected.
        if !co_data {
            return Ok(());
        }

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " Adjusted for Coordinated Omission:")?;

        let mut aggregate_co_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_co_total_time: usize = 0;
        let mut aggregate_co_counter: usize = 0;
        let mut aggregate_co_min_time: usize = 0;
        let mut aggregate_co_max_time: usize = 0;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
            "Name", "Avg (ms)", "Std Dev", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // Now display Coordinated Omission data.
        for (request_key, request) in self.requests.iter().sorted() {
            let co_average;
            let standard_deviation;
            let co_minimum;
            let co_maximum;
            if let Some(co_data) = request.coordinated_omission_data.as_ref() {
                let raw_average = match request.raw_data.counter {
                    0 => 0.0,
                    _ => request.raw_data.total_time as f32 / request.raw_data.counter as f32,
                };
                co_average = match co_data.counter {
                    0 => 0.0,
                    _ => co_data.total_time as f32 / co_data.counter as f32,
                };
                standard_deviation = util::standard_deviation(raw_average, co_average);
                aggregate_co_times = merge_times(aggregate_co_times, co_data.times.clone());
                aggregate_co_counter += co_data.counter;
                // If user had new fastest response time, update global fastest response time.
                aggregate_co_min_time =
                    update_min_time(aggregate_co_min_time, co_data.minimum_time);
                // If user had new slowest response time, update global slowest response time.
                aggregate_co_max_time =
                    update_max_time(aggregate_raw_max_time, co_data.maximum_time);
                aggregate_co_total_time += co_data.total_time;
                co_minimum = co_data.minimum_time;
                co_maximum = co_data.maximum_time;
            } else {
                co_average = 0.0;
                standard_deviation = 0.0;
                co_minimum = 0;
                co_maximum = 0;
            }
            let co_average_precision = determine_precision(co_average);
            let standard_deviation_precision = determine_precision(standard_deviation);

            // Coordinated Omission Mitigation was enabled for this request, display the extra data:
            if let Some(co_data) = request.coordinated_omission_data.as_ref() {
                writeln!(
                    fmt,
                    " {:<24} | {:>11.co_avg_precision$} | {:>10.sd_precision$} | {:>11} | {:>10}",
                    util::truncate_string(request_key, 24),
                    co_average,
                    standard_deviation,
                    format_number(co_maximum),
                    format_number(util::median(
                        &co_data.times,
                        co_data.counter,
                        co_minimum,
                        co_maximum,
                    )),
                    co_avg_precision = co_average_precision,
                    sd_precision = standard_deviation_precision,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<24} | {:>11} | {:>10} | {:>11} | {:>10}",
                    util::truncate_string(request_key, 24),
                    "-",
                    "-",
                    "-",
                    "-",
                )?;
            }
        }

        // Display aggregated Coordinate Omission data if there was more than one request.
        if self.requests.len() > 1 {
            let co_average = match aggregate_co_counter {
                0 => 0.0,
                _ => aggregate_co_total_time as f32 / aggregate_co_counter as f32,
            };
            let co_average_precision = determine_precision(co_average);
            let standard_deviation = util::standard_deviation(raw_average, co_average);
            let standard_deviation_precision = determine_precision(standard_deviation);

            writeln!(
                fmt,
                " -------------------------+-------------+------------+-------------+-----------"
            )?;

            writeln!(
                fmt,
                " {:<24} | {:>11.avg_precision$} | {:>10.sd_precision$} | {:>11} | {:>10}",
                "Aggregated",
                co_average,
                standard_deviation,
                format_number(aggregate_co_max_time),
                format_number(util::median(
                    &aggregate_co_times,
                    aggregate_co_counter,
                    aggregate_co_min_time,
                    aggregate_co_max_time
                )),
                avg_precision = co_average_precision,
                sd_precision = standard_deviation_precision,
            )?;
        }

        Ok(())
    }

    /// Optionally prepares a table of slowest response times within several percentiles.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_percentiles(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only include percentiles when displaying the final metrics report.
        if !self.final_metrics {
            return Ok(());
        }

        let mut raw_aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut raw_aggregate_total_response_time: usize = 0;
        let mut raw_aggregate_response_time_counter: usize = 0;
        let mut raw_aggregate_min_response_time: usize = 0;
        let mut raw_aggregate_max_response_time: usize = 0;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " Slowest page load within specified percentile of requests (in ms):"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        // Track whether or not Coordinated Omission Mitigation kicked in.
        let mut co_data = false;
        for (request_key, request) in self.requests.iter().sorted() {
            if !co_data && request.coordinated_omission_data.is_some() {
                co_data = true;
            }

            // Iterate over user response times, and merge into global response times.
            raw_aggregate_response_times =
                merge_times(raw_aggregate_response_times, request.raw_data.times.clone());

            // Increment total response time counter.
            raw_aggregate_total_response_time += &request.raw_data.total_time;

            // Increment counter tracking individual response times seen.
            raw_aggregate_response_time_counter += &request.raw_data.counter;

            // If user had new fastest response time, update global fastest response time.
            raw_aggregate_min_response_time = update_min_time(
                raw_aggregate_min_response_time,
                request.raw_data.minimum_time,
            );

            // If user had new slowest response time, update global slowest response time.
            raw_aggregate_max_response_time = update_max_time(
                raw_aggregate_max_response_time,
                request.raw_data.maximum_time,
            );
            // Sort response times so we can calculate a mean.
            writeln!(
                fmt,
                " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                util::truncate_string(request_key, 24),
                calculate_response_time_percentile(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                    0.5
                ),
                calculate_response_time_percentile(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                    0.75
                ),
                calculate_response_time_percentile(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                    0.98
                ),
                calculate_response_time_percentile(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                    0.99
                ),
                calculate_response_time_percentile(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                    0.999
                ),
                calculate_response_time_percentile(
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                    0.9999
                ),
            )?;
        }
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+--------+--------+--------+--------+--------+-------"
            )?;
            writeln!(
                fmt,
                " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                "Aggregated",
                calculate_response_time_percentile(
                    &raw_aggregate_response_times,
                    raw_aggregate_response_time_counter,
                    raw_aggregate_min_response_time,
                    raw_aggregate_max_response_time,
                    0.5
                ),
                calculate_response_time_percentile(
                    &raw_aggregate_response_times,
                    raw_aggregate_response_time_counter,
                    raw_aggregate_min_response_time,
                    raw_aggregate_max_response_time,
                    0.75
                ),
                calculate_response_time_percentile(
                    &raw_aggregate_response_times,
                    raw_aggregate_response_time_counter,
                    raw_aggregate_min_response_time,
                    raw_aggregate_max_response_time,
                    0.98
                ),
                calculate_response_time_percentile(
                    &raw_aggregate_response_times,
                    raw_aggregate_response_time_counter,
                    raw_aggregate_min_response_time,
                    raw_aggregate_max_response_time,
                    0.99
                ),
                calculate_response_time_percentile(
                    &raw_aggregate_response_times,
                    raw_aggregate_response_time_counter,
                    raw_aggregate_min_response_time,
                    raw_aggregate_max_response_time,
                    0.999
                ),
                calculate_response_time_percentile(
                    &raw_aggregate_response_times,
                    raw_aggregate_response_time_counter,
                    raw_aggregate_min_response_time,
                    raw_aggregate_max_response_time,
                    0.9999
                ),
            )?;
        }

        // If there's no Coordinated Omission Mitigation data to display, exit.
        if !co_data {
            return Ok(());
        }

        let mut co_aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut co_aggregate_total_response_time: usize = 0;
        let mut co_aggregate_response_time_counter: usize = 0;
        let mut co_aggregate_min_response_time: usize = 0;
        let mut co_aggregate_max_response_time: usize = 0;

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " Adjusted for Coordinated Omission:")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        for (request_key, request) in self.requests.iter().sorted() {
            if let Some(coordinated_omission_data) = request.coordinated_omission_data.as_ref() {
                // Iterate over user response times, and merge into global response times.
                co_aggregate_response_times = merge_times(
                    co_aggregate_response_times,
                    coordinated_omission_data.times.clone(),
                );

                // Increment total response time counter.
                co_aggregate_total_response_time += &coordinated_omission_data.total_time;

                // Increment counter tracking individual response times seen.
                co_aggregate_response_time_counter += &coordinated_omission_data.counter;

                // If user had new fastest response time, update global fastest response time.
                co_aggregate_min_response_time = update_min_time(
                    co_aggregate_min_response_time,
                    coordinated_omission_data.minimum_time,
                );

                // If user had new slowest response time, update global slowest response time.
                co_aggregate_max_response_time = update_max_time(
                    co_aggregate_max_response_time,
                    coordinated_omission_data.maximum_time,
                );

                // Sort response times so we can calculate a mean.
                writeln!(
                    fmt,
                    " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                    util::truncate_string(request_key, 24),
                    calculate_response_time_percentile(
                        &coordinated_omission_data.times,
                        coordinated_omission_data.counter,
                        coordinated_omission_data.minimum_time,
                        coordinated_omission_data.maximum_time,
                        0.5
                    ),
                    calculate_response_time_percentile(
                        &coordinated_omission_data.times,
                        coordinated_omission_data.counter,
                        coordinated_omission_data.minimum_time,
                        coordinated_omission_data.maximum_time,
                        0.75
                    ),
                    calculate_response_time_percentile(
                        &coordinated_omission_data.times,
                        coordinated_omission_data.counter,
                        coordinated_omission_data.minimum_time,
                        coordinated_omission_data.maximum_time,
                        0.98
                    ),
                    calculate_response_time_percentile(
                        &coordinated_omission_data.times,
                        coordinated_omission_data.counter,
                        coordinated_omission_data.minimum_time,
                        coordinated_omission_data.maximum_time,
                        0.99
                    ),
                    calculate_response_time_percentile(
                        &coordinated_omission_data.times,
                        coordinated_omission_data.counter,
                        coordinated_omission_data.minimum_time,
                        coordinated_omission_data.maximum_time,
                        0.999
                    ),
                    calculate_response_time_percentile(
                        &coordinated_omission_data.times,
                        coordinated_omission_data.counter,
                        coordinated_omission_data.minimum_time,
                        coordinated_omission_data.maximum_time,
                        0.9999
                    ),
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                    util::truncate_string(request_key, 24),
                    "-",
                    "-",
                    "-",
                    "-",
                    "-",
                    "-"
                )?;
            }
        }
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " -------------------------+--------+--------+--------+--------+--------+-------"
            )?;
            writeln!(
                fmt,
                " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                "Aggregated",
                calculate_response_time_percentile(
                    &co_aggregate_response_times,
                    co_aggregate_response_time_counter,
                    co_aggregate_min_response_time,
                    co_aggregate_max_response_time,
                    0.5
                ),
                calculate_response_time_percentile(
                    &co_aggregate_response_times,
                    co_aggregate_response_time_counter,
                    co_aggregate_min_response_time,
                    co_aggregate_max_response_time,
                    0.75
                ),
                calculate_response_time_percentile(
                    &co_aggregate_response_times,
                    co_aggregate_response_time_counter,
                    co_aggregate_min_response_time,
                    co_aggregate_max_response_time,
                    0.98
                ),
                calculate_response_time_percentile(
                    &co_aggregate_response_times,
                    co_aggregate_response_time_counter,
                    co_aggregate_min_response_time,
                    co_aggregate_max_response_time,
                    0.99
                ),
                calculate_response_time_percentile(
                    &co_aggregate_response_times,
                    co_aggregate_response_time_counter,
                    co_aggregate_min_response_time,
                    co_aggregate_max_response_time,
                    0.999
                ),
                calculate_response_time_percentile(
                    &co_aggregate_response_times,
                    co_aggregate_response_time_counter,
                    co_aggregate_min_response_time,
                    co_aggregate_max_response_time,
                    0.9999
                ),
            )?;
        }

        Ok(())
    }

    /// Optionally prepares a table of response status codes.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_status_codes(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if !self.display_status_codes {
            return Ok(());
        }

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " {:<24} | {:>51} ", "Name", "Status codes")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
        for (request_key, request) in self.requests.iter().sorted() {
            let codes = prepare_status_codes(
                &request.status_code_counts,
                &mut Some(&mut aggregated_status_code_counts),
            );

            writeln!(
                fmt,
                " {:<24} | {:>51}",
                util::truncate_string(request_key, 24),
                codes,
            )?;
        }
        writeln!(
            fmt,
            " -------------------------+----------------------------------------------------"
        )?;
        let codes = prepare_status_codes(&aggregated_status_code_counts, &mut None);
        writeln!(fmt, " {:<24} | {:>51} ", "Aggregated", codes)?;

        Ok(())
    }

    /// Optionally prepares a table of errors.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_errors(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only include errors when displaying the final metrics report, and if there are
        // errors to display.
        if !self.final_metrics || self.errors.is_empty() {
            return Ok(());
        }

        // Write the errors into a vector which can then be sorted by occurrences.
        let mut errors: Vec<(usize, String)> = Vec::new();
        for error in self.errors.values() {
            errors.push((
                error.occurrences,
                format!("{} {}: {}", error.method, error.name, error.error),
            ));
        }

        writeln!(
            fmt,
            "\n === ERRORS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " {:<11} | Error", "Count")?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // Reverse sort errors to display the error occuring the most first.
        for (occurrences, error) in errors.iter().sorted().rev() {
            writeln!(fmt, " {:<12}  {}", format_number(*occurrences), error)?;
        }

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        Ok(())
    }

    // Determine the seconds, minutes and hours between two chrono:DateTimes.
    fn get_seconds_minutes_hours(
        &self,
        start: &chrono::DateTime<chrono::Utc>,
        end: &chrono::DateTime<chrono::Utc>,
    ) -> (i64, i64, i64) {
        let duration = end.timestamp() - start.timestamp();
        let seconds = duration % 60;
        let minutes = (duration / 60) % 60;
        let hours = duration / 60 / 60;
        (seconds, minutes, hours)
    }

    /// Optionally prepares an overview table.
    ///
    /// This function is invoked by [`GooseMetrics::print()`].
    pub(crate) fn fmt_overview(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only display overview in the final metrics.
        if !self.final_metrics || self.history.is_empty() {
            return Ok(());
        }

        writeln!(
            fmt,
            "\n === OVERVIEW ===\n ------------------------------------------------------------------------------"
        )?;

        writeln!(
            fmt,
            " {:<12} {:<21} {:<19} {:<10} Users",
            "Action", "Started", "Stopped", "Elapsed",
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        // Step through history looking at current step and next step.
        for step in self.history.windows(2) {
            let (seconds, minutes, hours) =
                self.get_seconds_minutes_hours(&step[0].timestamp, &step[1].timestamp);
            let started = Local
                .timestamp(step[0].timestamp.timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let stopped = Local
                .timestamp(step[1].timestamp.timestamp(), 0)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            match &step[0].action {
                // For maintaining just show the current number of users.
                TestPlanStepAction::Maintaining => {
                    writeln!(
                        fmt,
                        " {:<12} {} - {} ({:02}:{:02}:{:02}, {})",
                        format!("{:?}:", step[0].action),
                        started,
                        stopped,
                        hours,
                        minutes,
                        seconds,
                        step[0].users,
                    )?;
                }
                // For increasing show the current number of users to the new number of users.
                TestPlanStepAction::Increasing => {
                    writeln!(
                        fmt,
                        " {:<12} {} - {} ({:02}:{:02}:{:02}, {} -> {})",
                        format!("{:?}:", step[0].action),
                        started,
                        stopped,
                        hours,
                        minutes,
                        seconds,
                        step[0].users,
                        step[1].users,
                    )?;
                }
                // For decreasing show the new number of users from the current number of users.
                TestPlanStepAction::Decreasing => {
                    writeln!(
                        fmt,
                        " {:<12} {} - {} ({:02}:{:02}:{:02}, {} <- {})",
                        format!("{:?}:", step[0].action),
                        started,
                        stopped,
                        hours,
                        minutes,
                        seconds,
                        step[1].users,
                        step[0].users,
                    )?;
                }
                TestPlanStepAction::Finished => {
                    unreachable!("there shouldn't be a step after finished");
                }
            }
        }

        match self.hosts.len() {
            0 => {
                // A host is required to run a load test.
                writeln!(fmt, "\n Target host: undefined")?;
            }
            1 => {
                for host in &self.hosts {
                    writeln!(fmt, "\n Target host: {}", host)?;
                }
            }
            _ => {
                writeln!(fmt, "\n Target hosts: ")?;
                for host in &self.hosts {
                    writeln!(fmt, " - {}", host,)?;
                }
            }
        }
        writeln!(
            fmt,
            " {} v{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        )?;

        if self.hosts.len() == 1 {}
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;

        Ok(())
    }
}

impl Serialize for GooseMetrics {
    // GooseMetrics serialization can't be derived because of the started field.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("GooseMetrics", 10)?;
        s.serialize_field("hash", &self.hash)?;
        // Convert started field to a unix timestamp.
        /* @TODO: Fixme
        let timestamp = if let Some(started) = self.started {
            started.timestamp()
        } else {
            0
        };
        s.serialize_field("started", &timestamp)?;
        */
        s.serialize_field("duration", &self.duration)?;
        s.serialize_field("users", &self.users)?;
        s.serialize_field("requests", &self.requests)?;
        s.serialize_field("transactions", &self.transactions)?;
        s.serialize_field("errors", &self.errors)?;
        s.serialize_field("final_metrics", &self.final_metrics)?;
        s.serialize_field("display_status_codes", &self.display_status_codes)?;
        s.serialize_field("display_metrics", &self.display_metrics)?;
        s.end()
    }
}

/// Implement format trait to allow displaying metrics.
impl fmt::Display for GooseMetrics {
    // Implement display of metrics with `{}` marker.
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        // Formats metrics data in tables, depending on what data is contained and which
        // flags are set.
        self.fmt_transactions(fmt)?;
        self.fmt_transaction_times(fmt)?;
        self.fmt_requests(fmt)?;
        self.fmt_response_times(fmt)?;
        self.fmt_percentiles(fmt)?;
        self.fmt_status_codes(fmt)?;
        self.fmt_errors(fmt)?;
        self.fmt_overview(fmt)
    }
}

/// For tracking and counting requests made during a load test.
///
/// The request that Goose is making. User threads send this data to the parent thread
/// when metrics are enabled. This request object must be provided to calls to
/// [`set_success`](../goose/struct.GooseUser.html#method.set_success) or
/// [`set_failure`](../goose/struct.GooseUser.html#method.set_failure) so Goose knows
/// which request is being updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseErrorMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// The raw request that the GooseClient made.
    pub raw: GooseRawRequest,
    /// The optional name of the request.
    pub name: String,
    /// The final full URL that was requested, after redirects.
    pub final_url: String,
    /// Whether or not the request was redirected.
    pub redirected: bool,
    /// How many milliseconds the request took.
    pub response_time: u64,
    /// The HTTP response code (optional).
    pub status_code: u16,
    /// Which GooseUser thread processed the request.
    pub user: usize,
    /// The error caused by this request.
    pub error: String,
}

/// For tracking and counting errors detected during a load test.
///
/// When a load test completes, by default it will include a summary of all errors that
/// were detected during the load test. Multiple errors that share the same request method,
/// the same request name, and the same error text are contained within a single
/// GooseErrorMetric object, with `occurrences` indicating how many times this error was
/// seen.
///
/// Individual `GooseErrorMetric`s are stored within a
/// [`GooseErrorMetrics`](./type.GooseErrorMetrics.html) `BTreeMap` with a string key of
/// `error.method.name`. The `BTreeMap` is found in the `errors` field of what is returned
/// by [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load
/// test finishes.
///
/// Can be disabled with the `--no-error-summary` run-time option, or with
/// [GooseDefault::NoErrorSummary](../config/enum.GooseDefault.html#variant.NoErrorSummary).
///
/// # Example
/// In this example, requests to load the front page are failing:
/// ```text
/// GooseErrorMetric {
///     method: Get,
///     name: "(Anon) front page",
///     error: "503 Service Unavailable: /",
///     occurrences: 4588,
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct GooseErrorMetricAggregate {
    /// The method that resulted in an error.
    pub method: GooseMethod,
    /// The optional name of the request.
    pub name: String,
    /// The error string.
    pub error: String,
    /// A counter reflecting how many times this error occurred.
    pub occurrences: usize,
}
impl GooseErrorMetricAggregate {
    pub(crate) fn new(method: GooseMethod, name: String, error: String) -> Self {
        GooseErrorMetricAggregate {
            method,
            name,
            error,
            occurrences: 0,
        }
    }
}

impl GooseAttack {
    // If metrics are enabled, synchronize metrics from child threads to the parent. If
    // flush is true all metrics will be received regardless of how long it takes. If
    // flush is false, metrics will only be received for up to 400 ms before exiting to
    // continue on the next call to this function.
    pub(crate) async fn sync_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        flush: bool,
    ) -> Result<(), GooseError> {
        if !self.configuration.no_metrics {
            // Check if we're displaying running metrics.
            if let Some(running_metrics) = self.configuration.running_metrics {
                if self.attack_mode != AttackMode::Worker
                    && util::timer_expired(
                        goose_attack_run_state.running_metrics_timer,
                        running_metrics,
                    )
                {
                    goose_attack_run_state.running_metrics_timer = std::time::Instant::now();
                    goose_attack_run_state.display_running_metrics = true;
                }
            }

            // Load messages from user threads until the receiver queue is empty.
            let received_message = self.receive_metrics(goose_attack_run_state, flush).await?;

            // As worker, push metrics up to manager.
            if self.attack_mode == AttackMode::Worker && received_message {
                #[cfg(feature = "gaggle")]
                {
                    // Push metrics to manager process.
                    if !worker::push_metrics_to_manager(
                        &goose_attack_run_state.socket.clone().unwrap(),
                        vec![
                            GaggleMetrics::Requests(self.metrics.requests.clone()),
                            GaggleMetrics::Transactions(self.metrics.transactions.clone()),
                        ],
                        true,
                    ) {
                        // GooseUserCommand::Exit received, cancel.
                        goose_attack_run_state
                            .canceled
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    // The manager has all our metrics, reset locally.
                    self.metrics.requests = HashMap::new();
                    self.metrics.initialize_transaction_metrics(
                        &self.scenarios,
                        &self.configuration,
                        &self.defaults,
                    )?;
                }
            }
        }

        // If enabled, display running metrics after sync
        if goose_attack_run_state.display_running_metrics {
            goose_attack_run_state.display_running_metrics = false;
            self.update_duration();
            self.metrics.print_running();
        }

        Ok(())
    }

    // When the [`GooseAttack`](./struct.GooseAttack.html) goes from the `Starting`
    // phase to the `Running` phase, optionally flush metrics.
    pub(crate) async fn reset_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Flush metrics collected prior to all user threads running
        if !goose_attack_run_state.all_users_spawned {
            // Receive metrics before resetting them.
            self.sync_metrics(goose_attack_run_state, true).await?;

            goose_attack_run_state.all_users_spawned = true;
            // Only reset metrics on startup if not using `--test-plan`.
            if self.configuration.test_plan.is_none() {
                let users = self.configuration.users.unwrap();
                if !self.configuration.no_reset_metrics {
                    // Display the running metrics collected so far, before resetting them.
                    self.update_duration();
                    self.metrics.print_running();
                    // Reset running_metrics_timer.
                    goose_attack_run_state.running_metrics_timer = std::time::Instant::now();

                    if self.metrics.display_metrics {
                        // Users is required here so unwrap() is safe.
                        if self.metrics.users < users {
                            println!(
                                "{} of {} users hatched, timer expired, resetting metrics (disable with --no-reset-metrics).\n", self.metrics.users, users
                            );
                        } else {
                            println!(
                                "All {} users hatched, resetting metrics (disable with --no-reset-metrics).\n", users
                            );
                        }
                    }

                    self.metrics.requests = HashMap::new();
                    self.metrics.initialize_transaction_metrics(
                        &self.scenarios,
                        &self.configuration,
                        &self.defaults,
                    )?;
                } else if self.metrics.users < users {
                    println!(
                        "{} of {} users hatched, timer expired.\n",
                        self.metrics.users, users
                    );
                } else {
                    println!("All {} users hatched.\n", self.metrics.users);
                }
            } else {
                println!("{} users hatched.", self.metrics.users);
            }

            // Restart the timer now that all threads are launched.
            self.started = Some(std::time::Instant::now());
        }

        Ok(())
    }

    // Store `GooseRequestMetric` in a `GooseRequestMetricAggregate` within the
    // `GooseMetrics.requests` `HashMap`, merging if already existing, or creating new.
    // Also writes it to the request_file if enabled.
    async fn record_request_metric(&mut self, request_metric: &GooseRequestMetric) {
        let key = format!("{} {}", request_metric.raw.method, request_metric.name);
        let mut merge_request = match self.metrics.requests.get(&key) {
            Some(m) => m.clone(),
            None => GooseRequestMetricAggregate::new(
                &request_metric.name,
                request_metric.raw.method.clone(),
                0,
            ),
        };

        // Handle a metrics update.
        if request_metric.update {
            if request_metric.success {
                merge_request.success_count += 1;
                merge_request.fail_count -= 1;
            } else {
                merge_request.success_count -= 1;
                merge_request.fail_count += 1;
            }
        }
        // Store a new metric.
        else {
            merge_request.record_time(
                request_metric.response_time,
                request_metric.coordinated_omission_elapsed > 0,
            );
            if self.configuration.status_codes {
                merge_request.set_status_code(request_metric.status_code);
            }
            if request_metric.success {
                merge_request.success_count += 1;
            } else {
                merge_request.fail_count += 1;
            }
        }

        self.metrics.requests.insert(key, merge_request);
    }

    // Receive metrics from [`GooseUser`](./goose/struct.GooseUser.html) threads. If flush
    // is true all metrics will be received regardless of how long it takes. If flush is
    // false, metrics will only be received for up to 400 ms before exiting to continue on
    // the next call to this function.
    pub(crate) async fn receive_metrics(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        flush: bool,
    ) -> Result<bool, GooseError> {
        let mut received_message = false;
        let mut message = goose_attack_run_state.metrics_rx.try_recv();

        // Main loop wakes up every 500ms, so don't spend more than 400ms receiving metrics.
        let receive_timeout = 400;
        let receive_started = std::time::Instant::now();

        while message.is_ok() {
            received_message = true;
            match message.unwrap() {
                GooseMetric::Request(request_metric) => {
                    // If there was an error, store it.
                    if !request_metric.error.is_empty() {
                        self.record_error(&request_metric, goose_attack_run_state);
                    }

                    // If coordinated_omission_elapsed is non-zero, this was a statistically
                    // generated "request" to mitigate coordinated omission, loop to backfill
                    // with statistically generated metrics.
                    if request_metric.coordinated_omission_elapsed > 0
                        && request_metric.user_cadence > 0
                    {
                        // Build a statistically generated coordinated_omissiom metric starting
                        // with the metric that was sent by the affected GooseUser.
                        let mut co_metric = request_metric.clone();

                        // Use a signed integer as this value can drop below zero.
                        let mut response_time = request_metric.coordinated_omission_elapsed as i64
                            - request_metric.user_cadence as i64
                            - request_metric.response_time as i64;

                        loop {
                            // Backfill until reaching the expected request cadence.
                            if response_time > request_metric.response_time as i64 {
                                co_metric.response_time = response_time as u64;
                                self.record_request_metric(&co_metric).await;
                                response_time -= request_metric.user_cadence as i64;
                            } else {
                                break;
                            }
                        }
                    // Otherwise this is an actual request, record it normally.
                    } else {
                        // Merge the `GooseRequestMetric` into a `GooseRequestMetricAggregate` in
                        // `GooseMetrics.requests`, and write to the requests log if enabled.
                        self.record_request_metric(&request_metric).await;

                        if !self.configuration.report_file.is_empty() {
                            let seconds_since_start = (request_metric.elapsed / 1000) as usize;

                            let key =
                                format!("{} {}", request_metric.raw.method, request_metric.name);
                            self.graph_data
                                .record_requests_per_second(&key, seconds_since_start);
                            self.graph_data.record_average_response_time_per_second(
                                key.clone(),
                                seconds_since_start,
                                request_metric.response_time,
                            );

                            if !request_metric.success {
                                self.graph_data
                                    .record_errors_per_second(&key, seconds_since_start);
                            }
                        }
                    }
                }
                GooseMetric::Transaction(raw_transaction) => {
                    // Store a new metric.
                    self.metrics.transactions[raw_transaction.scenario_index]
                        [raw_transaction.transaction_index]
                        .set_time(raw_transaction.run_time, raw_transaction.success);

                    if !self.configuration.report_file.is_empty() {
                        self.graph_data.record_transactions_per_second(
                            (raw_transaction.elapsed / 1000) as usize,
                        );
                    }
                }
            }
            // Unless flushing all metrics, break out of receive loop after timeout.
            if !flush && util::ms_timer_expired(receive_started, receive_timeout) {
                break;
            }
            message = goose_attack_run_state.metrics_rx.try_recv();
        }

        Ok(received_message)
    }

    /// Update error metrics.
    pub(crate) fn record_error(
        &mut self,
        raw_request: &GooseRequestMetric,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) {
        // If error-log is enabled, convert the raw request to a GooseErrorMetric and send it
        // to the logger thread.
        if !self.configuration.error_log.is_empty() {
            if let Some(logger) = goose_attack_run_state.all_threads_logger_tx.as_ref() {
                // This is a best effort logger attempt, if the logger has alrady shut down it
                // will fail which we ignore.
                let _ = logger.send(Some(GooseLog::Error(GooseErrorMetric {
                    elapsed: raw_request.elapsed,
                    raw: raw_request.raw.clone(),
                    name: raw_request.name.clone(),
                    final_url: raw_request.final_url.clone(),
                    redirected: raw_request.redirected,
                    response_time: raw_request.response_time,
                    status_code: raw_request.status_code,
                    user: raw_request.user,
                    error: raw_request.error.clone(),
                })));
            }
        }

        // If the error summary is disabled, return without collecting errors.
        if self.configuration.no_error_summary {
            return;
        }

        // Create a string to uniquely identify errors for tracking metrics.
        let error_string = format!(
            "{}.{}.{}",
            raw_request.error, raw_request.raw.method, raw_request.name
        );

        let mut error_metrics = match self.metrics.errors.get(&error_string) {
            // We've seen this error before.
            Some(m) => m.clone(),
            // First time we've seen this error.
            None => GooseErrorMetricAggregate::new(
                raw_request.raw.method.clone(),
                raw_request.name.to_string(),
                raw_request.error.to_string(),
            ),
        };
        error_metrics.occurrences += 1;
        self.metrics.errors.insert(error_string, error_metrics);
    }

    // Update metrics showing how long the load test has been running.
    // 1.2 seconds will round down to 1 second. 1.6 seconds will round up to 2 seconds.
    pub(crate) fn update_duration(&mut self) {
        self.metrics.duration = if self.started.is_some() {
            self.started.unwrap().elapsed().as_secs_f32().round() as usize
        } else {
            0
        };
    }

    // Write an HTML-formatted report, if enabled.
    pub(crate) async fn write_html_report(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Only write the report if enabled.
        if let Some(report_file) = goose_attack_run_state.report_file.as_mut() {
            let test_start_time = self.metrics.history.first().unwrap().timestamp;

            // Prepare report summary variables.
            let users = self.metrics.users.to_string();

            let mut steps_overview = String::new();
            for step in self.metrics.history.windows(2) {
                let (seconds, minutes, hours) = self
                    .metrics
                    .get_seconds_minutes_hours(&step[0].timestamp, &step[1].timestamp);
                let started = step[0].timestamp.format("%y-%m-%d %H:%M:%S");
                let stopped = step[1].timestamp.format("%y-%m-%d %H:%M:%S");
                match &step[0].action {
                    // For maintaining just show the current number of users.
                    TestPlanStepAction::Maintaining => {
                        steps_overview.push_str(&format!(
                            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{:02}:{:02}:{:02}</td><td>{}</td></tr>",
                            step[0].action,
                            started,
                            stopped,
                            hours,
                            minutes,
                            seconds,
                            step[0].users,
                        ));
                    }
                    // For increasing show the current number of users to the new number of users.
                    TestPlanStepAction::Increasing => {
                        steps_overview.push_str(&format!(
                            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{:02}:{:02}:{:02}</td><td>{} &rarr; {}</td></tr>",
                            step[0].action,
                            started,
                            stopped,
                            hours,
                            minutes,
                            seconds,
                            step[0].users,
                            step[1].users,
                        ));
                    }
                    // For decreasing show the new number of users from the current number of users.
                    TestPlanStepAction::Decreasing => {
                        steps_overview.push_str(&format!(
                            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{:02}:{:02}:{:02}</td><td>{} &larr; {}</td></tr>",
                            step[0].action,
                            started,
                            stopped,
                            hours,
                            minutes,
                            seconds,
                            step[1].users,
                            step[0].users,
                        ));
                    }
                    TestPlanStepAction::Finished => {
                        unreachable!("there shouldn't be a step after finished");
                    }
                }
            }

            // Build a comma separated list of hosts.
            let hosts = &self.metrics.hosts.clone().into_iter().join(", ");

            // Prepare requests and responses variables.
            let mut raw_request_metrics = Vec::new();
            let mut co_request_metrics = Vec::new();
            let mut raw_response_metrics = Vec::new();
            let mut co_response_metrics = Vec::new();
            let mut raw_aggregate_total_count = 0;
            let mut co_aggregate_total_count = 0;
            let mut raw_aggregate_fail_count = 0;
            let mut raw_aggregate_response_time_counter: usize = 0;
            let mut raw_aggregate_response_time_minimum: usize = 0;
            let mut raw_aggregate_response_time_maximum: usize = 0;
            let mut raw_aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
            let mut co_aggregate_response_time_counter: usize = 0;
            let mut co_aggregate_response_time_maximum: usize = 0;
            let mut co_aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
            let mut co_data = false;
            for (request_key, request) in self.metrics.requests.iter().sorted() {
                // Determine whether or not to include Coordinated Omission data.
                if !co_data && request.coordinated_omission_data.is_some() {
                    co_data = true;
                }
                let method = format!("{}", request.method);
                // The request_key is "{method} {name}", so by stripping the "{method} "
                // prefix we get the name.
                let name = request_key
                    .strip_prefix(&format!("{} ", request.method))
                    .unwrap()
                    .to_string();
                let total_request_count = request.success_count + request.fail_count;
                let (requests_per_second, failures_per_second) = per_second_calculations(
                    self.metrics.duration,
                    total_request_count,
                    request.fail_count,
                );
                // Prepare per-request metrics.
                raw_request_metrics.push(report::RequestMetric {
                    method: method.to_string(),
                    name: name.to_string(),
                    number_of_requests: total_request_count,
                    number_of_failures: request.fail_count,
                    response_time_average: format!(
                        "{:.2}",
                        request.raw_data.total_time as f32 / request.raw_data.counter as f32
                    ),
                    response_time_minimum: request.raw_data.minimum_time,
                    response_time_maximum: request.raw_data.maximum_time,
                    requests_per_second: format!("{:.2}", requests_per_second),
                    failures_per_second: format!("{:.2}", failures_per_second),
                });

                // Prepare per-response metrics.
                raw_response_metrics.push(report::get_response_metric(
                    &method,
                    &name,
                    &request.raw_data.times,
                    request.raw_data.counter,
                    request.raw_data.minimum_time,
                    request.raw_data.maximum_time,
                ));

                // Collect aggregated request and response metrics.
                raw_aggregate_total_count += total_request_count;
                raw_aggregate_fail_count += request.fail_count;
                raw_aggregate_response_time_counter += request.raw_data.total_time;
                raw_aggregate_response_time_minimum = update_min_time(
                    raw_aggregate_response_time_minimum,
                    request.raw_data.minimum_time,
                );
                raw_aggregate_response_time_maximum = update_max_time(
                    raw_aggregate_response_time_maximum,
                    request.raw_data.maximum_time,
                );
                raw_aggregate_response_times =
                    merge_times(raw_aggregate_response_times, request.raw_data.times.clone());
            }

            // Prepare aggregate per-request metrics.
            let (raw_aggregate_requests_per_second, raw_aggregate_failures_per_second) =
                per_second_calculations(
                    self.metrics.duration,
                    raw_aggregate_total_count,
                    raw_aggregate_fail_count,
                );
            raw_request_metrics.push(report::RequestMetric {
                method: "".to_string(),
                name: "Aggregated".to_string(),
                number_of_requests: raw_aggregate_total_count,
                number_of_failures: raw_aggregate_fail_count,
                response_time_average: format!(
                    "{:.2}",
                    raw_aggregate_response_time_counter as f32 / raw_aggregate_total_count as f32
                ),
                response_time_minimum: raw_aggregate_response_time_minimum,
                response_time_maximum: raw_aggregate_response_time_maximum,
                requests_per_second: format!("{:.2}", raw_aggregate_requests_per_second),
                failures_per_second: format!("{:.2}", raw_aggregate_failures_per_second),
            });

            // Prepare aggregate per-response metrics.
            raw_response_metrics.push(report::get_response_metric(
                "",
                "Aggregated",
                &raw_aggregate_response_times,
                raw_aggregate_total_count,
                raw_aggregate_response_time_minimum,
                raw_aggregate_response_time_maximum,
            ));

            // Compile the request metrics template.
            let mut raw_requests_rows = Vec::new();
            for metric in raw_request_metrics {
                raw_requests_rows.push(report::raw_request_metrics_row(metric));
            }

            // Compile the response metrics template.
            let mut raw_responses_rows = Vec::new();
            for metric in raw_response_metrics {
                raw_responses_rows.push(report::response_metrics_row(metric));
            }

            let co_requests_template: String;
            let co_responses_template: String;
            if co_data {
                for (request_key, request) in self.metrics.requests.iter().sorted() {
                    if let Some(coordinated_omission_data) =
                        request.coordinated_omission_data.as_ref()
                    {
                        let method = format!("{}", request.method);
                        // The request_key is "{method} {name}", so by stripping the "{method} "
                        // prefix we get the name.
                        let name = request_key
                            .strip_prefix(&format!("{} ", request.method))
                            .unwrap()
                            .to_string();
                        let raw_average =
                            request.raw_data.total_time as f32 / request.raw_data.counter as f32;
                        let co_average = coordinated_omission_data.total_time as f32
                            / coordinated_omission_data.counter as f32;
                        // Prepare per-request metrics.
                        co_request_metrics.push(report::CORequestMetric {
                            method: method.to_string(),
                            name: name.to_string(),
                            response_time_average: format!("{:.2}", co_average),
                            response_time_standard_deviation: format!(
                                "{:.2}",
                                util::standard_deviation(raw_average, co_average)
                            ),
                            response_time_maximum: coordinated_omission_data.maximum_time,
                        });

                        // Prepare per-response metrics.
                        co_response_metrics.push(report::get_response_metric(
                            &method,
                            &name,
                            &coordinated_omission_data.times,
                            coordinated_omission_data.counter,
                            coordinated_omission_data.minimum_time,
                            coordinated_omission_data.maximum_time,
                        ));

                        // Collect aggregated request and response metrics.
                        co_aggregate_response_time_counter += coordinated_omission_data.total_time;
                        co_aggregate_response_time_maximum = update_max_time(
                            co_aggregate_response_time_maximum,
                            coordinated_omission_data.maximum_time,
                        );
                        co_aggregate_response_times = merge_times(
                            co_aggregate_response_times,
                            coordinated_omission_data.times.clone(),
                        );
                    }
                    let total_request_count = request.success_count + request.fail_count;
                    co_aggregate_total_count += total_request_count;
                }
                let co_average =
                    co_aggregate_response_time_counter as f32 / co_aggregate_total_count as f32;
                let raw_average =
                    raw_aggregate_response_time_counter as f32 / raw_aggregate_total_count as f32;
                co_request_metrics.push(report::CORequestMetric {
                    method: "".to_string(),
                    name: "Aggregated".to_string(),
                    response_time_average: format!(
                        "{:.2}",
                        co_aggregate_response_time_counter as f32 / co_aggregate_total_count as f32
                    ),
                    response_time_standard_deviation: format!(
                        "{:.2}",
                        util::standard_deviation(raw_average, co_average),
                    ),
                    response_time_maximum: co_aggregate_response_time_maximum,
                });

                // Prepare aggregate per-response metrics.
                co_response_metrics.push(report::get_response_metric(
                    "",
                    "Aggregated",
                    &co_aggregate_response_times,
                    co_aggregate_total_count,
                    raw_aggregate_response_time_minimum,
                    co_aggregate_response_time_maximum,
                ));

                // Compile the co_request metrics rows.
                let mut co_request_rows = Vec::new();
                for metric in co_request_metrics {
                    co_request_rows.push(report::coordinated_omission_request_metrics_row(metric));
                }

                // Compile the status_code metrics template.
                co_requests_template = report::coordinated_omission_request_metrics_template(
                    &co_request_rows.join("\n"),
                );

                // Compile the co_request metrics rows.
                let mut co_response_rows = Vec::new();
                for metric in co_response_metrics {
                    co_response_rows
                        .push(report::coordinated_omission_response_metrics_row(metric));
                }

                // Compile the status_code metrics template.
                co_responses_template = report::coordinated_omission_response_metrics_template(
                    &co_response_rows.join("\n"),
                );
            } else {
                // If --status-codes is not enabled, return an empty template.
                co_requests_template = "".to_string();
                co_responses_template = "".to_string();
            }

            // Only build the transactions template if --no-transaction-metrics isn't enabled.
            let transactions_template: String;
            if !self.configuration.no_transaction_metrics {
                let mut transaction_metrics = Vec::new();
                let mut aggregate_total_count = 0;
                let mut aggregate_fail_count = 0;
                let mut aggregate_transaction_time_counter: usize = 0;
                let mut aggregate_transaction_time_minimum: usize = 0;
                let mut aggregate_transaction_time_maximum: usize = 0;
                let mut aggregate_transaction_times: BTreeMap<usize, usize> = BTreeMap::new();
                for (scenario_counter, scenario) in self.metrics.transactions.iter().enumerate() {
                    for (transaction_counter, transaction) in scenario.iter().enumerate() {
                        if transaction_counter == 0 {
                            // Only the scenario_name is used for scenarios.
                            transaction_metrics.push(report::TransactionMetric {
                                is_scenario: true,
                                transaction: "".to_string(),
                                name: transaction.scenario_name.to_string(),
                                number_of_requests: 0,
                                number_of_failures: 0,
                                response_time_average: "".to_string(),
                                response_time_minimum: 0,
                                response_time_maximum: 0,
                                requests_per_second: "".to_string(),
                                failures_per_second: "".to_string(),
                            });
                        }
                        let total_run_count = transaction.success_count + transaction.fail_count;
                        let (requests_per_second, failures_per_second) = per_second_calculations(
                            self.metrics.duration,
                            total_run_count,
                            transaction.fail_count,
                        );
                        let average = match transaction.counter {
                            0 => 0.00,
                            _ => transaction.total_time as f32 / transaction.counter as f32,
                        };
                        transaction_metrics.push(report::TransactionMetric {
                            is_scenario: false,
                            transaction: format!("{}.{}", scenario_counter, transaction_counter),
                            name: transaction.transaction_name.to_string(),
                            number_of_requests: total_run_count,
                            number_of_failures: transaction.fail_count,
                            response_time_average: format!("{:.2}", average),
                            response_time_minimum: transaction.min_time,
                            response_time_maximum: transaction.max_time,
                            requests_per_second: format!("{:.2}", requests_per_second),
                            failures_per_second: format!("{:.2}", failures_per_second),
                        });

                        aggregate_total_count += total_run_count;
                        aggregate_fail_count += transaction.fail_count;
                        aggregate_transaction_times =
                            merge_times(aggregate_transaction_times, transaction.times.clone());
                        aggregate_transaction_time_counter += &transaction.counter;
                        aggregate_transaction_time_minimum = update_min_time(
                            aggregate_transaction_time_minimum,
                            transaction.min_time,
                        );
                        aggregate_transaction_time_maximum = update_max_time(
                            aggregate_transaction_time_maximum,
                            transaction.max_time,
                        );
                    }
                }

                let (aggregate_requests_per_second, aggregate_failures_per_second) =
                    per_second_calculations(
                        self.metrics.duration,
                        aggregate_total_count,
                        aggregate_fail_count,
                    );
                transaction_metrics.push(report::TransactionMetric {
                    is_scenario: false,
                    transaction: "".to_string(),
                    name: "Aggregated".to_string(),
                    number_of_requests: aggregate_total_count,
                    number_of_failures: aggregate_fail_count,
                    response_time_average: format!(
                        "{:.2}",
                        raw_aggregate_response_time_counter as f32 / aggregate_total_count as f32
                    ),
                    response_time_minimum: aggregate_transaction_time_minimum,
                    response_time_maximum: aggregate_transaction_time_maximum,
                    requests_per_second: format!("{:.2}", aggregate_requests_per_second),
                    failures_per_second: format!("{:.2}", aggregate_failures_per_second),
                });
                let mut transactions_rows = Vec::new();
                // Compile the transaction metrics template.
                for metric in transaction_metrics {
                    transactions_rows.push(report::transaction_metrics_row(metric));
                }

                transactions_template = report::transaction_metrics_template(
                    &transactions_rows.join("\n"),
                    self.graph_data
                        .get_transactions_per_second_graph(!self.configuration.no_granular_report)
                        .get_markup(&self.metrics.history, test_start_time),
                );
            } else {
                transactions_template = "".to_string();
            }

            // Only build the transactions template if --no-transaction-metrics isn't enabled.
            let errors_template: String = if !self.metrics.errors.is_empty() {
                let mut error_rows = Vec::new();
                for error in self.metrics.errors.values() {
                    error_rows.push(report::error_row(error));
                }

                report::errors_template(
                    &error_rows.join("\n"),
                    self.graph_data
                        .get_errors_per_second_graph(!self.configuration.no_granular_report)
                        .get_markup(&self.metrics.history, test_start_time),
                )
            } else {
                "".to_string()
            };

            // Only build the status_code template if --status-codes is enabled.
            let status_code_template: String = if self.configuration.status_codes {
                let mut status_code_metrics = Vec::new();
                let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
                for (request_key, request) in self.metrics.requests.iter().sorted() {
                    let method = format!("{}", request.method);
                    // The request_key is "{method} {name}", so by stripping the "{method} "
                    // prefix we get the name.
                    let name = request_key
                        .strip_prefix(&format!("{} ", request.method))
                        .unwrap()
                        .to_string();

                    // Build a list of status codes, and update the aggregate record.
                    let codes = prepare_status_codes(
                        &request.status_code_counts,
                        &mut Some(&mut aggregated_status_code_counts),
                    );

                    // Add a row of data for the status code table.
                    status_code_metrics.push(report::StatusCodeMetric {
                        method,
                        name,
                        status_codes: codes,
                    });
                }

                // Build a list of aggregate status codes.
                let aggregated_codes =
                    prepare_status_codes(&aggregated_status_code_counts, &mut None);

                // Add a final row of aggregate data for the status code table.
                status_code_metrics.push(report::StatusCodeMetric {
                    method: "".to_string(),
                    name: "Aggregated".to_string(),
                    status_codes: aggregated_codes,
                });

                // Compile the status_code metrics rows.
                let mut status_code_rows = Vec::new();
                for metric in status_code_metrics {
                    status_code_rows.push(report::status_code_metrics_row(metric));
                }

                // Compile the status_code metrics template.
                report::status_code_metrics_template(&status_code_rows.join("\n"))
            } else {
                // If --status-codes is not enabled, return an empty template.
                "".to_string()
            };

            // Compile the report template.
            let report = report::build_report(
                &users,
                &steps_overview,
                hosts,
                report::GooseReportTemplates {
                    raw_requests_template: &raw_requests_rows.join("\n"),
                    raw_responses_template: &raw_responses_rows.join("\n"),
                    co_requests_template: &co_requests_template,
                    co_responses_template: &co_responses_template,
                    transactions_template: &transactions_template,
                    status_codes_template: &status_code_template,
                    errors_template: &errors_template,
                    graph_rps_template: &self
                        .graph_data
                        .get_requests_per_second_graph(!self.configuration.no_granular_report)
                        .get_markup(&self.metrics.history, test_start_time),
                    graph_average_response_time_template: &self
                        .graph_data
                        .get_average_response_time_graph(!self.configuration.no_granular_report)
                        .get_markup(&self.metrics.history, test_start_time),
                    graph_users_per_second: &self
                        .graph_data
                        .get_active_users_graph(!self.configuration.no_granular_report)
                        .get_markup(&self.metrics.history, test_start_time),
                },
            );

            // Write the report to file.
            if let Err(e) = report_file.write_all(report.as_ref()).await {
                return Err(GooseError::InvalidOption {
                    option: "--report-file".to_string(),
                    value: self.get_report_file_path().unwrap(),
                    detail: format!("Failed to create report file: {}", e),
                });
            };
            // Be sure the file flushes to disk.
            report_file.flush().await?;

            info!(
                "wrote html report file to: {}",
                self.get_report_file_path().unwrap()
            );
        }

        Ok(())
    }
}

/// Helper to calculate requests and fails per seconds.
pub(crate) fn per_second_calculations(duration: usize, total: usize, fail: usize) -> (f32, f32) {
    let requests_per_second;
    let fails_per_second;
    if duration == 0 {
        requests_per_second = 0.0;
        fails_per_second = 0.0;
    } else {
        requests_per_second = total as f32 / duration as f32;
        fails_per_second = fail as f32 / duration as f32;
    }
    (requests_per_second, fails_per_second)
}

fn determine_precision(value: f32) -> usize {
    if value < 1000.0 {
        2
    } else {
        0
    }
}

/// Format large number in locale appropriate style.
pub(crate) fn format_number(number: usize) -> String {
    (number).to_formatted_string(&Locale::en)
}

/// A helper function that merges together times.
///
/// Used in `lib.rs` to merge together per-thread times, and in `metrics.rs` to
/// aggregate all times.
pub(crate) fn merge_times(
    mut global_response_times: BTreeMap<usize, usize>,
    local_response_times: BTreeMap<usize, usize>,
) -> BTreeMap<usize, usize> {
    // Iterate over user response times, and merge into global response times.
    for (response_time, count) in &local_response_times {
        let counter = match global_response_times.get(response_time) {
            // We've seen this response_time before, increment counter.
            Some(c) => *c + count,
            // First time we've seen this response time, initialize counter.
            None => *count,
        };
        global_response_times.insert(*response_time, counter);
    }
    global_response_times
}

/// A helper function to update the global minimum time based on local time.
pub(crate) fn update_min_time(mut global_min: usize, min: usize) -> usize {
    if global_min == 0 || (min > 0 && min < global_min) {
        global_min = min;
    }
    global_min
}

/// A helper function to update the global maximum time based on local time.
pub(crate) fn update_max_time(mut global_max: usize, max: usize) -> usize {
    if global_max < max {
        global_max = max;
    }
    global_max
}

/// Get the response time that a certain number of percent of the requests finished within.
pub(crate) fn calculate_response_time_percentile(
    response_times: &BTreeMap<usize, usize>,
    total_requests: usize,
    min: usize,
    max: usize,
    percent: f32,
) -> String {
    let percentile_request = (total_requests as f32 * percent).round() as usize;
    debug!(
        "percentile: {}, request {} of total {}",
        percent, percentile_request, total_requests
    );

    let mut total_count: usize = 0;

    for (value, counter) in response_times {
        total_count += counter;
        if total_count >= percentile_request {
            if *value < min {
                return format_number(min);
            } else if *value > max {
                return format_number(max);
            } else {
                return format_number(*value);
            }
        }
    }
    format_number(0)
}

/// Helper to count and aggregate seen status codes.
pub(crate) fn prepare_status_codes(
    status_code_counts: &HashMap<u16, usize>,
    aggregate_counts: &mut Option<&mut HashMap<u16, usize>>,
) -> String {
    let mut codes: String = "".to_string();
    for (status_code, count) in status_code_counts {
        if codes.is_empty() {
            codes = format!(
                "{} [{}]",
                count.to_formatted_string(&Locale::en),
                status_code
            );
        } else {
            codes = format!(
                "{}, {} [{}]",
                codes.clone(),
                count.to_formatted_string(&Locale::en),
                status_code
            );
        }
        if let Some(aggregate_status_code_counts) = aggregate_counts.as_mut() {
            let new_count = if let Some(existing_status_code_count) =
                aggregate_status_code_counts.get(status_code)
            {
                *existing_status_code_count + *count
            } else {
                *count
            };
            aggregate_status_code_counts.insert(*status_code, new_count);
        }
    }
    codes
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn max_response_time() {
        let mut max_response_time = 99;
        // Update max response time to a higher value.
        max_response_time = update_max_time(max_response_time, 101);
        assert_eq!(max_response_time, 101);
        // Max response time doesn't update when updating with a lower value.
        max_response_time = update_max_time(max_response_time, 1);
        assert_eq!(max_response_time, 101);
    }

    #[test]
    fn min_response_time() {
        let mut min_response_time = 11;
        // Update min response time to a lower value.
        min_response_time = update_min_time(min_response_time, 9);
        assert_eq!(min_response_time, 9);
        // Min response time doesn't update when updating with a lower value.
        min_response_time = update_min_time(min_response_time, 22);
        assert_eq!(min_response_time, 9);
        // Min response time doesn't update when updating with a 0 value.
        min_response_time = update_min_time(min_response_time, 0);
        assert_eq!(min_response_time, 9);
    }

    #[test]
    fn response_time_merge() {
        let mut global_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let local_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        global_response_times = merge_times(global_response_times, local_response_times.clone());
        // @TODO: how can we do useful testing of private method and objects?
        assert_eq!(&global_response_times, &local_response_times);
    }

    #[test]
    fn max_response_time_percentile() {
        let mut response_times: BTreeMap<usize, usize> = BTreeMap::new();
        response_times.insert(1, 1);
        response_times.insert(2, 1);
        response_times.insert(3, 1);
        // 3 * .5 = 1.5, rounds to 2.
        assert!(calculate_response_time_percentile(&response_times, 3, 1, 3, 0.5) == "2");
        response_times.insert(3, 2);
        // 4 * .5 = 2
        assert!(calculate_response_time_percentile(&response_times, 4, 1, 3, 0.5) == "2");
        // 4 * .25 = 1
        assert!(calculate_response_time_percentile(&response_times, 4, 1, 3, 0.25) == "1");
        // 4 * .75 = 3
        assert!(calculate_response_time_percentile(&response_times, 4, 1, 3, 0.75) == "3");
        // 4 * 1 = 4 (and the 4th response time is also 3)
        assert!(calculate_response_time_percentile(&response_times, 4, 1, 3, 1.0) == "3");

        // 4 * .5 = 2, but uses specified minimum of 2
        assert!(calculate_response_time_percentile(&response_times, 4, 2, 3, 0.25) == "2");
        // 4 * .75 = 3, but uses specified maximum of 2
        assert!(calculate_response_time_percentile(&response_times, 4, 1, 2, 0.75) == "2");

        response_times.insert(10, 25);
        response_times.insert(20, 25);
        response_times.insert(30, 25);
        response_times.insert(50, 25);
        response_times.insert(100, 10);
        response_times.insert(200, 1);
        assert!(calculate_response_time_percentile(&response_times, 115, 1, 200, 0.9) == "50");
        assert!(calculate_response_time_percentile(&response_times, 115, 1, 200, 0.99) == "100");
        assert!(calculate_response_time_percentile(&response_times, 115, 1, 200, 0.999) == "200");
    }

    #[test]
    fn calculate_per_second() {
        // With duration of 0, requests and fails per second is always 0.
        let mut duration = 0;
        let mut total = 10;
        let fail = 10;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == 0.0);
        assert!(fails_per_second == 0.0);
        // Changing total doesn't affect requests and fails as duration is still 0.
        total = 100;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == 0.0);
        assert!(fails_per_second == 0.0);

        // With non-zero duration, requests and fails per second return properly.
        duration = 10;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!((requests_per_second - 10.0).abs() < f32::EPSILON);
        assert!((fails_per_second - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn goose_raw_request() {
        const PATH: &str = "http://127.0.0.1/";
        let raw_request = GooseRawRequest::new(GooseMethod::Get, PATH, vec![], "");
        let mut request_metric = GooseRequestMetric::new(raw_request, "/", 0, 0);
        assert_eq!(request_metric.raw.method, GooseMethod::Get);
        assert_eq!(request_metric.raw.url, PATH.to_string());
        assert_eq!(request_metric.name, "/".to_string());
        assert_eq!(request_metric.response_time, 0);
        assert_eq!(request_metric.status_code, 0);
        assert!(request_metric.success);
        assert!(!request_metric.update);

        let response_time = 123;
        request_metric.set_response_time(response_time);
        assert_eq!(request_metric.raw.method, GooseMethod::Get);
        assert_eq!(request_metric.name, "/".to_string());
        assert_eq!(request_metric.raw.url, PATH.to_string());
        assert_eq!(request_metric.response_time, response_time as u64);
        assert_eq!(request_metric.status_code, 0);
        assert!(request_metric.success);
        assert!(!request_metric.update);

        let status_code = http::StatusCode::OK;
        request_metric.set_status_code(Some(status_code));
        assert_eq!(request_metric.raw.method, GooseMethod::Get);
        assert_eq!(request_metric.name, "/".to_string());
        assert_eq!(request_metric.raw.url, PATH.to_string());
        assert_eq!(request_metric.response_time, response_time as u64);
        assert_eq!(request_metric.status_code, 200);
        assert!(request_metric.success);
        assert!(!request_metric.update);
    }

    #[test]
    fn goose_request() {
        let mut request = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0);
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.raw_data.times.len(), 0);
        assert_eq!(request.raw_data.minimum_time, 0);
        assert_eq!(request.raw_data.maximum_time, 0);
        assert_eq!(request.raw_data.total_time, 0);
        assert_eq!(request.raw_data.counter, 0);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking a response time updates several fields.
        request.record_time(1, false);
        // We've seen only one response time so far.
        assert_eq!(request.raw_data.times.len(), 1);
        // We've seen one response time of length 1.
        assert_eq!(request.raw_data.times[&1], 1);
        // The minimum response time seen so far is 1.
        assert_eq!(request.raw_data.minimum_time, 1);
        // The maximum response time seen so far is 1.
        assert_eq!(request.raw_data.maximum_time, 1);
        // We've seen a total of 1 ms of response time so far.
        assert_eq!(request.raw_data.total_time, 1);
        // We've seen a total of 2 response times so far.
        assert_eq!(request.raw_data.counter, 1);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.record_time(10, false);
        // We've added a new unique response time.
        assert_eq!(request.raw_data.times.len(), 2);
        // We've seen the 10 ms response time 1 time.
        assert_eq!(request.raw_data.times[&10], 1);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum is new response time.
        assert_eq!(request.raw_data.maximum_time, 10);
        // Total combined response times is now 11 ms.
        assert_eq!(request.raw_data.total_time, 11);
        // We've seen two response times so far.
        assert_eq!(request.raw_data.counter, 2);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.record_time(10, false);
        // We've incremented the counter of an existing response time.
        assert_eq!(request.raw_data.times.len(), 2);
        // We've seen the 10 ms response time 2 times.
        assert_eq!(request.raw_data.times[&10], 2);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum doesn't change.
        assert_eq!(request.raw_data.maximum_time, 10);
        // Total combined response times is now 21 ms.
        assert_eq!(request.raw_data.total_time, 21);
        // We've seen three response times so far.
        assert_eq!(request.raw_data.counter, 3);

        // Tracking another response time updates all related fields.
        request.record_time(101, false);
        // We've added a new response time for the first time.
        assert_eq!(request.raw_data.times.len(), 3);
        // The response time was internally rounded to 100, which we've seen once.
        assert_eq!(request.raw_data.times[&100], 1);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time, 101);
        // Total combined response times is now 122 ms.
        assert_eq!(request.raw_data.total_time, 122);
        // We've seen four response times so far.
        assert_eq!(request.raw_data.counter, 4);

        // Tracking another response time updates all related fields.
        request.record_time(102, false);
        // Due to rounding, this increments the existing 100 ms response time.
        assert_eq!(request.raw_data.times.len(), 3);
        // The response time was internally rounded to 100, which we've now seen twice.
        assert_eq!(request.raw_data.times[&100], 2);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time, 102);
        // Add 102 to the total response time so far.
        assert_eq!(request.raw_data.total_time, 224);
        // We've seen five response times so far.
        assert_eq!(request.raw_data.counter, 5);

        // Tracking another response time updates all related fields.
        request.record_time(155, false);
        // Adds a new response time.
        assert_eq!(request.raw_data.times.len(), 4);
        // The response time was internally rounded to 160, seen for the first time.
        assert_eq!(request.raw_data.times[&160], 1);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time, 155);
        // Add 155 to the total response time so far.
        assert_eq!(request.raw_data.total_time, 379);
        // We've seen six response times so far.
        assert_eq!(request.raw_data.counter, 6);

        // Tracking another response time updates all related fields.
        request.record_time(2345, false);
        // Adds a new response time.
        assert_eq!(request.raw_data.times.len(), 5);
        // The response time was internally rounded to 2000, seen for the first time.
        assert_eq!(request.raw_data.times[&2000], 1);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time, 2345);
        // Add 2345 to the total response time so far.
        assert_eq!(request.raw_data.total_time, 2724);
        // We've seen seven response times so far.
        assert_eq!(request.raw_data.counter, 7);

        // Tracking another response time updates all related fields.
        request.record_time(987654321, false);
        // Adds a new response time.
        assert_eq!(request.raw_data.times.len(), 6);
        // The response time was internally rounded to 987654000, seen for the first time.
        assert_eq!(request.raw_data.times[&987654000], 1);
        // Minimum doesn't change.
        assert_eq!(request.raw_data.minimum_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.raw_data.maximum_time, 987654321);
        // Add 987654321 to the total response time so far.
        assert_eq!(request.raw_data.total_time, 987657045);
        // We've seen eight response times so far.
        assert_eq!(request.raw_data.counter, 8);

        // Tracking status code updates all related fields.
        request.set_status_code(200);
        // We've seen only one status code.
        assert_eq!(request.status_code_counts.len(), 1);
        // First time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 1);
        // As status code tracking is optional, we don't track success/fail here.
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);
        // Nothing else changes.
        assert_eq!(request.raw_data.times.len(), 6);
        assert_eq!(request.raw_data.minimum_time, 1);
        assert_eq!(request.raw_data.maximum_time, 987654321);
        assert_eq!(request.raw_data.total_time, 987657045);
        assert_eq!(request.raw_data.counter, 8);

        // Tracking status code updates all related fields.
        request.set_status_code(200);
        // We've seen only one unique status code.
        assert_eq!(request.status_code_counts.len(), 1);
        // Second time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 2);

        // Tracking status code updates all related fields.
        request.set_status_code(0);
        // We've seen two unique status codes.
        assert_eq!(request.status_code_counts.len(), 2);
        // First time seeing a client-side error.
        assert_eq!(request.status_code_counts[&0], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(500);
        // We've seen three unique status codes.
        assert_eq!(request.status_code_counts.len(), 3);
        // First time seeing an internal server error.
        assert_eq!(request.status_code_counts[&500], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(308);
        // We've seen four unique status codes.
        assert_eq!(request.status_code_counts.len(), 4);
        // First time seeing an internal server error.
        assert_eq!(request.status_code_counts[&308], 1);

        // Tracking status code updates all related fields.
        request.set_status_code(200);
        // We've seen four unique status codes.
        assert_eq!(request.status_code_counts.len(), 4);
        // Third time seeing this status code.
        assert_eq!(request.status_code_counts[&200], 3);
        // Nothing else changes.
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);
        assert_eq!(request.raw_data.times.len(), 6);
        assert_eq!(request.raw_data.minimum_time, 1);
        assert_eq!(request.raw_data.maximum_time, 987654321);
        assert_eq!(request.raw_data.total_time, 987657045);
        assert_eq!(request.raw_data.counter, 8);
    }
}
