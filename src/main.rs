#[macro_use]
extern crate log;

//#[macro_use]
//extern crate goose_codegen;

extern crate structopt;

// @TODO: load goosefile as a dynamic library
mod goosefile;
mod util;

use std::ffi::OsStr;
use std::fs::File;
use std::path::PathBuf;
use std::time::Instant;
use std::{thread, time};

use num_format::{Locale, ToFormattedString};
use rand::thread_rng;
use rand::seq::SliceRandom;
use simplelog::*;
use structopt::StructOpt;

use goosefile::{GooseTaskSets, GooseTaskSet};

#[derive(Debug, Default, Clone)]
struct GooseState {
    configuration: Option<Configuration>,
    number_of_cpus: usize,
    max_clients: usize,
    active_clients: usize,
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

    /// Shows list of all possible Goose classes and exits
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

// Locate goosefile dynamic library
fn find_goosefile() -> Option<PathBuf> {
    let goosefile = PathBuf::from("goosefile.rs");
    trace!("goosefile: {:?}", goosefile);
    // @TODO: emulate how Locust does this
    //  - allow override in env
    //  - search from current directory up
    //  - return None if no goosefile is found
    Some(goosefile)
}

fn load_goosefile(goosefile: PathBuf) -> Result<GooseTaskSets, &'static str> {
    // @TODO: actually use _goosefile and load as dynamic library
    trace!("@TODO goosefile is currently hardcoded: {:?} ", goosefile);

    let mut goose_task_sets = GooseTaskSets::new();
    // @TODO: handle goosefile errors
    goose_task_sets.initialize_goosefile();
    Ok(goose_task_sets)
}

/// Returns a bucket of weighted Goose Task Sets, optionally shuffled
fn weight_task_sets(task_sets: &GooseTaskSets, shuffle: bool) -> Vec<usize> {
    trace!("weight_tasksets");

    let mut bucket;
    if task_sets.weighted_task_sets.len() > 0 {
        bucket = task_sets.weighted_task_sets.clone();
        trace!("re-using existing weighted_task_sets: {:?}", bucket);
    }
    else {
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
        // u will always be the greatest common divisor
        debug!("gcd: {}", u);

        bucket = Vec::new();
        for (index, task_set) in task_sets.task_sets.iter().enumerate() {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task_set.weight / u;
            trace!("{}: {} has weight of {} (reduced with gcd to {})", index, task_set.name, task_set.weight, weight);
            let mut task_sets = vec![index; weight];
            bucket.append(&mut task_sets);
        }
        trace!("created weighted_task_sets: {:?}", bucket);
    }

    if shuffle {
        bucket.shuffle(&mut thread_rng());
    }
    bucket
}

/// Returns a bucket of weighted Goose Tasks, optionally shuffled
fn weight_tasks(task_set: &GooseTaskSet, shuffle: bool) -> Vec<usize> {
    trace!("weight_tasks for {}", task_set.name);

    let mut bucket;
    if task_set.weighted_tasks.len() > 0 {
        bucket = task_set.weighted_tasks.clone();
        trace!("re-using existing weighted_tasks: {:?}", bucket);
    }
    else {
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
        // u will always be the greatest common divisor
        debug!("gcd: {}", u);

        bucket = Vec::new();
        for (index, task) in task_set.tasks.iter().enumerate() {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!("{}: {} has weight of {} (reduced with gcd to {})", index, task.name, task.weight, weight);
            let mut tasks = vec![index; weight];
            bucket.append(&mut tasks);
        }
        trace!("created weighted_tasks: {:?}", bucket);
    }
    if shuffle {
        bucket.shuffle(&mut thread_rng());
    }
    bucket
}

fn run_timer(started: Instant, run_time: usize, configuration: &Configuration, goose_task_sets: &GooseTaskSets) {
    if run_time > 0 && started.elapsed().as_secs() >= run_time as u64 {
        info!("exiting after {:?} seconds", run_time);
        if configuration.print_stats {
            display_stats(goose_task_sets);
        }
        std::process::exit(0);
    }
}

fn display_stats(goose_task_sets: &GooseTaskSets) {
    for task_set in &goose_task_sets.task_sets {
        println!("{}:", task_set.name);
        for task in &task_set.tasks {
            println!(" - {} ({} times)", task.name, task.counter.to_formatted_string(&Locale::en));
        }
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

    let run_time: usize;
    if configuration.run_time != "" {
        run_time = util::parse_timespan(&configuration.run_time);
    }
    else {
        run_time = 0;
    }
    info!("run_time = {}", run_time);

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
            info!("max concurrent clients defaulted to {} (number of CPUs)", c);
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
        println!("Available task sets:");
        for task_set in goose_task_sets.task_sets {
            println!(" - {} (weight: {})", task_set.name, task_set.weight);
            for task in task_set.tasks {
                println!("    o {} (weight: {})", task.name, task.weight);
            }
        }
        std::process::exit(0);
    }


    for task_set in &mut goose_task_sets.task_sets {
        task_set.weighted_tasks = weight_tasks(&task_set, true);
        debug!("weighted {} tasks: {:?}", task_set.name, task_set.weighted_tasks);
    }
    debug!("goose_task_sets: {:?}", goose_task_sets);

    // Weight and shuffle task sets
    goose_task_sets.weighted_task_sets = weight_task_sets(&goose_task_sets, true);
    let mut task_set_iter = goose_task_sets.weighted_task_sets.iter();
    let sleep_float = 1.0 / hatch_rate as f32;
    let sleep_duration = time::Duration::from_secs_f32(sleep_float);
    let started = Instant::now();
    let mut clients = vec![];
    while goose_state.active_clients < goose_state.max_clients {
        run_timer(started, run_time, &configuration, &goose_task_sets);
        let task_set = match task_set_iter.next() {
            Some(t) => t,
            // We reached the end of the iterator, so reshuffle and start over.
            None => {
                goose_task_sets.weighted_task_sets = weight_task_sets(&goose_task_sets, true);
                debug!("re-shuffled tasksets: {:?}", goose_task_sets.weighted_task_sets);
                task_set_iter = goose_task_sets.weighted_task_sets.iter();
                match task_set_iter.next() {
                    Some(t) => t,
                    // Goosefile has to have at least one TaskSet, so we can't get here.
                    None => unreachable!(),
                }
            }
        };
        goose_task_sets.task_sets[*task_set].counter += 1;
        // We can only run a task if the task list is non-empty
        if goose_task_sets.task_sets[*task_set].weighted_tasks.len() > 0 {
            // @TODO: track counters from threads
            //goose_task_sets.task_sets[*task_set].tasks[weighted_task].counter += 1;
            let thread_task_set = goose_task_sets.task_sets[*task_set].clone();
            // @TODO: gracefully join/exit children

            // Launch a new client
            let client = thread::spawn(move || {
                info!("launching {} client...", thread_task_set.name);
                let mut thread_weighted_tasks = weight_tasks(&thread_task_set, true);
                let mut thread_weighted_position = 0;
                loop {
                    if thread_task_set.tasks.len() <= thread_weighted_position {
                        thread_weighted_tasks = weight_tasks(&thread_task_set, true);
                        debug!("re-shuffled {} tasks: {:?}", thread_task_set.name, thread_weighted_tasks);
                        thread_weighted_position = 0;
                    }
                    let thread_weighted_task = thread_weighted_tasks[thread_weighted_position];
                    debug!("launching {} task from {}", thread_task_set.tasks[thread_weighted_task].name, thread_task_set.name);
                    thread_weighted_position += 1;
                    // @TODO: delay
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
        run_timer(started, run_time, &configuration, &goose_task_sets);
        let one_second = time::Duration::from_secs(1);
        thread::sleep(one_second);
    }
}
