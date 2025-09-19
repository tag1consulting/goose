use super::{
    merge_times, per_second_calculations, prepare_status_codes, report, update_max_time,
    update_min_time, CoMetricsSummary, GooseErrorMetricAggregate, GooseMetrics,
};
use crate::{
    report::{
        CORequestMetric, RequestMetric, ResponseMetric, ScenarioMetric, StatusCodeMetric,
        TransactionMetric,
    },
    util,
};
use itertools::Itertools;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, serde::Serialize)]
pub(crate) struct ReportData<'m> {
    pub raw_metrics: &'m GooseMetrics,

    pub raw_request_metrics: Vec<RequestMetric>,
    pub raw_response_metrics: Vec<ResponseMetric>,

    pub co_request_metrics: Option<Vec<CORequestMetric>>,
    pub co_response_metrics: Option<Vec<ResponseMetric>>,

    pub scenario_metrics: Option<Vec<ScenarioMetric>>,
    pub transaction_metrics: Option<Vec<TransactionMetric>>,

    pub status_code_metrics: Option<Vec<StatusCodeMetric>>,

    pub errors: Option<Vec<&'m GooseErrorMetricAggregate>>,

    pub coordinated_omission_metrics: Option<CoMetricsSummary>,
}

pub struct ReportOptions {
    pub no_transaction_metrics: bool,
    pub no_scenario_metrics: bool,
    pub no_status_codes: bool,
}

pub fn prepare_data(options: ReportOptions, metrics: &GooseMetrics) -> ReportData<'_> {
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

    for (request_key, request) in metrics.requests.iter().sorted() {
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
        let (requests_per_second, failures_per_second) =
            per_second_calculations(metrics.duration, total_request_count, request.fail_count);
        // Prepare per-request metrics.
        raw_request_metrics.push(report::RequestMetric {
            method: method.to_string(),
            name: name.to_string(),
            number_of_requests: total_request_count,
            number_of_failures: request.fail_count,
            response_time_average: request.raw_data.total_time as f32
                / request.raw_data.counter as f32,
            response_time_minimum: request.raw_data.minimum_time,
            response_time_maximum: request.raw_data.maximum_time,
            requests_per_second,
            failures_per_second,
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

        // Add status code response time breakdowns if multiple status codes exist
        if request.status_code_response_times.len() > 1 {
            let mut status_codes: Vec<_> = request.status_code_response_times.keys().collect();
            status_codes.sort();

            for &status_code in status_codes {
                if let Some(timing_data) = request.status_code_response_times.get(&status_code) {
                    // Calculate percentage of requests with this status code
                    let status_count = request
                        .status_code_counts
                        .get(&status_code)
                        .copied()
                        .unwrap_or(0);
                    let total_count = request.success_count + request.fail_count;
                    let percentage = if total_count > 0 {
                        (status_count as f32 / total_count as f32) * 100.0
                    } else {
                        0.0
                    };

                    raw_response_metrics.push(report::get_response_metric(
                        &format!("└─ {} ({:.1}%)", status_code, percentage),
                        &name,
                        &timing_data.times,
                        timing_data.counter,
                        timing_data.minimum_time,
                        timing_data.maximum_time,
                    ));
                }
            }
        }

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
            metrics.duration,
            raw_aggregate_total_count,
            raw_aggregate_fail_count,
        );
    raw_request_metrics.push(report::RequestMetric {
        method: "".to_string(),
        name: "Aggregated".to_string(),
        number_of_requests: raw_aggregate_total_count,
        number_of_failures: raw_aggregate_fail_count,
        response_time_average: raw_aggregate_response_time_counter as f32
            / raw_aggregate_total_count as f32,
        response_time_minimum: raw_aggregate_response_time_minimum,
        response_time_maximum: raw_aggregate_response_time_maximum,
        requests_per_second: raw_aggregate_requests_per_second,
        failures_per_second: raw_aggregate_failures_per_second,
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

    let (co_request_metrics, co_response_metrics) = if co_data {
        for (request_key, request) in metrics.requests.iter().sorted() {
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
                co_request_metrics.push(report::CORequestMetric {
                    method: method.to_string(),
                    name: name.to_string(),
                    response_time_average: co_average,
                    response_time_standard_deviation: util::standard_deviation(
                        raw_average,
                        co_average,
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
            response_time_average: co_aggregate_response_time_counter as f32
                / co_aggregate_total_count as f32,
            response_time_standard_deviation: util::standard_deviation(raw_average, co_average),
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

        (Some(co_request_metrics), Some(co_response_metrics))
    } else {
        (None, None)
    };

    // Only build the transactions template if --no-transaction-metrics isn't enabled.
    let transaction_metrics = if !options.no_transaction_metrics {
        let mut transaction_metrics = Vec::new();
        let mut aggregate_total_count = 0;
        let mut aggregate_fail_count = 0;
        let mut aggregate_transaction_time_counter: usize = 0;
        let mut aggregate_transaction_time_minimum: usize = 0;
        let mut aggregate_transaction_time_maximum: usize = 0;
        let mut aggregate_transaction_times: BTreeMap<usize, usize> = BTreeMap::new();
        for (scenario_counter, scenario) in metrics.transactions.iter().enumerate() {
            for (transaction_counter, transaction) in scenario.iter().enumerate() {
                if transaction_counter == 0 {
                    // Only the scenario_name is used for scenarios.
                    transaction_metrics.push(report::TransactionMetric {
                        is_scenario: true,
                        transaction: "".to_string(),
                        name: transaction.scenario_name.to_string(),
                        number_of_requests: 0,
                        number_of_failures: 0,
                        response_time_average: None,
                        response_time_minimum: 0,
                        response_time_maximum: 0,
                        requests_per_second: None,
                        failures_per_second: None,
                    });
                }
                let total_run_count = transaction.success_count + transaction.fail_count;
                let (requests_per_second, failures_per_second) = per_second_calculations(
                    metrics.duration,
                    total_run_count,
                    transaction.fail_count,
                );
                let average = match transaction.counter {
                    0 => 0.00,
                    _ => transaction.total_time as f32 / transaction.counter as f32,
                };
                transaction_metrics.push(report::TransactionMetric {
                    is_scenario: false,
                    transaction: format!("{scenario_counter}.{transaction_counter}"),
                    name: transaction
                        .transaction_name
                        .name_for_transaction()
                        .to_string(),
                    number_of_requests: total_run_count,
                    number_of_failures: transaction.fail_count,
                    response_time_average: Some(average),
                    response_time_minimum: transaction.min_time,
                    response_time_maximum: transaction.max_time,
                    requests_per_second: Some(requests_per_second),
                    failures_per_second: Some(failures_per_second),
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
                metrics.duration,
                aggregate_total_count,
                aggregate_fail_count,
            );
        transaction_metrics.push(report::TransactionMetric {
            is_scenario: false,
            transaction: "".to_string(),
            name: "Aggregated".to_string(),
            number_of_requests: aggregate_total_count,
            number_of_failures: aggregate_fail_count,
            response_time_average: Some(
                raw_aggregate_response_time_counter as f32 / aggregate_total_count as f32,
            ),
            response_time_minimum: aggregate_transaction_time_minimum,
            response_time_maximum: aggregate_transaction_time_maximum,
            requests_per_second: Some(aggregate_requests_per_second),
            failures_per_second: Some(aggregate_failures_per_second),
        });
        Some(transaction_metrics)
    } else {
        None
    };

    // Only build the scenarios template if --no-senario-metrics isn't enabled.
    let scenario_metrics = if !options.no_scenario_metrics {
        let mut scenario_metrics = Vec::new();
        let mut aggregate_users = 0;
        let mut aggregate_count = 0;
        let mut aggregate_scenario_time_counter: usize = 0;
        let mut aggregate_scenario_time_minimum: usize = 0;
        let mut aggregate_scenario_time_maximum: usize = 0;
        let mut aggregate_scenario_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_iterations = 0.0;
        let mut aggregate_response_time_counter = 0.0;
        for scenario in &metrics.scenarios {
            let (count_per_second, _failures_per_second) =
                per_second_calculations(metrics.duration, scenario.counter, 0);
            let average = match scenario.counter {
                0 => 0.00,
                _ => scenario.total_time as f32 / scenario.counter as f32,
            };
            let iterations = scenario.counter as f32 / scenario.users.len() as f32;
            scenario_metrics.push(report::ScenarioMetric {
                name: scenario.name.to_string(),
                users: scenario.users.len(),
                count: scenario.counter,
                response_time_average: average,
                response_time_minimum: scenario.min_time,
                response_time_maximum: scenario.max_time,
                count_per_second,
                iterations,
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
            per_second_calculations(metrics.duration, aggregate_count, 0);
        scenario_metrics.push(report::ScenarioMetric {
            name: "Aggregated".to_string(),
            users: aggregate_users,
            count: aggregate_count,
            response_time_average: aggregate_response_time_counter / aggregate_count as f32,
            response_time_minimum: aggregate_scenario_time_minimum,
            response_time_maximum: aggregate_scenario_time_maximum,
            count_per_second: aggregate_count_per_second,
            iterations: aggregate_iterations,
        });

        Some(scenario_metrics)
    } else {
        None
    };

    let status_code_metrics = if !options.no_status_codes {
        let mut status_code_metrics = Vec::new();
        let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
        for (request_key, request) in metrics.requests.iter().sorted() {
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
        let aggregated_codes = prepare_status_codes(&aggregated_status_code_counts, &mut None);

        // Add a final row of aggregate data for the status code table.
        status_code_metrics.push(report::StatusCodeMetric {
            method: "".to_string(),
            name: "Aggregated".to_string(),
            status_codes: aggregated_codes,
        });

        Some(status_code_metrics)
    } else {
        None
    };

    ReportData {
        raw_metrics: metrics,
        raw_request_metrics,
        raw_response_metrics,
        co_request_metrics,
        co_response_metrics,
        scenario_metrics,
        transaction_metrics,
        status_code_metrics,
        errors: (!metrics.errors.is_empty()).then(|| metrics.errors.values().collect::<Vec<_>>()),
        coordinated_omission_metrics: metrics
            .coordinated_omission_metrics
            .as_ref()
            .map(|co| co.get_summary()),
    }
}
