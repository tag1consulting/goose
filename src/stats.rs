use num_format::{Locale, ToFormattedString};
use std::collections::HashMap;
use std::f32;

use crate::Configuration;
use crate::goose::{GooseTaskSets, GooseClient, GooseRequest};
use crate::util;

trait FloatIterExt {
    fn float_min(&mut self) -> f32;
    fn float_max(&mut self) -> f32;
}

impl<T> FloatIterExt for T where T: Iterator<Item=f32> {
    fn float_max(&mut self) -> f32 {
        self.fold(f32::NAN, f32::max)
    }
    
    fn float_min(&mut self) -> f32 {
        self.fold(f32::NAN, f32::min)
    }
}

/// Get the response time that a certain number of percent of the requests finished within.
fn calculate_response_time_percentile(mut response_times: Vec<f32>, percent: f32) -> f32 {
    let total_requests = response_times.len();
    let percentile_request = (total_requests as f32 * percent) as usize;
    debug!("percentile: {}, request {} of total {}", percent, percentile_request, total_requests);
    // Sort response times after which it's trivial to get the slowest request in a percentile
    response_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    return response_times[percentile_request];
}

/// Display a table of requests and fails.
fn print_requests_and_fails(requests: &HashMap<String, GooseRequest>, elapsed: usize) {
    // Display stats from merged HashMap
    println!("------------------------------------------------------------------------------ ");
    println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}", "Name", "# reqs", "# fails", "req/s", "fail/s");
    println!(" ----------------------------------------------------------------------------- ");
    let mut aggregate_fail_count = 0;
    let mut aggregate_total_count = 0;
    for (request_key, request) in requests {
        let total_count = request.success_count + request.fail_count;
        let fail_percent: f32;
        if request.fail_count > 0 {
            fail_percent = request.fail_count as f32 / total_count as f32 * 100.0;
        }
        else {
            fail_percent = 0.0;
        }
        // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
        if fail_percent == 100.0 || fail_percent == 0.0 {
            println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                &request_key,
                total_count.to_formatted_string(&Locale::en),
                format!("{} ({}%)", request.fail_count.to_formatted_string(&Locale::en), fail_percent as usize),
                (total_count / elapsed).to_formatted_string(&Locale::en),
                (request.fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        }
        else {
            println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                &request_key,
                total_count.to_formatted_string(&Locale::en),
                format!("{} ({:.1}%)", request.fail_count.to_formatted_string(&Locale::en), fail_percent),
                (total_count / elapsed).to_formatted_string(&Locale::en),
                (request.fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        }
        aggregate_total_count += total_count;
        aggregate_fail_count += request.fail_count;
    }
    let aggregate_fail_percent: f32;
    if aggregate_fail_count > 0 {
        aggregate_fail_percent = aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0;
    }
    else {
        aggregate_fail_percent = 0.0;
    }
    println!(" ------------------------+----------------+----------------+-------+---------- ");
    // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
    if aggregate_fail_percent == 100.0 || aggregate_fail_percent == 0.0 {
        println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
            "Aggregated",
            aggregate_total_count.to_formatted_string(&Locale::en),
            format!("{} ({}%)", aggregate_fail_count.to_formatted_string(&Locale::en), aggregate_fail_percent as usize),
            (aggregate_total_count / elapsed).to_formatted_string(&Locale::en),
            (aggregate_fail_count / elapsed).to_formatted_string(&Locale::en),
        );
    }
    else {
        println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
            "Aggregated",
            aggregate_total_count.to_formatted_string(&Locale::en),
            format!("{} ({:.1}%)", aggregate_fail_count.to_formatted_string(&Locale::en), aggregate_fail_percent),
            (aggregate_total_count / elapsed).to_formatted_string(&Locale::en),
            (aggregate_fail_count / elapsed).to_formatted_string(&Locale::en),
        );
    }
}

fn print_response_times(requests: &HashMap<String, GooseRequest>, display_percentiles: bool) {
    let mut aggregate_response_times: Vec<f32> = Vec::new();
    println!("-------------------------------------------------------------------------------");
    println!(" {:<23} | {:<10} | {:<10} | {:<10} | {:<10}", "Name", "Avg (ms)", "Min", "Max", "Mean");
    println!(" ----------------------------------------------------------------------------- ");
    for (request_key, mut request) in requests.clone() {
        // Sort response times so we can calculate a mean.
        request.response_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        aggregate_response_times.append(&mut request.response_times.clone());
        println!(" {:<23} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
            &request_key,
            util::mean(&request.response_times),
            &request.response_times.iter().cloned().float_min(),
            &request.response_times.iter().cloned().float_max(),
            util::median(&request.response_times),
        );
    }
    println!(" ------------------------+------------+------------+------------+------------- ");
    println!(" {:<23} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
        "Aggregated",
        util::mean(&aggregate_response_times),
        &aggregate_response_times.iter().cloned().float_min(),
        &aggregate_response_times.iter().cloned().float_max(),
        util::median(&aggregate_response_times),
    );

    if display_percentiles {
        println!("-------------------------------------------------------------------------------");
        println!(" Slowest page load within specified percentile of requests (in ms):");
        println!(" ------------------------------------------------------------------------------");
        println!(" {:<23} | {:<6} | {:<6} | {:<6} | {:<6} | {:<6} | {:6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%");
        println!(" ----------------------------------------------------------------------------- ");
        for (request_key, request) in requests {
            // Sort response times so we can calculate a mean.
            println!(" {:<23} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:6.2}",
                &request_key,
                calculate_response_time_percentile(request.response_times.clone(), 0.5),
                calculate_response_time_percentile(request.response_times.clone(), 0.75),
                calculate_response_time_percentile(request.response_times.clone(), 0.98),
                calculate_response_time_percentile(request.response_times.clone(), 0.99),
                calculate_response_time_percentile(request.response_times.clone(), 0.999),
                calculate_response_time_percentile(request.response_times.clone(), 0.9999),
            );
        }
        println!(" ------------------------+------------+------------+------------+------------- ");
        println!(" {:<23} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:6.2}",
            "Aggregated",
            calculate_response_time_percentile(aggregate_response_times.clone(), 0.5),
            calculate_response_time_percentile(aggregate_response_times.clone(), 0.75),
            calculate_response_time_percentile(aggregate_response_times.clone(), 0.98),
            calculate_response_time_percentile(aggregate_response_times.clone(), 0.99),
            calculate_response_time_percentile(aggregate_response_times.clone(), 0.999),
            calculate_response_time_percentile(aggregate_response_times.clone(), 0.9999),
        );
    }
}

fn print_status_codes(requests: &HashMap<String, GooseRequest>) {
    println!("-------------------------------------------------------------------------------");
    println!(" {:<23} | {:<25} ", "Name", "Status codes");
    println!(" ----------------------------------------------------------------------------- ");
    let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
    for (request_key, request) in requests {
        let mut codes: String = "".to_string();
        for (status_code, count) in &request.status_code_counts {
            if codes.len() > 0 {
                codes = format!("{}, {} [{}]", codes.clone(), count.to_formatted_string(&Locale::en), status_code);
            }
            else {
                codes = format!("{} [{}]", count.to_formatted_string(&Locale::en), status_code);
            }
            let new_count;
            if let Some(existing_status_code_count) = aggregated_status_code_counts.get(&status_code) {
                new_count = *existing_status_code_count + *count;
            }
            else {
                new_count = *count;
            }
            aggregated_status_code_counts.insert(*status_code, new_count);
        }
        println!(" {:<23} | {:<25}",
            &request_key,
            codes,
        );
    }
    println!("-------------------------------------------------------------------------------");
    let mut codes: String = "".to_string();
    for (status_code, count) in &aggregated_status_code_counts {
        if codes.len() > 0 {
            codes = format!("{}, {} [{}]", codes.clone(), count.to_formatted_string(&Locale::en), status_code);
        }
        else {
            codes = format!("{} [{}]", count.to_formatted_string(&Locale::en), status_code);
        }
    }
    println!(" {:<23} | {:<25} ", "Aggregated", codes);
}

fn merge_stats(weighted_clients: &Vec<GooseClient>, config: &Configuration) -> HashMap<String, GooseRequest> {
    let mut merged_requests: HashMap<String, GooseRequest> = HashMap::new();
    for weighted_client in weighted_clients {
        for (request_key, request) in weighted_client.requests.clone() {
            let mut merged_request;
            if let Some(existing_request) = merged_requests.get(&request_key) {
                merged_request = existing_request.clone();
                merged_request.success_count += request.success_count;
                merged_request.fail_count += request.fail_count;
                merged_request.response_times.append(&mut request.response_times.clone());
                // Only merge status_code_counts if we're displaying the results
                if config.status_codes {
                    for (status_code, count) in request.status_code_counts.clone() {
                        let new_count;
                        if let Some(existing_status_code_count) = merged_request.status_code_counts.get(&status_code) {
                            new_count = *existing_status_code_count + count;
                        }
                        else {
                            new_count = count;
                        }
                        merged_request.status_code_counts.insert(status_code, new_count);
                    }
                }
                merged_requests.insert(request_key, merged_request);
            }
            else {
                merged_requests.insert(request_key, request);
            }
        }
    }
    merged_requests
}

/// Display running and ending statistics
pub fn print_final_stats(config: &Configuration, goose_task_sets: &GooseTaskSets, elapsed: usize) {
    // 1) merge statistics from all clients.
    let merged_requests = merge_stats(&goose_task_sets.weighted_clients, config);
    // 2) print request and fail statistics.
    print_requests_and_fails(&merged_requests, elapsed);
    // 3) print respones time statistics, with percentiles
    print_response_times(&merged_requests, true);
    // 4) print status_codes
    if config.status_codes {
        print_status_codes(&merged_requests);
    }
}

pub fn print_running_stats(config: &Configuration, goose_task_sets: &GooseTaskSets, elapsed: usize) {
    info!("printing running statistics after {} seconds...", elapsed);
    // 1) merge statistics from all clients.
    let merged_requests = merge_stats(&goose_task_sets.weighted_clients, config);
    // 2) print request and fail statistics.
    print_requests_and_fails(&merged_requests, elapsed);
    // 3) print respones time statistics, without percentiles
    print_response_times(&merged_requests, false);
    println!();
}