use num_format::{Locale, ToFormattedString};
use std::collections::{BTreeMap, HashMap};
use std::f32;

use crate::goose::GooseRequest;
use crate::{
    merge_response_times, update_max_response_time, update_min_response_time, util, GooseAttack,
};

/// Get the response time that a certain number of percent of the requests finished within.
fn calculate_response_time_percentile(
    response_times: &BTreeMap<usize, usize>,
    total_requests: usize,
    min: usize,
    max: usize,
    percent: f32,
) -> usize {
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
                return min;
            } else if *value > max {
                return max;
            } else {
                return *value;
            }
        }
    }
    0
}

/// Display a table of requests and fails.
pub fn print_requests_and_fails(requests: &HashMap<String, GooseRequest>, elapsed: usize) {
    debug!("entering print_requests_and_fails");
    // Display stats from merged HashMap
    println!("------------------------------------------------------------------------------ ");
    println!(
        " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
        "Name", "# reqs", "# fails", "req/s", "fail/s"
    );
    println!(" ----------------------------------------------------------------------------- ");
    let mut aggregate_fail_count = 0;
    let mut aggregate_total_count = 0;
    for (request_key, request) in requests {
        let total_count = request.success_count + request.fail_count;
        let fail_percent = if request.fail_count > 0 {
            request.fail_count as f32 / total_count as f32 * 100.0
        } else {
            0.0
        };
        // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
        if fail_percent as usize == 100 || fail_percent as usize == 0 {
            println!(
                " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                util::truncate_string(&request_key, 23),
                total_count.to_formatted_string(&Locale::en),
                format!(
                    "{} ({}%)",
                    request.fail_count.to_formatted_string(&Locale::en),
                    fail_percent as usize
                ),
                (total_count / elapsed).to_formatted_string(&Locale::en),
                (request.fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        } else {
            println!(
                " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                util::truncate_string(&request_key, 23),
                total_count.to_formatted_string(&Locale::en),
                format!(
                    "{} ({:.1}%)",
                    request.fail_count.to_formatted_string(&Locale::en),
                    fail_percent
                ),
                (total_count / elapsed).to_formatted_string(&Locale::en),
                (request.fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        }
        aggregate_total_count += total_count;
        aggregate_fail_count += request.fail_count;
    }
    if requests.len() > 1 {
        let aggregate_fail_percent = if aggregate_fail_count > 0 {
            aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
        } else {
            0.0
        };
        println!(" ------------------------+----------------+----------------+--------+--------- ");
        // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
        if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
            println!(
                " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                "Aggregated",
                aggregate_total_count.to_formatted_string(&Locale::en),
                format!(
                    "{} ({}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent as usize
                ),
                (aggregate_total_count / elapsed).to_formatted_string(&Locale::en),
                (aggregate_fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        } else {
            println!(
                " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                "Aggregated",
                aggregate_total_count.to_formatted_string(&Locale::en),
                format!(
                    "{} ({:.1}%)",
                    aggregate_fail_count.to_formatted_string(&Locale::en),
                    aggregate_fail_percent
                ),
                (aggregate_total_count / elapsed).to_formatted_string(&Locale::en),
                (aggregate_fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        }
    }
}

fn print_response_times(requests: &HashMap<String, GooseRequest>, display_percentiles: bool) {
    debug!("entering print_response_times");
    let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
    let mut aggregate_total_response_time: usize = 0;
    let mut aggregate_response_time_counter: usize = 0;
    let mut aggregate_min_response_time: usize = 0;
    let mut aggregate_max_response_time: usize = 0;
    println!("-------------------------------------------------------------------------------");
    println!(
        " {:<23} | {:<10} | {:<10} | {:<10} | {:<10}",
        "Name", "Avg (ms)", "Min", "Max", "Median"
    );
    println!(" ----------------------------------------------------------------------------- ");
    for (request_key, request) in requests.clone() {
        // Iterate over client response times, and merge into global response times.
        aggregate_response_times =
            merge_response_times(aggregate_response_times, request.response_times.clone());

        // Increment total response time counter.
        aggregate_total_response_time += &request.total_response_time;

        // Increment counter tracking individual response times seen.
        aggregate_response_time_counter += &request.response_time_counter;

        // If client had new fastest response time, update global fastest response time.
        aggregate_min_response_time =
            update_min_response_time(aggregate_min_response_time, request.min_response_time);

        // If client had new slowest response time, update global slowest resposne time.
        aggregate_max_response_time =
            update_max_response_time(aggregate_max_response_time, request.max_response_time);

        println!(
            " {:<23} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
            util::truncate_string(&request_key, 23),
            request.total_response_time / request.response_time_counter,
            request.min_response_time,
            request.max_response_time,
            util::median(
                &request.response_times,
                request.response_time_counter,
                request.min_response_time,
                request.max_response_time
            ),
        );
    }
    if requests.len() > 1 {
        println!(" ------------------------+------------+------------+------------+------------- ");
        if aggregate_response_time_counter == 0 {
            aggregate_response_time_counter = 1;
        }
        println!(
            " {:<23} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
            "Aggregated",
            aggregate_total_response_time / aggregate_response_time_counter,
            aggregate_min_response_time,
            aggregate_max_response_time,
            util::median(
                &aggregate_response_times,
                aggregate_response_time_counter,
                aggregate_min_response_time,
                aggregate_max_response_time
            ),
        );
    }

    if display_percentiles {
        println!("-------------------------------------------------------------------------------");
        println!(" Slowest page load within specified percentile of requests (in ms):");
        println!(" ------------------------------------------------------------------------------");
        println!(
            " {:<23} | {:<6} | {:<6} | {:<6} | {:<6} | {:<6} | {:6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%"
        );
        println!(" ----------------------------------------------------------------------------- ");
        for (request_key, request) in requests {
            // Sort response times so we can calculate a mean.
            println!(
                " {:<23} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:6.2}",
                util::truncate_string(&request_key, 23),
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
            );
        }
        if requests.len() > 1 {
            println!(
                " ------------------------+--------+--------+--------+--------+--------+------- "
            );
            println!(
                " {:<23} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:6.2}",
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
            );
        }
    }
}

fn print_status_codes(requests: &HashMap<String, GooseRequest>) {
    debug!("entering print_status_codes");
    println!("-------------------------------------------------------------------------------");
    println!(" {:<23} | {:<25} ", "Name", "Status codes");
    println!(" ----------------------------------------------------------------------------- ");
    let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
    for (request_key, request) in requests {
        let mut codes: String = "".to_string();
        for (status_code, count) in &request.status_code_counts {
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
            let new_count;
            if let Some(existing_status_code_count) =
                aggregated_status_code_counts.get(&status_code)
            {
                new_count = *existing_status_code_count + *count;
            } else {
                new_count = *count;
            }
            aggregated_status_code_counts.insert(*status_code, new_count);
        }
        println!(
            " {:<23} | {:<25}",
            util::truncate_string(&request_key, 23),
            codes,
        );
    }
    println!("-------------------------------------------------------------------------------");
    let mut codes: String = "".to_string();
    for (status_code, count) in &aggregated_status_code_counts {
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
    }
    println!(" {:<23} | {:<25} ", "Aggregated", codes);
}

/// Display running and ending statistics
pub fn print_final_stats(goose_attack: &GooseAttack, elapsed: usize) {
    if !goose_attack.configuration.worker {
        info!("printing final statistics after {} seconds...", elapsed);
        // 1) print request and fail statistics.
        print_requests_and_fails(&goose_attack.merged_requests, elapsed);
        // 2) print respones time statistics, with percentiles
        print_response_times(&goose_attack.merged_requests, true);
        // 3) print status_codes
        if goose_attack.configuration.status_codes {
            print_status_codes(&goose_attack.merged_requests);
        }
    }
}

pub fn print_running_stats(goose_attack: &GooseAttack, elapsed: usize) {
    if !goose_attack.configuration.worker && !goose_attack.merged_requests.is_empty() {
        info!("printing running statistics after {} seconds...", elapsed);
        // 1) print request and fail statistics.
        print_requests_and_fails(&goose_attack.merged_requests, elapsed);
        // 2) print respones time statistics, without percentiles
        print_response_times(&goose_attack.merged_requests, false);
        println!();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn max_response_time() {
        let mut response_times: BTreeMap<usize, usize> = BTreeMap::new();
        response_times.insert(1, 1);
        response_times.insert(2, 1);
        response_times.insert(3, 1);
        // 3 * .5 = 1.5, rounds to 2.
        assert_eq!(
            calculate_response_time_percentile(&response_times, 3, 1, 3, 0.5),
            2
        );
        response_times.insert(3, 2);
        // 4 * .5 = 2
        assert_eq!(
            calculate_response_time_percentile(&response_times, 4, 1, 3, 0.5),
            2
        );
        // 4 * .25 = 1
        assert_eq!(
            calculate_response_time_percentile(&response_times, 4, 1, 3, 0.25),
            1
        );
        // 4 * .75 = 3
        assert_eq!(
            calculate_response_time_percentile(&response_times, 4, 1, 3, 0.75),
            3
        );
        // 4 * 1 = 4 (and the 4th response time is also 3)
        assert_eq!(
            calculate_response_time_percentile(&response_times, 4, 1, 3, 1.0),
            3
        );

        // 4 * .5 = 2, but uses specified minimum of 2
        assert_eq!(
            calculate_response_time_percentile(&response_times, 4, 2, 3, 0.25),
            2
        );
        // 4 * .75 = 3, but uses specified maximum of 2
        assert_eq!(
            calculate_response_time_percentile(&response_times, 4, 1, 2, 0.75),
            2
        );

        response_times.insert(10, 25);
        response_times.insert(20, 25);
        response_times.insert(30, 25);
        response_times.insert(50, 25);
        response_times.insert(100, 10);
        response_times.insert(200, 1);
        assert_eq!(
            calculate_response_time_percentile(&response_times, 115, 1, 200, 0.9),
            50
        );
        assert_eq!(
            calculate_response_time_percentile(&response_times, 115, 1, 200, 0.99),
            100
        );
        assert_eq!(
            calculate_response_time_percentile(&response_times, 115, 1, 200, 0.999),
            200
        );
    }
}
