//! Response times kept in microseconds: the bucket scheme's error bound,
//! percentile accuracy, merging, display, old reports as baselines and
//! minimums under a millisecond (#585, #737, #366).

use super::buckets::{bucket_end, bucket_index, bucket_value};
use super::common::{load_baseline_file, prepare_data_with_baseline, ReportData, ReportOptions};
use super::dashboard_snapshot::{
    build_dashboard_snapshot, DashboardSnapshotInput, SeriesWindow, SnapshotSave, SnapshotSettings,
};
use super::millis::{format_micros, Millis};
use super::*;
use crate::goose::{GooseMethod, GooseUser, TransactionName};
use crate::metrics::coordinated_omission::CoordinatedOmissionMetrics;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// The default `--timeout`, 60 seconds, in microseconds.
const DEFAULT_TIMEOUT_US: u64 = 60_000_000;

/// A small seeded generator (SplitMix64), so every run draws the same times.
struct Seeded(u64);

impl Seeded {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[low, high]`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.next_u64() % (high - low + 1)
    }

    /// Uniform in `(0, 1)`.
    fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// Standard normal, by Box Muller.
    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// A time in microseconds kept between 1 µs and the default timeout.
fn clamp_us(us: f64) -> u64 {
    (us.round() as u64).clamp(1, DEFAULT_TIMEOUT_US)
}

fn lognormal(count: usize, seed: u64) -> Vec<u64> {
    let mut rng = Seeded(seed);
    // A median of 20 ms, most times from about 2 ms to 200 ms.
    (0..count)
        .map(|_| clamp_us((20_000f64.ln() + rng.normal()).exp()))
        .collect()
}

fn bimodal(count: usize, seed: u64) -> Vec<u64> {
    let mut rng = Seeded(seed);
    (0..count)
        .map(|i| {
            if i % 2 == 0 {
                clamp_us(300.0 + 30.0 * rng.normal())
            } else {
                clamp_us(40_000.0 + 4_000.0 * rng.normal())
            }
        })
        .collect()
}

fn uniform(count: usize, seed: u64) -> Vec<u64> {
    let mut rng = Seeded(seed);
    (0..count).map(|_| rng.between(1, 5_000_000)).collect()
}

/// Timing data holding `times`, in microseconds, recorded one by one.
fn recorded(times: &[u64]) -> GooseRequestMetricTimingData {
    let mut data = GooseRequestMetricTimingData::default();
    for &us in times {
        data.record_time_us(us);
    }
    data
}

/// The time `percent` of `times` finished within, from the times themselves:
/// the one at that rank once sorted, as Goose ranks a percentile.
fn exact_percentile(sorted: &[u64], percent: f32) -> u64 {
    let rank = ((sorted.len() as f32 * percent).round() as usize).max(1);
    sorted[rank - 1]
}

fn within_one_percent(value: u64, expected: u64) -> bool {
    (value as f64 - expected as f64).abs() <= expected as f64 * 0.01
}

/// The value a bucket reports for `us` is within 1/128 of `us` and falls in
/// the same bucket, and that bucket reports the same value for it.
fn assert_bucket_holds(us: u64) {
    let value = bucket_value(us);
    assert!(
        value.abs_diff(us) * 128 <= us,
        "{} µs is reported as {} µs, more than 1/128 off",
        us,
        value
    );
    assert_eq!(
        bucket_index(value),
        bucket_index(us),
        "the value {value} µs of the bucket of {us} µs is in another bucket"
    );
    assert_eq!(bucket_value(value), value, "{us} µs: value is not stable");
}

#[test]
fn bucket_error_bound_from_one_microsecond_to_the_default_timeout() {
    // Every microsecond to 100 ms.
    let mut previous = bucket_index(0);
    for us in 1..=100_000 {
        assert_bucket_holds(us);
        let index = bucket_index(us);
        assert!(index >= previous, "bucket index falls at {} µs", us);
        previous = index;
    }

    // A million times from 100 ms to the default timeout.
    let mut rng = Seeded(585);
    let mut drawn: Vec<u64> = (0..1_000_000)
        .map(|_| rng.between(100_000, DEFAULT_TIMEOUT_US))
        .collect();
    for &us in &drawn {
        assert_bucket_holds(us);
    }
    drawn.sort_unstable();
    for pair in drawn.windows(2) {
        assert!(
            bucket_index(pair[0]) <= bucket_index(pair[1]),
            "bucket index falls between {} and {} µs",
            pair[0],
            pair[1]
        );
    }

    // Each boundary, and the times either side of it: a bucket ends where
    // the next index begins.
    assert_eq!(bucket_value(0), 0);
    let mut start = 0;
    while start <= DEFAULT_TIMEOUT_US {
        let end = bucket_end(start);
        assert!(end > start, "the bucket from {} µs is empty", start);
        assert_eq!(bucket_index(end), bucket_index(start) + 1);
        assert_eq!(bucket_index(end - 1), bucket_index(start));
        for us in [end - 1, end, end + 1] {
            if us > 0 {
                assert_bucket_holds(us);
            }
        }
        start = end;
    }
}

/// The bucket boundaries around 1 ms and 10 ms.
#[test]
fn bucket_boundaries_of_the_log_linear_scheme() {
    use super::buckets::bucket_floor;
    // 8 µs wide just under 1 ms, 16 µs from 1,024 µs and 128 µs around
    // 10 ms.
    assert_eq!((bucket_floor(999), bucket_end(999)), (992, 1_000));
    assert_eq!((bucket_floor(1_000), bucket_end(1_000)), (1_000, 1_008));
    assert_eq!((bucket_floor(1_010), bucket_end(1_010)), (1_008, 1_016));
    assert_eq!((bucket_floor(1_024), bucket_end(1_024)), (1_024, 1_040));
    assert_eq!(bucket_floor(9_990), 9_984);
    assert_eq!(bucket_floor(10_000), 9_984);
    assert_eq!((bucket_floor(10_010), bucket_end(10_010)), (9_984, 10_112));
    for us in [999, 1_000, 1_010, 2_200, 2_500, 9_990, 10_000, 10_010] {
        assert_bucket_holds(us);
    }
    // Shown as hundredths up to 10 ms, from buckets at most 1/128 off: 9.99 ms
    // is in the 9.984 to 10.112 ms bucket and reads 10.
    let shown: f64 = Millis(bucket_value(9_990)).to_string().parse().unwrap();
    assert!((shown - 9.99).abs() <= 9.99 * 0.01, "{}", shown);
    assert_eq!(Millis(10_000).to_string(), "10");
}

/// A JSON report holds each request's buckets once, in `times_us`, with no
/// whole millisecond `times` map beside them.
#[test]
fn json_report_writes_each_bucket_once() {
    let metrics = request_metrics(&[("GET /", &[400, 2_350, 37_999])]);
    let raw = serde_json::to_value(&metrics.requests["GET /"].raw_data).unwrap();
    assert!(raw.get("times").is_none(), "{}", raw);
    assert_eq!(raw["times_us"].as_object().unwrap().len(), 3);
    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    scenario.update_us(700, 0);
    let scenario = serde_json::to_value(&scenario).unwrap();
    assert!(scenario.get("times").is_none(), "{}", scenario);
    assert_eq!(scenario["times_us"].as_object().unwrap().len(), 1);
}

#[test]
fn sub_millisecond_request_is_kept_and_shown() {
    let data = recorded(&[400]);
    assert_eq!(data.minimum_time_us, 400);
    assert_eq!(data.maximum_time_us, 400);
    assert_eq!(data.counter, 1);
    let p50 = data.percentile_us(0.5);
    assert!(p50 < 1000 && within_one_percent(p50, 400), "p50 {}", p50);
    assert_eq!(Millis(400).to_string(), "0.4");
    assert_eq!(format_micros(400, false), "0.4");

    // Through the report: minimum 0.4, not 1, and the JSON row keeps whole
    // milliseconds.
    let metrics = request_metrics(&[("GET /", &[400])]);
    let report = report_of(&metrics, None);
    let row = &report.raw_request_metrics[0];
    assert_eq!(row.response_time_minimum.to_string(), "0.4");
    assert_eq!(row.response_time_maximum.to_string(), "0.4");
    assert_eq!(
        format!("{:.2}", row.response_time_average.value().0),
        "0.40"
    );
    let json = serde_json::to_value(row).unwrap();
    assert_eq!(json["response_time_minimum"], 0);
    assert_eq!(json["response_time_maximum"], 0);
    assert_eq!(
        report.raw_response_metrics[0].percentile_50.to_string(),
        "0.4"
    );

    // The HTML row and the console table show it too.
    let html = crate::report::raw_request_metrics_row(row.clone());
    assert!(html.contains("<td>0.4</td>"), "{}", html);
    let cells = console_response_time_cells(&metrics, "GET /");
    assert_eq!(cells, ["GET /", "0.4", "0.4", "0.4", "0.4"]);
}

#[test]
fn two_point_two_and_two_point_five_milliseconds_are_told_apart() {
    // #366.
    let a = recorded(&[2_200; 100]);
    let b = recorded(&[2_500; 100]);
    let (pa, pb) = (a.percentile_us(0.5), b.percentile_us(0.5));
    assert_ne!(pa, pb);
    assert!(within_one_percent(pa, 2_200), "{}", pa);
    assert!(within_one_percent(pb, 2_500), "{}", pb);
    let metrics = request_metrics(&[("GET /a", &[2_200; 100]), ("GET /b", &[2_500; 100])]);
    let report = report_of(&metrics, None);
    assert_eq!(
        report.raw_response_metrics[0].percentile_95.to_string(),
        "2.2"
    );
    assert_eq!(
        report.raw_response_metrics[1].percentile_95.to_string(),
        "2.5"
    );
}

#[test]
fn one_point_four_and_one_point_six_seconds_are_told_apart() {
    // #737: 1.4 s and 1.6 s were stored as 1000 and 2000 ms.
    let spread = |center: u64| -> Vec<u64> {
        (0..201)
            .map(|i| center - 10_000 + i * 100)
            .collect::<Vec<u64>>()
    };
    let (slow, slower) = (spread(1_400_000), spread(1_600_000));
    let metrics = request_metrics(&[("GET /slow", &slow), ("GET /slower", &slower)]);
    let report = report_of(&metrics, None);
    let p50 = |row: usize| {
        serde_json::to_value(&report.raw_response_metrics[row]).unwrap()["percentile_50"]
            .as_u64()
            .unwrap()
    };
    let (a, b) = (p50(0), p50(1));
    assert_ne!(a, b);
    for value in [a, b] {
        assert!(value != 1000 && value != 2000, "p50 {}", value);
    }
    assert!(within_one_percent(a, 1400), "{}", a);
    assert!(within_one_percent(b, 1600), "{}", b);
}

/// 37,999 µs prints rounded, as 38, and serializes truncated, as 37.
#[test]
fn thirty_seven_point_999_milliseconds_prints_38_and_serializes_as_37() {
    let metrics = request_metrics(&[("GET /", &[37_000, 37_999])]);
    let report = report_of(&metrics, None);
    let row = &report.raw_request_metrics[0];
    assert_eq!(row.response_time_minimum.to_string(), "37");
    assert_eq!(row.response_time_maximum.to_string(), "38");
    assert_eq!(Millis(37_999).to_string(), "38");
    let json = serde_json::to_value(row).unwrap();
    assert_eq!(json["response_time_minimum"], 37);
    assert_eq!(json["response_time_maximum"], 37);
    let percentiles = serde_json::to_value(&report.raw_response_metrics[0]).unwrap();
    assert_eq!(percentiles["percentile_100"], 37);
}

#[test]
fn percentiles_from_the_buckets_are_within_one_percent() {
    for (name, mut times) in [
        ("lognormal", lognormal(100_000, 1)),
        ("bimodal", bimodal(100_000, 2)),
        ("uniform", uniform(100_000, 3)),
    ] {
        let data = recorded(&times);
        times.sort_unstable();
        for percent in [0.5, 0.9, 0.95, 0.99, 0.999] {
            let exact = exact_percentile(&times, percent);
            let got = data.percentile_us(percent);
            assert!(
                within_one_percent(got, exact),
                "{} p{}: {} µs from the buckets, {} µs exactly",
                name,
                percent,
                got,
                exact
            );
        }
    }
}

#[test]
fn merging_batches_equals_recording_once() {
    // With times of 0 µs, which have a bucket of their own.
    let mut times = lognormal(10_000, 4);
    for at in (0..times.len()).step_by(97) {
        times[at] = 0;
    }
    let whole = recorded(&times);
    for per_batch in [1, 4, 64] {
        let mut merged = GooseRequestMetricTimingData::default();
        for batch in times.chunks(per_batch) {
            // As a user's batch records them: in the order they came.
            let mut pushed = GooseRequestMetricTimingData::default();
            for &us in batch {
                pushed.record_time_us_unsorted(us);
            }
            merged.merge(&pushed);
        }
        assert_eq!(merged, whole, "{per_batch} per batch");
        assert_eq!(merged.times_us, whole.times_us);
        assert_eq!(merged.minimum_time_us, whole.minimum_time_us);
        assert_eq!(merged.maximum_time_us, whole.maximum_time_us);
        assert_eq!(merged.total_time_us, whole.total_time_us);
        for percent in [0.5, 0.9, 0.99, 1.0] {
            assert_eq!(merged.percentile_us(percent), whole.percentile_us(percent));
        }
    }
}

/// The deltas a report of 400 µs and 1,410 ms requests shows against
/// `baseline`: for `slow`, the name of the baseline's 1.4 s request, for
/// `fast`, its sub millisecond one, and for the minimum of the transaction
/// `index`.
fn baseline_deltas(fixture: &str, fast: &str, slow: &str, index_minimum: &str) {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let baseline = load_baseline_file(&path).expect("the old report loads as a baseline");
    let mut metrics = request_metrics(&[(fast, &[400; 5]), (slow, &[1_410_000; 10])]);
    let mut index = TransactionMetricAggregate::new(
        0,
        Arc::from("Fast"),
        0,
        TransactionName::TransactionOnly(Arc::from("index")),
    );
    index.set_time_us(400, true);
    metrics.transactions = vec![vec![index]];
    let report = report_of(&metrics, Some(&baseline));
    let row = |key: &str| {
        let name = key.strip_prefix("GET ").unwrap();
        report
            .raw_request_metrics
            .iter()
            .position(|row| row.name == name)
            .unwrap()
    };
    let slow_row = &report.raw_request_metrics[row(slow)];
    assert_eq!(slow_row.number_of_requests.to_string(), "10 (+2)");
    assert_eq!(slow_row.response_time_minimum.to_string(), "1410 (+10)");
    assert_eq!(slow_row.response_time_maximum.to_string(), "1410 (+8)");
    assert_eq!(
        report.raw_response_metrics[row(slow)]
            .percentile_50
            .to_string(),
        "1410 (+10)"
    );
    let fast_row = &report.raw_request_metrics[row(fast)];
    assert_eq!(fast_row.response_time_minimum.to_string(), "0.4 (+0.4)");
    let transactions = report.transaction_metrics.unwrap();
    let index = transactions.iter().find(|row| row.name == "index").unwrap();
    assert_eq!(index.response_time_minimum.to_string(), index_minimum);
}

#[test]
fn a_0_18_1_report_loads_as_a_baseline() {
    // The 0.18.1 report names its transactions with plain strings.
    baseline_deltas("report-0.18.1.json", "GET index", "GET slow", "0.4 (+0.4)");
}

#[test]
fn a_report_from_main_before_microseconds_loads_as_a_baseline() {
    baseline_deltas("report-main.json", "GET /", "GET /delay/1400", "0.4 (-0.6)");
}

#[test]
fn a_report_with_microseconds_round_trips_as_a_baseline() {
    let metrics = request_metrics(&[("GET /", &[400, 2_350, 37_999])]);
    let report = report_of(&metrics, None);
    let dir = std::env::temp_dir().join(format!("goose-us-baseline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("report.json");
    std::fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();
    let baseline = load_baseline_file(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    // The JSON rows are whole milliseconds; the raw timing data keeps every
    // microsecond bucket, and the rows are rebuilt from it.
    let raw = &baseline.raw_metrics.requests["GET /"].raw_data;
    assert_eq!(raw, &metrics.requests["GET /"].raw_data);
    assert_eq!(
        baseline.raw_request_metrics[0]
            .response_time_minimum
            .to_string(),
        "0.4"
    );
}

/// A report with microseconds, loaded as a baseline, gives no delta to the
/// same times: its rows are rebuilt from its microsecond buckets, not read
/// from its whole millisecond rows.
#[test]
fn a_report_with_microseconds_gives_no_delta_to_the_same_times() {
    let mut metrics = request_metrics(&[("GET /", &[2_350; 5])]);
    let mut transaction = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("t")),
    );
    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    for _ in 0..5 {
        transaction.set_time_us(2_350, true);
        scenario.update_us(2_350, 0);
    }
    metrics.transactions = vec![vec![transaction]];
    metrics.scenarios = vec![scenario];
    let dir = std::env::temp_dir().join(format!("goose-us-no-delta-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("report.json");
    std::fs::write(
        &path,
        serde_json::to_string(&report_of(&metrics, None)).unwrap(),
    )
    .unwrap();
    let baseline = load_baseline_file(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let report = report_of(&metrics, Some(&baseline));
    let request = &report.raw_request_metrics[0];
    assert_eq!(request.response_time_minimum.to_string(), "2.35 (+0)");
    assert_eq!(request.response_time_maximum.to_string(), "2.35 (+0)");
    assert_eq!(
        report.raw_response_metrics[0].percentile_50.to_string(),
        "2.35 (+0)"
    );
    let transactions = report.transaction_metrics.unwrap();
    let transaction = transactions.iter().find(|row| row.name == "t").unwrap();
    assert_eq!(transaction.response_time_minimum.to_string(), "2.35 (+0)");
    assert_eq!(transaction.response_time_maximum.to_string(), "2.35 (+0)");
    let scenario = &report.scenario_metrics.unwrap()[0];
    assert_eq!(scenario.response_time_minimum.to_string(), "2.35 (+0)");
    assert_eq!(scenario.response_time_maximum.to_string(), "2.35 (+0)");
}

/// A report with microseconds and coordinated omission adjusted rows, loaded
/// as a baseline, gives no delta to the same adjusted times: those rows are
/// rebuilt from the microsecond buckets too.
#[test]
fn a_report_with_microseconds_gives_no_coordinated_omission_delta_to_the_same_times() {
    let mut metrics = request_metrics(&[("GET /", &[2_350; 5])]);
    let mut adjusted = GooseRequestMetricTimingData::default();
    for _ in 0..5 {
        adjusted.record_time_us(2_350);
    }
    metrics
        .requests
        .get_mut("GET /")
        .unwrap()
        .coordinated_omission_data = Some(adjusted);
    let dir = std::env::temp_dir().join(format!("goose-us-co-no-delta-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("report.json");
    std::fs::write(
        &path,
        serde_json::to_string(&report_of(&metrics, None)).unwrap(),
    )
    .unwrap();
    let baseline = load_baseline_file(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let report = report_of(&metrics, Some(&baseline));
    let co_request = &report.co_request_metrics.as_ref().unwrap()[0];
    assert_eq!(co_request.name, "/");
    assert_eq!(co_request.response_time_maximum.to_string(), "2.35 (+0)");
    let co_response = &report.co_response_metrics.as_ref().unwrap()[0];
    assert_eq!(co_response.name, "/");
    assert_eq!(co_response.percentile_50.to_string(), "2.35 (+0)");
}

/// A report with microseconds from a run with transactions but no requests,
/// loaded as a baseline, gives no delta to the same transaction and scenario
/// times: its rows are rebuilt too.
#[test]
fn a_report_with_microseconds_and_no_requests_gives_no_delta_to_the_same_times() {
    let mut metrics = GooseMetrics {
        duration: 60,
        ..Default::default()
    };
    let mut transaction = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("t")),
    );
    transaction.set_time_us(140, true);
    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    scenario.update_us(140, 0);
    metrics.transactions = vec![vec![transaction]];
    metrics.scenarios = vec![scenario];
    let dir = std::env::temp_dir().join(format!("goose-us-no-requests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("report.json");
    std::fs::write(
        &path,
        serde_json::to_string(&report_of(&metrics, None)).unwrap(),
    )
    .unwrap();
    let baseline = load_baseline_file(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let report = report_of(&metrics, Some(&baseline));
    let transactions = report.transaction_metrics.unwrap();
    let transaction = transactions.iter().find(|row| row.name == "t").unwrap();
    assert_eq!(transaction.response_time_minimum.to_string(), "0.14 (+0)");
    let scenario = &report.scenario_metrics.unwrap()[0];
    assert_eq!(scenario.response_time_maximum.to_string(), "0.14 (+0)");
}

/// A report with microseconds whose request key does not start with its
/// method is refused, not rebuilt.
#[test]
fn a_report_with_microseconds_and_a_key_without_its_method_is_refused() {
    let metrics = request_metrics(&[("GET /", &[2_350; 5])]);
    let mut json = serde_json::to_value(report_of(&metrics, None)).unwrap();
    let requests = json["raw_metrics"]["requests"].as_object_mut().unwrap();
    let request = requests.remove("GET /").unwrap();
    requests.insert("/".to_string(), request);
    let dir = std::env::temp_dir().join(format!("goose-us-bad-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("report.json");
    std::fs::write(&path, json.to_string()).unwrap();
    let loaded = load_baseline_file(&path);
    std::fs::remove_dir_all(&dir).unwrap();
    let error = format!("{:?}", loaded.expect_err("refused"));
    assert!(
        error.contains("does not start with its method"),
        "{}",
        error
    );
}

#[test]
fn transaction_and_scenario_minimums_under_a_millisecond() {
    let mut metrics = GooseMetrics::default();
    let mut fast = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("fast")),
    );
    for us in [400, 450, 500] {
        fast.set_time_us(us, true);
    }
    assert_eq!(fast.run_times.minimum_time_us, 400);
    let mut instant = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        1,
        TransactionName::TransactionOnly(Arc::from("instant")),
    );
    // A time of 0 is a real minimum, not an unset one.
    instant.set_time_us(0, true);
    instant.set_time_us(5_000, true);
    assert_eq!(instant.run_times.minimum_time_us, 0);
    metrics.transactions = vec![vec![fast, instant]];

    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    scenario.update_us(700, 0);
    scenario.update_us(900, 1);
    assert_eq!(scenario.run_times.minimum_time_us, 700);
    let mut zero = ScenarioMetricAggregate::new(1, Arc::from("Zero"));
    zero.update_us(0, 0);
    zero.update_us(3_000, 0);
    assert_eq!(zero.run_times.minimum_time_us, 0);
    metrics.scenarios = vec![scenario, zero];

    let report = report_of(&metrics, None);
    let transactions = report.transaction_metrics.unwrap();
    let minimum = |name: &str| {
        transactions
            .iter()
            .find(|row| row.name == name && !row.is_scenario)
            .unwrap()
            .response_time_minimum
            .to_string()
    };
    assert_eq!(minimum("fast"), "0.4");
    assert_eq!(minimum("instant"), "0");
    assert_eq!(minimum("Aggregated"), "0");
    let scenarios = report.scenario_metrics.unwrap();
    assert_eq!(scenarios[0].response_time_minimum.to_string(), "0.7");
    assert_eq!(scenarios[1].response_time_minimum.to_string(), "0");
    assert_eq!(scenarios[2].name, "Aggregated");
    assert_eq!(scenarios[2].response_time_minimum.to_string(), "0");
}

#[test]
fn sub_millisecond_transaction_without_a_zero_reports_its_minimum() {
    let mut metrics = GooseMetrics::default();
    let mut only = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("only")),
    );
    only.set_time_us(250, true);
    only.set_time_us(600, true);
    metrics.transactions = vec![vec![only]];
    let report = report_of(&metrics, None);
    let transactions = report.transaction_metrics.unwrap();
    let aggregated = transactions.last().unwrap();
    assert_eq!(aggregated.name, "Aggregated");
    // Not 1, and not 0.
    assert_eq!(aggregated.response_time_minimum.to_string(), "0.25");
}

#[test]
fn request_and_transaction_metrics_carry_their_microseconds() {
    let mut request = GooseRequestMetric::new(
        GooseRawRequest::new(GooseMethod::Get, "/", vec![], ""),
        TransactionDetail {
            scenario_index: 0,
            scenario_name: Arc::from("Scenario"),
            transaction_index: None,
            transaction_name: TransactionName::TransactionOnly(Arc::from("")),
        },
        "/",
        0,
        0,
    );
    request.set_response_duration(Duration::from_micros(2_345));
    assert_eq!(request.response_time, 2);
    assert_eq!(request.response_time_fraction_us, 345);
    assert_eq!(request.response_time_micros(), 2_345);
    // An old log line without the fraction reads as whole milliseconds.
    let mut json = serde_json::to_value(&request).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("response_time_fraction_us");
    let old: GooseRequestMetric = serde_json::from_value(json).unwrap();
    assert_eq!(old.response_time_micros(), 2_000);

    let mut transaction = TransactionMetric::new(0, 0, 0, Arc::from("t"), 0);
    transaction.set_duration(Duration::from_micros(1_250), true);
    assert_eq!(transaction.run_time, 1);
    assert_eq!(transaction.run_time_fraction_us, 250);
    assert_eq!(transaction.run_time_micros(), 1_250);
}

/// A user whose request metrics arrive on the returned channel.
fn user_with_channel() -> (GooseUser, flume::Receiver<GooseMetric>) {
    let configuration = crate::config::GooseConfiguration::parse_args_default(&[] as &[&str])
        .expect("default configuration");
    let mut user =
        GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();
    let (tx, rx) = flume::unbounded();
    user.metrics_channel = Some(tx);
    (user, rx)
}

fn received_request(rx: &flume::Receiver<GooseMetric>) -> GooseRequestMetric {
    match rx.try_recv().expect("a request metric") {
        GooseMetric::Request(request) => *request,
        other => panic!("expected a request metric, got {:?}", other),
    }
}

#[tokio::test]
async fn custom_requests_land_with_their_microseconds() {
    let (mut user, rx) = user_with_channel();
    user.record_custom_request_duration(
        "UDP",
        "ping",
        Duration::from_micros(400),
        true,
        None,
        None,
    )
    .await
    .unwrap();
    user.flush_pending_request().unwrap();
    let ping = received_request(&rx);
    assert_eq!(ping.response_time, 0);
    assert_eq!(ping.response_time_micros(), 400);

    user.record_custom_request("TCP", "send", 3, true, None, None)
        .await
        .unwrap();
    user.flush_pending_request().unwrap();
    let send = received_request(&rx);
    assert_eq!(send.response_time, 3);
    assert_eq!(send.response_time_micros(), 3_000);

    // Both land in their buckets.
    let mut aggregate = GooseRequestMetricAggregate::new("ping", GooseMethod::Custom, 0, "UDP");
    aggregate.record_time_us(ping.response_time_micros(), false);
    aggregate.record_time_us(send.response_time_micros(), false);
    assert_eq!(aggregate.raw_data.minimum_time_us, 400);
    assert_eq!(aggregate.raw_data.maximum_time_us, 3_000);
}

fn snapshot_input(metrics: &GooseMetrics) -> DashboardSnapshotInput<'_> {
    DashboardSnapshotInput {
        metrics,
        series: SeriesWindow::empty(),
        active_users: 0,
        maximum_users: 0,
        target_users: 0,
        total_users: 0,
        phase: "maintain".to_string(),
        stopping: false,
        series_window_secs: 300,
        no_status_codes: false,
        metrics_disabled: false,
        no_transaction_metrics: false,
        no_scenario_metrics: false,
        save: SnapshotSave::off("goose-runs"),
        settings: SnapshotSettings::idle(),
        percentiles_over_time: false,
        series_keys: HashSet::new(),
    }
}

#[test]
fn dashboard_snapshot_carries_microseconds() {
    let mut metrics = request_metrics(&[("GET /", &[40, 400, 400, 900])]);
    metrics.duration = 10;
    let snap = build_dashboard_snapshot(snapshot_input(&metrics));
    let us = snap
        .aggregate
        .response_time_us
        .as_ref()
        .expect("microseconds");
    assert_eq!((us.min, us.max), (40, 900));
    assert!(within_one_percent(us.p50, 400), "{}", us.p50);
    assert!(within_one_percent(us.p99, 900), "{}", us.p99);
    // The whole milliseconds beside them are truncated from them.
    assert_eq!(snap.aggregate.response_time_min_ms, 0);
    assert_eq!(snap.aggregate.percentile_ms.p50, 0);
    assert!((snap.aggregate.response_time_avg_ms - 0.435).abs() < 1e-9);
    let row = &snap.requests[0];
    let row_us = row.response_time_us.as_ref().expect("row microseconds");
    assert_eq!((row_us.min, row_us.max), (40, 900));

    // A transaction and a scenario row of 400 µs.
    let mut transaction = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("t")),
    );
    transaction.set_time_us(400, true);
    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    scenario.update_us(400, 0);
    metrics.transactions = vec![vec![transaction]];
    metrics.scenarios = vec![scenario];
    let snap = build_dashboard_snapshot(snapshot_input(&metrics));
    for us in [
        &snap.transactions[0].response_time_us,
        &snap.scenarios[0].response_time_us,
    ] {
        assert_eq!(
            (us.min, us.max, us.p50, us.p95, us.p99),
            (400, 400, 400, 400, 400)
        );
    }

    // Coordinated omission adjusted figures too.
    let mut metrics = GooseMetrics {
        coordinated_omission_metrics: Some(CoordinatedOmissionMetrics::new(
            GooseCoordinatedOmissionMitigation::Average,
        )),
        ..Default::default()
    };
    metrics
        .coordinated_omission_metrics
        .as_mut()
        .unwrap()
        .record_co_event(
            Duration::from_millis(100),
            Duration::from_millis(1000),
            5,
            0,
            "scenario".to_string(),
        );
    let mut a = GooseRequestMetricAggregate::new("/a", GooseMethod::Get, 0, "");
    a.success_count = 20;
    for _ in 0..20 {
        a.record_time_us(400, false);
    }
    a.record_time(5, true);
    metrics.requests.insert("GET /a".to_string(), a);
    let snap = build_dashboard_snapshot(snapshot_input(&metrics));
    let co = snap.aggregate.co_adjusted.as_ref().expect("adjusted");
    let co_us = co.response_time_us.as_ref().expect("adjusted microseconds");
    assert_eq!((co_us.min, co_us.max), (400, 5_000));
    let row_co = snap.requests[0].co_adjusted.as_ref().unwrap();
    assert_eq!(row_co.response_time_us.as_ref().unwrap().max, 5_000);
}

/// Metrics holding one request aggregate per `(key, times)`, its times in
/// microseconds, all successful.
fn request_metrics(requests: &[(&str, &[u64])]) -> GooseMetrics {
    let mut metrics = GooseMetrics {
        duration: 60,
        ..Default::default()
    };
    for (key, times) in requests {
        let (method, path) = key.split_once(' ').unwrap();
        assert_eq!(method, "GET");
        let mut aggregate = GooseRequestMetricAggregate::new(path, GooseMethod::Get, 0, "");
        for &us in times.iter() {
            aggregate.record_time_us(us, false);
        }
        aggregate.success_count = times.len();
        metrics.requests.insert(key.to_string(), aggregate);
    }
    metrics
}

fn report_of<'m>(metrics: &'m GooseMetrics, baseline: Option<&ReportData>) -> ReportData<'m> {
    prepare_data_with_baseline(
        ReportOptions {
            no_transaction_metrics: false,
            no_scenario_metrics: false,
            no_status_codes: false,
        },
        metrics,
        baseline,
    )
}

/// The cells of `key`'s row in the console's response time table: name,
/// average, minimum, maximum and median.
fn console_response_time_cells(metrics: &GooseMetrics, key: &str) -> Vec<String> {
    let text = metrics.to_string();
    let table = text
        .split("Avg (ms)")
        .nth(1)
        .expect("a response time table");
    let line = table
        .lines()
        .find(|line| line.trim_start().starts_with(key))
        .expect("the request's row");
    line.split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// The slowest time is the 100th percentile, and the fastest the rank 1
/// percentile, even past the counts an `f32` holds exactly: 29,416,273
/// times, which an `f32` rounds to 29,416,272, put the 100th percentile one
/// time short of the slowest.
#[test]
fn percentiles_rank_every_time_past_f32_precision() {
    let total = 29_416_273;
    let buckets = [(bucket_value(1_680), total - 1), (bucket_value(1_700), 1)];
    let maximum = super::buckets::percentile(buckets, total, 1_680, 1_700, 1.0);
    assert_eq!(maximum, 1_700, "the 100th percentile is the slowest time");

    let mut timing = GooseRequestMetricTimingData::new(None);
    timing.record_time_us(1_680);
    timing.record_time_us(1_700);
    timing
        .times_us
        .insert(super::buckets::bucket_floor(1_680), total - 1);
    timing.counter = total;
    assert_eq!(timing.percentile_us(1.0), 1_700);
}

/// An average and a standard deviation are times, shown as times are, in
/// the HTML and Markdown reports: a mean of 1.5 ms reads 1.5, not 1.50.
#[test]
fn html_and_markdown_reports_show_averages_as_times() {
    let mut metrics = request_metrics(&[("GET /", &[1_000, 2_000])]);
    let mut adjusted = GooseRequestMetricTimingData::default();
    adjusted.record_time_us(1_000);
    adjusted.record_time_us(2_000);
    metrics
        .requests
        .get_mut("GET /")
        .unwrap()
        .coordinated_omission_data = Some(adjusted);
    let mut transaction = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("t")),
    );
    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    for us in [1_000, 2_000] {
        transaction.set_time_us(us, true);
        scenario.update_us(us, 0);
    }
    metrics.transactions = vec![vec![transaction]];
    metrics.scenarios = vec![scenario];
    let report = report_of(&metrics, None);

    let html = crate::report::raw_request_metrics_row(report.raw_request_metrics[0].clone());
    assert!(html.contains("<td>1.5</td>"), "{}", html);
    let transactions = report.transaction_metrics.clone().unwrap();
    let row = transactions.iter().find(|row| row.name == "t").unwrap();
    let html = crate::report::transaction_metrics_row(row.clone());
    assert!(html.contains("<td>1.5</td>"), "{}", html);
    let html =
        crate::report::scenario_metrics_row(report.scenario_metrics.clone().unwrap()[0].clone());
    assert!(html.contains("<td>1.5</td>"), "{}", html);
    let html = crate::report::coordinated_omission_request_metrics_row(
        report.co_request_metrics.clone().unwrap()[0].clone(),
    );
    assert!(html.contains("<td>1.5</td>"), "{}", html);
    // The standard deviation of the measured and adjusted averages, 0.
    assert!(html.contains("<td>0</td>"), "{}", html);

    let mut markdown = Vec::new();
    crate::report::write_markdown_report(&mut markdown, report).unwrap();
    let markdown = String::from_utf8(markdown).unwrap();
    assert!(
        markdown.contains("| GET | / | 2 | 0 | 1.5 | 1 | 2 |"),
        "{}",
        markdown
    );
    assert!(
        markdown.contains("| 0.0 t | 2 | 0 | 1.5 | 1 | 2 |"),
        "{}",
        markdown
    );
    assert!(
        markdown.contains("| Scenario | 1 | 2 | 1.5 | 1  | 2 |"),
        "{}",
        markdown
    );
    assert!(!markdown.contains("1.50"), "{}", markdown);
}

/// A status code breakdown's average is a time, shown as times are, in the
/// console and the HTML and Markdown reports: a mean of 1.5 ms reads 1.5.
#[test]
fn status_code_breakdown_averages_show_as_times() {
    let mut metrics = request_metrics(&[("GET /", &[1_000, 2_000, 1_000, 2_000])]);
    let request = metrics.requests.get_mut("GET /").unwrap();
    for (status, us) in [(200, 1_000), (200, 2_000), (500, 1_000), (500, 2_000)] {
        request.record_status_code_time_us(status, us);
    }
    let console = metrics.to_string();
    let breakdowns: Vec<Vec<&str>> = console
        .lines()
        .filter(|line| line.contains("\u{2514}\u{2500}"))
        .map(|line| line.split('|').map(str::trim).collect())
        .collect();
    assert_eq!(breakdowns.len(), 2, "{}", console);
    for cells in &breakdowns {
        assert_eq!(cells[1..4], ["1.5", "1", "2"], "{}", console);
    }
    let report = report_of(&metrics, None);
    let rows: Vec<_> = report
        .raw_request_metrics
        .iter()
        .filter(|row| row.is_breakdown)
        .collect();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let html = crate::report::raw_request_metrics_row(row.clone());
        assert!(html.contains("<td>1.5</td>"), "{}", html);
    }
    let mut markdown = Vec::new();
    crate::report::write_markdown_report(&mut markdown, report).unwrap();
    let markdown = String::from_utf8(markdown).unwrap();
    for status in [200, 500] {
        let line = format!("| | \u{2514}\u{2500} {status} (50.0%) | 2 | | 1.5 | 1 | 2 | | |");
        assert!(markdown.contains(&line), "{}", markdown);
    }
}

/// A status code breakdown's average on a half microsecond, 4.5 µs, reads
/// the same in the console and the HTML report.
#[test]
fn a_breakdown_average_on_a_half_microsecond_reads_the_same_everywhere() {
    let mut metrics = request_metrics(&[("GET /", &[4, 5, 1_000])]);
    let request = metrics.requests.get_mut("GET /").unwrap();
    for (status, us) in [(200, 4), (200, 5), (500, 1_000)] {
        request.record_status_code_time_us(status, us);
    }
    let console = metrics.to_string();
    let line = console
        .lines()
        .find(|line| line.contains("\u{2514}\u{2500} 200"))
        .expect("the 200 breakdown");
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    assert_eq!(cells[1], "0.01", "{}", console);
    let report = report_of(&metrics, None);
    let row = report
        .raw_request_metrics
        .iter()
        .find(|row| row.is_breakdown && row.method.starts_with("200"))
        .unwrap();
    let html = crate::report::raw_request_metrics_row(row.clone());
    assert!(html.contains("<td>0.01</td>"), "{}", html);
    // The maximum, 5 µs, also reads 0.01, so check the average cell itself.
    assert_eq!(format_value(&row.response_time_average), "0.01");
}

/// Two saved runs with microseconds compare by their microseconds: each
/// run's rows are rebuilt from its raw timing data, as a baseline's are.
#[cfg(feature = "dashboard")]
#[test]
fn saved_runs_with_microseconds_compare_by_their_microseconds() {
    let metrics = request_metrics(&[("GET /", &[2_350; 5])]);
    let dir = std::env::temp_dir().join(format!("goose-us-compare-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("report.json");
    std::fs::write(
        &path,
        serde_json::to_string(&report_of(&metrics, None)).unwrap(),
    )
    .unwrap();
    let markdown = super::common::compare_reports(&path, "run", &path, "baseline").unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let markdown = String::from_utf8(markdown).unwrap();
    // Average, minimum and maximum.
    assert!(
        markdown.contains("| 2.35 (+0) | 2.35 (+0) | 2.35 (+0) |"),
        "{}",
        markdown
    );
}
