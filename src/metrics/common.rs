use super::{
    delta::*, merge_times, per_second_calculations, prepare_status_codes, update_max_time,
    update_min_time, CoMetricsSummary, GooseMetrics, NullableFloat,
};
use crate::{
    goose::GooseMethod,
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
    baseline: Option<&ReportData>,
) -> ReportData<'a> {
    let prepare = Prepare::new(options, metrics);
    let mut result = prepare.build();
    if let Some(baseline) = baseline {
        apply_baseline_deltas(&mut result, baseline);
    }
    result
}

struct RawIntermediate {
    raw_aggregate_response_time_counter: usize,
    raw_aggregate_response_time_minimum: usize,
    raw_aggregate_total_count: usize,
}

struct Prepare<'m> {
    options: ReportOptions,
    metrics: &'m GooseMetrics,
    co_data: bool,
}

impl<'m> Prepare<'m> {
    fn new(options: ReportOptions, metrics: &'m GooseMetrics) -> Self {
        Self {
            options,
            metrics,
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
                is_breakdown: false,
            });

            // Add per-status-code breakdown rows when multiple status codes exist.
            if let Some(breakdowns) = request.status_code_breakdowns() {
                for b in breakdowns {
                    raw_request_metrics.push(RequestMetric {
                        method: format!("{} ({:.1}%)", b.status_code, b.percentage),
                        name: String::new(),
                        number_of_requests: b.count.into(),
                        number_of_failures: 0usize.into(),
                        response_time_average: b.average.into(),
                        response_time_minimum: b.min_time.into(),
                        response_time_maximum: b.max_time.into(),
                        requests_per_second: 0.0f32.into(),
                        failures_per_second: 0.0f32.into(),
                        is_breakdown: true,
                    });
                }
            }

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
            is_breakdown: false,
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
                        number_of_requests: 0usize.into(),
                        number_of_failures: 0usize.into(),
                        response_time_average: None,
                        response_time_minimum: 0usize.into(),
                        response_time_maximum: 0usize.into(),
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
                status_codes: codes,
            });
        }

        // Build a list of aggregate status codes.
        let aggregated_codes = prepare_status_codes(&aggregated_status_code_counts, &mut None);

        // Add a final row of aggregate data for the status code table.
        status_code_metrics.push(StatusCodeMetric {
            method: "".to_string(),
            name: "Aggregated".to_string(),
            status_codes: aggregated_codes,
        });

        Some(status_code_metrics)
    }

    fn build_errors(&self) -> Option<Vec<ErrorMetric>> {
        if self.metrics.errors.is_empty() {
            return None;
        }

        let errors: Vec<ErrorMetric> = self
            .metrics
            .errors
            .values()
            .map(|error| ErrorMetric {
                method: error.method,
                name: error.name.clone(),
                error: error.error.clone(),
                occurrences: error.occurrences.into(),
            })
            .collect();

        Some(errors)
    }
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

    // Apply deltas to CO request metrics if present
    if let (Some(current_co), Some(baseline_co)) = (
        &mut current.co_request_metrics,
        &baseline.co_request_metrics,
    ) {
        correlate_deltas(current_co, baseline_co, |r| {
            (r.method.clone(), r.name.clone())
        });
    }

    // Apply deltas to CO response metrics if present
    if let (Some(current_co), Some(baseline_co)) = (
        &mut current.co_response_metrics,
        &baseline.co_response_metrics,
    ) {
        correlate_deltas(current_co, baseline_co, |r| {
            (r.method.clone(), r.name.clone())
        });
    }

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

    // Apply deltas to error metrics if present
    if let (Some(current_errors), Some(baseline_errors)) = (&mut current.errors, &baseline.errors) {
        correlate_deltas(current_errors, baseline_errors, |e| {
            (e.method, e.name.clone(), e.error.clone())
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
    #[serde(default)]
    pub is_breakdown: bool,
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
            is_breakdown: baseline.is_breakdown,
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
    pub method: GooseMethod,
    pub name: String,
    pub error: String,
    pub occurrences: usize,
}

impl From<BaselineErrorMetric> for ErrorMetric {
    fn from(baseline: BaselineErrorMetric) -> Self {
        ErrorMetric {
            method: baseline.method,
            name: baseline.name,
            error: baseline.error,
            occurrences: Value::Plain(baseline.occurrences),
        }
    }
}

/// Validate that baseline data is structurally sound.
fn validate_baseline_data(baseline: &ReportData, path_str: &str) -> Result<(), GooseError> {
    // Request metrics (excluding per-status-code breakdown rows) must match
    // response metrics 1:1, and there must be at least one.
    let non_breakdown_request_count = baseline
        .raw_request_metrics
        .iter()
        .filter(|r| !r.is_breakdown)
        .count();
    let response_count = baseline.raw_response_metrics.len();

    if non_breakdown_request_count != response_count {
        return Err(GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: format!(
                "Inconsistent baseline data: {} request metrics vs {} response metrics",
                non_breakdown_request_count, response_count
            ),
        });
    }

    if non_breakdown_request_count == 0 {
        return Err(GooseError::InvalidOption {
            option: "--baseline-file".to_string(),
            value: path_str.to_string(),
            detail: "Baseline file contains no request metrics".to_string(),
        });
    }

    Ok(())
}

/// Take a current slice of metrics, and apply correlated baseline metrics.
///
/// This will iterate over all the current metrics, fetch the correlated baseline metrics and call
/// [`DeltaTo::delta_to`] on it. Entries are correlated by the key returned from the function `f`.
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
    use crate::report::RequestMetric;

    #[test]
    fn delta_value_usize() {
        assert_eq!(100usize.delta(50usize), 50isize);
        assert_eq!(usize::MAX.delta(usize::MAX), 0isize);
        assert_eq!(usize::MAX.delta(0usize), isize::MAX);
        assert_eq!(0usize.delta(usize::MAX), isize::MIN);
    }

    #[test]
    fn correlate_deltas_matches_by_key() {
        let mut current = vec![
            RequestMetric {
                method: "GET".into(),
                name: "/".into(),
                number_of_requests: 200usize.into(),
                number_of_failures: 5usize.into(),
                response_time_average: 50.0f32.into(),
                response_time_minimum: 10usize.into(),
                response_time_maximum: 500usize.into(),
                requests_per_second: 20.0f32.into(),
                failures_per_second: 0.5f32.into(),
                is_breakdown: false,
            },
            RequestMetric {
                method: "POST".into(),
                name: "/api".into(),
                number_of_requests: 100usize.into(),
                number_of_failures: 2usize.into(),
                response_time_average: 80.0f32.into(),
                response_time_minimum: 20usize.into(),
                response_time_maximum: 800usize.into(),
                requests_per_second: 10.0f32.into(),
                failures_per_second: 0.2f32.into(),
                is_breakdown: false,
            },
        ];

        let baseline = vec![RequestMetric {
            method: "GET".into(),
            name: "/".into(),
            number_of_requests: 150usize.into(),
            number_of_failures: 3usize.into(),
            response_time_average: 40.0f32.into(),
            response_time_minimum: 8usize.into(),
            response_time_maximum: 400usize.into(),
            requests_per_second: 15.0f32.into(),
            failures_per_second: 0.3f32.into(),
            is_breakdown: false,
        }];

        correlate_deltas(&mut current, &baseline, |r| {
            (r.method.clone(), r.name.clone())
        });

        // GET / should have deltas applied
        assert_eq!(current[0].number_of_requests.value(), 200);
        assert!(matches!(current[0].number_of_requests, Value::Delta { .. }));

        // POST /api has no baseline match, so it stays Plain
        assert_eq!(current[1].number_of_requests.value(), 100);
        assert!(matches!(current[1].number_of_requests, Value::Plain(_)));
    }

    /// Helper: build a minimal valid baseline JSON string.
    fn minimal_baseline_json(
        requests: usize,
        failures: usize,
        avg_response: f32,
        min_response: usize,
        max_response: usize,
    ) -> String {
        format!(
            r#"{{
                "raw_metrics": {{
                    "hash": 0,
                    "duration": 60,
                    "maximum_users": 10,
                    "total_users": 10,
                    "requests": {{}},
                    "transactions": [],
                    "errors": {{}}
                }},
                "raw_request_metrics": [
                    {{
                        "method": "GET",
                        "name": "/",
                        "number_of_requests": {requests},
                        "number_of_failures": {failures},
                        "response_time_average": {avg_response},
                        "response_time_minimum": {min_response},
                        "response_time_maximum": {max_response},
                        "requests_per_second": 10.0,
                        "failures_per_second": 0.5
                    }}
                ],
                "raw_response_metrics": [
                    {{
                        "method": "GET",
                        "name": "/",
                        "percentile_50": 50,
                        "percentile_60": 60,
                        "percentile_70": 70,
                        "percentile_80": 80,
                        "percentile_90": 90,
                        "percentile_95": 95,
                        "percentile_99": 99,
                        "percentile_100": 100
                    }}
                ]
            }}"#,
        )
    }

    #[test]
    fn load_baseline_file_valid() {
        let path = std::env::temp_dir().join("goose_test_valid_baseline.json");
        std::fs::write(&path, minimal_baseline_json(500, 10, 45.5, 5, 200)).unwrap();
        let result = load_baseline_file(&path);
        std::fs::remove_file(&path).ok();

        let data = result.expect("should load valid baseline");
        assert_eq!(data.raw_request_metrics.len(), 1);
        assert_eq!(data.raw_response_metrics.len(), 1);
        assert_eq!(data.raw_request_metrics[0].number_of_requests.value(), 500);
        assert_eq!(data.raw_request_metrics[0].number_of_failures.value(), 10);
        assert_eq!(data.raw_response_metrics[0].percentile_50.value(), 50);
        assert_eq!(data.raw_response_metrics[0].percentile_100.value(), 100);
        // All values should be Plain (no deltas yet)
        assert!(matches!(
            data.raw_request_metrics[0].number_of_requests,
            Value::Plain(500)
        ));
    }

    #[test]
    fn apply_baseline_deltas_all_metric_types() {
        use crate::report::{
            CORequestMetric, ErrorMetric, ResponseMetric, ScenarioMetric, TransactionMetric,
        };

        let mut current = ReportData {
            raw_metrics: Cow::Owned(GooseMetrics::default()),
            raw_request_metrics: vec![RequestMetric {
                method: "GET".into(),
                name: "/".into(),
                number_of_requests: 200usize.into(),
                number_of_failures: 5usize.into(),
                response_time_average: 50.0f32.into(),
                response_time_minimum: 10usize.into(),
                response_time_maximum: 500usize.into(),
                requests_per_second: 20.0f32.into(),
                failures_per_second: 0.5f32.into(),
                is_breakdown: false,
            }],
            raw_response_metrics: vec![ResponseMetric {
                method: "GET".into(),
                name: "/".into(),
                percentile_50: 50usize.into(),
                percentile_60: 60usize.into(),
                percentile_70: 70usize.into(),
                percentile_80: 80usize.into(),
                percentile_90: 90usize.into(),
                percentile_95: 95usize.into(),
                percentile_99: 99usize.into(),
                percentile_100: 100usize.into(),
            }],
            co_request_metrics: Some(vec![CORequestMetric {
                method: "GET".into(),
                name: "/".into(),
                response_time_average: 55.0f32.into(),
                response_time_standard_deviation: 5.0f32.into(),
                response_time_maximum: 600usize.into(),
            }]),
            co_response_metrics: Some(vec![ResponseMetric {
                method: "GET".into(),
                name: "/".into(),
                percentile_50: 55usize.into(),
                percentile_60: 65usize.into(),
                percentile_70: 75usize.into(),
                percentile_80: 85usize.into(),
                percentile_90: 95usize.into(),
                percentile_95: 100usize.into(),
                percentile_99: 110usize.into(),
                percentile_100: 120usize.into(),
            }]),
            transaction_metrics: Some(vec![TransactionMetric {
                is_scenario: false,
                transaction: "LoadTest".into(),
                name: "front page".into(),
                number_of_requests: 200usize.into(),
                number_of_failures: 5usize.into(),
                response_time_average: Some(50.0f32.into()),
                response_time_minimum: 10usize.into(),
                response_time_maximum: 500usize.into(),
                requests_per_second: Some(20.0f32.into()),
                failures_per_second: Some(0.5f32.into()),
            }]),
            scenario_metrics: Some(vec![ScenarioMetric {
                name: "LoadTest".into(),
                users: 10usize.into(),
                count: 200usize.into(),
                response_time_average: 50.0f32.into(),
                response_time_minimum: 10usize.into(),
                response_time_maximum: 500usize.into(),
                count_per_second: 20.0f32.into(),
                iterations: 5.0f32.into(),
            }]),
            status_code_metrics: None,
            errors: Some(vec![ErrorMetric {
                method: crate::goose::GooseMethod::Get,
                name: "/".into(),
                error: "503 Service Unavailable".into(),
                occurrences: 5usize.into(),
            }]),
            coordinated_omission_metrics: None,
        };

        // Build a baseline with different values
        let baseline = ReportData {
            raw_metrics: Cow::Owned(GooseMetrics::default()),
            raw_request_metrics: vec![RequestMetric {
                method: "GET".into(),
                name: "/".into(),
                number_of_requests: 150usize.into(),
                number_of_failures: 3usize.into(),
                response_time_average: 40.0f32.into(),
                response_time_minimum: 8usize.into(),
                response_time_maximum: 400usize.into(),
                requests_per_second: 15.0f32.into(),
                failures_per_second: 0.3f32.into(),
                is_breakdown: false,
            }],
            raw_response_metrics: vec![ResponseMetric {
                method: "GET".into(),
                name: "/".into(),
                percentile_50: 40usize.into(),
                percentile_60: 50usize.into(),
                percentile_70: 60usize.into(),
                percentile_80: 70usize.into(),
                percentile_90: 80usize.into(),
                percentile_95: 85usize.into(),
                percentile_99: 90usize.into(),
                percentile_100: 95usize.into(),
            }],
            co_request_metrics: Some(vec![CORequestMetric {
                method: "GET".into(),
                name: "/".into(),
                response_time_average: 45.0f32.into(),
                response_time_standard_deviation: 4.0f32.into(),
                response_time_maximum: 500usize.into(),
            }]),
            co_response_metrics: Some(vec![ResponseMetric {
                method: "GET".into(),
                name: "/".into(),
                percentile_50: 45usize.into(),
                percentile_60: 55usize.into(),
                percentile_70: 65usize.into(),
                percentile_80: 75usize.into(),
                percentile_90: 85usize.into(),
                percentile_95: 90usize.into(),
                percentile_99: 100usize.into(),
                percentile_100: 110usize.into(),
            }]),
            transaction_metrics: Some(vec![TransactionMetric {
                is_scenario: false,
                transaction: "LoadTest".into(),
                name: "front page".into(),
                number_of_requests: 150usize.into(),
                number_of_failures: 3usize.into(),
                response_time_average: Some(40.0f32.into()),
                response_time_minimum: 8usize.into(),
                response_time_maximum: 400usize.into(),
                requests_per_second: Some(15.0f32.into()),
                failures_per_second: Some(0.3f32.into()),
            }]),
            scenario_metrics: Some(vec![ScenarioMetric {
                name: "LoadTest".into(),
                users: 8usize.into(),
                count: 150usize.into(),
                response_time_average: 40.0f32.into(),
                response_time_minimum: 8usize.into(),
                response_time_maximum: 400usize.into(),
                count_per_second: 15.0f32.into(),
                iterations: 4.0f32.into(),
            }]),
            status_code_metrics: None,
            errors: Some(vec![ErrorMetric {
                method: crate::goose::GooseMethod::Get,
                name: "/".into(),
                error: "503 Service Unavailable".into(),
                occurrences: 3usize.into(),
            }]),
            coordinated_omission_metrics: None,
        };

        apply_baseline_deltas(&mut current, &baseline);

        // Request metrics: 200 - 150 = +50
        assert!(matches!(
            current.raw_request_metrics[0].number_of_requests,
            Value::Delta {
                value: 200,
                delta: 50
            }
        ));
        // Response percentile: 50 - 40 = +10
        assert!(matches!(
            current.raw_response_metrics[0].percentile_50,
            Value::Delta {
                value: 50,
                delta: 10
            }
        ));
        // CO request: 600 - 500 = +100
        assert!(matches!(
            current.co_request_metrics.as_ref().unwrap()[0].response_time_maximum,
            Value::Delta {
                value: 600,
                delta: 100
            }
        ));
        // CO response percentile: 55 - 45 = +10
        assert!(matches!(
            current.co_response_metrics.as_ref().unwrap()[0].percentile_50,
            Value::Delta {
                value: 55,
                delta: 10
            }
        ));
        // Transaction: 200 - 150 = +50
        assert!(matches!(
            current.transaction_metrics.as_ref().unwrap()[0].number_of_requests,
            Value::Delta {
                value: 200,
                delta: 50
            }
        ));
        // Scenario: 10 - 8 = +2
        assert!(matches!(
            current.scenario_metrics.as_ref().unwrap()[0].users,
            Value::Delta {
                value: 10,
                delta: 2
            }
        ));
        // Error: 5 - 3 = +2
        assert!(matches!(
            current.errors.as_ref().unwrap()[0].occurrences,
            Value::Delta { value: 5, delta: 2 }
        ));
    }

    #[test]
    fn round_trip_serialize_then_load_as_baseline() {
        use crate::report::ResponseMetric;

        // Build a ReportData, serialize to JSON, load as baseline
        let original = ReportData {
            raw_metrics: Cow::Owned(GooseMetrics::default()),
            raw_request_metrics: vec![RequestMetric {
                method: "POST".into(),
                name: "/api/submit".into(),
                number_of_requests: 1000usize.into(),
                number_of_failures: 25usize.into(),
                response_time_average: 120.5f32.into(),
                response_time_minimum: 15usize.into(),
                response_time_maximum: 3000usize.into(),
                requests_per_second: 16.7f32.into(),
                failures_per_second: 0.42f32.into(),
                is_breakdown: false,
            }],
            raw_response_metrics: vec![ResponseMetric {
                method: "POST".into(),
                name: "/api/submit".into(),
                percentile_50: 100usize.into(),
                percentile_60: 110usize.into(),
                percentile_70: 130usize.into(),
                percentile_80: 150usize.into(),
                percentile_90: 200usize.into(),
                percentile_95: 500usize.into(),
                percentile_99: 1500usize.into(),
                percentile_100: 3000usize.into(),
            }],
            co_request_metrics: None,
            co_response_metrics: None,
            scenario_metrics: None,
            transaction_metrics: None,
            status_code_metrics: None,
            errors: None,
            coordinated_omission_metrics: None,
        };

        let json = serde_json::to_string_pretty(&original).unwrap();

        let path = std::env::temp_dir().join("goose_test_roundtrip.json");
        std::fs::write(&path, &json).unwrap();
        let loaded = load_baseline_file(&path);
        std::fs::remove_file(&path).ok();

        let data = loaded.expect("round-trip should succeed");
        assert_eq!(data.raw_request_metrics.len(), 1);
        assert_eq!(data.raw_request_metrics[0].method, "POST");
        assert_eq!(data.raw_request_metrics[0].name, "/api/submit");
        assert_eq!(data.raw_request_metrics[0].number_of_requests.value(), 1000);
        assert_eq!(data.raw_response_metrics[0].percentile_99.value(), 1500);
    }

    #[test]
    fn load_baseline_file_invalid_path() {
        let result = load_baseline_file("/nonexistent/path.json");
        assert!(result.is_err());
    }

    #[test]
    fn load_baseline_file_invalid_json() {
        let path = std::env::temp_dir().join("goose_test_invalid.json");
        std::fs::write(&path, "not valid json").unwrap();
        let result = load_baseline_file(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }

    #[test]
    fn load_baseline_file_empty_metrics() {
        let path = std::env::temp_dir().join("goose_test_empty.json");
        std::fs::write(
            &path,
            r#"{
                "raw_metrics": {
                    "hash": 0,
                    "duration": 10,
                    "maximum_users": 1,
                    "total_users": 1,
                    "requests": {},
                    "transactions": [],
                    "errors": {}
                },
                "raw_request_metrics": [],
                "raw_response_metrics": []
            }"#,
        )
        .unwrap();
        let result = load_baseline_file(&path);
        std::fs::remove_file(&path).ok();
        match result {
            Err(GooseError::InvalidOption { detail, .. }) => {
                assert!(
                    detail.contains("no request metrics"),
                    "unexpected detail: {}",
                    detail
                );
            }
            other => panic!("expected InvalidOption error, got: {:?}", other),
        }
    }
}
