extern crate structopt;

use std::path::PathBuf;

use structopt::StructOpt;

#[derive(StructOpt, Debug, Clone)]
#[structopt(name = "client")]
pub struct Configuration {
    /// Host to load test in the following format: http://10.21.32.33
    #[structopt(short = "H", long)]
    host: String,

    /// Rust module file to import, e.g. '../other.rs'.
    #[structopt(short = "f", long, default_value="goosefile")]
    goose_file: String,

    /// Number of concurrent Goose users.
    #[structopt(short, long, default_value="1")]
    clients: usize,

    /// The rate per second in which clients are spawned.
    #[structopt(short = "r", long, default_value="1")]
    hatch_rate: usize,

    /// Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.).
    #[structopt(short = "t", long)]
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
    #[structopt(short, long)]
    stop_timeout: usize,
}

// Attempt to locate a goosefile, either explicitly or by searching parent dirs.
fn find_goose_file() -> Option<PathBuf> {
    // @TODO: emulate how Locust does this
    //  - allow override in env
    //  - optionally append ".rs" ie "goosefile.rs"
    //  - search from current directory up
    //  - return None if no goosefile is found
    Some(PathBuf::from("goosefile"))
}

fn main() {
    let _configuration = Configuration::from_args();
    let _goose_file = find_goose_file();
}
