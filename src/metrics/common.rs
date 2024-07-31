use super::{
    delta::*, merge_times, per_second_calculations, prepare_status_codes, update_max_time,
    update_min_time, GooseMetrics,
};
use crate::report::ErrorMetric;
use crate::{
    report::{
        get_response_metric, CORequestMetric, RequestMetric, ResponseMetric, ScenarioMetric,
        StatusCodeMetric, TransactionMetric,
    },
    util, GooseError,
};
use itertools::Itertools;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::fs::File;
use std::hash::Hash;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
}

pub struct ReportOptions {
    pub no_transaction_metrics: bool,
    pub no_scenario_metrics: bool,
    pub no_status_codes: bool,
}

struct RawIntermediate {
    raw_aggregate_response_time_counter: usize,
    raw_aggregate_response_time_minimum: usize,
    raw_aggregate_total_count: usize,
}

struct Prepare<'m, 'b> {
    options: ReportOptions,
    metrics: &'m GooseMetrics,
    baseline: &'b Option<ReportData<'b>>,

    co_data: bool,
}

impl<'m, 'b> Prepare<'m, 'b> {
    fn new(
        options: ReportOptions,
        metrics: &'m GooseMetrics,
        baseline: &'b Option<ReportData<'b>>,
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

        // correlate with baseline

        if let Some(baseline) = self.baseline {
            correlate_deltas(
                &mut raw_request_metrics,
                &baseline.raw_request_metrics,
                |entry| (entry.method.clone(), entry.name.clone()),
            );
            correlate_deltas(
                &mut raw_response_metrics,
                &baseline.raw_response_metrics,
                |entry| (entry.method.clone(), entry.name.clone()),
            );
        }

        // return result

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

        let mut co_request_metrics = Vec::new();
        let mut co_response_metrics = Vec::new();
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

        if let Some(baseline) = self.baseline {
            if let Some(baseline_co_request_metrics) = &baseline.co_request_metrics {
                correlate_deltas(
                    &mut co_request_metrics,
                    baseline_co_request_metrics,
                    |entry| (entry.method.clone(), entry.name.clone()),
                );
            }
            if let Some(baseline_co_response_metrics) = &baseline.co_response_metrics {
                correlate_deltas(
                    &mut co_response_metrics,
                    baseline_co_response_metrics,
                    |entry| (entry.method.clone(), entry.name.clone()),
                );
            }
        }

        (Some(co_request_metrics), Some(co_response_metrics))
    }

    fn build_transaction(&self, intermediate: &RawIntermediate) -> Option<Vec<TransactionMetric>> {
        if self.options.no_transaction_metrics {
            return None;
        }

        // Only build the transactions template if --no-transaction-metrics isn't enabled.

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
                    transaction: format!("{}.{}", scenario_counter, transaction_counter),
                    name: transaction.transaction_name.to_string(),
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

        if let Some(baseline_transaction_metrics) = self
            .baseline
            .as_ref()
            .and_then(|baseline| baseline.transaction_metrics.as_ref())
        {
            correlate_deltas(
                &mut transaction_metrics,
                baseline_transaction_metrics,
                |entry| (entry.transaction.clone(), entry.name.clone()),
            );
        }

        Some(transaction_metrics)
    }

    fn build_scenario(&self) -> Option<Vec<ScenarioMetric>> {
        // Only build the scenarios template if --no-senario-metrics isn't enabled.

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
            let iterations = match scenario.users.len() {
                0 => 0f32,
                n => scenario.counter as f32 / n as f32,
            };
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

        if let Some(baseline_scenario_metrics) = self
            .baseline
            .as_ref()
            .and_then(|baseline| baseline.scenario_metrics.as_ref())
        {
            correlate_deltas(&mut scenario_metrics, baseline_scenario_metrics, |entry| {
                entry.name.clone()
            });
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

        let mut errors = self
            .metrics
            .errors
            .values()
            .map(|error| ErrorMetric {
                method: error.method,
                name: error.name.clone(),
                error: error.error.clone(),
                occurrences: error.occurrences.into(),
            })
            .collect::<Vec<_>>();

        if let Some(baseline_errors) = self
            .baseline
            .as_ref()
            .and_then(|baseline| baseline.errors.as_ref())
        {
            correlate_deltas(&mut errors, baseline_errors, |error| {
                (error.method, error.name.clone(), error.error.clone())
            });
        }

        Some(errors)
    }
}

pub fn prepare_data<'a, 'b>(
    options: ReportOptions,
    metrics: &'a GooseMetrics,
    baseline: &'b Option<ReportData<'b>>,
) -> ReportData<'a> {
    Prepare::new(options, metrics, baseline).build()
}

/// Load a baseline file
pub(crate) fn load_baseline_file(
    path: impl AsRef<Path>,
) -> Result<ReportData<'static>, GooseError> {
    Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
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
