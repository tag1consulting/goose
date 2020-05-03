use num_format::{Locale, ToFormattedString};
use std::collections::{HashMap, BTreeMap};
use std::f32;

use crate::{GooseState, util, merge_response_times, update_min_response_time, update_max_response_time};
use crate::goose::GooseRequest;

/// Get the response time that a certain number of percent of the requests finished within.
fn calculate_response_time_percentile(mut response_times: Vec<usize>, percent: f32) -> usize {
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
                util::truncate_string(&request_key, 23),
                total_count.to_formatted_string(&Locale::en),
                format!("{} ({}%)", request.fail_count.to_formatted_string(&Locale::en), fail_percent as usize),
                (total_count / elapsed).to_formatted_string(&Locale::en),
                (request.fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        }
        else {
            println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                util::truncate_string(&request_key, 23),
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
    println!(" ------------------------+----------------+----------------+--------+--------- ");
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
    let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
    let mut aggregate_total_response_time: usize = 0;
    let mut aggregate_response_time_counter: usize = 0;
    let mut aggregate_min_response_time: usize = 0;
    let mut aggregate_max_response_time: usize = 0;
    println!("-------------------------------------------------------------------------------");
    println!(" {:<23} | {:<10} | {:<10} | {:<10} | {:<10}", "Name", "Avg (ms)", "Min", "Max", "Mean");
    println!(" ----------------------------------------------------------------------------- ");
    for (request_key, request) in requests.clone() {
        // Iterate over client response times, and merge into global response times.
        aggregate_response_times = merge_response_times(
            aggregate_response_times,
            request.response_times.clone(),
        );

        // Increment total response time counter.
        aggregate_total_response_time += &request.total_response_time;

        // Increment counter tracking individual response times seen.
        aggregate_response_time_counter += &request.response_time_counter;

        // If client had new fastest response time, update global fastest response time.
        aggregate_min_response_time = update_min_response_time(aggregate_min_response_time, request.min_response_time);

        // If client had new slowest response time, update global slowest resposne time.
        aggregate_max_response_time = update_max_response_time(aggregate_max_response_time, request.max_response_time);

        println!(" {:<23} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
            util::truncate_string(&request_key, 23),
            request.total_response_time / request.response_time_counter,
            request.min_response_time,
            request.max_response_time,
            // @TODO: fix median calculation
            //util::median(&request.response_times),
            0.0,
        );
    }
    println!(" ------------------------+------------+------------+------------+------------- ");
    println!(" {:<23} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
        "Aggregated",
        aggregate_total_response_time / aggregate_response_time_counter,
        aggregate_min_response_time,
        aggregate_max_response_time,
        0.0,
        //util::median(&aggregate_response_times),
    );

    /*
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
                util::truncate_string(&request_key, 23),
                calculate_response_time_percentile(request.response_times.clone(), 0.5),
                calculate_response_time_percentile(request.response_times.clone(), 0.75),
                calculate_response_time_percentile(request.response_times.clone(), 0.98),
                calculate_response_time_percentile(request.response_times.clone(), 0.99),
                calculate_response_time_percentile(request.response_times.clone(), 0.999),
                calculate_response_time_percentile(request.response_times.clone(), 0.9999),
            );
        }
        println!(" ------------------------+--------+--------+--------+--------+--------+------- ");
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
    */
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
            util::truncate_string(&request_key, 23),
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

fn merge_stats(goose_state: &GooseState) -> HashMap<String, GooseRequest> {
    let mut merged_requests: HashMap<String, GooseRequest> = HashMap::new();
    for weighted_client in &goose_state.weighted_clients {
        for (request_key, request) in weighted_client.requests.clone() {
            let mut merged_request;
            if let Some(existing_request) = merged_requests.get(&request_key) {
                merged_request = existing_request.clone();
                merged_request.success_count += request.success_count;
                merged_request.fail_count += request.fail_count;

                // Iterate over client response times, and merge into global response times.
                merged_request.response_times = merge_response_times(
                    merged_request.response_times,
                    request.response_times.clone(),
                );

                // Increment total response time counter.
                merged_request.total_response_time += &request.total_response_time;

                // Increment counter tracking individual response times seen.
                merged_request.response_time_counter += &request.response_time_counter;

                // If client had new fastest response time, update global fastest response time.
                merged_request.min_response_time = update_min_response_time(merged_request.min_response_time, request.min_response_time);

                // If client had new slowest response time, update global slowest resposne time.
                merged_request.max_response_time = update_max_response_time(merged_request.max_response_time, request.max_response_time);

                // Only merge status_code_counts if we're displaying the results
                if goose_state.configuration.status_codes {
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
pub fn print_final_stats(goose_state: &GooseState, elapsed: usize) {
    // 1) merge statistics from all clients.
    let merged_requests = merge_stats(&goose_state);
    // 2) print request and fail statistics.
    print_requests_and_fails(&merged_requests, elapsed);
    // 3) print respones time statistics, with percentiles
    print_response_times(&merged_requests, true);
    // 4) print status_codes
    if goose_state.configuration.status_codes {
        print_status_codes(&merged_requests);
    }
}

pub fn print_running_stats(goose_state: &GooseState, elapsed: usize) {
    info!("printing running statistics after {} seconds...", elapsed);
    // 1) merge statistics from all clients.
    let merged_requests = merge_stats(&goose_state);
    // 2) print request and fail statistics.
    print_requests_and_fails(&merged_requests, elapsed);
    // 3) print respones time statistics, without percentiles
    print_response_times(&merged_requests, false);
    println!();
}