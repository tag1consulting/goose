extern crate structopt;

// @TODO: load this as a dynamic library
mod goosefile;
mod util;

use std::ffi::OsStr;
use std::path::PathBuf;

use structopt::StructOpt;

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
}

// Locate goosefile dynamic library
fn find_goosefile() -> Option<PathBuf> {
    // @TODO: emulate how Locust does this
    //  - allow override in env
    //  - search from current directory up
    //  - return None if no goosefile is found
    Some(PathBuf::from("goosefile.rs"))
}

// Load goosefile dynamic library and extract geese
fn load_goosefile(_goosefile: PathBuf) -> Option<Vec<String>> {
    Some(vec!["@TODO".to_string()])
}

fn main() {
    let configuration = Configuration::from_args();

    // @TODO: Logger

    let goosefile = match find_goosefile() {
        Some(g) => g,
        None => {
            eprintln!("Could not find any goosefile! Ensure file ends with '.rs' and see --help for availble options.");
            std::process::exit(1);
        }
    };

    if goosefile.file_name() == Some(OsStr::new("goose.rs")) {
        eprintln!("The goosfile must not be named `goose.rs`. Please rename the file and try again.");
        std::process::exit(1);
    }

    let geese = match load_goosefile(goosefile) {
        Some(g) => g,
        None => {
            eprintln!("No geese found in the goosefile! Please create a test plan and try again.");
            std::process::exit(1);
        }
    };

    if configuration.list {
        println!("Available Geese:");
        for goose in geese {
            println!(" - {}", goose);
        }
        std::process::exit(0);
    }

    let run_time: usize;
    if configuration.run_time != "" {
        run_time = util::parse_timespan(&configuration.run_time);
    }
    else {
        run_time = 0;
    }
    println!("run_time = {}", run_time);
}
