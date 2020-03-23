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

use simplelog::*;
use structopt::StructOpt;

use goosefile::{GooseTaskSets, GooseTaskSet, GooseTask};

pub trait TaskSet {
    fn tasksets();
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(name = "client")]
pub struct Configuration {
    /// Host to load test in the following format: http://10.21.32.33
    #[structopt(short = "H", long, required=false, default_value="")]
    host: String,

    /// Rust module file to import, e.g. '../other.rs'.
    //#[structopt(short = "f", long, default_value="goosefile")]
    //goosefile: String,

    /// Number of concurrent Goose users.
    #[structopt(short, long, default_value="1")]
    clients: usize,

    /// The rate per second in which clients are spawned.
    #[structopt(short = "r", long, default_value="1")]
    hatch_rate: usize,

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

    /// Number of seconds to wait for a simulated user to complete any executing task before existing. Default is to terminate immediately.
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
    // @TODO: emulate how Locust does this
    //  - allow override in env
    //  - search from current directory up
    //  - return None if no goosefile is found
    Some(PathBuf::from("goosefile.rs"))
}

/// @TODO Load goosefile dynamic library and extract geese
//fn load_goosefile(_goosefile: PathBuf) -> Option<Vec<String>> {
//    Some(vec!["@TODO".to_string()])
//}

fn main() {
    let configuration = Configuration::from_args();

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

    let log_file = PathBuf::from(configuration.log_file);

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

    // Initialize empty vector to track all task sets
    let mut goose_tasksets = GooseTaskSets::new();

    // @TODO: creation of GooseTaskSets should be completely accomplished
    // in the goosefile.

    // Register a website task set and contained tasks
    let mut website_tasks = GooseTaskSet::new("WebsiteTasks");
    website_tasks.register_task(GooseTask::new("on_start"));
    website_tasks.register_task(GooseTask::new("index"));
    website_tasks.register_task(GooseTask::new("about"));
    goose_tasksets.register_taskset(website_tasks);

    // Register an API task set and contained tasks
    let mut api_tasks = GooseTaskSet::new("APITasks");
    api_tasks.register_task(GooseTask::new("on_start"));
    api_tasks.register_task(GooseTask::new("listing"));
    goose_tasksets.register_taskset(api_tasks);

    debug!("goose_tasksets: {:?}", goose_tasksets);

    //let geese = match load_goosefile(goosefile) {
    //    Some(g) => g,
    //    None => {
    //        error!("No geese found in the goosefile! Please create a test plan and try again.");
    //        std::process::exit(1);
    //    }
    //};

    if configuration.list {
        println!("Available task sets:");
        for task_set in goose_tasksets.task_sets {
            println!(" - {}", task_set.name);
        }
        std::process::exit(0);
    }
}
