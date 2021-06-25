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
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::{f32, fmt};
use tokio::io::AsyncWriteExt;

use crate::goose::{GooseMethod, GooseTaskSet};
use crate::util;
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
/// [`GooseRequestMetricAggregate`], and [`GooseTaskMetric`]s into
/// [`GooseTaskMetricAggregate`]. [`GooseErrorMetric`]s do not require any further
/// aggregation. Aggregation happens in the parent process so the individual
/// [`GooseUser`](../goose/struct.GooseUser.html) threads can spend all their time
/// generating and validating load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GooseMetric {
    Request(GooseRequestMetric),
    Task(GooseTaskMetric),
    Error(GooseErrorMetric),
}

/// Mitigate the loss of data (coordinated omission) due to stalls on the upstream server.
///
/// Stalling can happen for many reasons, for example: garbage collection, a cache stampede,
/// even unrelated load on the same server. Without any mitigation, Goose loses
/// statistically relevant information as [`GooseUser`] threads are unable to make additional
/// requests while they are blocked by an upstream stall. Goose mitigates this by backfilling
/// the requests that would have been made during that time. Backfilled requests show up in
/// the `--request-file` if enabled, though they were not actually sent to the server.
///
/// By default, Goose is configured to backfill based on the Average response time seen for the
/// stalled request. However, different server configurations and testing plans can work on
/// different assumptions so the following configurations are supported.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum GooseCoordinatedOmissionMitigation {
    /// Backfill based on the average response_time for this request (default).
    Average,
    /// Backfill based on the maximum response_time for this request.
    Maximum,
    /// Backfill based on the minimum response_time for this request.
    Minimum,
    /// Completely disable coordinated omission mitigation.
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
        let matches = co_mitigation.matches(&s);
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
/// [`GooseDefault::NoMetrics`](../enum.GooseDefault.html#variant.NoMetrics).
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
/// [`GooseDefault::NoTaskMetrics`](../enum.GooseDefault.html#variant.NoTaskMetrics) or
/// [`GooseDefault::NoMetrics`](../enum.GooseDefault.html#variant.NoMetrics).
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
/// as a [`GooseErrorMetric`](./struct.GooseErrorMetric.html), and they are all stored
/// together within a `BTreeMap` which is returned by
/// [`GooseAttack::execute()`](../struct.GooseAttack.html#method.execute) when a load test
/// completes.
///
/// `GooseErrorMetrics` can be disabled with the `--no-error-summary` run-time option, or with
/// [GooseDefault::NoErrorSummary](../enum.GooseDefault.html#variant.NoErrorSummary).
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
pub type GooseErrorMetrics = BTreeMap<String, GooseErrorMetric>;

/// For tracking and counting requests made during a load test.
///
/// The request that Goose is making. User threads send this data to the parent thread
/// when metrics are enabled. This request object must be provided to calls to
/// [`set_success`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_success)
/// or
/// [`set_failure`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.set_failure)
/// so Goose knows which request is being updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseRequestMetric {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// The method being used (ie, Get, Post, etc).
    pub method: GooseMethod,
    /// The optional name of the request.
    pub name: String,
    /// The full URL that was requested.
    pub url: String,
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
    /// Which GooseUser thread processed the request.
    pub user: usize,
    /// The optional error caused by this request.
    pub error: String,
    /// If non-zero, Coordinated Omission Mitigation detected an abnormally long response time on
    /// the upstream server, blocking requests from being made.
    pub coordinated_omission_elapsed: u64,
    /// If non-zero, the expected cadence of looping through all GooseTasks by this GooseUser.
    pub coordinated_omission_cadence: u64,
}
impl GooseRequestMetric {
    pub(crate) fn new(
        method: GooseMethod,
        name: &str,
        url: &str,
        elapsed: u128,
        user: usize,
    ) -> Self {
        GooseRequestMetric {
            elapsed: elapsed as u64,
            method,
            name: name.to_string(),
            url: url.to_string(),
            final_url: "".to_string(),
            redirected: false,
            response_time: 0,
            status_code: 0,
            success: true,
            update: false,
            user,
            error: "".to_string(),
            coordinated_omission_elapsed: 0,
            coordinated_omission_cadence: 0,
        }
    }

    // Record the final URL returned.
    pub(crate) fn set_final_url(&mut self, final_url: &str) {
        self.final_url = final_url.to_string();
        if self.final_url != self.url {
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
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GooseRequestMetricAggregate {
    /// The request path for which metrics are being collected.
    ///
    /// For example: "/".
    pub path: String,
    /// The method for which metrics are being collected.
    ///
    /// For example: [`GooseMethod::Get`].
    pub method: GooseMethod,
    /// Per-response-time counters, tracking how often pages are returned with this response time.
    ///
    /// All response times between 1 and 100ms are stored without any rounding. Response times between
    /// 100 and 500ms are rounded to the nearest 10ms and then stored. Response times betwee 500 and
    /// 1000ms are rounded to the nearest 100ms. Response times larger than 1000ms are rounded to the
    /// nearest 1000ms.
    pub response_times: BTreeMap<usize, usize>,
    /// The shortest response time seen so far.
    ///
    /// For example a `min_response_time` of `3` means the quickest response for this method-path
    /// pair returned in 3 milliseconds. This value is not rounded.
    pub min_response_time: usize,
    /// The longest response time seen so far.
    ///
    /// For example a `max_response_time` of `2013` means the slowest response for this method-path
    /// pair returned in 2013 milliseconds. This value is not rounded.
    pub max_response_time: usize,
    /// Total combined response times seen so far.
    ///
    /// A running total of all response times returned for this method-path pair.
    pub total_response_time: usize,
    /// Total number of response times seen so far.
    ///
    /// A count of how many requests have been tracked for this method-path pair.
    pub response_time_counter: usize,
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
            response_times: BTreeMap::new(),
            min_response_time: 0,
            max_response_time: 0,
            total_response_time: 0,
            response_time_counter: 0,
            status_code_counts: HashMap::new(),
            success_count: 0,
            fail_count: 0,
            load_test_hash,
        }
    }

    /// Track response time.
    pub(crate) fn set_response_time(&mut self, response_time: u64) {
        // Perform this conversin only once, then re-use throughout this funciton.
        let response_time_usize = response_time as usize;

        // Update minimum if this one is fastest yet.
        if self.min_response_time == 0
            || (response_time_usize > 0 && response_time_usize < self.min_response_time)
        {
            self.min_response_time = response_time_usize;
        }

        // Update maximum if this one is slowest yet.
        if response_time_usize > self.max_response_time {
            self.max_response_time = response_time_usize;
        }

        // Update total_response time, adding in this one.
        self.total_response_time += response_time_usize;

        // Each time we store a new response time, increment counter by one.
        self.response_time_counter += 1;

        // Round the response time so we can combine similar times together and
        // minimize required memory to store and push upstream to the parent.
        // No rounding for 1-100ms response times.
        let rounded_response_time = if response_time < 100 {
            response_time_usize
        }
        // Round to nearest 10 for 100-500ms response times.
        else if response_time < 500 {
            ((response_time as f64 / 10.0).round() * 10.0) as usize
        }
        // Round to nearest 100 for 500-1000ms response times.
        else if response_time < 1000 {
            ((response_time as f64 / 100.0).round() * 100.0) as usize
        }
        // Round to nearest 1000 for all larger response times.
        else {
            ((response_time as f64 / 1000.0).round() * 1000.0) as usize
        };

        let counter = match self.response_times.get(&rounded_response_time) {
            // We've seen this response_time before, increment counter.
            Some(c) => {
                debug!("got {:?} counter: {}", rounded_response_time, c);
                *c + 1
            }
            // First time we've seen this response time, initialize counter.
            None => {
                debug!("no match for counter: {}", rounded_response_time);
                1
            }
        };
        self.response_times.insert(rounded_response_time, counter);
        debug!("incremented {} counter: {}", rounded_response_time, counter);
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
/// fn main() -> Result<(), GooseError> {
///     let goose_metrics: GooseMetrics = GooseAttack::initialize()?
///         .register_taskset(taskset!("ExampleUsers")
///             .register_task(task!(example_task))
///         )
///         // Set a default host so the load test will start.
///         .set_default(GooseDefault::Host, "http://localhost/")?
///         // Set a default run time so this test runs to completion.
///         .set_default(GooseDefault::RunTime, 1)?
///         .execute()?;
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
/// async fn example_task(user: &GooseUser) -> GooseTaskResult {
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
    /// An optional system timestamp indicating when the load test started.
    pub started: Option<DateTime<Local>>,
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
    /// [GooseDefault::NoMetrics](../enum.GooseDefault.html#variant.NoMetrics).
    pub requests: GooseRequestMetrics,
    /// Tracks details about each task that is invoked during the load test.
    ///
    /// Can be disabled with either the `--no-task-metrics` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoTaskMetrics](../enum.GooseDefault.html#variant.NoTaskMetrics) or
    /// [GooseDefault::NoMetrics](../enum.GooseDefault.html#variant.NoMetrics).
    pub tasks: GooseTaskMetrics,
    /// Tracks and counts each time an error is detected during the load test.
    ///
    /// Can be disabled with either the `--no-error-summary` or `--no-metrics` run-time options,
    /// or with either the
    /// [GooseDefault::NoErrorSummary](../enum.GooseDefault.html#variant.NoErrorSummary) or
    /// [GooseDefault::NoMetrics](../enum.GooseDefault.html#variant.NoMetrics).
    pub errors: GooseErrorMetrics,
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
    /// Initialize the task_metrics vector.
    pub(crate) fn initialize_task_metrics(
        &mut self,
        task_sets: &[GooseTaskSet],
        config: &GooseConfiguration,
    ) {
        self.tasks = Vec::new();
        if !config.no_metrics && !config.no_task_metrics {
            for task_set in task_sets {
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
        }
    }

    /// Consumes and display all enabled metrics from a completed load test.
    ///
    /// # Example
    /// ```rust
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .register_taskset(taskset!("ExampleUsers")
    ///             .register_task(task!(example_task))
    ///         )
    ///         // Set a default host so the load test will start.
    ///         .set_default(GooseDefault::Host, "http://localhost/")?
    ///         // Set a default run time so this test runs to completion.
    ///         .set_default(GooseDefault::RunTime, 1)?
    ///         .execute()?
    ///         .print();
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_task(user: &GooseUser) -> GooseTaskResult {
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
                    util::truncate_string(&request_key, 24),
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
                    util::truncate_string(&request_key, 24),
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
                        util::truncate_string(
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
                        util::truncate_string(
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
                        util::truncate_string(
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
                        util::truncate_string(
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
                    util::truncate_string(
                        &format!("  {}: {}", task.task_index + 1, task.task_name),
                        24
                    ),
                    average,
                    format_number(task.min_time),
                    format_number(task.max_time),
                    format_number(util::median(
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
                format_number(util::median(
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

        let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_total_response_time: usize = 0;
        let mut aggregate_response_time_counter: usize = 0;
        let mut aggregate_min_response_time: usize = 0;
        let mut aggregate_max_response_time: usize = 0;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<24} | {:>11} | {:>10} | {:>10} | {:>11}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        for (request_key, request) in self.requests.iter().sorted() {
            let average = match request.response_time_counter {
                0 => 0.0,
                _ => request.total_response_time as f32 / request.response_time_counter as f32,
            };
            let average_precision = determine_precision(average);

            // Iterate over user response times, and merge into global response times.
            aggregate_response_times =
                merge_times(aggregate_response_times, request.response_times.clone());

            // Increment total response time counter.
            aggregate_total_response_time += &request.total_response_time;

            // Increment counter tracking individual response times seen.
            aggregate_response_time_counter += &request.response_time_counter;

            // If user had new fastest response time, update global fastest response time.
            aggregate_min_response_time =
                update_min_time(aggregate_min_response_time, request.min_response_time);

            // If user had new slowest response time, update global slowest response time.
            aggregate_max_response_time =
                update_max_time(aggregate_max_response_time, request.max_response_time);

            writeln!(
                fmt,
                " {:<24} | {:>11.avg_precision$} | {:>10} | {:>11} | {:>10}",
                util::truncate_string(&request_key, 24),
                average,
                format_number(request.min_response_time),
                format_number(request.max_response_time),
                format_number(util::median(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time
                )),
                avg_precision = average_precision,
            )?;
        }
        if self.requests.len() > 1 {
            let average = match aggregate_response_time_counter {
                0 => 0.0,
                _ => aggregate_total_response_time as f32 / aggregate_response_time_counter as f32,
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
                format_number(aggregate_min_response_time),
                format_number(aggregate_max_response_time),
                format_number(util::median(
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time
                )),
                avg_precision = average_precision,
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

        let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_total_response_time: usize = 0;
        let mut aggregate_response_time_counter: usize = 0;
        let mut aggregate_min_response_time: usize = 0;
        let mut aggregate_max_response_time: usize = 0;
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
        for (request_key, request) in self.requests.iter().sorted() {
            // Iterate over user response times, and merge into global response times.
            aggregate_response_times =
                merge_times(aggregate_response_times, request.response_times.clone());

            // Increment total response time counter.
            aggregate_total_response_time += &request.total_response_time;

            // Increment counter tracking individual response times seen.
            aggregate_response_time_counter += &request.response_time_counter;

            // If user had new fastest response time, update global fastest response time.
            aggregate_min_response_time =
                update_min_time(aggregate_min_response_time, request.min_response_time);

            // If user had new slowest response time, update global slowest response time.
            aggregate_max_response_time =
                update_max_time(aggregate_max_response_time, request.max_response_time);
            // Sort response times so we can calculate a mean.
            writeln!(
                fmt,
                " {:<24} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6} | {:>6}",
                util::truncate_string(&request_key, 24),
                calculate_response_time_percentile(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                    0.5
                ),
                calculate_response_time_percentile(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                    0.75
                ),
                calculate_response_time_percentile(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                    0.98
                ),
                calculate_response_time_percentile(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                    0.99
                ),
                calculate_response_time_percentile(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                    0.999
                ),
                calculate_response_time_percentile(
                    &request.response_times,
                    request.response_time_counter,
                    request.min_response_time,
                    request.max_response_time,
                    0.999
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
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time,
                    0.5
                ),
                calculate_response_time_percentile(
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time,
                    0.75
                ),
                calculate_response_time_percentile(
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time,
                    0.98
                ),
                calculate_response_time_percentile(
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time,
                    0.99
                ),
                calculate_response_time_percentile(
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time,
                    0.999
                ),
                calculate_response_time_percentile(
                    &aggregate_response_times,
                    aggregate_response_time_counter,
                    aggregate_min_response_time,
                    aggregate_max_response_time,
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
                util::truncate_string(&request_key, 24),
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
        // Formats from zero to six tables of data, depending on what data is contained
        // and which contained flags are set.
        self.fmt_tasks(fmt)?;
        self.fmt_task_times(fmt)?;
        self.fmt_requests(fmt)?;
        self.fmt_response_times(fmt)?;
        self.fmt_percentiles(fmt)?;
        self.fmt_status_codes(fmt)?;
        self.fmt_errors(fmt)
    }
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
/// [GooseDefault::NoErrorSummary](../enum.GooseDefault.html#variant.NoErrorSummary).
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
pub struct GooseErrorMetric {
    /// The method that resulted in an error.
    pub method: GooseMethod,
    /// The optional name of the request.
    pub name: String,
    /// The error string.
    pub error: String,
    /// A counter reflecting how many times this error occurred.
    pub occurrences: usize,
}
impl GooseErrorMetric {
    pub(crate) fn new(method: GooseMethod, name: String, error: String) -> Self {
        GooseErrorMetric {
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
                    self.metrics
                        .initialize_task_metrics(&self.task_sets, &self.configuration);
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
                self.metrics
                    .initialize_task_metrics(&self.task_sets, &self.configuration);
                // Restart the timer now that all threads are launched.
                self.started = Some(std::time::Instant::now());
            } else if self.metrics.users < users {
                println!(
                    "{} of {} users hatched, timer expired.\n",
                    self.metrics.users, users
                );
            } else {
                println!("All {} users hatched.\n", self.metrics.users);
            }
        }

        Ok(())
    }

    // Store `GooseRequestMetric` in a `GooseRequestMetricAggregate` within the
    // `GooseMetrics.requests` `HashMap`, merging if already existing, or creating new.
    // Also writes it to the request_file if enabled.
    async fn record_request_metric(
        &mut self,
        request_metric: &GooseRequestMetric,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) {
        let key = format!("{} {}", request_metric.method, request_metric.name);
        let mut merge_request = match self.metrics.requests.get(&key) {
            Some(m) => m.clone(),
            None => GooseRequestMetricAggregate::new(
                &request_metric.name,
                request_metric.method.clone(),
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
            merge_request.set_response_time(request_metric.response_time);
            if self.configuration.status_codes {
                merge_request.set_status_code(request_metric.status_code);
            }
            if request_metric.success {
                merge_request.success_count += 1;
            } else {
                merge_request.fail_count += 1;
            }
        }

        // Options should appear above, search for formatted_log.
        let formatted_log = match self.configuration.requests_format.as_str() {
            // Use serde_json to create JSON.
            "json" => json!(request_metric).to_string(),
            // Manually create CSV, library doesn't support single-row string conversion.
            "csv" => GooseAttack::prepare_csv(&request_metric, goose_attack_run_state),
            // Raw format is Debug output for GooseRequestMetric structure.
            "raw" => format!("{:?}", request_metric),
            _ => unreachable!(),
        };
        if let Some(file) = goose_attack_run_state.requests_file.as_mut() {
            match file.write(format!("{}\n", formatted_log).as_ref()).await {
                Ok(_) => (),
                Err(e) => {
                    warn!(
                        "failed to write metrics to {}: {}",
                        // Unwrap is safe as we can't get here unless a requests file path
                        // is defined.
                        self.get_requests_file_path().unwrap(),
                        e
                    );
                }
            }
        }

        self.metrics.requests.insert(key.to_string(), merge_request);
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
                        self.record_error(&request_metric);
                    }

                    // If coordinated_omission_elapsed is non-zero, this was a statistically
                    // generated "request" to mitigate coordinated omission, loop to backfill
                    // with statistically generated metrics.
                    if request_metric.coordinated_omission_elapsed > 0
                        && request_metric.coordinated_omission_cadence > 0
                    {
                        // Build a statistically generated coordinated_omissiom metric starting
                        // with the metric that was sent by the affected GooseUser.
                        let mut co_metric = request_metric.clone();

                        // Use a signed integer as this value can drop below zero.
                        let mut response_time = request_metric.coordinated_omission_elapsed as i64;

                        loop {
                            // Backfill until reaching the expected request cadence.
                            if response_time > request_metric.response_time as i64 {
                                co_metric.response_time = response_time as u64;
                                self.record_request_metric(&co_metric, goose_attack_run_state)
                                    .await;
                                response_time -= request_metric.coordinated_omission_cadence as i64;
                            } else {
                                break;
                            }
                        }
                    // Otherwise this is an actual request, record it normally.
                    } else {
                        // Merge the `GooseRequestMetric` into a `GooseRequestMetricAggregate` in
                        // `GooseMetrics.requests`, and write to the requests log if enabled.
                        self.record_request_metric(&request_metric, goose_attack_run_state)
                            .await;
                    }
                }
                GooseMetric::Error(raw_error) => {
                    // Recreate the string used to uniquely identify errors.
                    let error_key = format!(
                        "{}.{}.{}",
                        raw_error.error, raw_error.method, raw_error.name
                    );
                    let mut merge_error = match self.metrics.errors.get(&error_key) {
                        Some(error) => error.clone(),
                        None => GooseErrorMetric::new(
                            raw_error.method.clone(),
                            raw_error.name.to_string(),
                            raw_error.error.to_string(),
                        ),
                    };
                    merge_error.occurrences += raw_error.occurrences;
                    self.metrics
                        .errors
                        .insert(error_key.to_string(), merge_error);
                }
                GooseMetric::Task(raw_task) => {
                    // Store a new metric.
                    self.metrics.tasks[raw_task.taskset_index][raw_task.task_index]
                        .set_time(raw_task.run_time, raw_task.success);
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
    pub(crate) fn record_error(&mut self, raw_request: &GooseRequestMetric) {
        // If the error summary is disabled, return immediately without collecting errors.
        if self.configuration.no_error_summary {
            return;
        }

        // Create a string to uniquely identify errors for tracking metrics.
        let error_string = format!(
            "{}.{}.{}",
            raw_request.error, raw_request.method, raw_request.name
        );

        let mut error_metrics = match self.metrics.errors.get(&error_string) {
            // We've seen this error before.
            Some(m) => m.clone(),
            // First time we've seen this error.
            None => GooseErrorMetric::new(
                raw_request.method.clone(),
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
        let counter = match global_response_times.get(&response_time) {
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
            if let Some(existing_status_code_count) = aggregate_status_code_counts.get(&status_code)
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
        let mut raw_request = GooseRequestMetric::new(GooseMethod::Get, "/", PATH, 0, 0);
        assert_eq!(raw_request.method, GooseMethod::Get);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.url, PATH.to_string());
        assert_eq!(raw_request.response_time, 0);
        assert_eq!(raw_request.status_code, 0);
        assert!(raw_request.success);
        assert!(!raw_request.update);

        let response_time = 123;
        raw_request.set_response_time(response_time);
        assert_eq!(raw_request.method, GooseMethod::Get);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.url, PATH.to_string());
        assert_eq!(raw_request.response_time, response_time as u64);
        assert_eq!(raw_request.status_code, 0);
        assert!(raw_request.success);
        assert!(!raw_request.update);

        let status_code = http::StatusCode::OK;
        raw_request.set_status_code(Some(status_code));
        assert_eq!(raw_request.method, GooseMethod::Get);
        assert_eq!(raw_request.name, "/".to_string());
        assert_eq!(raw_request.url, PATH.to_string());
        assert_eq!(raw_request.response_time, response_time as u64);
        assert_eq!(raw_request.status_code, 200);
        assert!(raw_request.success);
        assert!(!raw_request.update);
    }

    #[test]
    fn goose_request() {
        let mut request = GooseRequestMetricAggregate::new("/", GooseMethod::Get, 0);
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.response_times.len(), 0);
        assert_eq!(request.min_response_time, 0);
        assert_eq!(request.max_response_time, 0);
        assert_eq!(request.total_response_time, 0);
        assert_eq!(request.response_time_counter, 0);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking a response time updates several fields.
        request.set_response_time(1);
        // We've seen only one response time so far.
        assert_eq!(request.response_times.len(), 1);
        // We've seen one response time of length 1.
        assert_eq!(request.response_times[&1], 1);
        // The minimum response time seen so far is 1.
        assert_eq!(request.min_response_time, 1);
        // The maximum response time seen so far is 1.
        assert_eq!(request.max_response_time, 1);
        // We've seen a total of 1 ms of response time so far.
        assert_eq!(request.total_response_time, 1);
        // We've seen a total of 2 response times so far.
        assert_eq!(request.response_time_counter, 1);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.set_response_time(10);
        // We've added a new unique response time.
        assert_eq!(request.response_times.len(), 2);
        // We've seen the 10 ms response time 1 time.
        assert_eq!(request.response_times[&10], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum is new response time.
        assert_eq!(request.max_response_time, 10);
        // Total combined response times is now 11 ms.
        assert_eq!(request.total_response_time, 11);
        // We've seen two response times so far.
        assert_eq!(request.response_time_counter, 2);
        // Nothing else changes.
        assert_eq!(request.path, "/".to_string());
        assert_eq!(request.method, GooseMethod::Get);
        assert_eq!(request.status_code_counts.len(), 0);
        assert_eq!(request.success_count, 0);
        assert_eq!(request.fail_count, 0);

        // Tracking another response time updates all related fields.
        request.set_response_time(10);
        // We've incremented the counter of an existing response time.
        assert_eq!(request.response_times.len(), 2);
        // We've seen the 10 ms response time 2 times.
        assert_eq!(request.response_times[&10], 2);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum doesn't change.
        assert_eq!(request.max_response_time, 10);
        // Total combined response times is now 21 ms.
        assert_eq!(request.total_response_time, 21);
        // We've seen three response times so far.
        assert_eq!(request.response_time_counter, 3);

        // Tracking another response time updates all related fields.
        request.set_response_time(101);
        // We've added a new response time for the first time.
        assert_eq!(request.response_times.len(), 3);
        // The response time was internally rounded to 100, which we've seen once.
        assert_eq!(request.response_times[&100], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 101);
        // Total combined response times is now 122 ms.
        assert_eq!(request.total_response_time, 122);
        // We've seen four response times so far.
        assert_eq!(request.response_time_counter, 4);

        // Tracking another response time updates all related fields.
        request.set_response_time(102);
        // Due to rounding, this increments the existing 100 ms response time.
        assert_eq!(request.response_times.len(), 3);
        // The response time was internally rounded to 100, which we've now seen twice.
        assert_eq!(request.response_times[&100], 2);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 102);
        // Add 102 to the total response time so far.
        assert_eq!(request.total_response_time, 224);
        // We've seen five response times so far.
        assert_eq!(request.response_time_counter, 5);

        // Tracking another response time updates all related fields.
        request.set_response_time(155);
        // Adds a new response time.
        assert_eq!(request.response_times.len(), 4);
        // The response time was internally rounded to 160, seen for the first time.
        assert_eq!(request.response_times[&160], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 155);
        // Add 155 to the total response time so far.
        assert_eq!(request.total_response_time, 379);
        // We've seen six response times so far.
        assert_eq!(request.response_time_counter, 6);

        // Tracking another response time updates all related fields.
        request.set_response_time(2345);
        // Adds a new response time.
        assert_eq!(request.response_times.len(), 5);
        // The response time was internally rounded to 2000, seen for the first time.
        assert_eq!(request.response_times[&2000], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 2345);
        // Add 2345 to the total response time so far.
        assert_eq!(request.total_response_time, 2724);
        // We've seen seven response times so far.
        assert_eq!(request.response_time_counter, 7);

        // Tracking another response time updates all related fields.
        request.set_response_time(987654321);
        // Adds a new response time.
        assert_eq!(request.response_times.len(), 6);
        // The response time was internally rounded to 987654000, seen for the first time.
        assert_eq!(request.response_times[&987654000], 1);
        // Minimum doesn't change.
        assert_eq!(request.min_response_time, 1);
        // Maximum increases to actual maximum, not rounded maximum.
        assert_eq!(request.max_response_time, 987654321);
        // Add 987654321 to the total response time so far.
        assert_eq!(request.total_response_time, 987657045);
        // We've seen eight response times so far.
        assert_eq!(request.response_time_counter, 8);

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
        assert_eq!(request.response_times.len(), 6);
        assert_eq!(request.min_response_time, 1);
        assert_eq!(request.max_response_time, 987654321);
        assert_eq!(request.total_response_time, 987657045);
        assert_eq!(request.response_time_counter, 8);

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
        assert_eq!(request.response_times.len(), 6);
        assert_eq!(request.min_response_time, 1);
        assert_eq!(request.max_response_time, 987654321);
        assert_eq!(request.total_response_time, 987657045);
        assert_eq!(request.response_time_counter, 8);
    }
}
