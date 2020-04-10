#[macro_use]
extern crate log;

//#[macro_use]
//extern crate goose_codegen;

extern crate structopt;

// @TODO: load goosefile as a dynamic library
mod goose;
mod goosefile;
mod util;

use std::f32;
use std::ffi::OsStr;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::{thread, time};

use num_format::{Locale, ToFormattedString};
use rand::thread_rng;
use rand::seq::SliceRandom;
use simplelog::*;
use structopt::StructOpt;

use goose::{GooseTaskSets, GooseTaskSet, GooseTaskSetState, GooseClientMode, GooseClientCommand};

#[derive(Debug, Default, Clone)]
struct GooseState {
    configuration: Option<Configuration>,
    number_of_cpus: usize,
    run_time: usize,
    max_clients: usize,
    active_clients: usize,
}

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


#[derive(StructOpt, Debug, Clone)]
#[structopt(name = "client")]
pub struct Configuration {
    /// Host to load test in the following format: http://10.21.32.33
    #[structopt(short = "H", long, required=false, default_value="")]
    host: String,

    ///// Rust module file to import, e.g. '../other.rs'.
    //#[structopt(short = "f", long, default_value="goosefile")]
    //goosefile: String,

    /// Number of concurrent Goose users (defaults to available CPUs).
    #[structopt(short, long)]
    clients: Option<usize>,

    /// How many users to spawn per second (defaults to available CPUs).
    #[structopt(short = "r", long)]
    hatch_rate: Option<usize>,

    /// Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.).
    #[structopt(short = "t", long, required=false, default_value="")]
    run_time: String,

    /// Prints stats in the console
    #[structopt(long)]
    print_stats: bool,

    /// Only prints summary stats
    #[structopt(long)]
    only_summary: bool,

    /// Resets statistics once hatching has been completed
    #[structopt(long)]
    reset_stats: bool,

    /// Shows list of all possible Goose tasks and exits
    #[structopt(short, long)]
    list: bool,

    /// Number of seconds to wait for a simulated user to complete any executing task before exiting. Default is to terminate immediately.
    #[structopt(short, long, required=false, default_value="0")]
    stop_timeout: usize,

    // The number of occurrences of the `v/verbose` flag
    /// Debug level (-v, -vv, -vvv, etc.)
    #[structopt(short = "v", long, parse(from_occurrences))]
    verbose: u8,

    // The number of occurrences of the `g/log-level` flag
    /// Log level (-g, -gg, -ggg, etc.)
    #[structopt(short = "g", long, parse(from_occurrences))]
    log_level: u8,

    #[structopt(long, default_value="goose.log")]
    log_file: String,
}

/// Locate goosefile dynamic library
fn find_goosefile() -> Option<PathBuf> {
    let goosefile = PathBuf::from("goosefile.rs");
    trace!("goosefile: {:?}", goosefile);
    // @TODO: emulate how Locust does this
    //  - allow override in env
    //  - search from current directory up
    //  - return None if no goosefile is found
    Some(goosefile)
}

/// Load goosefile dynamic library (@TODO)
fn load_goosefile(goosefile: PathBuf) -> Result<GooseTaskSets, &'static str> {
    // @TODO: actually use _goosefile and load as dynamic library
    trace!("@TODO goosefile is currently hardcoded: {:?} ", goosefile);

    let mut goose_task_sets = GooseTaskSets::new();
    // @TODO: handle goosefile errors
    goose_task_sets.initialize_goosefile();
    Ok(goose_task_sets)
}

/// Allocate a vector of weighted GooseTaskSetStates
fn weight_task_set_states(task_sets: &GooseTaskSets, clients: usize) -> Vec<GooseTaskSetState> {
    trace!("weight_task_set_states");

    let mut u: usize = 0;
    let mut v: usize;
    for task_set in &task_sets.task_sets {
        if u == 0 {
            u = task_set.weight;
        }
        else {
            v = task_set.weight;
            trace!("calculating greatest common denominator of {} and {}", u, v);
            u = util::gcd(u, v);
            trace!("inner gcd: {}", u);
        }
    }
    // 'u' will always be the greatest common divisor
    debug!("gcd: {}", u);

    // Build a weighted lists of task sets (identified by index)
    let mut weighted_task_sets = Vec::new();
    for (index, task_set) in task_sets.task_sets.iter().enumerate() {
        // divide by greatest common divisor so vector is as short as possible
        let weight = task_set.weight / u;
        trace!("{}: {} has weight of {} (reduced with gcd to {})", index, task_set.name, task_set.weight, weight);
        let mut weighted_sets = vec![index; weight];
        weighted_task_sets.append(&mut weighted_sets);
    }
    // Shuffle the weighted list of task sets
    weighted_task_sets.shuffle(&mut thread_rng());

    // Allocate a state for each client that will be spawned.
    let mut weighted_states = Vec::new();
    let mut client_count = 0;
    loop {
        for task_sets_index in &weighted_task_sets {
            let task_count = task_sets.task_sets[*task_sets_index].tasks.len();
            weighted_states.push(GooseTaskSetState::new(task_count, *task_sets_index));
            client_count += 1;
            if client_count >= clients {
                trace!("created {} weighted_states", client_count);
                return weighted_states;
            }
        }
    }
}

/// Returns a bucket of weighted Goose Tasks, optionally shuffled
fn weight_tasks(task_set: &GooseTaskSet) -> Vec<usize> {
    trace!("weight_tasks for {}", task_set.name);

    let mut u: usize = 0;
    let mut v: usize;
    for task in &task_set.tasks {
        if u == 0 {
            u = task.weight;
        }
        else {
            v = task.weight;
            trace!("calculating greatest common denominator of {} and {}", u, v);
            u = util::gcd(u, v);
            trace!("inner gcd: {}", u);
        }
    }
    // 'u' will always be the greatest common divisor
    debug!("gcd: {}", u);

    let mut weighted_tasks = Vec::new();
    for (index, task) in task_set.tasks.iter().enumerate() {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!("{}: {} has weight of {} (reduced with gcd to {})", index, task.name, task.weight, weight);
        let mut tasks = vec![index; weight];
        weighted_tasks.append(&mut tasks);
    }
    trace!("created weighted_tasks: {:?}", weighted_tasks);
    weighted_tasks
}

/// If run_time was specified, detect when it's time to shut down
fn timer_expired(started: time::Instant, goose_state: &GooseState) -> bool {
    if goose_state.run_time > 0 && started.elapsed().as_secs() >= goose_state.run_time as u64 {
        true
    }
    else {
        false
    }
}

/// Get the response time that a certain number of percent of the requests finished within.
fn calculate_response_time_percentile(mut response_times: Vec<f32>, percent: f32) -> f32 {
    let total_requests = response_times.len();
    let percentile_request = (total_requests as f32 * percent) as usize;
    debug!("percentile: {}, request {} of total {}", percent, percentile_request, total_requests);
    // Sort response times
    response_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    return response_times[percentile_request];
}

/// Display running and ending statistics
fn display_stats(goose_task_sets: &GooseTaskSets, elapsed: usize) {
    // Prepare a vector of vectors, the outer vector task sets, the inner vector being tasks
    let task_set_count = &goose_task_sets.task_sets.len();
    let mut success_counts = vec![vec![]; *task_set_count];
    let mut fail_counts = vec![vec![]; *task_set_count];
    let mut response_times = vec![vec![]; *task_set_count];
    for (task_set_id, task_set) in goose_task_sets.task_sets.iter().enumerate() {
        let task_count = task_set.tasks.len();
        success_counts[task_set_id] = vec![0; task_count];
        fail_counts[task_set_id] = vec![0; task_count];
        response_times[task_set_id] = vec![vec![]; task_count];
    }
    // Merge stats from all clients.
    for state in &goose_task_sets.weighted_states {
        let task_sets_index = state.task_sets_index;
        for (client_id, count) in state.success_count.iter().enumerate() {
            success_counts[task_sets_index][client_id] += count;
        }
        for (client_id, count) in state.fail_count.iter().enumerate() {
            fail_counts[task_sets_index][client_id] += count;
        }
        for (client_id, times) in state.response_times.iter().enumerate() {
            response_times[task_sets_index][client_id].append(&mut times.clone())
        }
    }
    for (task_set_id, task_set) in goose_task_sets.task_sets.iter().enumerate() {
        // @TODO: adapt to actual terminal window size:
        //  - as window increases, expand columns to take advantage of additional space
        //  - when the window increases enough, include all information in a single table
        println!("-------------------------------------------------------------------------------");
        println!("{}:", task_set.name);
        println!("------------------------------------------------------------------------------ ");
        println!(" {:<23} | {:<14} | {:<14} | {:<6} | {:<5}", "Name", "# reqs", "# fails", "req/s", "fail/s");
        println!(" ----------------------------------------------------------------------------- ");
        let mut aggregate_fail_count = 0;
        let mut aggregate_total_count = 0;
        for (task_id, task) in task_set.tasks.iter().enumerate() {
            let success_count = success_counts[task_set_id][task_id];
            let fail_count = fail_counts[task_set_id][task_id];
            aggregate_fail_count += fail_count;
            let total_count = success_count + fail_count;
            aggregate_total_count += total_count;
            let fail_percent: f32;
            if fail_count > 0 {
                fail_percent = fail_count as f32 / total_count as f32 * 100.0;
            }
            else {
                fail_percent = 0.0;
            }
            println!(" GET {:<19} | {:<14} | {} ({:.1}%)       | {:<6} | {:<5}",
                task.name,
                total_count.to_formatted_string(&Locale::en),
                fail_count.to_formatted_string(&Locale::en),
                fail_percent,
                (total_count / elapsed).to_formatted_string(&Locale::en),
                (fail_count / elapsed).to_formatted_string(&Locale::en),
            );
        }
        let aggregate_fail_percent: f32;
        if aggregate_fail_count > 0 {
            aggregate_fail_percent = aggregate_fail_count as f32 / aggregate_total_count as f32 * 100.0;
        }
        else {
            aggregate_fail_percent = 0.0;
        }
        println!(" ------------------------+----------------+----------------+-------+---------- ");
        println!(" {:<23} | {:<14} | {} ({:.1}%)       | {:<6} | {:<5}",
            "Aggregated",
            aggregate_total_count.to_formatted_string(&Locale::en),
            aggregate_fail_count.to_formatted_string(&Locale::en),
            aggregate_fail_percent,
            (aggregate_total_count / elapsed).to_formatted_string(&Locale::en),
            (aggregate_fail_count / elapsed).to_formatted_string(&Locale::en),
        );
        println!("-------------------------------------------------------------------------------");
        println!(" {:<23} | {:<10} | {:<10} | {:<10} | {:<10}", "Name", "Avg (ms)", "Min", "Max", "Mean");
        println!(" ----------------------------------------------------------------------------- ");
        let mut aggregate_response_times: Vec<f32> = Vec::new();
        for (task_id, task) in task_set.tasks.iter().enumerate() {
            // Sort response times so we can calculate a mean.
            response_times[task_set_id][task_id].sort_by(|a, b| a.partial_cmp(b).unwrap());
            aggregate_response_times.append(&mut response_times[task_set_id][task_id].clone());
            println!(" GET {:<19} | {:<10.2} | {:<10.2} | {:<10.2} | {:<10.2}",
                task.name,
                util::mean(&response_times[task_set_id][task_id]),
                &response_times[task_set_id][task_id].iter().cloned().float_min(),
                &response_times[task_set_id][task_id].iter().cloned().float_max(),
                util::median(&response_times[task_set_id][task_id]),
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

        println!("-------------------------------------------------------------------------------");
        println!(" Slowest page load within specified percentile of requests (in ms):");
        println!(" ------------------------------------------------------------------------------");
        println!(" {:<23} | {:<6} | {:<6} | {:<6} | {:<6} | {:<6} | {:6}", "Name", "50%", "75%", "98%", "99%", "99.9%", "99.99%");
        println!(" ----------------------------------------------------------------------------- ");
        for (task_id, task) in task_set.tasks.iter().enumerate() {
            // Sort response times so we can calculate a mean.
            println!(" GET {:<19} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:6.2}",
                task.name,
                calculate_response_time_percentile(response_times[task_set_id][task_id].clone(), 0.5),
                calculate_response_time_percentile(response_times[task_set_id][task_id].clone(), 0.75),
                calculate_response_time_percentile(response_times[task_set_id][task_id].clone(), 0.98),
                calculate_response_time_percentile(response_times[task_set_id][task_id].clone(), 0.99),
                calculate_response_time_percentile(response_times[task_set_id][task_id].clone(), 0.999),
                calculate_response_time_percentile(response_times[task_set_id][task_id].clone(), 0.9999),
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

fn main() {
    let mut goose_state = GooseState::default();
    goose_state.configuration = Some(Configuration::from_args());

    // Clone configuration, also leave in goose_state for use in threads.
    let configuration = goose_state.configuration.clone().unwrap();

    // Allow optionally controlling debug output level
    let debug_level;
    match configuration.verbose {
        0 => debug_level = LevelFilter::Warn,
        1 => debug_level = LevelFilter::Info,
        2 => debug_level = LevelFilter::Debug,
        _ => debug_level = LevelFilter::Trace,
    }

    // Allow optionally controlling log level
    let log_level;
    match configuration.log_level {
        0 => log_level = LevelFilter::Info,
        1 => log_level = LevelFilter::Debug,
        _ => log_level = LevelFilter::Trace,
    }

    let log_file = PathBuf::from(&configuration.log_file);

    CombinedLogger::init(vec![
        TermLogger::new(
            debug_level,
            Config::default(),
            TerminalMode::Mixed).unwrap(),
        WriteLogger::new(
            log_level,
            Config::default(),
            File::create(&log_file).unwrap(),
        )]).unwrap();
    info!("Output verbosity level: {}", debug_level);
    info!("Logfile verbosity level: {}", log_level);
    info!("Writing to log file: {}", log_file.display());

    let goosefile = match find_goosefile() {
        Some(g) => g,
        None => {
            error!("Could not find any goosefile! Ensure file ends with '.rs' and see --help for availble options.");
            std::process::exit(1);
        }
    };

    if goosefile.file_name() == Some(OsStr::new("goose.rs")) {
        error!("The goosfile must not be named `goose.rs`. Please rename the file and try again.");
        std::process::exit(1);
    }

    if configuration.run_time != "" {
        goose_state.run_time = util::parse_timespan(&configuration.run_time);
    }
    else {
        goose_state.run_time = 0;
    }
    info!("run_time = {}", goose_state.run_time);

    goose_state.number_of_cpus = num_cpus::get();
    goose_state.max_clients = match configuration.clients {
        Some(c) => {
            if c == 0 {
                error!("At least 1 client is required.");
                std::process::exit(1);
            }
            else {
                c
            }
        }
        None => {
            let c = goose_state.number_of_cpus;
            info!("concurrent clients defaulted to {} (number of CPUs)", c);
            c
        }
    };
    debug!("clients = {}", goose_state.max_clients);
    let hatch_rate = match configuration.hatch_rate {
        Some(h) => {
            if h == 0 {
                error!("The hatch_rate must be greater than 0, and generally should be no more than 100 * NUM_CORES.");
                std::process::exit(1);
            }
            else {
                h
            }
        }
        None => {
            let h = goose_state.number_of_cpus;
            info!("hatch_rate defaulted to {} (number of CPUs)", h);
            h
        }
    };
    debug!("hatch_rate = {}", hatch_rate);

    // Load goosefile
    let mut goose_task_sets = match load_goosefile(goosefile) {
        Ok(g) => g,
        Err(e) => {
            error!("Error loading goosefile: {}", e);
            std::process::exit(1);
        }
    };
    
    if goose_task_sets.task_sets.len() <= 0 {
        error!("No goosefile tasksets defined in goosefile.");
        std::process::exit(1);
    }

    if configuration.list {
        println!("Available tasks:");
        for task_set in goose_task_sets.task_sets {
            println!(" - {} (weight: {})", task_set.name, task_set.weight);
            for task in task_set.tasks {
                println!("    o {} (weight: {})", task.name, task.weight);
            }
        }
        std::process::exit(0);
    }


    for task_set in &mut goose_task_sets.task_sets {
        task_set.weighted_tasks = weight_tasks(&task_set);
        debug!("weighted {} tasks: {:?}", task_set.name, task_set.weighted_tasks);
    }

    // Allocate a state for each of the clients we are about to start
    goose_task_sets.weighted_states = weight_task_set_states(&goose_task_sets, goose_state.max_clients);
    // Start with a simple 0..n-1 range (ie 0, 1, 2, 3, ... n-1)
    goose_task_sets.weighted_states_order = (0..goose_task_sets.weighted_states.len() - 1).collect::<Vec<_>>();

    // Spawn clients, each with their own weighted task_set
    let started = time::Instant::now();
    let sleep_float = 1.0 / hatch_rate as f32;
    let sleep_duration = time::Duration::from_secs_f32(sleep_float);
    let mut clients = vec![];
    let mut client_channels = vec![];
    // Single channel allowing all Goose child threads to sync state back to parent
    let (all_threads_sender, parent_receiver): (mpsc::Sender<GooseTaskSetState>, mpsc::Receiver<GooseTaskSetState>) = mpsc::channel();
    // @TODO: consider replacing this with a Arc<RwLock<>>
    for mut thread_state in goose_task_sets.weighted_states.clone() {
        if timer_expired(started, &goose_state) {
            // Stop launching threads if the run_timer has expired
            break;
        }
        thread_state.weighted_tasks = goose_task_sets.task_sets[thread_state.task_sets_index].weighted_tasks.clone();
        thread_state.weighted_tasks.shuffle(&mut thread_rng());
        thread_state.weighted_states_index = goose_state.active_clients;

        // Per-thread channel allowing parent to control Goose child threads
        let (parent_sender, thread_receiver): (mpsc::Sender<GooseClientCommand>, mpsc::Receiver<GooseClientCommand>) = mpsc::channel();
        client_channels.push(parent_sender);
        // We can only run a task if the task list is non-empty
        if thread_state.weighted_tasks.len() > 0 {
            // Copy the client-to-parent sender channel, used by all threads.
            let thread_sender = all_threads_sender.clone();

            // Hatching a new Goose client.
            thread_state.set_mode(GooseClientMode::HATCHING);
            thread_sender.send(thread_state.clone()).unwrap();

            // Copy the appropriate task_set into the thread.
            let thread_task_set = goose_task_sets.task_sets[thread_state.task_sets_index].clone();

            // Initialize per-task counters
            let task_count = thread_task_set.tasks.len();
            thread_state.response_times = vec![vec![]; task_count];
            thread_state.success_count = vec![0; task_count];
            thread_state.fail_count = vec![0; task_count];
            // active_clients starts at 0, for numbering threads we start at 1 (@TODO: why?)
            let thread_number = goose_state.active_clients + 1;

            // Launch a new client
            let client = thread::spawn(move || {
                info!("launching {} client {}...", thread_task_set.name, thread_number);
                thread_state.set_mode(GooseClientMode::RUNNING);
                thread_sender.send(thread_state.clone()).unwrap();
                let mut thread_continue = true;
                while thread_continue {
                    if thread_task_set.tasks.len() <= thread_state.weighted_position {
                        // Reshuffle the weighted tasks
                        thread_state.weighted_tasks.shuffle(&mut thread_rng());
                        debug!("re-shuffled {} tasks: {:?}", &thread_task_set.name, thread_state.weighted_tasks);
                        thread_state.weighted_position = 0;
                    }
                    let thread_weighted_task = thread_state.weighted_tasks[thread_state.weighted_position];
                    thread_task_set.tasks[thread_weighted_task].counter.fetch_add(1, Ordering::Relaxed);

                    let thread_task_name = &thread_task_set.tasks[thread_weighted_task].name;
                    debug!("launching {} task from {}", thread_task_name, thread_task_set.name);
                    let function = thread_task_set.tasks[thread_weighted_task].function.expect(&format!("{} {} missing load testing function", thread_task_set.name, thread_task_name));
                    function(&mut thread_state);
                    thread_state.weighted_position += 1;

                    let message = thread_receiver.try_recv();
                    if message.is_ok() {
                        match message.unwrap() {
                            GooseClientCommand::SYNC => {
                                thread_sender.send(thread_state.clone()).unwrap();
                                // Reset per-thread counters, as totals have been sent to the parent
                                thread_state.response_times = vec![vec![]; task_count];
                                thread_state.success_count = vec![0; task_count];
                                thread_state.fail_count = vec![0; task_count];
                            },
                            GooseClientCommand::EXIT => {
                                thread_state.set_mode(GooseClientMode::EXITING);
                                // No need to reset per-thread counters, we're exiting and memory will be freed
                                thread_sender.send(thread_state.clone()).unwrap();
                                thread_continue = false
                            }
                        }
                    }

                    // @TODO: configurable/optional delay (wait_time attribute)
                }
            });
            clients.push(client);
            goose_state.active_clients += 1;
            debug!("sleeping {:?} milliseconds...", sleep_duration);
            thread::sleep(sleep_duration);
        }
    }
    info!("launched {} clients...", goose_state.active_clients);

    loop {
        let mut message = parent_receiver.try_recv();
        while message.is_ok() {
            let unwrapped_message = message.unwrap();
            let weighted_states_index = unwrapped_message.weighted_states_index;
            for (client_id, response_times) in unwrapped_message.response_times.iter().enumerate() {
                goose_task_sets.weighted_states[weighted_states_index].response_times[client_id].extend_from_slice(&response_times);
            }
            for (client_id, success_count) in unwrapped_message.success_count.iter().enumerate() {
                goose_task_sets.weighted_states[weighted_states_index].success_count[client_id] += success_count;
            }
            for (client_id, fail_count) in unwrapped_message.fail_count.iter().enumerate() {
                goose_task_sets.weighted_states[weighted_states_index].fail_count[client_id] += fail_count;
            }
            goose_task_sets.weighted_states[weighted_states_index].weighted_position = unwrapped_message.weighted_position;
            goose_task_sets.weighted_states[weighted_states_index].mode = unwrapped_message.mode.clone();
            if goose_task_sets.weighted_states[weighted_states_index].weighted_tasks.len() == 0 {
                goose_task_sets.weighted_states[weighted_states_index].weighted_states_index = unwrapped_message.weighted_states_index;
                goose_task_sets.weighted_states[weighted_states_index].weighted_tasks = unwrapped_message.weighted_tasks.clone();
            }
            message = parent_receiver.try_recv();
        }
        if timer_expired(started, &goose_state) {
            info!("exiting after {} seconds...", configuration.run_time);
            for (index, send_to_client) in client_channels.iter().enumerate() {
                send_to_client.send(GooseClientCommand::EXIT).unwrap();
                debug!("telling client {} to sync stats", index);
            }
            debug!("waiting for clients to exit");
            for client in clients {
                let _ = client.join();
            }
            debug!("all clients exited");
            break;
        }

        let one_second = time::Duration::from_secs(1);
        thread::sleep(one_second);
        // @TODO: only sync when we need to report statistics
        for (index, send_to_client) in client_channels.iter().enumerate() {
            send_to_client.send(GooseClientCommand::SYNC).unwrap();
            debug!("telling client {} to sync stats", index);
        }
    }

    if configuration.print_stats {
        display_stats(&goose_task_sets, goose_state.run_time);
    }
}
