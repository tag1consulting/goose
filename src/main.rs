#[macro_use]
extern crate log;

//#[macro_use]
//extern crate goose_codegen;

extern crate structopt;

mod client;
mod goose;
mod goosefile;
mod stats;
mod util;

use std::collections::HashMap;
use std::f32;
use std::ffi::OsStr;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{thread, time};

use rand::thread_rng;
use rand::seq::SliceRandom;
use simplelog::*;
use structopt::StructOpt;

use goose::{GooseTaskSets, GooseTaskSet, GooseClient, GooseClientMode, GooseClientCommand, GooseRequest};

#[derive(Debug, Clone)]
struct GooseState {
    configuration: Configuration,
    number_of_cpus: usize,
    run_time: usize,
    clients: usize,
    active_clients: usize,
}
impl GooseState {
    fn new(configuration: Configuration) -> GooseState {
        GooseState {
            configuration: configuration,
            number_of_cpus: num_cpus::get(),
            run_time: 0,
            clients: 0,
            active_clients: 0,
        }
    }
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(name = "client")]
pub struct Configuration {
    /// Host to load test in the following format: http://10.21.32.33
    //#[structopt(short = "H", long, required=false, default_value="")]
    #[structopt(short = "H", long, required=true)]
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

    /// Includes status code counts in console stats
    #[structopt(long)]
    status_codes: bool,

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

/// Allocate a vector of weighted GooseClient
fn weight_task_set_clients(task_sets: &GooseTaskSets, clients: usize, state: &GooseState) -> Vec<GooseClient> {
    trace!("weight_task_set_clients");

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
    let mut weighted_clients = Vec::new();
    let mut client_count = 0;
    let config = state.configuration.clone();
    loop {
        for task_sets_index in &weighted_task_sets {
            weighted_clients.push(GooseClient::new(*task_sets_index, &config));
            client_count += 1;
            if client_count >= clients {
                trace!("created {} weighted_clients", client_count);
                return weighted_clients;
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
fn timer_expired(started: time::Instant, run_time: usize) -> bool {
    if run_time > 0 && started.elapsed().as_secs() >= run_time as u64 {
        true
    }
    else {
        false
    }
}

/// Merge per-client-statistics from client thread into global parent statistics
fn merge_from_client(
    parent_request: &GooseRequest,
    client_request: &GooseRequest,
    config: &Configuration,
) -> GooseRequest {
    // Make a mutable copy where we can merge things
    let mut merged_request = parent_request.clone();
    merged_request.response_times.extend_from_slice(&client_request.response_times);
    merged_request.success_count += &client_request.success_count;
    merged_request.fail_count += &client_request.fail_count;
    // Only accrue overhead of merging status_code_counts if we're going to display the results
    if config.status_codes {
        for (status_code, count) in &client_request.status_code_counts {
            let new_count;
            // Add client count into global count
            if let Some(existing_status_code_count) = merged_request.status_code_counts.get(&status_code) {
                new_count = *existing_status_code_count + *count;
            }
            // No global count exists yet, so start with client count
            else {
                new_count = *count;
            }
            merged_request.status_code_counts.insert(*status_code, new_count);
        }
    }
    merged_request
}

fn main() {
    let mut goose_state = GooseState::new(Configuration::from_args());

    // Allow optionally controlling debug output level
    let debug_level;
    match goose_state.configuration.verbose {
        0 => debug_level = LevelFilter::Warn,
        1 => debug_level = LevelFilter::Info,
        2 => debug_level = LevelFilter::Debug,
        _ => debug_level = LevelFilter::Trace,
    }

    // Allow optionally controlling log level
    let log_level;
    match goose_state.configuration.log_level {
        0 => log_level = LevelFilter::Info,
        1 => log_level = LevelFilter::Debug,
        _ => log_level = LevelFilter::Trace,
    }

    let log_file = PathBuf::from(&goose_state.configuration.log_file);

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

    // @TODO: remove, Goose will be a library that goosefiles depend on.
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

    // Don't allow overhead of collecting status codes unless we're printing statistics.
    if goose_state.configuration.status_codes && !goose_state.configuration.print_stats {
        error!("You must enable --print-stats to enable --status-codes.");
        std::process::exit(1);
    }

    // Don't allow overhead of collecting statistics unless we're printing them.
    if goose_state.configuration.only_summary && !goose_state.configuration.print_stats {
        error!("You must enable --print-stats to enable --only-summary.");
        std::process::exit(1);
    }

    // Configure maximum run time if specified, otherwise run until canceled.
    if goose_state.configuration.run_time != "" {
        goose_state.run_time = util::parse_timespan(&goose_state.configuration.run_time);
    }
    else {
        goose_state.run_time = 0;
    }
    info!("run_time = {}", goose_state.run_time);

    // Configure number of client threads to launch, default to the number of CPU cores available.
    goose_state.clients = match goose_state.configuration.clients {
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
    debug!("clients = {}", goose_state.clients);

    // Configure number of client threads to launch per second, default to the number of CPU cores available.
    let hatch_rate = match goose_state.configuration.hatch_rate {
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

    // Load task sets and tasks from goosefile.
    let mut goose_task_sets = match load_goosefile(goosefile) {
        Ok(g) => g,
        Err(e) => {
            error!("Error loading goosefile: {}", e);
            std::process::exit(1);
        }
    };
    
    // At least one task set is required.
    if goose_task_sets.task_sets.len() <= 0 {
        error!("No task sets defined in goosefile.");
        std::process::exit(1);
    }

    if goose_state.configuration.list {
        // Display task sets and tasks, then exit.
        println!("Available tasks:");
        for task_set in goose_task_sets.task_sets {
            println!(" - {} (weight: {})", task_set.name, task_set.weight);
            for task in task_set.tasks {
                println!("    o {} (weight: {})", task.name, task.weight);
            }
        }
        std::process::exit(0);
    }

    // Apply weights to tasks in each task set.
    for task_set in &mut goose_task_sets.task_sets {
        task_set.weighted_tasks = weight_tasks(&task_set);
        debug!("weighted {} tasks: {:?}", task_set.name, task_set.weighted_tasks);
    }

    // Allocate a state for each of the clients we are about to start.
    goose_task_sets.weighted_clients = weight_task_set_clients(&goose_task_sets, goose_state.clients, &goose_state);

    // Our load test is officially starting.
    let mut started = time::Instant::now();
    // Spawn clients at hatch_rate per second, or one every 1 / hatch_rate fraction of a second.
    let sleep_float = 1.0 / hatch_rate as f32;
    let sleep_duration = time::Duration::from_secs_f32(sleep_float);
    // Collect client threads in a vector for when we want to stop them later.
    let mut clients = vec![];
    // Collect client thread channels in a vector so we can talk to the client threads.
    let mut client_channels = vec![];
    // Create a single channel allowing all Goose child threads to sync state back to parent
    let (all_threads_sender, parent_receiver): (mpsc::Sender<GooseClient>, mpsc::Receiver<GooseClient>) = mpsc::channel();
    // Spawn clients, each with their own weighted task_set.
    for mut thread_client in goose_task_sets.weighted_clients.clone() {
        // Stop launching threads if the run_timer has expired.
        if timer_expired(started, goose_state.run_time) {
            break;
        }

        // Copy weighted tasks into the client thread, and shuffle them to run in a random order.
        thread_client.weighted_tasks = goose_task_sets.task_sets[thread_client.task_sets_index].weighted_tasks.clone();
        thread_client.weighted_tasks.shuffle(&mut thread_rng());
        // Remember which task group this client is using.
        thread_client.weighted_clients_index = goose_state.active_clients;

        // Create a per-thread channel allowing parent thread to control child threads.
        let (parent_sender, thread_receiver): (mpsc::Sender<GooseClientCommand>, mpsc::Receiver<GooseClientCommand>) = mpsc::channel();
        client_channels.push(parent_sender);

        // We can only launch tasks if the task list is non-empty
        if thread_client.weighted_tasks.len() > 0 {
            // Copy the client-to-parent sender channel, used by all threads.
            let thread_sender = all_threads_sender.clone();

            // Hatching a new Goose client.
            thread_client.set_mode(GooseClientMode::HATCHING);
            // Notify parent that our run mode has changed to Hatching.
            thread_sender.send(thread_client.clone()).unwrap();

            // Copy the appropriate task_set into the thread.
            let thread_task_set = goose_task_sets.task_sets[thread_client.task_sets_index].clone();

            // We number threads from 1 as they're human-visible (in the logs), whereas active_clients starts at 0.
            let thread_number = goose_state.active_clients + 1;

            // Launch a new client.
            let client = thread::spawn(move || {
                 client::client_main(thread_number, thread_task_set, thread_client, thread_receiver, thread_sender)
            });

            clients.push(client);
            goose_state.active_clients += 1;
            debug!("sleeping {:?} milliseconds...", sleep_duration);
            thread::sleep(sleep_duration);
        }
    }
    // Restart the timer now that all threads are launched.
    started = time::Instant::now();
    info!("launched {} clients...", goose_state.active_clients);

    // Initialize per-client state in parent by requesting all threads to sync.
    for (index, send_to_client) in client_channels.iter().enumerate() {
        send_to_client.send(GooseClientCommand::SYNC).unwrap();
        debug!("telling client {} to sync stats", index);
    }

    // Track whether or not we've (optionally) reset the statistics after all clients started.
    let mut statistics_reset: bool = false;

    // Catch ctrl-c to allow clean shutdown to display statistics.
    let canceled = Arc::new(AtomicBool::new(false));
    let caught_ctrlc = canceled.clone();
    ctrlc::set_handler(move || {
        println!("caught ctrl-c, exiting...");
        caught_ctrlc.store(true, Ordering::SeqCst);
    }).expect("Failed to set Ctrl-C signal handler.");

    // Determine when to display running statistics (if enabled).
    let mut statistics_timer = time::Instant::now();
    let mut display_running_statistics = false;

    // Move into a local variable, actual run_time may be less due to SIGINT (ctrl-c).
    let mut run_time = goose_state.run_time;
    loop {
        // When displaying running statistics, sync data from client threads first.
        if goose_state.configuration.print_stats {
            // Synchronize statistics from client threads into parent.
            if timer_expired(statistics_timer, 15) {
                statistics_timer = time::Instant::now();
                for (index, send_to_client) in client_channels.iter().enumerate() {
                    send_to_client.send(GooseClientCommand::SYNC).unwrap();
                    debug!("telling client {} to sync stats", index);
                }
                if !goose_state.configuration.only_summary {
                    display_running_statistics = true;
                }
            }

            // Load messages from client threads until the receiver queue is empty.
            let mut message = parent_receiver.try_recv();
            while message.is_ok() {
                // Messages contain per-client statistics: merge them into the global statistics.
                let unwrapped_message = message.unwrap();
                let weighted_clients_index = unwrapped_message.weighted_clients_index;
                goose_task_sets.weighted_clients[weighted_clients_index].weighted_position = unwrapped_message.weighted_position;
                goose_task_sets.weighted_clients[weighted_clients_index].mode = unwrapped_message.mode;
                // If our local copy of the task set doesn't have tasks, clone them from the remote thread
                if goose_task_sets.weighted_clients[weighted_clients_index].weighted_tasks.len() == 0 {
                    goose_task_sets.weighted_clients[weighted_clients_index].weighted_clients_index = unwrapped_message.weighted_clients_index;
                    goose_task_sets.weighted_clients[weighted_clients_index].weighted_tasks = unwrapped_message.weighted_tasks.clone();
                }
                // Syncronize client requests
                for (request_key, request) in unwrapped_message.requests {
                    trace!("request_key: {}", request_key);
                    let merged_request;
                    if let Some(parent_request) = goose_task_sets.weighted_clients[weighted_clients_index].requests.get(&request_key) {
                        merged_request = merge_from_client(parent_request, &request, &goose_state.configuration);
                    }
                    else {
                        // First time seeing this request, simply insert it.
                        merged_request = request.clone();
                    }
                    goose_task_sets.weighted_clients[weighted_clients_index].requests.insert(request_key.to_string(), merged_request);
                }
                message = parent_receiver.try_recv();
            }

            // Flush statistics collected prior to all client threads running
            if goose_state.configuration.reset_stats && !statistics_reset {
                info!("statistics reset...");
                for (client_index, client) in goose_task_sets.weighted_clients.clone().iter().enumerate() {
                    let mut reset_client = client.clone();
                    // Start again with an empty requests hashmap.
                    reset_client.requests = HashMap::new();
                    goose_task_sets.weighted_clients[client_index] = reset_client;
                }
                statistics_reset = true;
            }
        }

        if timer_expired(started, run_time) || canceled.load(Ordering::SeqCst) {
            run_time = started.elapsed().as_secs() as usize;
            info!("exiting after {} seconds...", run_time);
            for (index, send_to_client) in client_channels.iter().enumerate() {
                send_to_client.send(GooseClientCommand::EXIT).unwrap();
                debug!("telling client {} to sync stats", index);
            }
            debug!("waiting for clients to exit");
            for client in clients {
                let _ = client.join();
            }
            debug!("all clients exited");

            // If we're printing statistics, collect the final messages received from clients
            if goose_state.configuration.print_stats {
                let mut message = parent_receiver.try_recv();
                while message.is_ok() {
                    let unwrapped_message = message.unwrap();
                    let weighted_clients_index = unwrapped_message.weighted_clients_index;
                    goose_task_sets.weighted_clients[weighted_clients_index].mode = unwrapped_message.mode;
                    // Syncronize client requests
                    for (request_key, request) in unwrapped_message.requests {
                        trace!("request_key: {}", request_key);
                        let merged_request;
                        if let Some(parent_request) = goose_task_sets.weighted_clients[weighted_clients_index].requests.get(&request_key) {
                            merged_request = merge_from_client(parent_request, &request, &goose_state.configuration);
                        }
                        else {
                            // First time seeing this request, simply insert it.
                            merged_request = request.clone();
                        }
                        goose_task_sets.weighted_clients[weighted_clients_index].requests.insert(request_key.to_string(), merged_request);
                    }
                    message = parent_receiver.try_recv();
                }
            }

            // All clients are done, exit out of loop for final cleanup.
            break;
        }

        // If enabled, display running statistics after sync
        if display_running_statistics {
            display_running_statistics = false;
            stats::print_running_stats(&goose_state.configuration, &goose_task_sets, started.elapsed().as_secs() as usize);
        }

        let one_second = time::Duration::from_secs(1);
        thread::sleep(one_second);
    }

    if goose_state.configuration.print_stats {
        stats::print_final_stats(&goose_state.configuration, &goose_task_sets, started.elapsed().as_secs() as usize);
    }
}
