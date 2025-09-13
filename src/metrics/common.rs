use super::{
    delta::*, merge_times, per_second_calculations, prepare_status_codes, update_max_time,
    update_min_time, CoMetricsSummary, GooseMetrics, NullableFloat,
};
use crate::{
    report::{
        get_response_metric, CORequestMetric, ErrorMetric, RequestMetric, ResponseMetric,
        ScenarioMetric, StatusCodeMetric, TransactionMetric,
    },
    util, GooseError,
};
use itertools::Itertools;
use serde::{Deserialize, Deserializer};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::fs::File;
use std::hash::Hash;
use std::io::BufReader;
use std::path::Path;

/// Custom deserializer for f32 values that converts JSON null to f32::NAN
fn deserialize_nullable_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<f32> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or(f32::NAN))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ReportData<'m> {
    pub raw_metrics: Cow<'m, GooseMetrics>,

    pub raw_request_metrics: Vec<RequestMetric>,
    pub raw_response_metrics: Vec<ResponseMetric>,

    pub co_request_metrics: Option<Vec<CORequestMetric>>,
    pub co_response_metrics: Option<Vec<ResponseMetric>>,

    pub scenario_metrics: Option<Vec<ScenarioMetric>>,
    pub transaction_metrics: Option<Vec<TransactionMetric>>,

    pub status_code_metrics: Option<Vec<StatusCodeMetric>>,

    pub errors: Option<Vec<ErrorMetric>>,

    pub coordinated_omission_metrics: Option<CoMetricsSummary>,
}

pub struct ReportOptions {
    pub no_transaction_metrics: bool,
    pub no_scenario_metrics: bool,
    pub no_status_codes: bool,
}

pub fn prepare_data_with_baseline<'a>(
    options: ReportOptions,
    metrics: &'a GooseMetrics,
    baseline: Option<&'a ReportData<'a>>,
) -> ReportData<'a> {
    let prepare = Prepare::new(options, metrics, baseline);
    prepare.build()
}

struct RawIntermediate {
    raw_aggregate_response_time_counter: usize,
    raw_aggregate_response_time_minimum: usize,
    raw_aggregate_total_count: usize,
}

struct Prepare<'m> {
    options: ReportOptions,
    metrics: &'m GooseMetrics,
    baseline: Option<&'m ReportData<'m>>,
    co_data: bool,
}

impl<'m> Prepare<'m> {
    fn new(
        options: ReportOptions,
        metrics: &'m GooseMetrics,
        baseline: Option<&'m ReportData<'m>>,
    ) -> Self {
        Self {
            options,
            metrics,
            baseline,
            co_data: false,
        }
    }

    fn build(mut self) -> ReportData<'m> {
        // Determine whether or not to include Coordinated Omission data.
        self.co_data = self
            .metrics
            .requests
            .values()
            .any(|request| request.coordinated_omission_data.is_some());

        let (raw_request_metrics, raw_response_metrics, intermediate) = self.build_raw();
        let (co_request_metrics, co_response_metrics) = self.build_co(&intermediate);

        ReportData {
            raw_metrics: Cow::Borrowed(self.metrics),
            raw_request_metrics,
            raw_response_metrics,
            co_request_metrics,
            co_response_metrics,
            scenario_metrics: self.build_scenario(),
            transaction_metrics: self.build_transaction(&intermediate),
            status_code_metrics: self.build_status_code(),
            errors: self.build_errors(),
            coordinated_omission_metrics: self
                .metrics
                .coordinated_omission_metrics
                .as_ref()
                .map(|co| co.get_summary()),
        }
    }

    fn build_raw(&self) -> (Vec<RequestMetric>, Vec<ResponseMetric>, RawIntermediate) {
        // Prepare requests and responses variables.
        let mut raw_request_metrics = vec![];
        let mut raw_response_metrics = vec![];
        let mut raw_aggregate_total_count = 0;
        let mut raw_aggregate_fail_count = 0;
        let mut raw_aggregate_response_time_counter: usize = 0;
        let mut raw_aggregate_response_time_minimum: usize = 0;
        let mut raw_aggregate_response_time_maximum: usize = 0;
        let mut raw_aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();

        for (request_key, request) in self.metrics.requests.iter().sorted() {
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
            raw_request_metrics.push(RequestMetric {
                method: method.to_string(),
                name: name.to_string(),
                number_of_requests: total_request_count.into(),
                number_of_failures: request.fail_count.into(),
                response_time_average: (request.raw_data.total_time as f32
                    / request.raw_data.counter as f32)
                    .into(),
                response_time_minimum: request.raw_data.minimum_time.into(),
                response_time_maximum: request.raw_data.maximum_time.into(),
                requests_per_second: requests_per_second.into(),
                failures_per_second: failures_per_second.into(),
            });

            // Prepare per-response metrics.
            raw_response_metrics.push(get_response_metric(
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
        raw_request_metrics.push(RequestMetric {
            method: "".to_string(),
            name: "Aggregated".to_string(),
            number_of_requests: raw_aggregate_total_count.into(),
            number_of_failures: raw_aggregate_fail_count.into(),
            response_time_average: (raw_aggregate_response_time_counter as f32
                / raw_aggregate_total_count as f32)
                .into(),
            response_time_minimum: raw_aggregate_response_time_minimum.into(),
            response_time_maximum: raw_aggregate_response_time_maximum.into(),
            requests_per_second: raw_aggregate_requests_per_second.into(),
            failures_per_second: raw_aggregate_failures_per_second.into(),
        });

        // Prepare aggregate per-response metrics.
        raw_response_metrics.push(get_response_metric(
            "",
            "Aggregated",
            &raw_aggregate_response_times,
            raw_aggregate_total_count,
            raw_aggregate_response_time_minimum,
            raw_aggregate_response_time_maximum,
        ));

        // Apply baseline deltas if available
        if let Some(baseline) = &self.baseline {
            correlate_deltas(
                &mut raw_request_metrics,
                &baseline.raw_request_metrics,
                |r| (r.method.clone(), r.name.clone()),
            );
            correlate_deltas(
                &mut raw_response_metrics,
                &baseline.raw_response_metrics,
                |r| (r.method.clone(), r.name.clone()),
            );
        }

        (
            raw_request_metrics,
            raw_response_metrics,
            RawIntermediate {
                raw_aggregate_response_time_counter,
                raw_aggregate_response_time_minimum,
                raw_aggregate_total_count,
            },
        )
    }

    fn build_co(
        &self,
        intermediate: &RawIntermediate,
    ) -> (Option<Vec<CORequestMetric>>, Option<Vec<ResponseMetric>>) {
        if !self.co_data {
            return (None, None);
        }

        let mut co_request_metrics = vec![];
        let mut co_response_metrics = vec![];
        let mut co_aggregate_total_count = 0;
        let mut co_aggregate_response_time_counter: usize = 0;
        let mut co_aggregate_response_time_maximum: usize = 0;
        let mut co_aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();

        for (request_key, request) in self.metrics.requests.iter().sorted() {
            if let Some(coordinated_omission_data) = request.coordinated_omission_data.as_ref() {
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
                co_request_metrics.push(CORequestMetric {
                    method: method.to_string(),
                    name: name.to_string(),
                    response_time_average: co_average.into(),
                    response_time_standard_deviation: util::standard_deviation(
                        raw_average,
                        co_average,
                    )
                    .into(),
                    response_time_maximum: coordinated_omission_data.maximum_time.into(),
                });

                // Prepare per-response metrics.
                co_response_metrics.push(get_response_metric(
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
        let raw_average = intermediate.raw_aggregate_response_time_counter as f32
            / intermediate.raw_aggregate_total_count as f32;
        co_request_metrics.push(CORequestMetric {
            method: "".to_string(),
            name: "Aggregated".to_string(),
            response_time_average: (co_aggregate_response_time_counter as f32
                / co_aggregate_total_count as f32)
                .into(),
            response_time_standard_deviation: util::standard_deviation(raw_average, co_average)
                .into(),
            response_time_maximum: co_aggregate_response_time_maximum.into(),
        });

        // Prepare aggregate per-response metrics.
        co_response_metrics.push(get_response_metric(
            "",
            "Aggregated",
            &co_aggregate_response_times,
            co_aggregate_total_count,
            intermediate.raw_aggregate_response_time_minimum,
            co_aggregate_response_time_maximum,
        ));

        (Some(co_request_metrics), Some(co_response_metrics))
    }

    fn build_transaction(&self, intermediate: &RawIntermediate) -> Option<Vec<TransactionMetric>> {
        if self.options.no_transaction_metrics {
            return None;
        }

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
                    transaction_metrics.push(TransactionMetric {
                        is_scenario: true,
                        transaction: "".to_string(),
                        name: transaction.scenario_name.to_string(),
                        number_of_requests: 0.into(),
                        number_of_failures: 0.into(),
                        response_time_average: None,
                        response_time_minimum: 0.into(),
                        response_time_maximum: 0.into(),
                        requests_per_second: None,
                        failures_per_second: None,
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
                transaction_metrics.push(TransactionMetric {
                    is_scenario: false,
                    transaction: format!("{scenario_counter}.{transaction_counter}"),
                    name: transaction
                        .transaction_name
                        .name_for_transaction()
                        .to_string(),
                    number_of_requests: total_run_count.into(),
                    number_of_failures: transaction.fail_count.into(),
                    response_time_average: Some(average.into()),
                    response_time_minimum: transaction.min_time.into(),
                    response_time_maximum: transaction.max_time.into(),
                    requests_per_second: Some(requests_per_second.into()),
                    failures_per_second: Some(failures_per_second.into()),
                });

                aggregate_total_count += total_run_count;
                aggregate_fail_count += transaction.fail_count;
                aggregate_transaction_times =
                    merge_times(aggregate_transaction_times, transaction.times.clone());
                aggregate_transaction_time_counter += &transaction.counter;
                aggregate_transaction_time_minimum =
                    update_min_time(aggregate_transaction_time_minimum, transaction.min_time);
                aggregate_transaction_time_maximum =
                    update_max_time(aggregate_transaction_time_maximum, transaction.max_time);
            }
        }

        let (aggregate_requests_per_second, aggregate_failures_per_second) =
            per_second_calculations(
                self.metrics.duration,
                aggregate_total_count,
                aggregate_fail_count,
            );
        transaction_metrics.push(TransactionMetric {
            is_scenario: false,
            transaction: "".to_string(),
            name: "Aggregated".to_string(),
            number_of_requests: aggregate_total_count.into(),
            number_of_failures: aggregate_fail_count.into(),
            response_time_average: Some(
                (intermediate.raw_aggregate_response_time_counter as f32
                    / aggregate_total_count as f32)
                    .into(),
            ),
            response_time_minimum: aggregate_transaction_time_minimum.into(),
            response_time_maximum: aggregate_transaction_time_maximum.into(),
            requests_per_second: Some(aggregate_requests_per_second.into()),
            failures_per_second: Some(aggregate_failures_per_second.into()),
        });

        // Apply baseline deltas if available
        if let Some(baseline) = &self.baseline {
            if let Some(baseline_transactions) = &baseline.transaction_metrics {
                correlate_deltas(&mut transaction_metrics, baseline_transactions, |t| {
                    (t.transaction.clone(), t.name.clone())
                });
            }
        }

        Some(transaction_metrics)
    }

    fn build_scenario(&self) -> Option<Vec<ScenarioMetric>> {
        if self.options.no_scenario_metrics {
            return None;
        }

        let mut scenario_metrics = Vec::new();
        let mut aggregate_users = 0;
        let mut aggregate_count = 0;
        let mut aggregate_scenario_time_counter: usize = 0;
        let mut aggregate_scenario_time_minimum: usize = 0;
        let mut aggregate_scenario_time_maximum: usize = 0;
        let mut aggregate_scenario_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_iterations = 0.0;
        let mut aggregate_response_time_counter = 0.0;
        for scenario in &self.metrics.scenarios {
            let (count_per_second, _failures_per_second) =
                per_second_calculations(self.metrics.duration, scenario.counter, 0);
            let average = match scenario.counter {
                0 => 0.00,
                _ => scenario.total_time as f32 / scenario.counter as f32,
            };
            let iterations = scenario.counter as f32 / scenario.users.len() as f32;
            scenario_metrics.push(ScenarioMetric {
                name: scenario.name.to_string(),
                users: scenario.users.len().into(),
                count: scenario.counter.into(),
                response_time_average: average.into(),
                response_time_minimum: scenario.min_time.into(),
                response_time_maximum: scenario.max_time.into(),
                count_per_second: count_per_second.into(),
                iterations: iterations.into(),
            });

            aggregate_users += scenario.users.len();
            aggregate_count += scenario.counter;
            aggregate_scenario_times =
                merge_times(aggregate_scenario_times, scenario.times.clone());
            aggregate_scenario_time_counter += &scenario.counter;
            aggregate_scenario_time_minimum =
                update_min_time(aggregate_scenario_time_minimum, scenario.min_time);
            aggregate_scenario_time_maximum =
                update_max_time(aggregate_scenario_time_maximum, scenario.max_time);
            aggregate_iterations += iterations;
            aggregate_response_time_counter += scenario.total_time as f32;
        }

        let (aggregate_count_per_second, _aggregate_failures_per_second) =
            per_second_calculations(self.metrics.duration, aggregate_count, 0);
        scenario_metrics.push(ScenarioMetric {
            name: "Aggregated".to_string(),
            users: aggregate_users.into(),
            count: aggregate_count.into(),
            response_time_average: (aggregate_response_time_counter / aggregate_count as f32)
                .into(),
            response_time_minimum: aggregate_scenario_time_minimum.into(),
            response_time_maximum: aggregate_scenario_time_maximum.into(),
            count_per_second: aggregate_count_per_second.into(),
            iterations: aggregate_iterations.into(),
        });

        // Apply baseline deltas if available
        if let Some(baseline) = &self.baseline {
            if let Some(baseline_scenarios) = &baseline.scenario_metrics {
                correlate_deltas(&mut scenario_metrics, baseline_scenarios, |s| {
                    s.name.clone()
                });
            }
        }

        Some(scenario_metrics)
    }

    fn build_status_code(&self) -> Option<Vec<StatusCodeMetric>> {
        if self.options.no_status_codes {
            return None;
        }

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
            status_code_metrics.push(StatusCodeMetric {
                method,
                name,
                status_codes: Value::Plain(codes),
            });
        }

        // Build a list of aggregate status codes.
        let aggregated_codes = prepare_status_codes(&aggregated_status_code_counts, &mut None);

        // Add a final row of aggregate data for the status code table.
        status_code_metrics.push(StatusCodeMetric {
            method: "".to_string(),
            name: "Aggregated".to_string(),
            status_codes: Value::Plain(aggregated_codes),
        });

        // Apply baseline deltas if available
        if let Some(baseline) = &self.baseline {
            if let Some(baseline_status_codes) = &baseline.status_code_metrics {
                correlate_deltas(&mut status_code_metrics, baseline_status_codes, |s| {
                    (s.method.clone(), s.name.clone())
                });
            }
        }

        Some(status_code_metrics)
    }
    fn build_errors(&self) -> Option<Vec<ErrorMetric>> {
        if self.metrics.errors.is_empty() {
            return None;
        }

        let mut errors: Vec<ErrorMetric> = self
            .metrics
            .errors
            .values()
            .map(|error| ErrorMetric {
                method: error.method.clone(),
                name: error.name.clone(),
                error: error.error.clone(),
                occurrences: error.occurrences.into(),
            })
            .collect();

        // Apply baseline deltas if available
        if let Some(baseline) = &self.baseline {
            if let Some(baseline_errors) = &baseline.errors {
                correlate_deltas(&mut errors, baseline_errors, |e| {
                    (e.method.clone(), e.name.clone(), e.error.clone())
                });
            }
        }

        Some(errors)
    }
}

pub fn prepare_data(options: ReportOptions, metrics: &GooseMetrics) -> ReportData<'_> {
    prepare_data_with_baseline(options, metrics, None)
}

pub fn prepare_data_with_baseline_owned<'a>(
    options: ReportOptions,
    metrics: &'a GooseMetrics,
    baseline: Option<ReportData<'static>>,
) -> ReportData<'a> {
    // For now, we'll store the baseline and process it separately
    // This is a transitional approach - the real solution is to unify the baseline handling
    let prepare = Prepare::new(options, metrics, None);
    let mut result = prepare.build();

    // If we have a baseline, apply the deltas manually
    if let Some(baseline) = baseline {
        apply_baseline_deltas(&mut result, &baseline);
    }

    result
}

/// Apply baseline deltas to the current report data
fn apply_baseline_deltas(current: &mut ReportData, baseline: &ReportData) {
    // Apply deltas to raw request metrics
    correlate_deltas(
        &mut current.raw_request_metrics,
        &baseline.raw_request_metrics,
        |r| (r.method.clone(), r.name.clone()),
    );

    // Apply deltas to raw response metrics
    correlate_deltas(
        &mut current.raw_response_metrics,
        &baseline.raw_response_metrics,
        |r| (r.method.clone(), r.name.clone()),
    );

    // Apply deltas to transaction metrics if present
    if let (Some(current_transactions), Some(baseline_transactions)) = (
        &mut current.transaction_metrics,
        &baseline.transaction_metrics,
    ) {
        correlate_deltas(current_transactions, baseline_transactions, |t| {
            (t.transaction.clone(), t.name.clone())
        });
    }

    // Apply deltas to scenario metrics if present
    if let (Some(current_scenarios), Some(baseline_scenarios)) =
        (&mut current.scenario_metrics, &baseline.scenario_metrics)
    {
        correlate_deltas(current_scenarios, baseline_scenarios, |s| s.name.clone());
    }

    // Apply deltas to status code metrics if present
    if let (Some(current_status_codes), Some(baseline_status_codes)) = (
        &mut current.status_code_metrics,
        &baseline.status_code_metrics,
    ) {
        correlate_deltas(current_status_codes, baseline_status_codes, |s| {
            (s.method.clone(), s.name.clone())
        });
    }

    // Apply deltas to error metrics if present
    if let (Some(current_errors), Some(baseline_errors)) = (&mut current.errors, &baseline.errors) {
        correlate_deltas(current_errors, baseline_errors, |e| {
            (e.method.clone(), e.name.clone(), e.error.clone())
        });
    }
}

/// Load baseline data from a JSON file.
pub fn load_baseline_file<P: AsRef<Path>>(path: P) -> Result<ReportData<'static>, GooseError> {
    let path_str = path.as_ref().to_string_lossy();

    // Open and read the baseline file
    let file = File::open(&path).map_err(|err| GooseError::InvalidOption {
        option: "--baseline-file".to_string(),
        value: path_str.to_string(),
        detail: format!("Failed to open baseline file: {err}"),
    })?;

    let reader = BufReader::new(file);

    // First, deserialize as a BaselineReportData that can handle plain values
    let baseline_data: BaselineReportData =
        serde_json::from_reader(reader).map_err(|err| GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: format!("Failed to parse baseline file as JSON: {err}"),
        })?;

    // Convert BaselineReportData to ReportData with proper Value<T> wrapping
    let mut baseline = baseline_data.into_report_data();

    // Validate the loaded baseline data
    validate_baseline_data(&baseline, &path_str)?;

    // Convert borrowed data to owned for static lifetime
    baseline.raw_metrics = Cow::Owned(baseline.raw_metrics.into_owned());

    Ok(baseline)
}

/// Intermediate structure for deserializing baseline data with plain values
/// that need to be converted to Value<T> enum format
#[derive(Debug, Deserialize)]
struct BaselineReportData {
    pub raw_metrics: BaselineGooseMetrics,
    pub raw_request_metrics: Vec<BaselineRequestMetric>,
    pub raw_response_metrics: Vec<BaselineResponseMetric>,
    pub co_request_metrics: Option<Vec<BaselineCORequestMetric>>,
    pub co_response_metrics: Option<Vec<BaselineResponseMetric>>,
    pub scenario_metrics: Option<Vec<BaselineScenarioMetric>>,
    pub transaction_metrics: Option<Vec<BaselineTransactionMetric>>,
    pub status_code_metrics: Option<Vec<StatusCodeMetric>>,
    pub errors: Option<Vec<BaselineErrorMetric>>,
    pub coordinated_omission_metrics: Option<CoMetricsSummary>,
}

/// Baseline GooseMetrics structure that handles missing history field
#[derive(Debug, Deserialize)]
struct BaselineGooseMetrics {
    pub hash: u64,
    #[serde(default)]
    pub history: Vec<crate::test_plan::TestPlanHistory>,
    pub duration: usize,
    pub maximum_users: usize,
    pub total_users: usize,
    pub requests: super::GooseRequestMetrics,
    pub transactions: super::TransactionMetrics,
    #[serde(default)]
    pub scenarios: super::ScenarioMetrics,
    pub errors: super::GooseErrorMetrics,
    #[serde(default)]
    pub hosts: std::collections::HashSet<String>,
    pub coordinated_omission_metrics: Option<super::CoordinatedOmissionMetrics>,
    #[serde(default)]
    pub final_metrics: bool,
    #[serde(default)]
    pub display_status_codes: bool,
    #[serde(default)]
    pub display_metrics: bool,
}

impl From<BaselineGooseMetrics> for GooseMetrics {
    fn from(baseline: BaselineGooseMetrics) -> Self {
        GooseMetrics {
            hash: baseline.hash,
            history: baseline.history,
            duration: baseline.duration,
            maximum_users: baseline.maximum_users,
            total_users: baseline.total_users,
            requests: baseline.requests,
            transactions: baseline.transactions,
            scenarios: baseline.scenarios,
            errors: baseline.errors,
            hosts: baseline.hosts,
            coordinated_omission_metrics: baseline.coordinated_omission_metrics,
            final_metrics: baseline.final_metrics,
            display_status_codes: baseline.display_status_codes,
            display_metrics: baseline.display_metrics,
        }
    }
}

impl BaselineReportData {
    fn into_report_data(self) -> ReportData<'static> {
        ReportData {
            raw_metrics: Cow::Owned(self.raw_metrics.into()),
            raw_request_metrics: self
                .raw_request_metrics
                .into_iter()
                .map(Into::into)
                .collect(),
            raw_response_metrics: self
                .raw_response_metrics
                .into_iter()
                .map(Into::into)
                .collect(),
            co_request_metrics: self
                .co_request_metrics
                .map(|metrics| metrics.into_iter().map(Into::into).collect()),
            co_response_metrics: self
                .co_response_metrics
                .map(|metrics| metrics.into_iter().map(Into::into).collect()),
            scenario_metrics: self
                .scenario_metrics
                .map(|metrics| metrics.into_iter().map(Into::into).collect()),
            transaction_metrics: self
                .transaction_metrics
                .map(|metrics| metrics.into_iter().map(Into::into).collect()),
            status_code_metrics: self.status_code_metrics,
            errors: self
                .errors
                .map(|errors| errors.into_iter().map(Into::into).collect()),
            coordinated_omission_metrics: self.coordinated_omission_metrics,
        }
    }
}

/// Baseline request metric with plain values
#[derive(Debug, Deserialize)]
struct BaselineRequestMetric {
    pub method: String,
    pub name: String,
    pub number_of_requests: usize,
    pub number_of_failures: usize,
    #[serde(deserialize_with = "deserialize_nullable_f32")]
    pub response_time_average: f32,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    #[serde(deserialize_with = "deserialize_nullable_f32")]
    pub requests_per_second: f32,
    #[serde(deserialize_with = "deserialize_nullable_f32")]
    pub failures_per_second: f32,
}

impl From<BaselineRequestMetric> for RequestMetric {
    fn from(baseline: BaselineRequestMetric) -> Self {
        RequestMetric {
            method: baseline.method,
            name: baseline.name,
            number_of_requests: Value::Plain(baseline.number_of_requests),
            number_of_failures: Value::Plain(baseline.number_of_failures),
            response_time_average: Value::Plain(NullableFloat(baseline.response_time_average)),
            response_time_minimum: Value::Plain(baseline.response_time_minimum),
            response_time_maximum: Value::Plain(baseline.response_time_maximum),
            requests_per_second: Value::Plain(NullableFloat(baseline.requests_per_second)),
            failures_per_second: Value::Plain(NullableFloat(baseline.failures_per_second)),
        }
    }
}

/// Baseline response metric with plain values
#[derive(Debug, Deserialize)]
struct BaselineResponseMetric {
    pub method: String,
    pub name: String,
    pub percentile_50: usize,
    pub percentile_60: usize,
    pub percentile_70: usize,
    pub percentile_80: usize,
    pub percentile_90: usize,
    pub percentile_95: usize,
    pub percentile_99: usize,
    pub percentile_100: usize,
}

impl From<BaselineResponseMetric> for ResponseMetric {
    fn from(baseline: BaselineResponseMetric) -> Self {
        ResponseMetric {
            method: baseline.method,
            name: baseline.name,
            percentile_50: Value::Plain(baseline.percentile_50),
            percentile_60: Value::Plain(baseline.percentile_60),
            percentile_70: Value::Plain(baseline.percentile_70),
            percentile_80: Value::Plain(baseline.percentile_80),
            percentile_90: Value::Plain(baseline.percentile_90),
            percentile_95: Value::Plain(baseline.percentile_95),
            percentile_99: Value::Plain(baseline.percentile_99),
            percentile_100: Value::Plain(baseline.percentile_100),
        }
    }
}

/// Baseline coordinated omission request metric with plain values
#[derive(Debug, Deserialize)]
struct BaselineCORequestMetric {
    pub method: String,
    pub name: String,
    pub response_time_average: f32,
    pub response_time_standard_deviation: f32,
    pub response_time_maximum: usize,
}

impl From<BaselineCORequestMetric> for CORequestMetric {
    fn from(baseline: BaselineCORequestMetric) -> Self {
        CORequestMetric {
            method: baseline.method,
            name: baseline.name,
            response_time_average: Value::Plain(NullableFloat(baseline.response_time_average)),
            response_time_standard_deviation: Value::Plain(NullableFloat(
                baseline.response_time_standard_deviation,
            )),
            response_time_maximum: Value::Plain(baseline.response_time_maximum),
        }
    }
}

/// Baseline transaction metric with plain values
#[derive(Debug, Deserialize)]
struct BaselineTransactionMetric {
    pub is_scenario: bool,
    pub transaction: String,
    pub name: String,
    pub number_of_requests: usize,
    pub number_of_failures: usize,
    pub response_time_average: Option<f32>,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub requests_per_second: Option<f32>,
    pub failures_per_second: Option<f32>,
}

impl From<BaselineTransactionMetric> for TransactionMetric {
    fn from(baseline: BaselineTransactionMetric) -> Self {
        TransactionMetric {
            is_scenario: baseline.is_scenario,
            transaction: baseline.transaction,
            name: baseline.name,
            number_of_requests: Value::Plain(baseline.number_of_requests),
            number_of_failures: Value::Plain(baseline.number_of_failures),
            response_time_average: baseline
                .response_time_average
                .map(|v| Value::Plain(NullableFloat(v))),
            response_time_minimum: Value::Plain(baseline.response_time_minimum),
            response_time_maximum: Value::Plain(baseline.response_time_maximum),
            requests_per_second: baseline
                .requests_per_second
                .map(|v| Value::Plain(NullableFloat(v))),
            failures_per_second: baseline
                .failures_per_second
                .map(|v| Value::Plain(NullableFloat(v))),
        }
    }
}

/// Baseline scenario metric with plain values
#[derive(Debug, Deserialize)]
struct BaselineScenarioMetric {
    pub name: String,
    pub users: usize,
    pub count: usize,
    pub response_time_average: Option<f32>,
    pub response_time_minimum: usize,
    pub response_time_maximum: usize,
    pub count_per_second: f32,
    pub iterations: Option<f32>,
}

impl From<BaselineScenarioMetric> for ScenarioMetric {
    fn from(baseline: BaselineScenarioMetric) -> Self {
        ScenarioMetric {
            name: baseline.name,
            users: Value::Plain(baseline.users),
            count: Value::Plain(baseline.count),
            response_time_average: Value::Plain(NullableFloat(
                baseline.response_time_average.unwrap_or(f32::NAN),
            )),
            response_time_minimum: Value::Plain(baseline.response_time_minimum),
            response_time_maximum: Value::Plain(baseline.response_time_maximum),
            count_per_second: Value::Plain(NullableFloat(baseline.count_per_second)),
            iterations: Value::Plain(NullableFloat(baseline.iterations.unwrap_or(f32::NAN))),
        }
    }
}

/// Baseline error metric with plain values
#[derive(Debug, Deserialize)]
struct BaselineErrorMetric {
    pub method: String,
    pub name: String,
    pub error: String,
    pub occurrences: usize,
}

impl From<BaselineErrorMetric> for ErrorMetric {
    fn from(baseline: BaselineErrorMetric) -> Self {
        use crate::goose::GooseMethod;

        // Parse method string back to GooseMethod enum
        let method = match baseline.method.as_str() {
            "Get" => GooseMethod::Get,
            "Post" => GooseMethod::Post,
            "Head" => GooseMethod::Head,
            "Put" => GooseMethod::Put,
            "Delete" => GooseMethod::Delete,
            "Patch" => GooseMethod::Patch,
            _ => GooseMethod::Get, // fallback to Get for unknown methods
        };

        ErrorMetric {
            method,
            name: baseline.name,
            error: baseline.error,
            occurrences: Value::Plain(baseline.occurrences),
        }
    }
}

/// Validate that baseline data is structurally sound and contains expected data.
fn validate_baseline_data(baseline: &ReportData, path_str: &str) -> Result<(), GooseError> {
    // Validate raw metrics consistency
    validate_raw_metrics_consistency(baseline, path_str)?;

    // Validate coordinated omission metrics if present
    if let Some(co_request_metrics) = &baseline.co_request_metrics {
        validate_coordinated_omission_metrics(baseline, co_request_metrics, path_str)?;
    }

    // Validate transaction metrics if present
    if let Some(transaction_metrics) = &baseline.transaction_metrics {
        validate_transaction_metrics(transaction_metrics, path_str)?;
    }

    // Validate scenario metrics if present
    if let Some(scenario_metrics) = &baseline.scenario_metrics {
        validate_scenario_metrics(scenario_metrics, path_str)?;
    }

    // Validate error metrics if present
    if let Some(errors) = &baseline.errors {
        validate_error_metrics(errors, path_str)?;
    }

    // Validate status code metrics if present
    if let Some(status_codes) = &baseline.status_code_metrics {
        validate_status_code_metrics(status_codes, path_str)?;
    }

    Ok(())
}

/// Validate raw request and response metrics consistency
fn validate_raw_metrics_consistency(
    baseline: &ReportData,
    path_str: &str,
) -> Result<(), GooseError> {
    // Check that request and response metrics have matching counts
    let request_count = baseline.raw_request_metrics.len();
    let response_count = baseline.raw_response_metrics.len();

    // Allow for the aggregated row (request_count should equal response_count, both > 0)
    if request_count != response_count {
        return Err(GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: format!(
                "Inconsistent baseline data: {} request metrics vs {} response metrics. These should match.",
                request_count, response_count
            ),
        });
    }

    if request_count == 0 {
        return Err(GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: "Baseline file contains no request metrics. Cannot perform meaningful baseline comparison.".to_string(),
        });
    }

    // Validate that request metrics have reasonable values
    for (index, request_metric) in baseline.raw_request_metrics.iter().enumerate() {
        if request_metric.method.is_empty() && request_metric.name != "Aggregated" {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!("Invalid request metric at index {}: method cannot be empty unless it's an aggregated metric", index),
            });
        }

        // Note: usize values cannot be negative, so these checks are mainly for structural validation
        // The main validation is ensuring the values exist and are properly structured
    }

    Ok(())
}

/// Validate coordinated omission metrics
fn validate_coordinated_omission_metrics(
    baseline: &ReportData,
    co_request_metrics: &[CORequestMetric],
    path_str: &str,
) -> Result<(), GooseError> {
    // If we have CO request metrics, we should also have CO response metrics
    if baseline.co_response_metrics.is_none() {
        return Err(GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: "Baseline contains coordinated omission request metrics but no corresponding response metrics".to_string(),
        });
    }

    let co_response_count = baseline.co_response_metrics.as_ref().unwrap().len();
    if co_request_metrics.len() != co_response_count {
        return Err(GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: format!(
                "Coordinated omission metrics inconsistent: {} request metrics vs {} response metrics",
                co_request_metrics.len(), co_response_count
            ),
        });
    }

    // Validate individual CO metrics structure
    for (index, co_metric) in co_request_metrics.iter().enumerate() {
        if co_metric.method.is_empty() && co_metric.name != "Aggregated" {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!("Invalid coordinated omission metric at index {}: method cannot be empty unless it's an aggregated metric", index),
            });
        }
    }

    Ok(())
}

/// Validate transaction metrics
fn validate_transaction_metrics(
    transaction_metrics: &[TransactionMetric],
    path_str: &str,
) -> Result<(), GooseError> {
    for transaction in transaction_metrics.iter() {
        // Validate that failures don't exceed total requests
        if transaction.number_of_failures > transaction.number_of_requests {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!("Invalid transaction metric '{}': failures ({}) cannot exceed total requests ({})",
                    transaction.name, transaction.number_of_failures, transaction.number_of_requests),
            });
        }

        // Note: We allow empty names for transactions as they can be legitimate in some cases
        // (e.g., unnamed transactions or transactions that are identified by their transaction ID)
    }

    Ok(())
}

/// Validate scenario metrics
fn validate_scenario_metrics(
    scenario_metrics: &[ScenarioMetric],
    path_str: &str,
) -> Result<(), GooseError> {
    for (index, scenario) in scenario_metrics.iter().enumerate() {
        if scenario.name.is_empty() && scenario.name != "Aggregated" {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!("Invalid scenario metric at index {}: name cannot be empty unless it's an aggregated metric", index),
            });
        }
    }

    Ok(())
}

/// Validate error metrics
fn validate_error_metrics(error_metrics: &[ErrorMetric], path_str: &str) -> Result<(), GooseError> {
    for (index, error) in error_metrics.iter().enumerate() {
        if error.error.is_empty() {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!(
                    "Invalid error metric at index {}: error description cannot be empty",
                    index
                ),
            });
        }
    }

    Ok(())
}

/// Validate status code metrics
fn validate_status_code_metrics(
    status_code_metrics: &[StatusCodeMetric],
    path_str: &str,
) -> Result<(), GooseError> {
    for (index, status_code) in status_code_metrics.iter().enumerate() {
        if status_code.method.is_empty() && status_code.name != "Aggregated" {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!("Invalid status code metric at index {}: method cannot be empty unless it's an aggregated metric", index),
            });
        }

        if status_code.status_codes.is_empty() {
            return Err(GooseError::InvalidOption {
                option: "--baseline-file".to_string(),
                value: path_str.to_string(),
                detail: format!(
                    "Invalid status code metric at index {}: status codes cannot be empty",
                    index
                ),
            });
        }
    }

    Ok(())
}

/// take a current slice of metrics, and apply correlated baseline metrics.
///
/// This will iterate over all the current metrics, fetch the correlated baseline metrics and call
/// [`DeltaEval::eval`] on it. Entries are correlated by the key returned from the function `f`.
fn correlate_deltas<T, F, K>(current: &mut [T], baseline: &[T], f: F)
where
    T: DeltaTo,
    F: Fn(&T) -> K,
    K: Eq + Hash,
{
    let mut current = current
        .iter_mut()
        .map(|request| (f(request), request))
        .collect::<HashMap<_, _>>();
    let previous = baseline
        .iter()
        .map(|request| (f(request), request))
        .collect::<HashMap<_, _>>();

    for (k, v) in &mut current {
        if let Some(previous) = previous.get(k) {
            v.delta_to(previous);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn delta_value_usize() {
        assert_eq!(100usize.delta(50usize), 50isize);
        assert_eq!(usize::MAX.delta(usize::MAX), 0isize);
        assert_eq!(usize::MAX.delta(0usize), isize::MAX);
        assert_eq!(0usize.delta(usize::MAX), isize::MIN);
    }
}
