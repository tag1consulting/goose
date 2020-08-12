use itertools::Itertools;
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::{f32, fmt};

use crate::goose::{GooseRequest, GooseTaskSet};
use crate::util;
use crate::GooseConfiguration;

/// Goose optionally tracks statistics about requests made during a load test.
pub type GooseRequestStats = HashMap<String, GooseRequest>;

/// Goose optionally tracks statistics about tasks run during a load test.
pub type GooseTaskStats = Vec<Vec<GooseTaskStat>>;

#[derive(Debug, Clone, Serialize)]
pub struct GooseRawTask {
    /// How many milliseconds the load test has been running.
    pub elapsed: u64,
    /// An index into GooseAttack.task_sets, indicating which task set this is.
    pub taskset_index: usize,
    /// An index into GooseTaskSet.task, indicating which task this is.
    pub task_index: usize,
    /// The optional name of the task.
    pub name: String,
    /// How long task ran.
    pub run_time: u64,
    /// Whether or not the request was successful.
    pub success: bool,
    /// Which GooseUser thread processed the request.
    pub user: usize,
}
impl GooseRawTask {
    pub fn new(
        elapsed: u128,
        taskset_index: usize,
        task_index: usize,
        name: String,
        user: usize,
    ) -> Self {
        GooseRawTask {
            elapsed: elapsed as u64,
            taskset_index,
            task_index,
            name,
            run_time: 0,
            success: true,
            user,
        }
    }

    pub fn set_time(&mut self, time: u128, success: bool) {
        self.run_time = time as u64;
        self.success = success;
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GooseTaskStat {
    /// An index into GooseAttack.task_sets, indicating which task set this is.
    pub taskset_index: usize,
    /// The task set name.
    pub taskset_name: String,
    /// An index into GooseTaskSet.task, indicating which task this is.
    pub task_index: usize,
    /// An optional name for the task.
    pub task_name: String,
    /// Per-run-time counters, tracking how often tasks take a given time to complete.
    pub times: BTreeMap<usize, usize>,
    /// The shortest run-time for this task.
    pub min_time: usize,
    /// The longest run-time for this task.
    pub max_time: usize,
    /// Total combined run-times for this task.
    pub total_time: usize,
    /// Total number of times task has run.
    pub counter: usize,
    /// Total number of times task has run successfully.
    pub success_count: usize,
    /// Total number of times task has failed.
    pub fail_count: usize,
    /// Load test hash.
    pub load_test_hash: u64,
}
impl GooseTaskStat {
    pub fn new(
        taskset_index: usize,
        taskset_name: &str,
        task_index: usize,
        task_name: &str,
        load_test_hash: u64,
    ) -> Self {
        GooseTaskStat {
            taskset_index,
            taskset_name: taskset_name.to_string(),
            task_index,
            task_name: task_name.to_string(),
            times: BTreeMap::new(),
            min_time: 0,
            max_time: 0,
            total_time: 0,
            counter: 0,
            success_count: 0,
            fail_count: 0,
            load_test_hash,
        }
    }

    // Track task function elapsed time.
    pub fn set_time(&mut self, time: u64, success: bool) {
        // Perform this conversion only once, then re-use throughout this function.
        let time_usize = time as usize;

        // Update minimum if this one is fastest yet.
        if self.min_time == 0 || time_usize < self.min_time {
            self.min_time = time_usize;
        }

        // Update maximum if this one is slowest yet.
        if time_usize > self.max_time {
            self.max_time = time_usize;
        }

        // Update total_time, adding in this one.
        self.total_time += time_usize;

        // Each time we store a new time, increment counter by one.
        self.counter += 1;

        if success {
            self.success_count += 1;
        } else {
            self.fail_count += 1;
        }

        // Round the time so we can combine similar times together and
        // minimize required memory to store and push upstream to the parent.
        let rounded_time = match time {
            // No rounding for times 0-100 ms.
            0..=100 => time_usize,
            // Round to nearest 10 for times 100-500 ms.
            101..=500 => ((time as f64 / 10.0).round() * 10.0) as usize,
            // Round to nearest 100 for times 500-1000 ms.
            501..=1000 => ((time as f64 / 100.0).round() * 10.0) as usize,
            // Round to nearest 1000 for larger times.
            _ => ((time as f64 / 1000.0).round() * 10.0) as usize,
        };

        let counter = match self.times.get(&rounded_time) {
            // We've seen this time before, increment counter.
            Some(c) => *c + 1,
            // First time we've seen this time, initialize counter.
            None => 1,
        };
        self.times.insert(rounded_time, counter);
        debug!("incremented {} counter: {}", rounded_time, counter);
    }
}

/// Statistics collected during a Goose load test.
///
/// # Example
/// ```rust,no_run
/// use goose::prelude::*;
///
/// fn main() -> Result<(), GooseError> {
///     let goose_stats: GooseStats = GooseAttack::initialize()?
///         .register_taskset(taskset!("ExampleUsers")
///             .register_task(task!(example_task))
///         )
///         .execute()?;
///
///     // It is now possible to do something with the statistics collected by Goose.
///     // For now, we'll just pretty-print the entire object.
///     println!("{:#?}", goose_stats);
///
///     Ok(())
/// }
///
/// async fn example_task(user: &GooseUser) -> GooseTaskResult {
///     let _goose = user.get("/").await?;
///
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct GooseStats {
    /// A hash of the load test, useful to verify if different statistics are from
    /// the same load test.
    pub hash: u64,
    /// How many seconds the load test ran.
    pub duration: usize,
    /// Total number of users simulated during this load test.
    pub users: usize,
    /// Goose request statistics.
    pub requests: GooseRequestStats,
    /// Goose task statistics.
    pub tasks: GooseTaskStats,
    /// Flag indicating whether or not to display percentile. Because we're deriving Default,
    /// this defaults to false.
    pub display_percentile: bool,
    /// Flag indicating whether or not to display status_codes. Because we're deriving Default,
    /// this defaults to false.
    pub display_status_codes: bool,
}

impl GooseStats {
    pub fn initialize_task_stats(
        &mut self,
        task_sets: &[GooseTaskSet],
        config: &GooseConfiguration,
    ) {
        self.tasks = Vec::new();
        if !config.no_stats && !config.no_task_stats {
            for task_set in task_sets {
                let mut task_vector = Vec::new();
                for task in &task_set.tasks {
                    task_vector.push(GooseTaskStat::new(
                        task_set.task_sets_index,
                        &task_set.name,
                        task.tasks_index,
                        &task.name,
                        0,
                    ));
                }
                self.tasks.push(task_vector);
            }
        }
    }

    /// Consumes and display all statistics from a completed load test.
    ///
    /// # Example
    /// ```rust,no_run
    /// use goose::prelude::*;
    ///
    /// fn main() -> Result<(), GooseError> {
    ///     GooseAttack::initialize()?
    ///         .register_taskset(taskset!("ExampleUsers")
    ///             .register_task(task!(example_task))
    ///         )
    ///         .execute()?
    ///         .print();
    ///
    ///     Ok(())
    /// }
    ///
    /// async fn example_task(user: &GooseUser) -> GooseTaskResult {
    ///     let _goose = user.get("/").await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn print(&self) {
        info!("printing statistics after {} seconds...", self.duration);

        print!("{}", self);
    }

    /// Consumes and displays statistics from a running load test.
    pub fn print_running(&self) {
        info!(
            "printing running statistics after {} seconds...",
            self.duration
        );

        // Include a blank line after printing running statistics.
        println!("{}", self);
    }

    /// Optionally prepares a table of requests and fails.
    pub fn fmt_requests(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.requests.is_empty() {
            return Ok(());
        }

        // Display stats from merged HashMap
        writeln!(
            fmt,
            "------------------------------------------------------------------------------ "
        )?;
        writeln!(
            fmt,
            " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
            "Name", "# reqs", "# fails", "req/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ----------------------------------------------------------------------------- "
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        for (request_key, request) in self.requests.iter().sorted() {
            let total_count = request.success_count + request.fail_count;
            let fail_percent = if request.fail_count > 0 {
                request.fail_count as f32 / total_count as f32 * 100.0
            } else {
                0.0
            };
            let (req_s, fail_s) =
                per_second_calculations(self.duration, total_count, request.fail_count);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if fail_percent as usize == 100 || fail_percent as usize == 0 {
                writeln!(
                    fmt,
                    " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                    util::truncate_string(&request_key, 23),
                    total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({}%)",
                        request.fail_count.to_formatted_string(&Locale::en),
                        fail_percent as usize
                    ),
                    req_s,
                    fail_s,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                    util::truncate_string(&request_key, 23),
                    total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({:.1}%)",
                        request.fail_count.to_formatted_string(&Locale::en),
                        fail_percent
                    ),
                    req_s,
                    fail_s,
                )?;
            }
            aggregate_total_count += total_count;
            aggregate_fail_count += request.fail_count;
        }
        if self.requests.len() > 1 {
            let aggregate_fail_percent = if aggregate_fail_count > 0 {
                aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
            } else {
                0.0
            };
            writeln!(
                fmt,
                " ------------------------+----------------+----------------+--------+--------- "
            )?;
            let (req_s, fail_s) =
                per_second_calculations(self.duration, aggregate_total_count, aggregate_fail_count);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
                writeln!(
                    fmt,
                    " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent as usize
                    ),
                    req_s,
                    fail_s,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({:.1}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent
                    ),
                    req_s,
                    fail_s,
                )?;
            }
        }

        Ok(())
    }

    /// Optionally prepares a table of tasks.
    pub fn fmt_tasks(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.tasks.is_empty() {
            return Ok(());
        }

        // Display stats from merged HashMap
        writeln!(
            fmt,
            "------------------------------------------------------------------------------ "
        )?;
        writeln!(
            fmt,
            " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
            "Name", "# times run", "# fails", "task/s", "fail/s"
        )?;
        writeln!(
            fmt,
            " ----------------------------------------------------------------------------- "
        )?;
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        let mut task_count = 0;
        for task_set in &self.tasks {
            let mut displayed_task_set = false;
            for task in task_set {
                task_count += 1;
                let total_count = task.success_count + task.fail_count;
                let fail_percent = if task.fail_count > 0 {
                    task.fail_count as f32 / total_count as f32 * 100.0
                } else {
                    0.0
                };
                let (run_s, fail_s) =
                    per_second_calculations(self.duration, total_count, task.fail_count);

                // First time through display name of task set.
                if !displayed_task_set {
                    writeln!(
                        fmt,
                        " {:23 } |",
                        util::truncate_string(
                            &format!("{}: {}", task.taskset_index + 1, &task.taskset_name),
                            60
                        ),
                    )?;
                    displayed_task_set = true;
                }

                if fail_percent as usize == 100 || fail_percent as usize == 0 {
                    writeln!(
                        fmt,
                        " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                        util::truncate_string(
                            &format!("  {}: {}", task.task_index + 1, task.task_name),
                            23
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        format!(
                            "{} ({}%)",
                            task.fail_count.to_formatted_string(&Locale::en),
                            fail_percent as usize
                        ),
                        run_s,
                        fail_s,
                    )?;
                } else {
                    writeln!(
                        fmt,
                        " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                        util::truncate_string(
                            &format!("  {}: {}", task.task_index + 1, task.task_name),
                            23
                        ),
                        total_count.to_formatted_string(&Locale::en),
                        format!(
                            "{} ({:.1}%)",
                            task.fail_count.to_formatted_string(&Locale::en),
                            fail_percent
                        ),
                        run_s,
                        fail_s,
                    )?;
                }
                aggregate_total_count += total_count;
                aggregate_fail_count += task.fail_count;
            }
        }
        if task_count > 1 {
            let aggregate_fail_percent = if aggregate_fail_count > 0 {
                aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0
            } else {
                0.0
            };
            writeln!(
                fmt,
                " ------------------------+----------------+----------------+--------+--------- "
            )?;
            let (run_s, fail_s) =
                per_second_calculations(self.duration, aggregate_total_count, aggregate_fail_count);
            // Compress 100.0 and 0.0 to 100 and 0 respectively to save width.
            if aggregate_fail_percent as usize == 100 || aggregate_fail_percent as usize == 0 {
                writeln!(
                    fmt,
                    " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent as usize
                    ),
                    run_s,
                    fail_s,
                )?;
            } else {
                writeln!(
                    fmt,
                    " {:<23} | {:<14} | {:<14} | {:<6} | {:<5}",
                    "Aggregated",
                    aggregate_total_count.to_formatted_string(&Locale::en),
                    format!(
                        "{} ({:.1}%)",
                        aggregate_fail_count.to_formatted_string(&Locale::en),
                        aggregate_fail_percent
                    ),
                    run_s,
                    fail_s,
                )?;
            }
        }

        Ok(())
    }

    // Optionally prepares a table of response times.
    pub fn fmt_response_times(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if self.requests.is_empty() {
            return Ok(());
        }

        let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_total_response_time: usize = 0;
        let mut aggregate_response_time_counter: usize = 0;
        let mut aggregate_min_response_time: usize = 0;
        let mut aggregate_max_response_time: usize = 0;
        writeln!(
            fmt,
            "-------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<23} | {:<10} | {:<10} | {:<10} | {:<10}",
            "Name", "Avg (ms)", "Min", "Max", "Median"
        )?;
        writeln!(
            fmt,
            " ----------------------------------------------------------------------------- "
        )?;
        for (request_key, request) in self.requests.iter().sorted() {
            // Iterate over user response times, and merge into global response times.
            aggregate_response_times =
                merge_response_times(aggregate_response_times, request.response_times.clone());

            // Increment total response time counter.
            aggregate_total_response_time += &request.total_response_time;

            // Increment counter tracking individual response times seen.
            aggregate_response_time_counter += &request.response_time_counter;

            // If user had new fastest response time, update global fastest response time.
            aggregate_min_response_time =
                update_min_response_time(aggregate_min_response_time, request.min_response_time);

            // If user had new slowest response time, update global slowest resposne time.
            aggregate_max_response_time =
                update_max_response_time(aggregate_max_response_time, request.max_response_time);

            writeln!(
                fmt,
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
            )?;
        }
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " ------------------------+------------+------------+------------+------------- "
            )?;
            if aggregate_response_time_counter == 0 {
                aggregate_response_time_counter = 1;
            }
            writeln!(
                fmt,
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
            )?;
        }

        Ok(())
    }

    // Optionallyl prepares a table of slowest response times within several percentiles.
    pub fn fmt_percentiles(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if !self.display_percentile {
            return Ok(());
        }

        let mut aggregate_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let mut aggregate_total_response_time: usize = 0;
        let mut aggregate_response_time_counter: usize = 0;
        let mut aggregate_min_response_time: usize = 0;
        let mut aggregate_max_response_time: usize = 0;
        writeln!(
            fmt,
            "-------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " Slowest page load within specified percentile of requests (in ms):"
        )?;
        writeln!(
            fmt,
            " ------------------------------------------------------------------------------"
        )?;
        writeln!(
            fmt,
            " {:<23} | {:<6} | {:<6} | {:<6} | {:<6} | {:<6} | {:6}",
            "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%"
        )?;
        writeln!(
            fmt,
            " ----------------------------------------------------------------------------- "
        )?;
        for (request_key, request) in self.requests.iter().sorted() {
            // Iterate over user response times, and merge into global response times.
            aggregate_response_times =
                merge_response_times(aggregate_response_times, request.response_times.clone());

            // Increment total response time counter.
            aggregate_total_response_time += &request.total_response_time;

            // Increment counter tracking individual response times seen.
            aggregate_response_time_counter += &request.response_time_counter;

            // If user had new fastest response time, update global fastest response time.
            aggregate_min_response_time =
                update_min_response_time(aggregate_min_response_time, request.min_response_time);

            // If user had new slowest response time, update global slowest resposne time.
            aggregate_max_response_time =
                update_max_response_time(aggregate_max_response_time, request.max_response_time);
            // Sort response times so we can calculate a mean.
            writeln!(
                fmt,
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
            )?;
        }
        if self.requests.len() > 1 {
            writeln!(
                fmt,
                " ------------------------+--------+--------+--------+--------+--------+------- "
            )?;
            writeln!(
                fmt,
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
            )?;
        }

        Ok(())
    }

    // Optionally prepares a table of response status codes.
    pub fn fmt_status_codes(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If there's nothing to display, exit immediately.
        if !self.display_status_codes {
            return Ok(());
        }

        writeln!(
            fmt,
            "-------------------------------------------------------------------------------"
        )?;
        writeln!(fmt, " {:<23} | {:<25} ", "Name", "Status codes")?;
        writeln!(
            fmt,
            " ----------------------------------------------------------------------------- "
        )?;
        let mut aggregated_status_code_counts: HashMap<u16, usize> = HashMap::new();
        for (request_key, request) in self.requests.iter().sorted() {
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

            writeln!(
                fmt,
                " {:<23} | {:<25}",
                util::truncate_string(&request_key, 23),
                codes,
            )?;
        }
        writeln!(
            fmt,
            "-------------------------------------------------------------------------------"
        )?;
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
        writeln!(fmt, " {:<23} | {:<25} ", "Aggregated", codes)?;

        Ok(())
    }
}

impl fmt::Display for GooseStats {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        // Formats from zero to four tables of data, depending on what data is contained
        // and which contained flags are set.
        self.fmt_tasks(fmt)?;
        self.fmt_requests(fmt)?;
        self.fmt_response_times(fmt)?;
        self.fmt_percentiles(fmt)?;
        self.fmt_status_codes(fmt)
    }
}

/// Helper to calculate requests and fails per seconds.
fn per_second_calculations(duration: usize, total: usize, fail: usize) -> (String, String) {
    let requests_per_second;
    let fails_per_second;
    if duration == 0 {
        requests_per_second = 0.to_formatted_string(&Locale::en);
        fails_per_second = 0.to_formatted_string(&Locale::en);
    } else {
        requests_per_second = (total / duration).to_formatted_string(&Locale::en);
        fails_per_second = (fail / duration).to_formatted_string(&Locale::en);
    }
    (requests_per_second, fails_per_second)
}

/// A helper function that merges together response times.
///
/// Used in `lib.rs` to merge together per-thread response times, and in `stats.rs`
/// to aggregate all response times.
pub fn merge_response_times(
    mut global_response_times: BTreeMap<usize, usize>,
    local_response_times: BTreeMap<usize, usize>,
) -> BTreeMap<usize, usize> {
    // Iterate over user response times, and merge into global response times.
    for (response_time, count) in &local_response_times {
        let counter = match global_response_times.get(&response_time) {
            // We've seen this response_time before, increment counter.
            Some(c) => *c + count,
            // First time we've seen this response time, initialize counter.
            None => *count,
        };
        global_response_times.insert(*response_time, counter);
    }
    global_response_times
}

// Update global minimum response time based on local resposne time.
pub fn update_min_response_time(mut global_min: usize, min: usize) -> usize {
    if global_min == 0 || (min > 0 && min < global_min) {
        global_min = min;
    }
    global_min
}

// Update global maximum response time based on local resposne time.
pub fn update_max_response_time(mut global_max: usize, max: usize) -> usize {
    if global_max < max {
        global_max = max;
    }
    global_max
}

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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn max_response_time() {
        let mut max_response_time = 99;
        // Update max response time to a higher value.
        max_response_time = update_max_response_time(max_response_time, 101);
        assert_eq!(max_response_time, 101);
        // Max response time doesn't update when updating with a lower value.
        max_response_time = update_max_response_time(max_response_time, 1);
        assert_eq!(max_response_time, 101);
    }

    #[test]
    fn min_response_time() {
        let mut min_response_time = 11;
        // Update min response time to a lower value.
        min_response_time = update_min_response_time(min_response_time, 9);
        assert_eq!(min_response_time, 9);
        // Min response time doesn't update when updating with a lower value.
        min_response_time = update_min_response_time(min_response_time, 22);
        assert_eq!(min_response_time, 9);
        // Min response time doesn't update when updating with a 0 value.
        min_response_time = update_min_response_time(min_response_time, 0);
        assert_eq!(min_response_time, 9);
    }

    #[test]
    fn response_time_merge() {
        let mut global_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        let local_response_times: BTreeMap<usize, usize> = BTreeMap::new();
        global_response_times =
            merge_response_times(global_response_times, local_response_times.clone());
        // @TODO: how can we do useful testing of private method and objects?
        assert_eq!(&global_response_times, &local_response_times);
    }

    #[test]
    fn max_response_time_percentile() {
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

    #[test]
    fn calculate_per_second() {
        // With duration of 0, requests and fails per second is always 0.
        let mut duration = 0;
        let mut total = 10;
        let fail = 10;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == "0");
        assert!(fails_per_second == "0");
        // Changing total doesn't affect requests and fails as duration is still 0.
        total = 100;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == "0");
        assert!(fails_per_second == "0");

        // With non-zero duration, requests and fails per second return properly.
        duration = 10;
        let (requests_per_second, fails_per_second) =
            per_second_calculations(duration, total, fail);
        assert!(requests_per_second == "10");
        assert!(fails_per_second == "1");
    }
}
