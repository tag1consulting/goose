//! Pass/fail rules on a load test's metrics.
//!
//! A [`Threshold`] is a rule such as `p95<500ms`, `error-rate<1%`, `rps>=200` or
//! `p99<2s for GET /checkout`, given with `--threshold` or
//! [`GooseDefault::Threshold`](../config/enum.GooseDefault.html#variant.Threshold).
//! Unless `--no-threshold-abort` is set, Goose checks every rule once a second from
//! when the run first enters Maintain, and the first rule that fails stops the run
//! through the killswitch. Every rule is judged once more when the run ends, and
//! [`GooseAttack::execute`](../struct.GooseAttack.html#method.execute) returns
//! [`GooseError::ThresholdsFailed`] when any fails. Each rule's result is kept in
//! [`GooseMetrics::thresholds`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::metrics::{
    common::RequestTotals, format_micros, GooseMetrics, GooseRequestMetricTimingData,
};
use crate::GooseError;

/// What a [`Threshold`] measures.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Metric {
    /// A response time percentile, as a percent greater than 0 and less than 100.
    Percentile(f64),
    /// The average response time.
    Average,
    /// The slowest response time.
    Maximum,
    /// Failed requests as a percent of all requests.
    ErrorRate,
    /// Requests per second while the run holds its users steady, in Maintain.
    RequestsPerSecond,
}

impl Metric {
    /// Whether this metric is a response time, so its value takes a time unit.
    fn is_time(self) -> bool {
        matches!(
            self,
            Metric::Percentile(_) | Metric::Average | Metric::Maximum
        )
    }
}

/// How a [`Threshold`] compares what it observes with its value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Operator {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Operator {
    /// Whether `observed` passes against `limit`.
    fn passes(self, observed: f64, limit: f64) -> bool {
        match self {
            Operator::Less => observed < limit,
            Operator::LessOrEqual => observed <= limit,
            Operator::Greater => observed > limit,
            Operator::GreaterOrEqual => observed >= limit,
        }
    }
}

/// When a [`Threshold`] is judged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Check {
    /// While the run is going: a rule with no samples yet is skipped.
    MidRun,
    /// When the run has ended: a rule with no samples fails.
    Final,
}

/// One pass/fail rule on the run's metrics, parsed from text such as `p95<500ms`.
///
/// The text is `<metric><op><value>`, optionally followed by ` for <request>`:
///  - `<metric>` is `pNN`, with optional decimals (a response time percentile, such
///    as `p95` or `p99.9`), `avg`, `max`, `error-rate` (failed requests as a percent
///    of all requests) or `rps` (requests per second while the run holds its users
///    steady, in Maintain);
///  - `<op>` is `<`, `<=`, `>` or `>=`;
///  - `<value>` is, for a time, a number with `us`, `ms` or `s`, a bare number being
///    milliseconds; for `error-rate`, a number with an optional `%`; for `rps`, a
///    number;
///  - ` for <request>` judges only the request of that name, as the HTML or Markdown
///    report names it, such as `GET /login`; without it, the rule judges all
///    requests together.
///
/// A rule that can never pass, such as `p95<0` or `error-rate>100%`, is refused.
///
/// # Example
/// ```rust
/// use goose::threshold::Threshold;
///
/// let rule: Threshold = "p99<2s for GET /checkout".parse().unwrap();
/// assert_eq!(rule.to_string(), "p99<2s for GET /checkout");
/// assert!("p100<2s".parse::<Threshold>().is_err());
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Threshold {
    /// The rule as it was given, trimmed.
    rule: String,
    metric: Metric,
    operator: Operator,
    /// The limit: microseconds for a time, a percent for `error-rate`, requests per
    /// second for `rps`.
    limit: f64,
    /// The request this rule judges, or `None` for all requests together.
    request: Option<String>,
}

/// The result of judging one [`Threshold`] when a run ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThresholdResult {
    /// The rule, as it was given.
    pub rule: String,
    /// What was observed, as the reports show it, such as `812ms`, `0.4%` or
    /// `213.5 from Maintain`; `no samples` when the rule's metric had none. For the
    /// rule that stopped the run, the value at the stop and the value at the end,
    /// as in `100% at abort, 0.8% at end`, when they differ.
    pub observed: String,
    /// What was observed at the end as a number, in milliseconds for a time, a
    /// percent for `error-rate` and requests per second for `rps`; `None` without
    /// samples.
    pub value: Option<f64>,
    /// Whether the rule passed. The rule that stopped the run never passed.
    pub passed: bool,
    /// Whether this rule failing stopped the run while it was going.
    pub aborted: bool,
}

impl ThresholdResult {
    /// The result as the reports show it, in words so it never rests on colour.
    pub(crate) fn verdict(&self) -> &'static str {
        match (self.passed, self.aborted) {
            (true, _) => "PASS",
            (false, false) => "FAIL",
            (false, true) => "FAIL, stopped the run",
        }
    }
}

/// A moment in the run: the run time, and how many times each request had been
/// made by then.
#[derive(Clone, Debug, Default)]
pub(crate) struct ThresholdMark {
    /// The run time in seconds, to the microsecond.
    seconds: f64,
    /// How many times each request had been made, successful or failed.
    requests: HashMap<String, usize>,
}

impl ThresholdMark {
    /// The moment `seconds` into the run, when each request had been made as often
    /// as `requests` says.
    pub(crate) fn new(seconds: f64, requests: HashMap<String, usize>) -> Self {
        ThresholdMark { seconds, requests }
    }

    /// How many times `request` had been made, or every request together for `None`.
    fn made(&self, request: Option<&str>) -> usize {
        match request {
            Some(key) => self.requests.get(key).copied().unwrap_or(0),
            None => self
                .requests
                .values()
                .fold(0, |sum: usize, &count| sum.saturating_add(count)),
        }
    }
}

/// What `rps` rules count: the time the run spent holding its users steady, in
/// Maintain, summed over every hold, and the requests made in it. Increase,
/// Decrease and Shutdown add nothing, with or without `--no-threshold-abort`.
#[derive(Clone, Debug, Default)]
pub(crate) struct RpsWindow {
    /// The seconds of every hold that has ended, to the microsecond.
    seconds: f64,
    /// How many times each request was made in the holds that have ended.
    requests: HashMap<String, usize>,
    /// Where the hold now going started, while the run is in Maintain.
    open: Option<ThresholdMark>,
}

impl RpsWindow {
    /// The run enters Maintain at `mark`: a hold starts.
    pub(crate) fn enter(&mut self, mark: ThresholdMark) {
        if self.open.is_none() {
            self.open = Some(mark);
        }
    }

    /// The run leaves Maintain at `mark`: the hold going ends, and its seconds and
    /// requests join those of the holds before it.
    pub(crate) fn leave(&mut self, mark: ThresholdMark) {
        let Some(start) = self.open.take() else {
            return;
        };
        self.seconds += (mark.seconds - start.seconds).max(0.0);
        for (key, made) in mark.requests {
            let before = start.requests.get(&key).copied().unwrap_or(0);
            *self.requests.entry(key).or_default() += made.saturating_sub(before);
        }
    }

    /// The metrics were reset, with the run's clock and its counts: nothing counted
    /// so far is kept, and a hold going starts again from the reset.
    pub(crate) fn reset(&mut self) {
        let open = self.open.is_some();
        *self = RpsWindow::default();
        if open {
            self.open = Some(ThresholdMark::default());
        }
    }

    /// The requests made while holding, and the seconds held, `seconds` into the
    /// run when `made` requests had been made: of `request`, or every request
    /// together for `None`.
    fn held(&self, request: Option<&str>, made: usize, seconds: f64) -> (usize, f64) {
        let ended = match request {
            Some(key) => self.requests.get(key).copied().unwrap_or(0),
            None => self
                .requests
                .values()
                .fold(0, |sum: usize, &count| sum.saturating_add(count)),
        };
        match &self.open {
            Some(start) => (
                ended + made.saturating_sub(start.made(request)),
                self.seconds + (seconds - start.seconds).max(0.0),
            ),
            None => (ended, self.seconds),
        }
    }
}

/// What [`Threshold::observe`] shows for a rule whose metric has no samples.
const NO_SAMPLES: &str = "no samples";

/// The metrics a rule can name, for the reasons rules are refused.
const METRICS: &str = "use pNN (such as p95 or p99.9), avg, max, error-rate or rps.";

/// Why a percentile out of range is refused.
const PERCENTILE_RANGE: &str =
    "the percentile must be greater than 0 and less than 100, such as p95 or p99.9.";

/// Why a rule with `for` and no request is refused.
const NO_REQUEST: &str =
    "\"for\" needs a request name as the HTML or Markdown report shows it, such as GET /login.";

/// Why a time value that does not parse is refused, after the value.
const NOT_A_TIME: &str =
    "is not a time; use a number with us, ms or s, such as 500ms. A bare number is milliseconds.";

/// Why an error rate value that does not parse is refused, after the value.
const NOT_A_PERCENT: &str = "is not a percent; use a number with an optional %, such as 1%.";

/// Why an error rate over 100% is refused.
const OVER_100: &str = "an error rate can not be more than 100%.";

/// Why a requests per second value that does not parse is refused, after the value.
const NOT_A_RATE: &str = "is not a number of requests per second, such as 200.";

/// The `--threshold` rule `rule` refused, with `detail` saying why.
fn invalid(rule: &str, detail: &str) -> GooseError {
    GooseError::InvalidOption {
        option: "`configuration.threshold`".to_string(),
        value: rule.to_string(),
        detail: format!("threshold {rule:?}: {detail}"),
    }
}

/// `text` as a number, when it is only ASCII digits with at most one `.` among
/// them: no sign, exponent, `NaN` or infinity.
fn plain_number(text: &str) -> Option<f64> {
    let mut digits = 0;
    let mut points = 0;
    for c in text.chars() {
        match c {
            '0'..='9' => digits += 1,
            '.' => points += 1,
            _ => return None,
        }
    }
    if digits == 0 || points > 1 {
        return None;
    }
    text.parse().ok()
}

/// `number` with at most two decimals, and no trailing zeros.
fn short_number(number: f64) -> String {
    let text = format!("{number:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

/// Why a rule that compares `metric` with `limit` by `operator` can never pass, if
/// it can not.
fn never_passes(metric: Metric, operator: Operator, limit: f64) -> Option<&'static str> {
    match (metric, operator) {
        (_, Operator::Less) if limit == 0.0 => Some(match metric {
            Metric::ErrorRate => "no error rate is less than 0%.",
            Metric::RequestsPerSecond => "no rate is less than 0.",
            _ => "no response time is less than 0.",
        }),
        (Metric::RequestsPerSecond, Operator::LessOrEqual) if limit == 0.0 => {
            Some("a rate is only judged once requests are made, so it is always above 0.")
        }
        (Metric::ErrorRate, Operator::Greater) if limit == 100.0 => {
            Some("no error rate is more than 100%.")
        }
        _ => None,
    }
}

impl FromStr for Threshold {
    type Err = GooseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rule = text.trim();
        if rule.is_empty() {
            return Err(invalid(
                rule,
                "a rule can not be empty; give one such as p95<500ms.",
            ));
        }

        // The metric ends at the first operator character.
        let Some(at) = rule.find(['<', '>', '=']) else {
            return Err(invalid(
                rule,
                "it has no operator; use <, <=, > or >=, as in p95<500ms.",
            ));
        };
        let name = rule[..at].trim();
        let metric = match name {
            "avg" => Metric::Average,
            "max" => Metric::Maximum,
            "error-rate" => Metric::ErrorRate,
            "rps" => Metric::RequestsPerSecond,
            "" => {
                return Err(invalid(
                    rule,
                    &format!("it has no metric before the operator; {METRICS}"),
                ))
            }
            _ => match name.strip_prefix('p').map(plain_number) {
                Some(Some(percent)) if percent > 0.0 && percent < 100.0 => {
                    Metric::Percentile(percent)
                }
                Some(Some(_)) => return Err(invalid(rule, PERCENTILE_RANGE)),
                _ => {
                    return Err(invalid(
                        rule,
                        &format!("{name:?} is not a metric; {METRICS}"),
                    ))
                }
            },
        };

        let rest = &rule[at..];
        let (operator, rest) = if let Some(rest) = rest.strip_prefix("<=") {
            (Operator::LessOrEqual, rest)
        } else if let Some(rest) = rest.strip_prefix(">=") {
            (Operator::GreaterOrEqual, rest)
        } else if let Some(rest) = rest.strip_prefix('<') {
            (Operator::Less, rest)
        } else if let Some(rest) = rest.strip_prefix('>') {
            (Operator::Greater, rest)
        } else {
            return Err(invalid(rule, "the operator must be <, <=, > or >=."));
        };
        if rest.starts_with(['<', '>', '=']) {
            return Err(invalid(rule, "the operator must be <, <=, > or >=."));
        }

        // The value ends at " for ", which names the request. The space before
        // it makes a rule with no value, as in "p95< for GET /", parse as one.
        let rest = format!(" {}", rest.trim());
        let (value, request) = match rest.find(" for ") {
            Some(at) => (rest[..at].trim(), Some(rest[at + 5..].trim())),
            None => match rest.strip_suffix(" for") {
                Some(value) => (value.trim(), Some("")),
                None => (rest.trim(), None),
            },
        };
        if value.is_empty() {
            return Err(invalid(rule, "it has no value after the operator."));
        }
        let request = match request {
            Some("") => return Err(invalid(rule, NO_REQUEST)),
            other => other.map(str::to_string),
        };

        let limit = if metric.is_time() {
            let (number, scale) = if let Some(number) = value.strip_suffix("us") {
                (number, 1.0)
            } else if let Some(number) = value.strip_suffix("ms") {
                (number, 1_000.0)
            } else if let Some(number) = value.strip_suffix('s') {
                (number, 1_000_000.0)
            } else {
                (value, 1_000.0)
            };
            match plain_number(number) {
                Some(number) => number * scale,
                None => return Err(invalid(rule, &format!("{value:?} {NOT_A_TIME}"))),
            }
        } else if metric == Metric::ErrorRate {
            match plain_number(value.strip_suffix('%').unwrap_or(value)) {
                Some(percent) if percent <= 100.0 => percent,
                Some(_) => return Err(invalid(rule, OVER_100)),
                None => return Err(invalid(rule, &format!("{value:?} {NOT_A_PERCENT}"))),
            }
        } else {
            match plain_number(value) {
                Some(rate) => rate,
                None => return Err(invalid(rule, &format!("{value:?} {NOT_A_RATE}"))),
            }
        };
        if let Some(reason) = never_passes(metric, operator, limit) {
            return Err(invalid(rule, &format!("it can never pass; {reason}")));
        }

        Ok(Threshold {
            rule: rule.to_string(),
            metric,
            operator,
            limit,
            request,
        })
    }
}

impl fmt::Display for Threshold {
    /// The rule as it was given.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rule)
    }
}

/// The figures a [`Threshold`] reads from: one request's, or every request's
/// together as the report's Aggregated row adds them.
struct Figures<'a> {
    times: &'a GooseRequestMetricTimingData,
    requests: usize,
    failures: usize,
    /// The seconds `requests` were made in, for `rps`.
    seconds: f64,
}

/// An observed value: in the rule's own unit for comparing, as a number for
/// [`ThresholdResult::value`], and as the reports show it.
struct Observed {
    compared: f64,
    value: f64,
    shown: String,
}

impl Threshold {
    /// The lowest rate this rule passes at, and whether it passes at exactly that
    /// rate, when it is an `rps>` or `rps>=` rule.
    pub(crate) fn rate_floor(&self) -> Option<(f64, bool)> {
        match (self.metric, self.operator) {
            (Metric::RequestsPerSecond, Operator::Greater) => Some((self.limit, false)),
            (Metric::RequestsPerSecond, Operator::GreaterOrEqual) => Some((self.limit, true)),
            _ => None,
        }
    }

    /// Whether this is an `rps` rule, which counts only the time the run holds its
    /// users steady.
    pub(crate) fn is_rps(&self) -> bool {
        self.metric == Metric::RequestsPerSecond
    }

    /// What this rule observes in `figures`, or `None` when its metric has no
    /// samples.
    fn observe(&self, figures: &Figures) -> Option<Observed> {
        let times = figures.times;
        let time = |us: u64| Observed {
            compared: us as f64,
            value: us as f64 / 1_000.0,
            shown: format!("{}ms", format_micros(us, false)),
        };
        match self.metric {
            Metric::Percentile(percent) if times.counter > 0 => {
                Some(time(times.percentile_us((percent / 100.0) as f32)))
            }
            Metric::Average if times.counter > 0 => {
                let us = times.total_time_us as f64 / times.counter as f64;
                Some(Observed {
                    compared: us,
                    value: us / 1_000.0,
                    shown: format!("{}ms", format_micros(us.round() as u64, false)),
                })
            }
            Metric::Maximum if times.counter > 0 => Some(time(times.maximum_time_us)),
            Metric::ErrorRate if figures.requests > 0 => {
                let percent = figures.failures as f64 / figures.requests as f64 * 100.0;
                Some(Observed {
                    compared: percent,
                    value: percent,
                    shown: format!("{}%", short_number(percent)),
                })
            }
            // Before a whole second held there is no rate to judge.
            Metric::RequestsPerSecond if figures.requests > 0 && figures.seconds >= 1.0 => {
                let rate = figures.requests as f64 / figures.seconds;
                Some(Observed {
                    compared: rate,
                    value: rate,
                    shown: format!("{} from Maintain", short_number(rate)),
                })
            }
            _ => None,
        }
    }
}

/// Every request's figures together, worked out once for all the rules that
/// judge every request.
struct Judge<'m> {
    metrics: &'m GooseMetrics,
    /// The run time in seconds, to the microsecond, as the rules are judged.
    seconds: f64,
    totals: Option<RequestTotals>,
}

impl<'m> Judge<'m> {
    fn new(metrics: &'m GooseMetrics, seconds: f64) -> Self {
        Judge {
            metrics,
            seconds,
            totals: None,
        }
    }

    /// What `threshold` observes, or `None` without samples. An `rps` rule counts
    /// only the requests made while the run held its users steady, over the
    /// seconds it held them.
    fn observe(&mut self, threshold: &Threshold) -> Option<Observed> {
        let metrics = self.metrics;
        let request = threshold.request.as_deref();
        let now = self.seconds;
        // The requests an `rps` rule counts, and their seconds; the other metrics
        // read every request made.
        let held = |made: usize| {
            if threshold.is_rps() {
                metrics.rps_window.held(request, made, now)
            } else {
                (made, now)
            }
        };
        match request {
            Some(key) => {
                let aggregate = metrics.requests.get(key)?;
                let (requests, seconds) = held(aggregate.success_count + aggregate.fail_count);
                threshold.observe(&Figures {
                    times: &aggregate.raw_data,
                    requests,
                    failures: aggregate.fail_count,
                    seconds,
                })
            }
            None => {
                let totals = self
                    .totals
                    .get_or_insert_with(|| RequestTotals::of(metrics.requests.values()));
                let (requests, seconds) = held(totals.requests);
                threshold.observe(&Figures {
                    times: &totals.times,
                    requests,
                    failures: totals.failures,
                    seconds,
                })
            }
        }
    }
}

/// A rule that failed while the run was going: its index, what it observed as
/// the reports show it, and the killswitch's reason.
pub(crate) struct Failure {
    pub(crate) index: usize,
    pub(crate) observed: String,
    pub(crate) reason: String,
}

/// The first of `thresholds` that fails on `metrics`, `seconds` into the run, while
/// it is going. A rule with no samples yet is skipped.
pub(crate) fn first_failure(
    thresholds: &[Threshold],
    metrics: &GooseMetrics,
    seconds: f64,
) -> Option<Failure> {
    let mut judge = Judge::new(metrics, seconds);
    thresholds
        .iter()
        .enumerate()
        .find_map(|(index, threshold)| {
            let observed = judge.observe(threshold)?;
            (!threshold
                .operator
                .passes(observed.compared, threshold.limit))
            .then(|| Failure {
                index,
                reason: format!("threshold {threshold} failed: observed {}", observed.shown),
                observed: observed.shown,
            })
        })
}

/// Judge every one of `thresholds` on `metrics`, `seconds` into the run, as `check`
/// does. `aborted` is the index of the rule that stopped the run and what it
/// observed then, if one did: that rule fails whatever it observes now.
pub(crate) fn evaluate(
    thresholds: &[Threshold],
    metrics: &GooseMetrics,
    check: Check,
    aborted: Option<(usize, &str)>,
    seconds: f64,
) -> Vec<ThresholdResult> {
    let mut judge = Judge::new(metrics, seconds);
    thresholds
        .iter()
        .enumerate()
        .map(|(index, threshold)| {
            let (observed, value, passed) = match judge.observe(threshold) {
                Some(observed) => (
                    observed.shown,
                    Some(observed.value),
                    threshold
                        .operator
                        .passes(observed.compared, threshold.limit),
                ),
                None => (NO_SAMPLES.to_string(), None, check == Check::MidRun),
            };
            match aborted {
                Some((aborted, at_abort)) if aborted == index => ThresholdResult {
                    rule: threshold.rule.clone(),
                    observed: if observed == at_abort {
                        observed
                    } else {
                        format!("{at_abort} at abort, {observed} at end")
                    },
                    value,
                    passed: false,
                    aborted: true,
                },
                _ => ThresholdResult {
                    rule: threshold.rule.clone(),
                    observed,
                    value,
                    passed,
                    aborted: false,
                },
            }
        })
        .collect()
}

/// A warning for each of `thresholds` that names a request the run never made,
/// suggesting the request it most likely means.
pub(crate) fn unknown_requests(thresholds: &[Threshold], metrics: &GooseMetrics) -> Vec<String> {
    thresholds
        .iter()
        .filter_map(|threshold| {
            let name = threshold.request.as_deref()?;
            if metrics.requests.contains_key(name) {
                return None;
            }
            let mut warning = format!("threshold {threshold}: no request named {name:?}");
            if let Some(key) = suggestion(name, metrics.requests.keys().map(String::as_str)) {
                warning.push_str(&format!("; did you mean {key:?}?"));
            }
            Some(warning)
        })
        .collect()
}

/// The one of `keys` that `name` most likely means: the same but for ASCII case,
/// or, for a name the console cut short with `..`, one that starts with the rest.
/// `None` when no key or more than one matches.
fn suggestion<'k>(name: &str, keys: impl IntoIterator<Item = &'k str>) -> Option<&'k str> {
    let cut = name.strip_suffix("..");
    let mut matches = keys.into_iter().filter(|key| {
        key.eq_ignore_ascii_case(name) || cut.is_some_and(|cut| key.starts_with(cut))
    });
    let key = matches.next()?;
    matches.next().is_none().then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::GooseRequestMetricAggregate;

    fn parse(rule: &str) -> Threshold {
        rule.parse()
            .unwrap_or_else(|e| panic!("{:?} was refused: {:?}", rule, e))
    }

    /// The reason `rule` is refused.
    fn refused(rule: &str) -> String {
        match rule.parse::<Threshold>() {
            Err(GooseError::InvalidOption {
                option,
                value,
                detail,
            }) => {
                assert_eq!(option, "`configuration.threshold`");
                assert_eq!(value, rule.trim());
                detail
            }
            other => panic!("{:?}: {:?}", rule, other),
        }
    }

    #[test]
    fn every_metric_operator_and_unit_parses() {
        use Metric::*;
        use Operator::*;
        #[rustfmt::skip]
        let cases: &[(&str, Metric, Operator, f64, Option<&str>)] = &[
            ("p50<500ms", Percentile(50.0), Less, 500_000.0, None),
            ("p95<=2s", Percentile(95.0), LessOrEqual, 2_000_000.0, None),
            ("p99.9>750us", Percentile(99.9), Greater, 750.0, None),
            ("p99.99<1s", Percentile(99.99), Less, 1_000_000.0, None),
            ("p1>=250", Percentile(1.0), GreaterOrEqual, 250_000.0, None),
            ("avg<1.5ms", Average, Less, 1_500.0, None),
            ("max<0.5s", Maximum, Less, 500_000.0, None),
            ("p95<=0", Percentile(95.0), LessOrEqual, 0.0, None),
            ("error-rate<1%", ErrorRate, Less, 1.0, None),
            ("error-rate<=0.5", ErrorRate, LessOrEqual, 0.5, None),
            ("error-rate<=0", ErrorRate, LessOrEqual, 0.0, None),
            ("error-rate>=100%", ErrorRate, GreaterOrEqual, 100.0, None),
            ("rps>=200", RequestsPerSecond, GreaterOrEqual, 200.0, None),
            ("rps>12.5", RequestsPerSecond, Greater, 12.5, None),
            ("rps>0", RequestsPerSecond, Greater, 0.0, None),
            ("p99<2s for GET /checkout", Percentile(99.0), Less, 2_000_000.0, Some("GET /checkout")),
            (" p95 < 500ms for POST login for real ", Percentile(95.0), Less, 500_000.0, Some("POST login for real")),
        ];
        for &(rule, metric, operator, limit, request) in cases {
            let threshold = parse(rule);
            assert_eq!(threshold.metric, metric, "{rule}");
            assert_eq!(threshold.operator, operator, "{rule}");
            assert!(
                (threshold.limit - limit).abs() < 1e-6,
                "{rule}: {}",
                threshold.limit
            );
            assert_eq!(threshold.request.as_deref(), request, "{rule}");
            assert_eq!(threshold.to_string(), rule.trim());
        }
    }

    #[test]
    fn invalid_rules_are_refused_with_a_reason() {
        let cases = [
            ("", "can not be empty"),
            ("   ", "can not be empty"),
            ("p95", "no operator"),
            ("<500ms", "no metric"),
            ("latency<500ms", "\"latency\" is not a metric"),
            ("pp95<5ms", "is not a metric"),
            ("p<5ms", "is not a metric"),
            ("p0<5ms", "greater than 0 and less than 100"),
            ("p100<5ms", "greater than 0 and less than 100"),
            ("p100.0<5ms", "greater than 0 and less than 100"),
            ("p95=500ms", "must be <, <=, > or >="),
            ("p95==500ms", "must be <, <=, > or >="),
            ("p95<<500ms", "must be <, <=, > or >="),
            ("p95=<500ms", "must be <, <=, > or >="),
            ("p95<", "no value"),
            ("p95< for GET /", "no value"),
            ("p95<5m", "\"5m\" is not a time"),
            ("p95<5min", "is not a time"),
            ("p95<-5ms", "is not a time"),
            ("p95<NaNms", "is not a time"),
            ("p95<1e3ms", "is not a time"),
            ("p95<5 ms", "is not a time"),
            ("error-rate<1ms", "is not a percent"),
            ("error-rate<101%", "more than 100%"),
            ("rps>2s", "is not a number of requests per second"),
            ("rps>inf", "is not a number of requests per second"),
            ("p95<500ms for", "needs a request name"),
            ("p95<500ms for ", "needs a request name"),
            ("p95<500ms for   ", "needs a request name"),
            // Rules that can never pass.
            (
                "p95<0",
                "it can never pass; no response time is less than 0.",
            ),
            ("p99.9<0us", "it can never pass; no response time"),
            ("avg<0ms", "it can never pass; no response time"),
            ("max<0s for GET /", "it can never pass; no response time"),
            (
                "error-rate<0",
                "it can never pass; no error rate is less than 0%.",
            ),
            (
                "error-rate<0%",
                "it can never pass; no error rate is less than 0%.",
            ),
            (
                "error-rate>100",
                "it can never pass; no error rate is more than 100%.",
            ),
            (
                "error-rate>100%",
                "it can never pass; no error rate is more than 100%.",
            ),
            ("rps<0", "it can never pass; no rate is less than 0."),
            (
                "rps<=0",
                "it can never pass; a rate is only judged once requests",
            ),
            ("rps<=0.0 for GET /", "it can never pass"),
        ];
        for (rule, reason) in cases {
            let refused = refused(rule);
            // Every reason names the rule.
            assert!(
                refused.starts_with(&format!("threshold {:?}: ", rule.trim())),
                "{}: {}",
                rule,
                refused
            );
            assert!(refused.contains(reason), "{}: {}", rule, refused);
        }
    }

    /// A request named `name` with one time for each of `times_ms`, counted as
    /// `requests` requests of which `failures` failed.
    fn request(
        name: &str,
        times_ms: &[u64],
        requests: usize,
        failures: usize,
    ) -> GooseRequestMetricAggregate {
        let mut request =
            GooseRequestMetricAggregate::new(name, crate::goose::GooseMethod::Get, 0, "");
        for &ms in times_ms {
            request.raw_data.record_time(ms);
        }
        request.success_count = requests - failures;
        request.fail_count = failures;
        request
    }

    /// Metrics over `duration` seconds holding `requests`.
    fn metrics_with(duration: usize, requests: Vec<GooseRequestMetricAggregate>) -> GooseMetrics {
        let mut metrics = GooseMetrics {
            duration,
            ..Default::default()
        };
        for request in requests {
            metrics
                .requests
                .insert(format!("GET {}", request.path), request);
        }
        metrics
    }

    fn judge(rule: &str, metrics: &GooseMetrics, check: Check) -> ThresholdResult {
        evaluate(
            &[parse(rule)],
            metrics,
            check,
            None,
            metrics.duration as f64,
        )
        .remove(0)
    }

    #[test]
    fn evaluation_at_the_exact_boundary() {
        // One request a second for 4 seconds, each 200 ms, one of them failed, all
        // while the run held its users steady.
        let mut metrics = metrics_with(4, vec![request("/", &[200, 200, 200, 200], 4, 1)]);
        metrics.rps_window.enter(mark(0.0, 0));
        let cases = [
            ("max<200ms", false),
            ("max<=200ms", true),
            ("max>200ms", false),
            ("max>=200ms", true),
            ("avg<200", false),
            ("avg<=200", true),
            ("avg>0.2s", false),
            ("avg>=200000us", true),
            ("p50<200ms", false),
            ("p50<=200ms", true),
            ("p50>200ms", false),
            ("p50>=200ms", true),
            ("error-rate<25%", false),
            ("error-rate<=25%", true),
            ("error-rate>25", false),
            ("error-rate>=25", true),
            ("rps<1", false),
            ("rps<=1", true),
            ("rps>1", false),
            ("rps>=1", true),
        ];
        for (rule, passed) in cases {
            let result = judge(rule, &metrics, Check::Final);
            assert_eq!(
                result.passed, passed,
                "{rule}: observed {}",
                result.observed
            );
        }
        assert_eq!(judge("max<1s", &metrics, Check::Final).observed, "200ms");
        assert_eq!(
            judge("error-rate<1%", &metrics, Check::Final).observed,
            "25%"
        );
        assert_eq!(
            judge("rps>1", &metrics, Check::Final).observed,
            "1 from Maintain"
        );
        assert_eq!(judge("max<1s", &metrics, Check::Final).value, Some(200.0));
    }

    #[test]
    fn percentiles_come_from_the_aggregate_row() {
        // The report's Aggregated row merges every request's times.
        let metrics = metrics_with(
            10,
            vec![
                request("/a", &[10, 20, 30, 40, 50], 5, 0),
                request("/b", &[60, 70, 80, 90, 100], 5, 0),
            ],
        );
        let data = crate::metrics::common::prepare_data_with_baseline(
            crate::metrics::common::ReportOptions {
                no_transaction_metrics: true,
                no_scenario_metrics: true,
                no_status_codes: true,
            },
            &metrics,
            None,
        );
        let aggregated = data.raw_response_metrics.last().unwrap();
        assert_eq!(aggregated.name, "Aggregated");
        let p90 = judge("p90<1s", &metrics, Check::Final);
        assert_eq!(
            p90.observed,
            format!("{}ms", aggregated.percentile_90.value())
        );
        // A rule for one request reads only its times.
        let a = judge("max<1s for GET /a", &metrics, Check::Final);
        assert_eq!(a.observed, "50ms");
    }

    #[test]
    fn zero_samples_skip_mid_run_and_fail_at_the_end() {
        let empty = metrics_with(5, vec![]);
        let none = metrics_with(5, vec![request("/", &[], 0, 0)]);
        for rule in [
            "p95<500ms",
            "avg<1s",
            "max<1s",
            "error-rate<1%",
            "rps>1",
            "rps<1000",
        ] {
            for metrics in [&empty, &none] {
                let mid = judge(rule, metrics, Check::MidRun);
                assert!(mid.passed, "{}", rule);
                let end = judge(rule, metrics, Check::Final);
                assert!(!end.passed, "{}", rule);
                assert_eq!(end.observed, "no samples");
                assert_eq!(end.value, None);
            }
            assert!(first_failure(&[parse(rule)], &empty, 5.0).is_none());
        }
        let metrics = metrics_with(5, vec![request("/", &[10], 1, 0)]);
        let end = judge("p95<500ms for GET /never", &metrics, Check::Final);
        assert!(!end.passed);
        assert_eq!(end.observed, "no samples");
        // Before a whole second held, there is no rate yet.
        let mut metrics = metrics_with(0, vec![request("/", &[10], 1, 0)]);
        metrics.rps_window.enter(mark(0.0, 0));
        assert!(first_failure(&[parse("rps>100")], &metrics, 0.5).is_none());
    }

    #[test]
    fn first_failure_names_the_rule_and_the_observed_value() {
        let metrics = metrics_with(2, vec![request("/", &[812], 1, 0)]);
        let rules = [parse("error-rate<1%"), parse("p95<500ms"), parse("max<1ms")];
        let failure = first_failure(&rules, &metrics, 2.0).unwrap();
        assert_eq!(failure.index, 1);
        assert_eq!(failure.observed, "812ms");
        assert_eq!(failure.reason, "threshold p95<500ms failed: observed 812ms");
        let results = evaluate(&rules, &metrics, Check::Final, Some((1, "812ms")), 2.0);
        assert_eq!(
            results
                .iter()
                .map(|r| (r.passed, r.aborted, r.observed.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (true, false, "0%"),
                (false, true, "812ms"),
                (false, false, "812ms")
            ]
        );
    }

    /// The rule that stopped the run fails even when, by the end, what it observes
    /// is back inside its limit, and its row shows both values.
    #[test]
    fn the_rule_that_stopped_the_run_always_fails() {
        // At the end 5 of 1,000 requests failed, 0.5%; the rule stopped the run
        // when every request so far had failed.
        let metrics = metrics_with(10, vec![request("/", &[10], 1_000, 5)]);
        let rules = [parse("error-rate<1%"), parse("p95<5s")];
        let results = evaluate(&rules, &metrics, Check::Final, Some((0, "100%")), 10.0);
        assert!(!results[0].passed);
        assert!(results[0].aborted);
        assert_eq!(results[0].verdict(), "FAIL, stopped the run");
        assert_eq!(results[0].observed, "100% at abort, 0.5% at end");
        assert_eq!(results[0].value, Some(0.5));
        // The other rules are judged as always.
        assert!(results[1].passed);
        assert!(!results[1].aborted);
        // When the value at the end is the value at the stop, it is shown once.
        let results = evaluate(&rules, &metrics, Check::Final, Some((0, "0.5%")), 10.0);
        assert_eq!(results[0].observed, "0.5%");
        assert!(!results[0].passed);
    }

    /// An `rps` rule counts only the requests made while the run held its users
    /// steady, over the seconds it held them, summed over every hold, and says so.
    #[test]
    fn rps_counts_only_the_holds() {
        let mut metrics = metrics_with(10, vec![request("/", &[10], 100, 0)]);
        // Without a hold there is no rate: skipped while the run goes, and a
        // failure at the end, though the other metrics read the whole run.
        assert!(first_failure(&[parse("rps>1000")], &metrics, 10.0).is_none());
        let result = judge("rps>=10", &metrics, Check::Final);
        assert_eq!(result.observed, NO_SAMPLES);
        assert!(!result.passed);
        assert_eq!(
            judge("error-rate<1%", &metrics, Check::Final).observed,
            "0%"
        );

        // A hold going since 4.5 seconds in, with 20 requests made: 80 in 5.5
        // seconds, judged to the microsecond rather than the whole seconds of the
        // duration.
        metrics.rps_window.enter(mark(4.5, 20));
        let result = judge("rps>=14.5", &metrics, Check::Final);
        assert_eq!(result.observed, "14.55 from Maintain");
        assert!(result.passed);
        let result = judge("rps>=14.6 for GET /", &metrics, Check::Final);
        assert_eq!(result.observed, "14.55 from Maintain");
        assert!(!result.passed);
        // Within the first second held there is no rate yet.
        assert!(first_failure(&[parse("rps>1000")], &metrics, 5.4).is_none());
        let failure = first_failure(&[parse("rps>1000")], &metrics, 5.5).unwrap();
        assert_eq!(failure.observed, "80 from Maintain");

        // The hold ended at 9.5 with 80 made: 60 in 5 seconds, whatever came after.
        metrics.rps_window.leave(mark(9.5, 80));
        let result = judge("rps>=12 for GET /", &metrics, Check::Final);
        assert_eq!(result.observed, "12 from Maintain");
        assert!(result.passed);
        assert_eq!(
            judge("rps<12", &metrics, Check::Final).observed,
            "12 from Maintain"
        );

        // A second hold, from 12 seconds in with 90 made: what came between the two
        // holds is not counted. At 14 seconds, with 100 made, 70 in 7 seconds.
        metrics.rps_window.enter(mark(12.0, 90));
        assert_eq!(
            evaluate(&[parse("rps>=10")], &metrics, Check::Final, None, 14.0)[0].observed,
            "10 from Maintain"
        );

        // A reset starts over: until a whole second is held again, no rate.
        metrics.rps_window.reset();
        assert_eq!(
            evaluate(&[parse("rps>=10")], &metrics, Check::Final, None, 0.5)[0].observed,
            NO_SAMPLES
        );
    }

    /// `seconds` into the run, with `made` requests of `GET /`.
    fn mark(seconds: f64, made: usize) -> ThresholdMark {
        ThresholdMark::new(
            seconds,
            std::iter::once(("GET /".to_string(), made)).collect(),
        )
    }

    #[test]
    fn an_unknown_request_suggests_the_one_meant() {
        let keys = ["GET /Login", "GET /a-very-long-request-path"];
        // Only the case differs.
        assert_eq!(suggestion("get /login", keys), Some("GET /Login"));
        // Cut short as the console's requests table does.
        let cut = crate::util::truncate_string(keys[1], 24);
        assert_eq!(cut, "GET /a-very-long-reque..");
        assert_eq!(suggestion(&cut, keys), Some(keys[1]));
        // Two that match are no suggestion.
        let keys = [
            "GET /a-very-long-request-path-1",
            "GET /a-very-long-request-path-2",
        ];
        assert_eq!(suggestion(&cut, keys), None);
        assert_eq!(suggestion("GET /other", keys), None);

        let metrics = metrics_with(5, vec![request("/Login", &[10], 1, 0)]);
        let rules = [
            parse("p95<1s for get /login"),
            parse("p95<1s for GET /Login"),
            parse("p95<1s for POST /"),
            parse("p95<1s"),
        ];
        assert_eq!(
            unknown_requests(&rules, &metrics),
            [
                "threshold p95<1s for get /login: no request named \"get /login\"; did you mean \"GET /Login\"?",
                "threshold p95<1s for POST /: no request named \"POST /\"",
            ]
        );
    }
}
