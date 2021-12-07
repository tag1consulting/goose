//! Optional metrics collected and aggregated during load tests.
//!
//! By default, Goose collects a large number of metrics while performing a load test.
//! When [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) completes
//! it returns a [`GooseMetrics`] object.
//!
//! When the [`GooseMetrics`] object is viewed with [`std::fmt::Display`], the
//! contained [`GooseTaskMetrics`], [`GooseRequestMetrics`], and
//! [`GooseErrorMetrics`] are displayed in tables.

use chrono::prelude::*;
use http::StatusCode;
use itertools::Itertools;
use num_format::{Locale, ToFormattedString};
use regex::RegexSet;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use std::cmp::{max, Ordering};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::{f32, fmt};
use tokio::io::AsyncWriteExt;

use crate::config::GooseDefaults;
use crate::goose::{get_base_url, GooseMethod, GooseTaskSet};
use crate::logger::GooseLog;
use crate::report;
use crate::util::{
    median, ms_timer_expired, standard_deviation, timer_expired, truncate_string, MovingAverage,
};
#[cfg(feature = "gaggle")]
use crate::worker::{self, GaggleMetrics};
use crate::{AttackMode, GooseAttack, GooseAttackRunState, GooseConfiguration, GooseError};

/// Used to send metrics from [`GooseUser`](../goose/struct.GooseUser.html) threads
/// to the parent Goose process.
///
/// [`GooseUser`](../goose/struct.GooseUser.html) threads send these metrics to the
/// Goose parent process using an
/// [`unbounded Flume channel`](https://docs.rs/flume/*/flume/fn.unbounded.html).
///
/// The parent process will spend up to 80% of its time receiving and aggregating
/// these metrics. The parent process aggregates [`GooseRequestMetric`]s into
/// [`GooseRequestMetricAggregate`], [`GooseTaskMetric`]s into [`GooseTaskMetricAggregate`],
/// and [`GooseErrorMetric`]s into [`GooseErrorMetricAggregate`]. Aggregation happens in the
/// parent process so the individual [`GooseUser`](../goose/struct.GooseUser.html) threads
/// can spend all their time generating and validating load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GooseMetric {
    Request(GooseRequestMetric),
    Task(GooseTaskMetric),
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

/// All tasks executed during a load test.
///
/// Goose optionally tracks metrics about tasks executed during a load test. The
/// metrics can be disabled with either the `--no-task-metrics` or the `--no-metrics`
/// run-time option, or with either
/// [`GooseDefault::NoTaskMetrics`](../config/enum.GooseDefault.html#variant.NoTaskMetrics) or
/// [`GooseDefault::NoMetrics`](../config/enum.GooseDefault.html#variant.NoMetrics).
///
/// Aggregated tasks ([`GooseTaskMetricAggregate`]) are stored in a Vector of Vectors
/// keyed to the order the task is created in the load test.
///
/// # Example
/// When viewed with [`std::fmt::Display`], [`GooseTaskMetrics`] are displayed in
/// a table:
/// ```text
///  === PER TASK METRICS ===
/// ------------------------------------------------------------------------------
/// Name                     |   # times run |        # fails |   task/s |  fail/s
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
pub type GooseTaskMetrics = Vec<Vec<GooseTaskMetricAggregate>>;

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
    /// [`GooseTask`](../goose/struct.GooseTask.html)s by this
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
    /// Counts requests per second. Each element of the vector represents one second.
    pub requests_per_second: Vec<u32>,
    /// Counts errors per second. Each element of the vector represents one second.
    pub errors_per_second: Vec<u32>,
    /// Maintains average response time per second. Each element of the vector represents one second.
    pub average_response_time_per_second: Vec<MovingAverage>,
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
            requests_per_second: Vec::new(),
            errors_per_second: Vec::new(),
            average_response_time_per_second: Vec::new(),
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

    /// Record requests per second metric.
    pub(crate) fn record_requests_per_second(&mut self, second: usize) {
        expand_per_second_metric_array(&mut self.requests_per_second, second, 0);
        self.requests_per_second[second] += 1;

        debug!(
            "incremented second {} for requests per second counter: {}",
            second, self.requests_per_second[second]
        );
    }

    /// Record errors per second metric.
    pub(crate) fn record_errors_per_second(&mut self, second: usize) {
        expand_per_second_metric_array(&mut self.errors_per_second, second, 0);
        self.errors_per_second[second] += 1;

        debug!(
            "incremented second {} for errors per second counter: {}",
            second, self.errors_per_second[second]
        );
    }

    /// Record average response time per second metric.
    pub(crate) fn record_average_response_time_per_second(
        &mut self,
        second: usize,
        response_time: u64,
    ) {
        expand_per_second_metric_array(
            &mut self.average_response_time_per_second,
            second,
            MovingAverage::new(),
        );
        self.average_response_time_per_second[second].add_item(response_time as f32);

        debug!(
            "updated second {} for average response time per second: {}",
            second, self.average_response_time_per_second[second].average
        );
    }
}

/// Expands vectors that collect per-second data for HTML report graphs with a
/// default value.
///
/// We need to do that since we don't know for how long the load test will run
/// and we can't initialize these vectors at the beginning. It is also
/// better to do it as we go to save memory.
fn expand_per_second_metric_array<T: Clone>(data: &mut Vec<T>, second: usize, initial: T) {
    // Each element in per second metric vectors (self.requests_per_second,
    // self.errors_per_second, ...) is count for a given second since the start
    // of the test. Since we don't know how long the test will at the beginning
    // we need to push new elements (second counters) as the test is running.
    if data.len() <= second {
        for _ in 0..(second - data.len() + 1) {
            data.push(initial.clone());
        }
    };
}

/// Implement equality for GooseRequestMetricAggregate. We can't simply derive since
/// we have floats in the struct.
///
/// `Eq` trait has no functions on it - it is there only to inform the compiler
/// that this is an equivalence rather than a partial equivalence.
///
/// See https://doc.rust-lang.org/std/cmp/trait.Eq.html for more information.
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

/// The per-task metrics collected each time a task is invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseTaskMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// An index into [`GooseAttack`]`.task_sets`, indicating which task set this is.
    pub taskset_index: usize,
    /// An index into [`GooseTaskSet`]`.task`, indicating which task this is.
    pub task_index: usize,
    /// The optional name of the task.
    pub name: String,
    /// How long task ran.
    pub run_time: u64,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Which GooseUser thread processed the request.
    pub user: usize,
}
impl GooseTaskMetric {
    /// Create a new GooseTaskMetric metric.
    pub(crate) fn new(
        elapsed: u128,
        taskset_index: usize,
        task_index: usize,
        name: String,
        user: usize,
    ) -> Self {
        GooseTaskMetric {
            elapsed: elapsed as u64,
            taskset_index,
            task_index,
            name,
            run_time: 0,
            success: true,
            user,
        }
    }

    /// Update a GooseTaskMetric metric.
    pub(crate) fn set_time(&mut self, time: u128, success: bool) {
        self.run_time = time as u64;
        self.success = success;
    }
}

/// Aggregated per-task metrics updated each time a task is invoked.
///
/// [`GooseTaskMetric`]s are sent by [`GooseUser`](../goose/struct.GooseUser.html)
/// threads to the Goose parent process where they are aggregated together into this
/// structure, and stored in [`GooseMetrics::tasks`].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GooseTaskMetricAggregate {
    /// An index into [`GooseAttack`](../struct.GooseAttack.html)`.task_sets`,
    /// indicating which task set this is.
    pub taskset_index: usize,
    /// The task set name.
    pub taskset_name: String,
    /// An index into [`GooseTaskSet`](../goose/struct.GooseTaskSet.html)`.task`,
    /// indicating which task this is.
    pub task_index: usize,
    /// An optional name for the task.
    pub task_name: String,
    /// Per-run-time counters, tracking how often tasks take a given time to complete.
    pub times: BTreeMap<usize, usize>,
    /// The shortest run-time for this task.
    pub min_time: usize,
    /// The longest run-time for this task.
    pub max_time: usize,
    /// Total combined run-times for this task.
    pub total_time: usize,
    /// Total number of times task has run.
    pub counter: usize,
    /// Total number of times task has run successfully.
    pub success_count: usize,
    /// Total number of times task has failed.
    pub fail_count: usize,
}
impl GooseTaskMetricAggregate {
    /// Create a new GooseTaskMetricAggregate.
    pub(crate) fn new(
        taskset_index: usize,
        taskset_name: &str,
        task_index: usize,
        task_name: &str,
    ) -> Self {
        GooseTaskMetricAggregate {
            taskset_index,
            taskset_name: taskset_name.to_string(),
            task_index,
            task_name: task_name.to_string(),
            times: BTreeMap::new(),
            min_time: 0,
            max_time: 0,
            total_time: 0,
            counter: 0,
            success_count: 0,
            fail_count: 0,
        }
    }

    /// Track task function elapsed time in milliseconds.
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
///         .register_taskset(taskset!("ExampleUsers")
///             .register_task(task!(example_task))
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
///     $ cargo run -- -H http://example.com -v -u1 -t1
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
///         tasks: [
///             [
///                 GooseTaskMetricAggregate {
///                     taskset_index: 0,
///                     taskset_name: "ExampleUsers",
///                     task_index: 0,
///                     task_name: "",
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
/// async fn example_task(user: &mut GooseUser) -> GooseTaskResult {
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
    /// Tracks when the load test first started with an optional system timestamp.
    pub starting: Option<DateTime<Local>>,
    /// Tracks when all [`GooseUser`](../goose/struct.GooseUser.html) threads fully
    /// started with an optional system timestamp.
    pub started: Option<DateTime<Local>>,
    /// Tracks when the load test first began stopping with an optional system timestamp.
    pub stopping: Option<DateTime<Local>>,
    /// Tracks when the load test stopped with an optional system timestamp.
    pub stopped: Option<DateTime<Local>>,
    /// Total number of seconds the load test ran.
    pub duration: usize,
    /// Total number of users simulated during this load test.
    ///
    /// This value may be smaller than what was configured at start time if the test
    /// didn't run long enough for all configured users to start.
    pub users: usize,
    /// Number of users at the end of each second of the test. Each element of the vector
    /// represents one second.
    pub users_per_second: Vec<usize>,
    /// Tracks details about each request made during the load test.
    ///
    /// Can be disabled with the `--no-metrics` run-time option, or with
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub requests: GooseRequestMetrics,
    /// Tracks details about each task that is invoked during the load test.
    ///
    /// Can be disabled with either the `--no-task-metrics` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoTaskMetrics](../config/enum.GooseDefault.html#variant.NoTaskMetrics) or
    /// [GooseDefault::NoMetrics](../config/enum.GooseDefault.html#variant.NoMetrics).
    pub tasks: GooseTaskMetrics,
    /// Number of tasks at the end of each second of the test. Each element of the vector
    /// represents one second.
    pub tasks_per_second: Vec<u32>,
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
    /// Initialize the task_metrics vector, and determine which hosts are being
    /// load tested to display when printing metrics.
    pub(crate) fn initialize_task_metrics(
        &mut self,
        task_sets: &[GooseTaskSet],
        config: &GooseConfiguration,
        defaults: &GooseDefaults,
    ) -> Result<(), GooseError> {
        self.tasks = Vec::new();
        for task_set in task_sets {
            // Don't initialize task metrics if metrics or task_metrics are disabled.
            if !config.no_metrics {
                if !config.no_task_metrics {
                    let mut task_vector = Vec::new();
                    for task in &task_set.tasks {
                        task_vector.push(GooseTaskMetricAggregate::new(
                            task_set.task_sets_index,
                            &task_set.name,
                            task.tasks_index,
                            &task.name,
                        ));
                    }
                    self.tasks.push(task_vector);
                }

                // The host is not needed on the Worker, metrics are only printed on
                // the Manager.
                if !config.worker {
                    // Determine the base_url for this task based on which of the following
                    // are configured so metrics can be printed.
                    self.hosts.insert(
                        get_base_url(
                            // Determine if --host was configured.
                            if !config.host.is_empty() {
                                Some(config.host.to_string())
                            } else {
                                None
                            },
                            // Determine if the task_set defines a host.
                            task_set.host.clone(),
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

    /// Consumes and display all enabled metrics from a completed load test.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .register_taskset(taskset!("ExampleUsers")
    ///             .register_task(task!(example_task))
    ///         )
    ///         // Set a default host so the load test will start.
    ///         .set_default(GooseDefault::Host, "http://localhost/")?
    ///         // Set a default run time so this test runs to completion.
    ///         .set_default(GooseDefault::RunTime, 1)?
    ///         .execute()
    ///         .await?
    ///         .print();
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_task(user: &mut GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn print(&self) {
        if self.display_metrics {
            info!("printing final metrics after {} seconds...", self.duration);
            print!("{}", self);
        }
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
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    truncate_string(request_key, 24),
                    total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({}%)",
                        request.fail_count.to_formatted_string(&Locale::en),
                        fail_percent as usize
                    ),
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    truncate_string(request_key, 24),
                    total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({:.1}%)",
                        request.fail_count.to_formatted_string(&Locale::en),
                        fail_percent
                    ),
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
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent as usize
                    ),
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.reqs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({:.1}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent
                    ),
                    reqs,
                    fails,
                    reqs_p = reqs_precision,
                    fails_p = fails_precision,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of tasks.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_tasks(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.tasks.is_empty() || !self.display_metrics {
            return Ok(());
        }

        // Display metrics from tasks Vector
        writeln!(
            fmt,
            "\n === PER TASK METRICS ===\n ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>13} | {:>14} | {:>8} | {:>7}",
            "Name", "# times run", "# fails", "task/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        let mut task_count = 0;
        for task_set in &self.tasks {
            let mut displayed_task_set = false;
            for task in task_set {
                task_count += 1;
                let total_count = task.success_count + task.fail_count;
                let fail_percent = if task.fail_count > 0 {
                    task.fail_count as f32 / total_count as f32 * 100.0
                } else {
                    0.0
                };
                let (runs, fails) =
                    per_second_calculations(self.duration, total_count, task.fail_count);
                let runs_precision = determine_precision(runs);
                let fails_precision = determine_precision(fails);

                // First time through display name of task set.
                if !displayed_task_set {
                    writeln!(
                        fmt,
                        " {:24 } |",
                        truncate_string(
                            &format!("{}: {}", task.taskset_index + 1, &task.taskset_name),
                            60
                        ),
                    )?;
                    displayed_task_set = true;
                }

                if fail_percent as usize == 100 || fail_percent as usize == 0 {
                    writeln!(
                        fmt,
                        " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                        truncate_string(
                            &format!("  {}: {}", task.task_index + 1, task.task_name),
                            24
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        format!(
                            "{} ({}%)",
                            task.fail_count.to_formatted_string(&Locale::en),
                            fail_percent as usize
                        ),
                        runs,
                        fails,
                        runs_p = runs_precision,
                        fails_p = fails_precision,
                    )?;
                } else {
                    writeln!(
                        fmt,
                        " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                        truncate_string(
                            &format!("  {}: {}", task.task_index + 1, task.task_name),
                            24
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        format!(
                            "{} ({:.1}%)",
                            task.fail_count.to_formatted_string(&Locale::en),
                            fail_percent
                        ),
                        runs,
                        fails,
                        runs_p = runs_precision,
                        fails_p = fails_precision,
                    )?;
                }
                aggregate_total_count += total_count;
                aggregate_fail_count += task.fail_count;
            }
        }
        if task_count > 1 {
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
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent as usize
                    ),
                    runs,
                    fails,
                    runs_p = runs_precision,
                    fails_p = fails_precision,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<24} | {:>13} | {:>14} | {:>8.runs_p$} | {:>7.fails_p$}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({:.1}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent
                    ),
                    runs,
                    fails,
                    runs_p = runs_precision,
                    fails_p = fails_precision,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of task times.
    ///
    /// This function is invoked by `GooseMetrics::print()` and
    /// `GooseMetrics::print_running()`.
    pub(crate) fn fmt_task_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.tasks.is_empty() || !self.display_metrics {
            return Ok(());
        }

        let mut aggregate_task_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_total_task_time: usize = 0;
        let mut aggregate_task_time_counter: usize = 0;
        let mut aggregate_min_task_time: usize = 0;
        let mut aggregate_max_task_time: usize = 0;
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
        let mut task_count = 0;
        for task_set in &self.tasks {
            let mut displayed_task_set = false;
            for task in task_set {
                task_count += 1;
                // First time through display name of task set.
                if !displayed_task_set {
                    writeln!(
                        fmt,
                        " {:24 } |",
                        truncate_string(
                            &format!("{}: {}", task.taskset_index + 1, &task.taskset_name),
                            60
                        ),
                    )?;
                    displayed_task_set = true;
                }

                // Iterate over user task times, and merge into global task times.
                aggregate_task_times = merge_times(aggregate_task_times, task.times.clone());

                // Increment total task time counter.
                aggregate_total_task_time += &task.total_time;

                // Increment counter tracking individual task times seen.
                aggregate_task_time_counter += &task.counter;

                // If user had new fastest task time, update global fastest task time.
                aggregate_min_task_time = update_min_time(aggregate_min_task_time, task.min_time);

                // If user had new slowest task` time, update global slowest task` time.
                aggregate_max_task_time = update_max_time(aggregate_max_task_time, task.max_time);

                let average = match task.counter {
                    0 => 0.00,
                    _ => task.total_time as f32 / task.counter as f32,
                };
                let average_precision = determine_precision(average);

                writeln!(
                    fmt,
                    " {:<24} | {:>11.avg_precision$} | {:>10} | {:>11} | {:>10}",
                    truncate_string(
                        &format!("  {}: {}", task.task_index + 1, task.task_name),
                        24
                    ),
                    average,
                    format_number(task.min_time),
                    format_number(task.max_time),
                    format_number(median(
                        &task.times,
                        task.counter,
                        task.min_time,
                        task.max_time
                    )),
                    avg_precision = average_precision,
                )?;
            }
        }
        if task_count > 1 {
            let average = match aggregate_task_time_counter {
                0 => 0.00,
                _ => aggregate_total_task_time as f32 / aggregate_task_time_counter as f32,
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
                format_number(aggregate_min_task_time),
                format_number(aggregate_max_task_time),
                format_number(median(
                    &aggregate_task_times,
                    aggregate_task_time_counter,
                    aggregate_min_task_time,
                    aggregate_max_task_time
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
                truncate_string(request_key, 24),
                raw_average,
                format_number(request.raw_data.minimum_time),
                format_number(request.raw_data.maximum_time),
                format_number(median(
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
                format_number(median(
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
            let std_deviation;
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
                std_deviation = standard_deviation(raw_average, co_average);
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
                std_deviation = 0.0;
                co_minimum = 0;
                co_maximum = 0;
            }
            let co_average_precision = determine_precision(co_average);
            let standard_deviation_precision = determine_precision(std_deviation);

            // Coordinated Omission Mitigation was enabled for this request, display the extra data:
            if let Some(co_data) = request.coordinated_omission_data.as_ref() {
                writeln!(
                    fmt,
                    " {:<24} | {:>11.co_avg_precision$} | {:>10.sd_precision$} | {:>11} | {:>10}",
                    truncate_string(request_key, 24),
                    co_average,
                    std_deviation,
                    format_number(co_maximum),
                    format_number(median(
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
                    truncate_string(request_key, 24),
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
            let standard_deviation = standard_deviation(raw_average, co_average);
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
                format_number(median(
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
                truncate_string(request_key, 24),
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
                    truncate_string(request_key, 24),
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
                    truncate_string(request_key, 24),
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
                truncate_string(request_key, 24),
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
        start: &chrono::DateTime<chrono::Local>,
        end: &chrono::DateTime<chrono::Local>,
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
        if !self.final_metrics || self.starting.is_none() {
            return Ok(());
        }

        // Calculations necessary for overview table.
        let starting = self.starting.unwrap();
        let starting_time = starting.format("%Y-%m-%d %H:%M:%S").to_string();
        let started = if self.started.is_some() {
            self.started.unwrap()
        } else {
            self.stopping.unwrap()
        };
        let (starting_seconds, starting_minutes, starting_hours) =
            self.get_seconds_minutes_hours(&starting, &started);
        let start_time = started.format("%Y-%m-%d %H:%M:%S").to_string();
        let stopping = self.stopping.unwrap();
        let (running_seconds, running_minutes, running_hours) =
            self.get_seconds_minutes_hours(&started, &stopping);
        let stopping_time = stopping.format("%Y-%m-%d %H:%M:%S").to_string();
        let stopped = self.stopped.unwrap();
        let stopped_time = stopped.format("%Y-%m-%d %H:%M:%S").to_string();
        let (stopping_seconds, stopping_minutes, stopping_hours) =
            self.get_seconds_minutes_hours(&stopping, &stopped);

        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " Users: {}", self.users)?;
        match self.hosts.len() {
            0 => {
                // A host is required to run a load test.
                unreachable!();
            }
            1 => {
                for host in &self.hosts {
                    writeln!(fmt, " Target host: {}", host)?;
                }
            }
            _ => {
                writeln!(fmt, " Target hosts: ")?;
                for host in &self.hosts {
                    writeln!(fmt, " - {}", host,)?;
                }
            }
        }
        writeln!(
            fmt,
            " Starting: {} - {} (duration: {:02}:{:02}:{:02})",
            starting_time, start_time, starting_hours, starting_minutes, starting_seconds,
        )?;
        // Only display time running if the load test fully started.
        if self.started.is_some() {
            writeln!(
                fmt,
                " Running:  {} - {} (duration: {:02}:{:02}:{:02})",
                start_time, stopping_time, running_hours, running_minutes, running_seconds,
            )?;
        }
        writeln!(
            fmt,
            " Stopping: {} - {} (duration: {:02}:{:02}:{:02})",
            stopping_time, stopped_time, stopping_hours, stopping_minutes, stopping_seconds,
        )?;
        writeln!(
            fmt,
            "\n {} v{}",
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

    /// Records number of tasks and users for a current second.
    ///
    /// This is called from [`GooseAttack::receive_metrics()`] and the data
    /// collected is used to display users and tasks graphs on the HTML report.
    pub(crate) fn record_users_tasks_per_second(&mut self, tasks: u32) {
        if let Some(starting) = self.starting {
            let second = (Utc::now().timestamp() - starting.timestamp()) as usize;

            expand_per_second_metric_array(&mut self.users_per_second, second, 0);
            self.users_per_second[second] = self.users;

            expand_per_second_metric_array(&mut self.tasks_per_second, second, 0);
            self.tasks_per_second[second] = tasks;
        }
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
        let timestamp;
        if let Some(started) = self.started {
            timestamp = started.timestamp();
        } else {
            timestamp = 0;
        }
        s.serialize_field("started", &timestamp)?;
        s.serialize_field("duration", &self.duration)?;
        s.serialize_field("users", &self.users)?;
        s.serialize_field("requests", &self.requests)?;
        s.serialize_field("tasks", &self.tasks)?;
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
        self.fmt_tasks(fmt)?;
        self.fmt_task_times(fmt)?;
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
                    && timer_expired(
                        goose_attack_run_state.running_metrics_timer,
                        running_metrics,
                    )
                {
                    goose_attack_run_state.running_metrics_timer = std::time::Instant::now();
                    goose_attack_run_state.display_running_metrics = true;
                }
            }

            // Record current users and tasks for users/tasks per second graph in HTML report.
            if self.attack_mode != AttackMode::Worker {
                let mut tasks = 0;
                for set in self.task_sets.iter() {
                    tasks += set.tasks.len();
                }
                self.metrics.record_users_tasks_per_second(tasks as u32);
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
                            GaggleMetrics::Tasks(self.metrics.tasks.clone()),
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
                    self.metrics.initialize_task_metrics(
                        &self.task_sets,
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
                self.metrics.initialize_task_metrics(
                    &self.task_sets,
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
            if !self.configuration.report_file.is_empty() {
                if let Some(starting) = self.metrics.starting {
                    let second_since_start =
                        (Utc::now().timestamp() - starting.timestamp()) as usize;

                    merge_request.record_requests_per_second(second_since_start);
                    merge_request.record_average_response_time_per_second(
                        second_since_start,
                        request_metric.response_time,
                    );

                    if !request_metric.success {
                        merge_request.record_errors_per_second(second_since_start);
                    }
                }
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
                    if !request_metric.success && !request_metric.error.is_empty() {
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
                    }
                }
                GooseMetric::Task(raw_task) => {
                    // Store a new metric.
                    self.metrics.tasks[raw_task.taskset_index][raw_task.task_index]
                        .set_time(raw_task.run_time, raw_task.success);
                }
            }
            // Unless flushing all metrics, break out of receive loop after timeout.
            if !flush && ms_timer_expired(receive_started, receive_timeout) {
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
        // If error-file is enabled, convert the raw request to a GooseErrorMetric and send it
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
    pub(crate) fn update_duration(&mut self) {
        if let Some(started) = self.started {
            self.metrics.duration = started.elapsed().as_secs() as usize;
        } else {
            self.metrics.duration = 0;
        }
    }

    // Write an HTML-formatted report, if enabled.
    pub(crate) async fn write_html_report(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // Only write the report if enabled.
        if let Some(report_file) = goose_attack_run_state.report_file.as_mut() {
            // Prepare report summary variables.
            let users = self.metrics.users.to_string();

            let starting = self.metrics.starting.unwrap();
            let started = if self.metrics.started.is_some() {
                self.metrics.started.unwrap()
            } else {
                self.metrics.stopping.unwrap()
            };
            let (starting_seconds, starting_minutes, starting_hours) =
                self.metrics.get_seconds_minutes_hours(&starting, &started);
            let stopping = self.metrics.stopping.unwrap();
            let (running_seconds, running_minutes, running_hours) =
                self.metrics.get_seconds_minutes_hours(&started, &stopping);
            let stopped = self.metrics.stopped.unwrap();
            let (stopping_seconds, stopping_minutes, stopping_hours) =
                self.metrics.get_seconds_minutes_hours(&stopping, &stopped);

            let mut report_range = format!(
                "<p>Starting: <span>{} - {} (Duration: {:02}:{:02}:{:02})</span></p>",
                starting.format("%Y-%m-%d %H:%M:%S").to_string(),
                started.format("%Y-%m-%d %H:%M:%S").to_string(),
                starting_hours,
                starting_minutes,
                starting_seconds,
            );

            if self.metrics.started.is_some() {
                report_range.push_str(&format!(
                    "<p>Running: <span>{} - {} (Duration: {:02}:{:02}:{:02})</span></p>",
                    started.format("%Y-%m-%d %H:%M:%S").to_string(),
                    stopping.format("%Y-%m-%d %H:%M:%S").to_string(),
                    running_hours,
                    running_minutes,
                    running_seconds,
                ));
            }

            report_range.push_str(&format!(
                "<p>Stopping: <span>{} - {} (Duration: {:02}:{:02}:{:02})</span></p>",
                stopping.format("%Y-%m-%d %H:%M:%S").to_string(),
                stopped.format("%Y-%m-%d %H:%M:%S").to_string(),
                stopping_hours,
                stopping_minutes,
                stopping_seconds,
            ));

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

            // Generate graphs

            // If the metrics were reset when the load test was started we don't display
            // the starting period on the graph.
            let (graph_starting, graph_started) = if self.configuration.no_reset_metrics
                && Some(starting) == self.metrics.starting
                && Some(started) == self.metrics.started
            {
                (Some(starting), Some(started))
            } else {
                (None, None)
            };

            // If stopping was done in less than a second do not display it as it won't be visible
            // on the graph.
            let (graph_stopping, graph_stopped) = if Some(stopping) == self.metrics.stopping
                && Some(stopped) == self.metrics.stopped
                && stopped == stopping
            {
                (Some(stopping), Some(stopped))
            } else {
                (None, None)
            };

            let mut total_graph_seconds = 0;
            for path_metric in self.metrics.requests.values() {
                total_graph_seconds =
                    max(total_graph_seconds, path_metric.requests_per_second.len());
            }
            for path_metric in self.metrics.requests.values() {
                total_graph_seconds = max(total_graph_seconds, path_metric.errors_per_second.len());
            }
            for path_metric in self.metrics.requests.values() {
                total_graph_seconds = max(
                    total_graph_seconds,
                    path_metric.average_response_time_per_second.len(),
                );
            }
            total_graph_seconds = max(total_graph_seconds, self.metrics.users_per_second.len());

            // Generate requests per second graph.
            let mut rps = vec![0; total_graph_seconds];
            for path_metric in self.metrics.requests.values() {
                for (second, count) in path_metric.requests_per_second.iter().enumerate() {
                    rps[second] += count;
                }
            }

            let graph_rps_template = report::graph_rps_template(
                &self.add_timestamp_to_html_graph_data(rps, &starting, &started),
                graph_starting,
                graph_started,
                graph_stopping,
                graph_stopped,
            );

            // Generate average response times per second graph.
            let mut response_times = vec![MovingAverage::new(); total_graph_seconds];
            for path_metric in self.metrics.requests.values() {
                for (second, avg) in path_metric
                    .average_response_time_per_second
                    .iter()
                    .enumerate()
                {
                    response_times[second].add_item(avg.average);
                }
            }

            let response_times = response_times
                .iter()
                .map(|moving_average| moving_average.average as u32)
                .collect::<Vec<_>>();

            let graph_average_response_time_template = report::graph_average_response_time_template(
                &self.add_timestamp_to_html_graph_data(response_times, &starting, &started),
                graph_starting,
                graph_started,
                graph_stopping,
                graph_stopped,
            );

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
                                standard_deviation(raw_average, co_average)
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
                        standard_deviation(raw_average, co_average),
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

            // Only build the tasks template if --no-task-metrics isn't enabled.
            let tasks_template: String;
            if !self.configuration.no_task_metrics {
                let mut task_metrics = Vec::new();
                let mut aggregate_total_count = 0;
                let mut aggregate_fail_count = 0;
                let mut aggregate_task_time_counter: usize = 0;
                let mut aggregate_task_time_minimum: usize = 0;
                let mut aggregate_task_time_maximum: usize = 0;
                let mut aggregate_task_times: BTreeMap<usize, usize> = BTreeMap::new();
                for (task_set_counter, task_set) in self.metrics.tasks.iter().enumerate() {
                    for (task_counter, task) in task_set.iter().enumerate() {
                        if task_counter == 0 {
                            // Only the taskset_name is used for task sets.
                            task_metrics.push(report::TaskMetric {
                                is_task_set: true,
                                task: "".to_string(),
                                name: task.taskset_name.to_string(),
                                number_of_requests: 0,
                                number_of_failures: 0,
                                response_time_average: "".to_string(),
                                response_time_minimum: 0,
                                response_time_maximum: 0,
                                requests_per_second: "".to_string(),
                                failures_per_second: "".to_string(),
                            });
                        }
                        let total_run_count = task.success_count + task.fail_count;
                        let (requests_per_second, failures_per_second) = per_second_calculations(
                            self.metrics.duration,
                            total_run_count,
                            task.fail_count,
                        );
                        let average = match task.counter {
                            0 => 0.00,
                            _ => task.total_time as f32 / task.counter as f32,
                        };
                        task_metrics.push(report::TaskMetric {
                            is_task_set: false,
                            task: format!("{}.{}", task_set_counter, task_counter),
                            name: task.task_name.to_string(),
                            number_of_requests: total_run_count,
                            number_of_failures: task.fail_count,
                            response_time_average: format!("{:.2}", average),
                            response_time_minimum: task.min_time,
                            response_time_maximum: task.max_time,
                            requests_per_second: format!("{:.2}", requests_per_second),
                            failures_per_second: format!("{:.2}", failures_per_second),
                        });

                        aggregate_total_count += total_run_count;
                        aggregate_fail_count += task.fail_count;
                        aggregate_task_times =
                            merge_times(aggregate_task_times, task.times.clone());
                        aggregate_task_time_counter += &task.counter;
                        aggregate_task_time_minimum =
                            update_min_time(aggregate_task_time_minimum, task.min_time);
                        aggregate_task_time_maximum =
                            update_max_time(aggregate_task_time_maximum, task.max_time);
                    }
                }

                let (aggregate_requests_per_second, aggregate_failures_per_second) =
                    per_second_calculations(
                        self.metrics.duration,
                        aggregate_total_count,
                        aggregate_fail_count,
                    );
                task_metrics.push(report::TaskMetric {
                    is_task_set: false,
                    task: "".to_string(),
                    name: "Aggregated".to_string(),
                    number_of_requests: aggregate_total_count,
                    number_of_failures: aggregate_fail_count,
                    response_time_average: format!(
                        "{:.2}",
                        raw_aggregate_response_time_counter as f32 / aggregate_total_count as f32
                    ),
                    response_time_minimum: aggregate_task_time_minimum,
                    response_time_maximum: aggregate_task_time_maximum,
                    requests_per_second: format!("{:.2}", aggregate_requests_per_second),
                    failures_per_second: format!("{:.2}", aggregate_failures_per_second),
                });
                let mut tasks_rows = Vec::new();
                // Compile the task metrics template.
                for metric in task_metrics {
                    tasks_rows.push(report::task_metrics_row(metric));
                }

                // Generate active users graph.
                let graph_users_per_second = report::graph_users_per_second_template(
                    &self.add_timestamp_to_html_graph_data(
                        self.metrics.users_per_second.clone(),
                        &starting,
                        &started,
                    ),
                    graph_starting,
                    graph_started,
                    graph_stopping,
                    graph_stopped,
                );

                // Generate active tasks graph.
                let graph_tasks_per_second = report::graph_tasks_per_second_template(
                    &self.add_timestamp_to_html_graph_data(
                        self.metrics.tasks_per_second.clone(),
                        &starting,
                        &started,
                    ),
                    graph_starting,
                    graph_started,
                    graph_stopping,
                    graph_stopped,
                );

                tasks_template = report::task_metrics_template(
                    &tasks_rows.join("\n"),
                    &graph_tasks_per_second,
                    &graph_users_per_second,
                );
            } else {
                tasks_template = "".to_string();
            }

            // Only build the tasks template if --no-task-metrics isn't enabled.
            let errors_template: String;
            if !self.metrics.errors.is_empty() {
                let mut error_rows = Vec::new();
                for error in self.metrics.errors.values() {
                    error_rows.push(report::error_row(error));
                }

                // Generate errors per second graph.
                let mut eps = vec![0; total_graph_seconds];
                for path_metric in self.metrics.requests.values() {
                    for (second, count) in path_metric.errors_per_second.iter().enumerate() {
                        eps[second] += count;
                    }
                }

                let graph_eps_template = report::graph_eps_template(
                    &self.add_timestamp_to_html_graph_data(eps, &starting, &started),
                    graph_starting,
                    graph_started,
                    graph_stopping,
                    graph_stopped,
                );

                errors_template =
                    report::errors_template(&error_rows.join("\n"), &graph_eps_template);
            } else {
                errors_template = "".to_string();
            }

            // Only build the status_code template if --status-codes is enabled.
            let status_code_template: String;
            if self.configuration.status_codes {
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
                status_code_template =
                    report::status_code_metrics_template(&status_code_rows.join("\n"));
            } else {
                // If --status-codes is not enabled, return an empty template.
                status_code_template = "".to_string();
            }

            // Compile the report template.
            let report = report::build_report(
                &users,
                &report_range,
                hosts,
                report::GooseReportTemplates {
                    raw_requests_template: &raw_requests_rows.join("\n"),
                    raw_responses_template: &raw_responses_rows.join("\n"),
                    co_requests_template: &co_requests_template,
                    co_responses_template: &co_responses_template,
                    tasks_template: &tasks_template,
                    status_codes_template: &status_code_template,
                    errors_template: &errors_template,
                    graph_rps_template: &graph_rps_template,
                    graph_average_response_time_template: &graph_average_response_time_template,
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

    fn add_timestamp_to_html_graph_data<T: Copy>(
        &self,
        data: Vec<T>,
        starting: &DateTime<Local>,
        started: &DateTime<Local>,
    ) -> Vec<(String, T)> {
        data.iter()
            .enumerate()
            .filter(|(second, _)| {
                if self.configuration.no_reset_metrics {
                    true
                } else {
                    *second as i64 + starting.timestamp() >= started.timestamp()
                }
            })
            .map(|(second, &count)| {
                (
                    Local
                        .timestamp(second as i64 + starting.timestamp(), 0)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    count,
                )
            })
            .collect::<Vec<_>>()
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
            let new_count;
            if let Some(existing_status_code_count) = aggregate_status_code_counts.get(status_code)
            {
                new_count = *existing_status_code_count + *count;
            } else {
                new_count = *count;
            }
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

    #[test]
    fn goose_record_requests_per_second() {
        // Should be initialized with empty requests per second vector.
        let mut metric_aggregate = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0);
        assert_eq!(metric_aggregate.requests_per_second.len(), 0);

        metric_aggregate.record_requests_per_second(0);
        metric_aggregate.record_requests_per_second(0);
        metric_aggregate.record_requests_per_second(0);
        metric_aggregate.record_requests_per_second(1);
        metric_aggregate.record_requests_per_second(2);
        metric_aggregate.record_requests_per_second(2);
        metric_aggregate.record_requests_per_second(2);
        metric_aggregate.record_requests_per_second(2);
        metric_aggregate.record_requests_per_second(2);
        assert_eq!(metric_aggregate.requests_per_second.len(), 3);
        assert_eq!(metric_aggregate.requests_per_second[0], 3);
        assert_eq!(metric_aggregate.requests_per_second[1], 1);
        assert_eq!(metric_aggregate.requests_per_second[2], 5);

        metric_aggregate.record_requests_per_second(100);
        metric_aggregate.record_requests_per_second(100);
        metric_aggregate.record_requests_per_second(100);
        metric_aggregate.record_requests_per_second(0);
        metric_aggregate.record_requests_per_second(1);
        metric_aggregate.record_requests_per_second(2);
        metric_aggregate.record_requests_per_second(5);
        assert_eq!(metric_aggregate.requests_per_second.len(), 101);
        assert_eq!(metric_aggregate.requests_per_second[0], 4);
        assert_eq!(metric_aggregate.requests_per_second[1], 2);
        assert_eq!(metric_aggregate.requests_per_second[2], 6);
        assert_eq!(metric_aggregate.requests_per_second[3], 0);
        assert_eq!(metric_aggregate.requests_per_second[4], 0);
        assert_eq!(metric_aggregate.requests_per_second[5], 1);
        assert_eq!(metric_aggregate.requests_per_second[100], 3);
        for second in 6..100 {
            assert_eq!(metric_aggregate.requests_per_second[second], 0);
        }
    }

    #[test]
    fn goose_record_errors_per_second() {
        // Should be initialized with empty errors per second vector.
        let mut metric_aggregate = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0);
        assert_eq!(metric_aggregate.errors_per_second.len(), 0);

        metric_aggregate.record_errors_per_second(0);
        metric_aggregate.record_errors_per_second(0);
        metric_aggregate.record_errors_per_second(0);
        metric_aggregate.record_errors_per_second(1);
        metric_aggregate.record_errors_per_second(2);
        metric_aggregate.record_errors_per_second(2);
        metric_aggregate.record_errors_per_second(2);
        metric_aggregate.record_errors_per_second(2);
        metric_aggregate.record_errors_per_second(2);
        assert_eq!(metric_aggregate.errors_per_second.len(), 3);
        assert_eq!(metric_aggregate.errors_per_second[0], 3);
        assert_eq!(metric_aggregate.errors_per_second[1], 1);
        assert_eq!(metric_aggregate.errors_per_second[2], 5);

        metric_aggregate.record_errors_per_second(100);
        metric_aggregate.record_errors_per_second(100);
        metric_aggregate.record_errors_per_second(100);
        metric_aggregate.record_errors_per_second(0);
        metric_aggregate.record_errors_per_second(1);
        metric_aggregate.record_errors_per_second(2);
        metric_aggregate.record_errors_per_second(5);
        assert_eq!(metric_aggregate.errors_per_second.len(), 101);
        assert_eq!(metric_aggregate.errors_per_second[0], 4);
        assert_eq!(metric_aggregate.errors_per_second[1], 2);
        assert_eq!(metric_aggregate.errors_per_second[2], 6);
        assert_eq!(metric_aggregate.errors_per_second[3], 0);
        assert_eq!(metric_aggregate.errors_per_second[4], 0);
        assert_eq!(metric_aggregate.errors_per_second[5], 1);
        assert_eq!(metric_aggregate.errors_per_second[100], 3);
        for second in 6..100 {
            assert_eq!(metric_aggregate.errors_per_second[second], 0);
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn goose_record_average_response_time_per_second() {
        // Should be initialized with empty errors per second vector.
        let mut metric_aggregate = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0);
        assert_eq!(metric_aggregate.average_response_time_per_second.len(), 0);

        metric_aggregate.record_average_response_time_per_second(0, 5);
        metric_aggregate.record_average_response_time_per_second(0, 4);
        metric_aggregate.record_average_response_time_per_second(0, 3);
        metric_aggregate.record_average_response_time_per_second(1, 1);
        metric_aggregate.record_average_response_time_per_second(2, 4);
        metric_aggregate.record_average_response_time_per_second(2, 8);
        metric_aggregate.record_average_response_time_per_second(2, 12);
        metric_aggregate.record_average_response_time_per_second(2, 4);
        metric_aggregate.record_average_response_time_per_second(2, 4);
        assert_eq!(metric_aggregate.average_response_time_per_second.len(), 3);
        assert_eq!(
            metric_aggregate.average_response_time_per_second[0].average,
            4.
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[1].average,
            1.
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[2].average,
            6.4
        );

        metric_aggregate.record_average_response_time_per_second(100, 5);
        metric_aggregate.record_average_response_time_per_second(100, 9);
        metric_aggregate.record_average_response_time_per_second(100, 7);
        metric_aggregate.record_average_response_time_per_second(0, 2);
        metric_aggregate.record_average_response_time_per_second(1, 2);
        metric_aggregate.record_average_response_time_per_second(2, 5);
        metric_aggregate.record_average_response_time_per_second(5, 2);
        assert_eq!(metric_aggregate.average_response_time_per_second.len(), 101);
        assert_eq!(
            metric_aggregate.average_response_time_per_second[0].average,
            3.5
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[1].average,
            1.5
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[2].average,
            6.166667
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[3].average,
            0.
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[4].average,
            0.
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[5].average,
            2.
        );
        assert_eq!(
            metric_aggregate.average_response_time_per_second[100].average,
            7.
        );
        for second in 6..100 {
            assert_eq!(
                metric_aggregate.average_response_time_per_second[second].average,
                0.
            );
        }
    }
}
