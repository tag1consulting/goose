//! Pass/fail rules on a load test's metrics.
//!
//! A [`Threshold`] is a rule such as `p95<500ms`, `error-rate<1%`, `rps>=200` or
//! `p99<2s for GET /checkout`, given with `--threshold` or
//! [`GooseDefault::Threshold`](../config/enum.GooseDefault.html#variant.Threshold).
//! Unless `--no-threshold-abort` is set, Goose checks every rule once a second while
//! the run's metrics are kept, and the first rule that fails stops the run through the
//! killswitch. Every rule is judged once more when the run ends, and
//! [`GooseAttack::execute`](../struct.GooseAttack.html#method.execute) returns
//! [`GooseError::ThresholdsFailed`] when any fails. Each rule's result is kept in
//! [`GooseMetrics::thresholds`].

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::metrics::{
    common::RequestTotals, format_micros, per_second_calculations, GooseMetrics,
    GooseRequestMetricTimingData,
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
    /// Requests per second over the run so far.
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
///  - `<metric>` is `pNN` or `pNN.N` (a response time percentile, such as `p95` or
///    `p99.9`), `avg`, `max`, `error-rate` (failed requests as a percent of all
///    requests) or `rps` (requests per second over the run so far);
///  - `<op>` is `<`, `<=`, `>` or `>=`;
///  - `<value>` is, for a time, a number with `us`, `ms` or `s`, a bare number being
///    milliseconds; for `error-rate`, a number with an optional `%`; for `rps`, a
///    number;
///  - ` for <request>` judges only the request the requests table names so, such as
///    `GET /login`; without it, the rule judges all requests together.
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
    /// `213.5`; `no samples` when the rule's metric had none.
    pub observed: String,
    /// What was observed as a number, in milliseconds for a time, a percent for
    /// `error-rate` and requests per second for `rps`; `None` without samples.
    pub value: Option<f64>,
    /// Whether the rule passed.
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

/// What [`Threshold::observe`] shows for a rule whose metric has no samples.
const NO_SAMPLES: &str = "no samples";

/// The metrics a rule can name, for the reasons rules are refused.
const METRICS: &str = "use pNN (such as p95 or p99.9), avg, max, error-rate or rps.";

/// Why a percentile out of range is refused.
const PERCENTILE_RANGE: &str =
    "the percentile must be greater than 0 and less than 100, such as p95 or p99.9.";

/// Why a rule with `for` and no request is refused.
const NO_REQUEST: &str =
    "\"for\" needs a request name as the requests table shows it, such as GET /login.";

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
        // it makes a rule with no value, as in "p95< for GET /", read so.
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
    duration: usize,
}

/// An observed value: in the rule's own unit for comparing, as a number for
/// [`ThresholdResult::value`], and as the reports show it.
struct Observed {
    compared: f64,
    value: f64,
    shown: String,
}

impl Threshold {
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
            // Before a whole second there is no rate to judge.
            Metric::RequestsPerSecond if figures.requests > 0 && figures.duration > 0 => {
                let (rate, _) = per_second_calculations(figures.duration, figures.requests, 0);
                let rate = f64::from(rate);
                Some(Observed {
                    compared: rate,
                    value: rate,
                    shown: short_number(rate),
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
    totals: Option<RequestTotals>,
}

impl<'m> Judge<'m> {
    fn new(metrics: &'m GooseMetrics) -> Self {
        Judge {
            metrics,
            totals: None,
        }
    }

    /// What `threshold` observes, or `None` without samples.
    fn observe(&mut self, threshold: &Threshold) -> Option<Observed> {
        let duration = self.metrics.duration;
        match &threshold.request {
            Some(key) => {
                let request = self.metrics.requests.get(key)?;
                threshold.observe(&Figures {
                    times: &request.raw_data,
                    requests: request.success_count + request.fail_count,
                    failures: request.fail_count,
                    duration,
                })
            }
            None => {
                let metrics = self.metrics;
                let totals = self
                    .totals
                    .get_or_insert_with(|| RequestTotals::of(metrics.requests.values()));
                threshold.observe(&Figures {
                    times: &totals.times,
                    requests: totals.requests,
                    failures: totals.failures,
                    duration,
                })
            }
        }
    }
}

/// The first of `thresholds` that fails on `metrics` while the run is going, as its
/// index and the killswitch's reason. A rule with no samples yet is skipped.
pub(crate) fn first_failure(
    thresholds: &[Threshold],
    metrics: &GooseMetrics,
) -> Option<(usize, String)> {
    let mut judge = Judge::new(metrics);
    thresholds
        .iter()
        .enumerate()
        .find_map(|(index, threshold)| {
            let observed = judge.observe(threshold)?;
            (!threshold
                .operator
                .passes(observed.compared, threshold.limit))
            .then(|| {
                (
                    index,
                    format!("threshold {threshold} failed: observed {}", observed.shown),
                )
            })
        })
}

/// Judge every one of `thresholds` on `metrics` as `check` does. `aborted` is the
/// index of the rule that stopped the run, if one did.
pub(crate) fn evaluate(
    thresholds: &[Threshold],
    metrics: &GooseMetrics,
    check: Check,
    aborted: Option<usize>,
) -> Vec<ThresholdResult> {
    let mut judge = Judge::new(metrics);
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
            ThresholdResult {
                rule: threshold.rule.clone(),
                observed,
                value,
                passed,
                aborted: aborted == Some(index),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GooseConfiguration, GooseDefault, GooseDefaultType};
    use crate::metrics::GooseRequestMetricAggregate;
    use crate::GooseAttack;
    use clap::Parser;

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
        let cases: &[(&str, Metric, Operator, f64, Option<&str>)] = &[
            (
                "p50<500ms",
                Metric::Percentile(50.0),
                Operator::Less,
                500_000.0,
                None,
            ),
            (
                "p95<=2s",
                Metric::Percentile(95.0),
                Operator::LessOrEqual,
                2_000_000.0,
                None,
            ),
            (
                "p99.9>750us",
                Metric::Percentile(99.9),
                Operator::Greater,
                750.0,
                None,
            ),
            (
                "p1>=250",
                Metric::Percentile(1.0),
                Operator::GreaterOrEqual,
                250_000.0,
                None,
            ),
            ("avg<1.5ms", Metric::Average, Operator::Less, 1_500.0, None),
            ("max<0.5s", Metric::Maximum, Operator::Less, 500_000.0, None),
            (
                "error-rate<1%",
                Metric::ErrorRate,
                Operator::Less,
                1.0,
                None,
            ),
            (
                "error-rate<=0.5",
                Metric::ErrorRate,
                Operator::LessOrEqual,
                0.5,
                None,
            ),
            (
                "rps>=200",
                Metric::RequestsPerSecond,
                Operator::GreaterOrEqual,
                200.0,
                None,
            ),
            (
                "rps>12.5",
                Metric::RequestsPerSecond,
                Operator::Greater,
                12.5,
                None,
            ),
            (
                "p99<2s for GET /checkout",
                Metric::Percentile(99.0),
                Operator::Less,
                2_000_000.0,
                Some("GET /checkout"),
            ),
            (
                " p95 < 500ms for POST login for real ",
                Metric::Percentile(95.0),
                Operator::Less,
                500_000.0,
                Some("POST login for real"),
            ),
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
        assert!(refused("").contains("can not be empty"));
        assert!(refused("   ").contains("can not be empty"));
        assert!(refused("p95").contains("no operator"));
        assert!(refused("<500ms").contains("no metric"));
        assert!(refused("latency<500ms").contains("\"latency\" is not a metric"));
        assert!(refused("pp95<5ms").contains("is not a metric"));
        assert!(refused("p<5ms").contains("is not a metric"));
        assert!(refused("p0<5ms").contains("greater than 0 and less than 100"));
        assert!(refused("p100<5ms").contains("greater than 0 and less than 100"));
        assert!(refused("p100.0<5ms").contains("greater than 0 and less than 100"));
        assert!(refused("p95=500ms").contains("must be <, <=, > or >="));
        assert!(refused("p95==500ms").contains("must be <, <=, > or >="));
        assert!(refused("p95<<500ms").contains("must be <, <=, > or >="));
        assert!(refused("p95=<500ms").contains("must be <, <=, > or >="));
        assert!(refused("p95<").contains("no value"));
        assert!(refused("p95< for GET /").contains("no value"));
        assert!(refused("p95<5m").contains("\"5m\" is not a time"));
        assert!(refused("p95<5min").contains("is not a time"));
        assert!(refused("p95<-5ms").contains("is not a time"));
        assert!(refused("p95<NaNms").contains("is not a time"));
        assert!(refused("p95<1e3ms").contains("is not a time"));
        assert!(refused("p95<5 ms").contains("is not a time"));
        assert!(refused("error-rate<1ms").contains("is not a percent"));
        assert!(refused("error-rate<101%").contains("more than 100%"));
        assert!(refused("rps>2s").contains("is not a number of requests per second"));
        assert!(refused("rps>inf").contains("is not a number of requests per second"));
        assert!(refused("p95<500ms for").contains("needs a request name"));
        assert!(refused("p95<500ms for ").contains("needs a request name"));
        assert!(refused("p95<500ms for   ").contains("needs a request name"));
        // Every reason names the rule.
        assert!(refused("p0<5ms").starts_with("threshold \"p0<5ms\": "));
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
        evaluate(&[parse(rule)], metrics, check, None).remove(0)
    }

    #[test]
    fn evaluation_at_the_exact_boundary() {
        // One request a second for 4 seconds, each 200 ms, one of them failed.
        let metrics = metrics_with(4, vec![request("/", &[200, 200, 200, 200], 4, 1)]);
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
        assert_eq!(judge("rps>1", &metrics, Check::Final).observed, "1");
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
            assert_eq!(first_failure(&[parse(rule)], &empty), None);
        }
        let metrics = metrics_with(5, vec![request("/", &[10], 1, 0)]);
        let end = judge("p95<500ms for GET /never", &metrics, Check::Final);
        assert!(!end.passed);
        assert_eq!(end.observed, "no samples");
        // Before a whole second, there is no rate yet.
        let metrics = metrics_with(0, vec![request("/", &[10], 1, 0)]);
        assert_eq!(first_failure(&[parse("rps>100")], &metrics), None);
    }

    #[test]
    fn first_failure_names_the_rule_and_the_observed_value() {
        let metrics = metrics_with(2, vec![request("/", &[812], 1, 0)]);
        let rules = [parse("error-rate<1%"), parse("p95<500ms"), parse("max<1ms")];
        assert_eq!(
            first_failure(&rules, &metrics),
            Some((1, "threshold p95<500ms failed: observed 812ms".to_string()))
        );
        let results = evaluate(&rules, &metrics, Check::Final, Some(1));
        assert_eq!(
            results
                .iter()
                .map(|r| (r.passed, r.aborted))
                .collect::<Vec<_>>(),
            vec![(true, false), (false, true), (false, false)]
        );
    }

    /// The configuration `args` and `defaults` resolve to, or the reason it is refused.
    fn configure(
        args: &[&str],
        defaults: &crate::config::GooseDefaults,
    ) -> Result<GooseConfiguration, GooseError> {
        let mut configuration = GooseConfiguration::try_parse_from(
            std::iter::once("loadtest").chain(args.iter().copied()),
        )
        .expect("the command line parses");
        configuration.configure(defaults)?;
        configuration.validate()?;
        Ok(configuration)
    }

    fn detail(result: Result<GooseConfiguration, GooseError>) -> String {
        match result {
            Err(GooseError::InvalidOption { detail, .. }) => detail,
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn abort_delay_parses_like_run_time() {
        for (delay, secs) in [
            ("30", 30),
            ("30s", 30),
            ("2m", 120),
            ("1h30m", 5400),
            ("0", 0),
            ("1m30s", 90),
        ] {
            let configuration = configure(
                &["--threshold", "p95<1s", "--threshold-abort-delay", delay],
                &Default::default(),
            )
            .unwrap();
            assert_eq!(configuration.threshold_abort_delay_secs(), secs, "{delay}");
        }
        let defaults = defaults_with(|attack| {
            attack
                .set_default(GooseDefault::Threshold, "p95<1s")?
                .set_default(GooseDefault::ThresholdAbortDelay, 45)
        });
        let configuration = configure(&[], &defaults).unwrap();
        assert_eq!(configuration.threshold_abort_delay_secs(), 45);
        // The command line wins over a default.
        let configuration = configure(&["--threshold-abort-delay", "3"], &defaults).unwrap();
        assert_eq!(configuration.threshold_abort_delay_secs(), 3);
    }

    #[test]
    fn abort_delay_needs_a_threshold_that_can_abort() {
        let refused = detail(configure(
            &["--threshold-abort-delay", "10s"],
            &Default::default(),
        ));
        assert!(
            refused.contains("needs a `configuration.threshold`"),
            "{}",
            refused
        );
        let refused = detail(configure(
            &[
                "--threshold",
                "p95<1s",
                "--no-threshold-abort",
                "--threshold-abort-delay",
                "10s",
            ],
            &Default::default(),
        ));
        assert!(
            refused.contains("`configuration.no_threshold_abort`"),
            "{}",
            refused
        );
        let defaults =
            defaults_with(|attack| attack.set_default(GooseDefault::ThresholdAbortDelay, 5));
        assert!(configure(&[], &defaults).is_err());
        let refused = detail(configure(
            &["--threshold-abort-delay", "1x"],
            &Default::default(),
        ));
        assert!(refused.contains("threshold_abort_delay"), "{}", refused);
    }

    #[test]
    fn thresholds_need_metrics() {
        let refused = detail(configure(
            &["--threshold", "p95<1s", "--no-metrics"],
            &Default::default(),
        ));
        assert!(
            refused.contains("`configuration.no_metrics`"),
            "{}",
            refused
        );
        let defaults = defaults_with(|attack| attack.set_default(GooseDefault::Threshold, "rps>1"));
        assert!(configure(&["--no-metrics"], &defaults).is_err());
    }

    #[test]
    fn rules_from_the_command_line_and_from_defaults() {
        let defaults = defaults_with(|attack| {
            attack
                .set_default(GooseDefault::Threshold, "p95<500ms")?
                .set_default(GooseDefault::Threshold, "error-rate<1%")
        });
        // Two defaults are two rules.
        let configuration = configure(&[], &defaults).unwrap();
        let rules: Vec<String> = configuration
            .thresholds
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(rules, ["p95<500ms", "error-rate<1%"]);
        assert!(configuration.aborts_on_thresholds());
        // Command line rules replace the defaults.
        let configuration = configure(
            &["--threshold", "rps>=200", "--threshold", "max<2s"],
            &defaults,
        )
        .unwrap();
        let rules: Vec<String> = configuration
            .thresholds
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(rules, ["rps>=200", "max<2s"]);
        // --no-threshold-abort keeps the rules for the end of the run.
        let configuration = configure(&["--no-threshold-abort"], &defaults).unwrap();
        assert_eq!(configuration.thresholds.len(), 2);
        assert!(!configuration.aborts_on_thresholds());
        let defaults = defaults_with(|attack| {
            attack
                .set_default(GooseDefault::Threshold, "p95<500ms")?
                .set_default(GooseDefault::NoThresholdAbort, true)
        });
        assert!(!configure(&[], &defaults).unwrap().aborts_on_thresholds());
        // No rules, nothing to abort on.
        assert!(!configure(&[], &Default::default())
            .unwrap()
            .aborts_on_thresholds());
        // An invalid rule is refused before the load test, from either place.
        let refused = detail(configure(&["--threshold", "p100<1s"], &Default::default()));
        assert!(refused.contains("less than 100"), "{}", refused);
        match GooseAttack::initialize_with_config(GooseConfiguration::default())
            .unwrap()
            .set_default(GooseDefault::Threshold, "p95")
        {
            Err(GooseError::InvalidOption { detail, .. }) => {
                assert!(detail.contains("no operator"))
            }
            Err(other) => panic!("{:?}", other),
            Ok(_) => panic!("p95 was taken"),
        }
    }

    /// The defaults `set` gives a new `GooseAttack`.
    fn defaults_with(
        set: impl FnOnce(GooseAttack) -> Result<Box<GooseAttack>, GooseError>,
    ) -> crate::config::GooseDefaults {
        let attack = GooseAttack::initialize_with_config(GooseConfiguration::default()).unwrap();
        set(attack).unwrap().defaults.clone()
    }
}
