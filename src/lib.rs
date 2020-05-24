//! # Goose
//! 
//! Have you ever been attacked by a goose?
//! 
//! Goose is a load testing tool inspired by [Locust](https://locust.io/).
//! User behavior is defined with standard Rust code.
//! 
//! Goose load tests, called Goose Attacks, are built by creating an application
//! with Cargo, and declaring a dependency on the Goose library.
//! 
//! Goose uses the [`reqwest::blocking`](https://docs.rs/reqwest/*/reqwest/blocking/)
//! API to provide a convenient HTTP client. (Async support is on the roadmap, also
//! provided through the `reqwest` library.)
//! 
//! ## Creating and running a Goose load test
//! 
//! ### Creating a simple Goose load test
//! 
//! First create a new empty cargo application, for example:
//! 
//! ```bash
//! $ cargo new loadtest
//!      Created binary (application) `loadtest` package
//! $ cd loadtest/
//! ```
//! 
//! Add Goose as a dependency in `Cargo.toml`:
//! 
//! ```toml
//! [dependencies]
//! goose = "0.6"
//! ```
//! 
//! Add the following boilerplate use declarations at the top of your `src/main.rs`:
//! 
//! ```rust
//! use goose::GooseAttack;
//! use goose::goose::{GooseTaskSet, GooseClient, GooseTask};
//! ```
//! 
//! Below your `main` function (which currently is the default `Hello, world!`), add
//! one or more load test functions. The names of these functions are arbitrary, but it is
//! recommended you use self-documenting names. Each load test function must accept a mutable
//! GooseClient pointer. For example:
//! 
//! ```rust
//! use goose::goose::GooseClient;
//!
//! fn loadtest_foo(client: &mut GooseClient) {
//!   let _response = client.get("/path/to/foo");
//! }   
//! ```
//! 
//! In the above example, we're using the GooseClient helper method `get` to load a path
//! on the website we are load testing. This helper creates a Reqwest request builder, and
//! uses it to build and execute a request for the above path. If you want access to the
//! request builder object, you can instead use the `goose_get` helper, for example to
//! set a timout on this specific request:
//! 
//! ```rust
//! use std::time;
//!
//! use goose::goose::GooseClient;
//! 
//! fn loadtest_bar(client: &mut GooseClient) {
//!   let request_builder = client.goose_get("/path/to/bar");
//!   let _response = client.goose_send(request_builder.timeout(time::Duration::from_secs(3)));
//! }   
//! ```
//! 
//! We pass the `request_builder` object to `goose_send` which builds and executes it, also
//! collecting useful statistics.
//! 
//! Once all our tasks are created, we edit the main function to initialize goose and register
//! the tasks. In this very simple example we only have two tasks to register, while in a real
//! load test you can have any number of task sets with any number of individual tasks.
//! 
//! ```goose
//! fn main() {
//!     GooseAttack::initialize()
//!         .register_taskset(GooseTaskSet::new("LoadtestTasks")
//!             .set_wait_time(0, 3)
//!             // Register the foo task, assigning it a weight of 10.
//!             .register_task(GooseTask::new(loadtest_foo).set_weight(10))
//!             // Register the bar task, assigning it a weight of 2 (so it
//!             // runs 1/5 as often as bar). Apply a task name which shows up
//!             // in statistics.
//!             .register_task(GooseTask::new(loadtest_bar).set_name("bar").set_weight(2))
//!         )
//!         // You could also set a default host here, for example:
//!         //.set_host("http://dev.local/")
//!         .execute();
//! }
//! ```
//! 
//! Goose now spins up a configurable number of clients, each simulating a user on your
//! website. Thanks to Reqwest, each user maintains its own client state, handling cookies
//! and more so your "users" can log in, fill out forms, and more, as real users on your
//! sites would do.
//! 
//! ### Running the Goose load test
//! 
//! Attempts to run our example will result in an error, as we have not yet defined the
//! host against which this load test should be run. We intentionally do not hard code the
//! host in the individual tasks, as this allows us to run the test against different
//! environments, such as local and staging.
//! 
//! ```bash
//! $ cargo run --release
//!    Compiling loadtest v0.1.0 (~/loadtest)
//!     Finished release [optimized] target(s) in 1.52s
//!      Running `target/release/loadtest`
//! 05:33:06 [ERROR] Host must be defined globally or per-TaskSet. No host defined for LoadtestTasks.
//! ```
//! Pass in the `-h` flag to see all available run-time options. For now, we'll use a few
//! options to customize our load test.
//! 
//! ```bash
//! $ cargo run --release -- --host http://dev.local -t 30s -v
//! ```
//! 
//! The first option we specified is `--host`, and in this case tells Goose to run the load test
//! against an 8-core VM on my local network. The `-t 30s` option tells Goose to end the load test
//! after 30 seconds (for real load tests you'll certainly want to run it longer, you can use `m` to
//! specify minutes and `h` to specify hours. For example, `-t 1h30m` would run the load test for 1
//! hour 30 minutes). Finally, the `-v` flag tells goose to display INFO and higher level logs to
//! stdout, giving more insight into what is happening. (Additional `-v` flags will result in
//! considerably more debug output, and are not recommended for running actual load tests; they're
//! only useful if you're trying to debug Goose itself.)
//! 
//! Running the test results in the following output (broken up to explain it as it goes):
//! 
//! ```bash
//!    Finished release [optimized] target(s) in 0.05s
//!     Running `target/release/loadtest --host 'http://dev.local' -t 30s -v`
//! 05:56:30 [ INFO] Output verbosity level: INFO
//! 05:56:30 [ INFO] Logfile verbosity level: INFO
//! 05:56:30 [ INFO] Writing to log file: goose.log

//! ```
//! 
//! By default Goose will write a log file with INFO and higher level logs into the same directory
//! as you run the test from.
//! 
//! ```bash
//! 05:56:30 [ INFO] run_time = 30
//! 05:56:30 [ INFO] concurrent clients defaulted to 8 (number of CPUs)
//! ```
//! 
//! Goose will default to launching 1 client per available CPU core, and will launch them all in
//! one second. You can change how many clients are launched with the `-c` option, and you can
//! change how many clients are launched per second with the `-r` option. For example, `-c 30 -r 2`
//! would launch 30 clients over 15 seconds, or two clients per second.
//! 
//! ```bash
//! 05:56:30 [ INFO] global host configured: http://dev.local
//! 05:56:30 [ INFO] launching client 1 from LoadtestTasks...
//! 05:56:30 [ INFO] launching client 2 from LoadtestTasks...
//! 05:56:30 [ INFO] launching client 3 from LoadtestTasks...
//! 05:56:30 [ INFO] launching client 4 from LoadtestTasks...
//! 05:56:30 [ INFO] launching client 5 from LoadtestTasks...
//! 05:56:30 [ INFO] launching client 6 from LoadtestTasks...
//! 05:56:30 [ INFO] launching client 7 from LoadtestTasks...
//! 05:56:31 [ INFO] launching client 8 from LoadtestTasks...
//! 05:56:31 [ INFO] launched 8 clients...
//! ```
//! 
//! Each client is launched in its own thread with its own client state. Goose is able to make
//! very efficient use of server resources.
//! 
//! ```bash
//! 05:56:46 [ INFO] printing running statistics after 15 seconds...
//! ------------------------------------------------------------------------------ 
//!  Name                    | # reqs         | # fails        | req/s  | fail/s
//!  ----------------------------------------------------------------------------- 
//!  GET /path/to/foo        | 15,795         | 0 (0%)         | 1,053  | 0    
//!  GET bar                 | 3,161          | 0 (0%)         | 210    | 0    
//!  ------------------------+----------------+----------------+--------+--------- 
//!  Aggregated              | 18,956         | 0 (0%)         | 1,263  | 0    
//! ------------------------------------------------------------------------------
//! ```
//! 
//! When printing statistics, by default Goose will display running values approximately
//! every 15 seconds. Running statistics are broken into two tables. The first, above,
//! shows how many requests have been made, how many of them failed (non-2xx response),
//! and the corresponding per-second rates.
//! 
//! Note that Goose respected the per-task weights we set, and `foo` (with a weight of
//! 10) is being loaded five times as often as `bar` (with a weight of 2). Also notice
//! that because we didn't name the `foo` task by default we see the URL loaded in the
//! statistics, whereas we did name the `bar` task so we see the name in the statistics.
//! 
//! ```bash
//!  Name                    | Avg (ms)   | Min        | Max        | Mean      
//!  ----------------------------------------------------------------------------- 
//!  GET /path/to/foo        | 0.67       | 0.31       | 13.51      | 0.53      
//!  GET bar                 | 0.60       | 0.33       | 13.42      | 0.53      
//!  ------------------------+------------+------------+------------+------------- 
//!  Aggregated              | 0.66       | 0.31       | 13.51      | 0.56      
//! ```
//! 
//! The second table in running statistics provides details on response times. In our
//! example (which is running over wifi from my development laptop), on average each
//! page is returning within `0.66` milliseconds. The quickest page response was for 
//! `foo` within `0.31` milliseconds. The slowest page response was also for `foo` within
//! `13.51` milliseconds.
//! 
//! 
//! ```bash
//! 05:37:10 [ INFO] stopping after 30 seconds...
//! 05:37:10 [ INFO] waiting for clients to exit
//! ```
//! 
//! Our example only runs for 30 seconds, so we only see running statistics once. When
//! the test completes, we get more detail in the final summary. The first two tables
//! are the same as what we saw earlier, however now they include all statistics for the
//! entire load test:
//! 
//! ```bash
//! ------------------------------------------------------------------------------ 
//!  Name                    | # reqs         | # fails        | req/s  | fail/s
//!  ----------------------------------------------------------------------------- 
//!  GET bar                 | 6,050          | 0 (0%)         | 201    | 0    
//!  GET /path/to/foo        | 30,257         | 0 (0%)         | 1,008  | 0    
//!  ------------------------+----------------+----------------+--------+---------- 
//!  Aggregated              | 36,307         | 0 (0%)         | 1,210  | 0    
//! -------------------------------------------------------------------------------
//!  Name                    | Avg (ms)   | Min        | Max        | Mean      
//!  ----------------------------------------------------------------------------- 
//!  GET bar                 | 0.66       | 0.32       | 108.87     | 0.53      
//!  GET /path/to/foo        | 0.68       | 0.31       | 109.50     | 0.53      
//!  ------------------------+------------+------------+------------+------------- 
//!  Aggregated              | 0.67       | 0.31       | 109.50     | 0.50      
//! -------------------------------------------------------------------------------
//! ```
//! 
//! The ratio between `foo` and `bar` remained 5:2 as expected. As the test ran,
//! however, we saw some slower page loads, with the slowest again `foo` this time
//! at `109.50` milliseconds.
//! 
//! ```bash
//! Slowest page load within specified percentile of requests (in ms):
//! ------------------------------------------------------------------------------
//! Name                    | 50%    | 75%    | 98%    | 99%    | 99.9%  | 99.99%
//! ----------------------------------------------------------------------------- 
//! GET bar                 | 0.53   | 0.66   | 2.17   | 5.37   | 18.72  | 123.16
//! GET /path/to/foo        | 0.53   | 0.66   | 2.65   | 10.60  | 18.00  | 107.32
//! ------------------------+------------+------------+------------+------------- 
//! Aggregated              | 0.53   | 0.66   | 2.37   | 6.45   | 18.32  | 108.18
//! ```
//! 
//! A new table shows additional information, breaking down response-time by
//! percentile. This shows that the slowest page loads only happened in the
//! slowest .001% of page loads, so were very much an edge case. 99.9% of the time
//! page loads happened in less than 20 milliseconds.
//! 
//! ## License
//! 
//! Copyright 2020 Jeremy Andrews
//! 
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//! 
//! http://www.apache.org/licenses/LICENSE-2.0
//! 
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

#[macro_use]
extern crate log;

//#[macro_use]
//extern crate goose_codegen;

extern crate structopt;

pub mod goose;

mod client;
#[cfg(feature = "gaggle")]
mod manager;
mod stats;
mod util;
#[cfg(feature = "gaggle")]
mod worker;

use std::collections::{BTreeMap, HashMap};
use std::collections::hash_map::DefaultHasher;
use std::f32;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, mpsc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{thread, time};

use lazy_static::lazy_static;
#[cfg(feature = "gaggle")]
use nng::Socket;
use rand::thread_rng;
use rand::seq::SliceRandom;
use serde::{Serialize, Deserialize};
use simplelog::*;
use structopt::StructOpt;
use url::Url;

use crate::goose::{GooseTaskSet, GooseTask, GooseClient, GooseClientMode, GooseClientCommand, GooseRequest};

/// Constant defining how often statistics should be displayed while load test is running.
const RUNNING_STATS_EVERY: usize = 15;

/// Constant defining Goose's default port when running a Gaggle.
const DEFAULT_PORT: &str = "5115";

// WORKER_ID is only used when running a gaggle (a distributed load test).
lazy_static! {
    static ref WORKER_ID: Mutex<AtomicUsize> = Mutex::new(AtomicUsize::new(0));
}

/// Worker ID to aid in tracing logs when running a Gaggle.
pub fn get_worker_id() -> usize {
    WORKER_ID.lock().unwrap().load(Ordering::Relaxed)
}

#[cfg(not(feature = "gaggle"))]
#[derive(Debug)]
/// Socket used for coordinating a Gaggle, a distributed load test.
pub struct Socket {}

/// Internal global state for load test.
#[derive(Clone)]
pub struct GooseAttack {
    /// A vector containing one copy of each GooseTaskSet that will run during this load test.
    task_sets: Vec<GooseTaskSet>,
    /// A checksum of the task_sets vector to be sure all workers are running the same load test.
    task_sets_hash: u64,
    /// A weighted vector containing a GooseClient object for each client that will run during this load test.
    weighted_clients: Vec<GooseClient>,
    /// An optional default host to run this load test against.
    host: Option<String>,
    /// Configuration object managed by StructOpt.
    configuration: GooseConfiguration,
    /// By default launch 1 client per number of CPUs.
    number_of_cpus: usize,
    /// Track how long the load test should run.
    run_time: usize,
    /// Track total number of clients to run for this load test.
    clients: usize,
    /// Track how many clients are already loaded.
    active_clients: usize,
    /// All requests statistics merged together.
    merged_requests: HashMap<String, GooseRequest>,
}
/// Goose's internal global state.
impl GooseAttack {
    /// Load configuration from command line and initialize a GooseAttack.
    /// 
    /// # Example
    /// ```rust,no_run
    ///     use goose::GooseAttack;
    ///
    ///     let mut goose_attack = GooseAttack::initialize();
    /// ```
    pub fn initialize() -> GooseAttack {
        let goose_attack = GooseAttack {
            task_sets: Vec::new(),
            task_sets_hash: 0,
            weighted_clients: Vec::new(),
            host: None,
            configuration: GooseConfiguration::from_args(),
            number_of_cpus: num_cpus::get(),
            run_time: 0,
            clients: 0,
            active_clients: 0,
            merged_requests: HashMap::new(),
        };
        goose_attack.setup()
    }

    /// Initialize a GooseAttack with an already loaded configuration.
    /// This should only be called by worker instances.
    /// 
    /// # Example
    /// ```rust,no_run
    ///     use goose::{GooseAttack, GooseConfiguration};
    ///     use structopt::StructOpt;
    ///
    ///     let configuration = GooseConfiguration::from_args();
    ///     let mut goose_attack = GooseAttack::initialize_with_config(configuration);
    /// ```
    pub fn initialize_with_config(config: GooseConfiguration) -> GooseAttack {
        GooseAttack {
            task_sets: Vec::new(),
            task_sets_hash: 0,
            weighted_clients: Vec::new(),
            host: None,
            configuration: config,
            number_of_cpus: num_cpus::get(),
            run_time: 0,
            clients: 0,
            active_clients: 0,
            merged_requests: HashMap::new(),
        }
    }
    
    pub fn setup(mut self) -> Self {
        // Allow optionally controlling debug output level
        let debug_level;
        match self.configuration.verbose {
            0 => debug_level = LevelFilter::Warn,
            1 => debug_level = LevelFilter::Info,
            2 => debug_level = LevelFilter::Debug,
            _ => debug_level = LevelFilter::Trace,
        }

        // Allow optionally controlling log level
        let log_level;
        match self.configuration.log_level {
            0 => log_level = LevelFilter::Info,
            1 => log_level = LevelFilter::Debug,
            _ => log_level = LevelFilter::Trace,
        }

        let log_file = PathBuf::from(&self.configuration.log_file);

        // @TODO: get rid of unwrap(), TermLogger fails if there's no terminal.
        match CombinedLogger::init(vec![
            match TermLogger::new(
                debug_level,
                Config::default(),
                TerminalMode::Mixed) {
                    Some(t) => t,
                    None => {
                        error!("failed to initialize TermLogger");
                        std::process::exit(1);
                    }
                },
            WriteLogger::new(
                log_level,
                Config::default(),
                File::create(&log_file).unwrap(),
            )]) {
                Ok(_) => (),
                Err(e) => {
                    error!("failed to initialize CombinedLogger: {}", e);
                }
            }
        info!("Output verbosity level: {}", debug_level);
        info!("Logfile verbosity level: {}", log_level);
        info!("Writing to log file: {}", log_file.display());

        // Don't allow overhead of collecting status codes unless we're printing statistics.
        if self.configuration.status_codes && self.configuration.no_stats {
            error!("You must not enable --no-stats when enabling --status-codes.");
            std::process::exit(1);
        }

        // Don't allow overhead of collecting statistics unless we're printing them.
        if self.configuration.only_summary && self.configuration.no_stats {
            error!("You must not enable --no-stats when enabling --only-summary.");
            std::process::exit(1);
        }

        // Configure maximum run time if specified, otherwise run until canceled.
        if self.configuration.worker {
            if self.configuration.run_time != "" {
                error!("The --run-time option is only available to the manager.");
                std::process::exit(1);
            }
            self.run_time = 0;
        }
        else {
            if self.configuration.run_time != "" {
                self.run_time = util::parse_timespan(&self.configuration.run_time);
                info!("run_time = {}", self.run_time);
            }
            else {
                self.run_time = 0;
            }
        }

        // Configure number of client threads to launch, default to the number of CPU cores available.
        self.clients = match self.configuration.clients {
            Some(c) => {
                if c == 0 {
                    if self.configuration.worker {
                        error!("At least 1 client is required.");
                        std::process::exit(1);
                    }
                    else {
                        0
                    }
                }
                else {
                    if self.configuration.worker {
                        error!("The --clients option is only available to the manager.");
                        std::process::exit(1);
                    }
                    c
                }
            }
            None => {
                let c = self.number_of_cpus;
                if !self.configuration.manager && !self.configuration.worker {
                    info!("concurrent clients defaulted to {} (number of CPUs)", c);
                }
                c
            }
        };

        if !self.configuration.manager && !self.configuration.worker {
            debug!("clients = {}", self.clients);
        }

        self
    }

    /// A load test must contain one or more `GooseTaskSet`s. Each task set must
    /// be registered into Goose's global state with this method for it to run.
    /// 
    /// # Example
    /// ```rust,no_run
    ///     use goose::GooseAttack;
    ///     use goose::goose::{GooseTaskSet, GooseTask, GooseClient};
    ///
    ///     GooseAttack::initialize()
    ///         .register_taskset(GooseTaskSet::new("ExampleTasks")
    ///             .register_task(GooseTask::new(example_task))
    ///         )
    ///         .register_taskset(GooseTaskSet::new("OtherTasks")
    ///             .register_task(GooseTask::new(other_task))
    ///         );
    ///
    ///     fn example_task(client: &mut GooseClient) {
    ///       let _response = client.get("/foo");
    ///     }
    ///
    ///     fn other_task(client: &mut GooseClient) {
    ///       let _response = client.get("/bar");
    ///     }
    /// ```
    pub fn register_taskset(mut self, mut taskset: GooseTaskSet) -> Self {
        taskset.task_sets_index = self.task_sets.len();
        self.task_sets.push(taskset);
        self
    }

    /// Optionally configure a default host for the load test. This is used if
    /// no per-GooseTaskSet host is defined, no `--host` CLI option is configurared,
    /// and if the GooseTask itself doesn't hard-code the host in its request. The
    /// host is prepended on all requests.
    /// 
    /// For example, your load test may default to running against your local development
    /// container, and the `--host` option could be used to override host to run the load
    /// test against production.
    /// 
    /// # Example
    /// ```rust,no_run
    ///     use goose::GooseAttack;
    ///
    ///     GooseAttack::initialize()
    ///         .set_host("local.dev");
    /// ```
    pub fn set_host(mut self, host: &str) -> Self {
        trace!("set_host: {}", host);
        // Host validation happens in main() at startup.
        self.host = Some(host.to_string());
        self
    }

    /// Allocate a vector of weighted GooseClient.
    fn weight_task_set_clients(&mut self) -> Vec<GooseClient> {
        trace!("weight_task_set_clients");

        let mut u: usize = 0;
        let mut v: usize;
        for task_set in &self.task_sets {
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
        for (index, task_set) in self.task_sets.iter().enumerate() {
            // divide by greatest common divisor so vector is as short as possible
            let weight = task_set.weight / u;
            trace!("{}: {} has weight of {} (reduced with gcd to {})", index, task_set.name, task_set.weight, weight);
            let mut weighted_sets = vec![index; weight];
            weighted_task_sets.append(&mut weighted_sets);
        }
        // Shuffle the weighted list of task sets
        weighted_task_sets.shuffle(&mut thread_rng());

        // Allocate a state for each client that will be spawned.
        info!("initializing client states...");
        let mut weighted_clients = Vec::new();
        let mut client_count = 0;
        let config = self.configuration.clone();
        loop {
            for task_sets_index in &weighted_task_sets {
                let task_set_host = self.task_sets[*task_sets_index].host.clone();
                weighted_clients.push(GooseClient::new(
                    client_count,
                    self.task_sets[*task_sets_index].task_sets_index,
                    self.host.clone(),
                    task_set_host,
                    self.task_sets[*task_sets_index].min_wait,
                    self.task_sets[*task_sets_index].max_wait,
                    &config,
                    self.task_sets_hash,
                ));
                client_count += 1;
                if client_count >= self.clients {
                    trace!("created {} weighted_clients", client_count);
                    return weighted_clients;
                }
            }
        }
    }

    /// Execute the load test.
    /// 
    /// # Example
    /// ```rust,no_run
    ///     use goose::GooseAttack;
    ///     use goose::goose::{GooseTaskSet, GooseTask, GooseClient};
    ///
    ///     GooseAttack::initialize()
    ///         .register_taskset(GooseTaskSet::new("ExampleTasks")
    ///             .register_task(GooseTask::new(example_task).set_weight(2))
    ///             .register_task(GooseTask::new(another_example_task).set_weight(3))
    ///         )
    ///         .execute();
    ///
    ///     fn example_task(client: &mut GooseClient) {
    ///       let _response = client.get("/foo");
    ///     }
    ///
    ///     fn another_example_task(client: &mut GooseClient) {
    ///       let _response = client.get("/bar");
    ///     }
    /// ```
    pub fn execute(mut self) {
        // At least one task set is required.
        if self.task_sets.len() <= 0 {
            error!("No task sets defined.");
            std::process::exit(1);
        }

        if self.configuration.list {
            // Display task sets and tasks, then exit.
            println!("Available tasks:");
            for task_set in self.task_sets {
                println!(" - {} (weight: {})", task_set.name, task_set.weight);
                for task in task_set.tasks {
                    println!("    o {} (weight: {})", task.name, task.weight);
                }
            }
            std::process::exit(0);
        }

        // Manager mode.
        if self.configuration.manager {
            // @TODO: support running in both manager and worker mode.
            if self.configuration.worker {
                error!("You can only run in manager or worker mode, not both.");
                std::process::exit(1);
            }

            if self.configuration.expect_workers < 1 {
                error!("You must set --expect-workers to 1 or more.");
                std::process::exit(1);
            }
            if self.configuration.expect_workers as usize > self.clients {
                error!("You must enable at least as many clients ({}) as workers ({}).",
                    self.clients, self.configuration.expect_workers);
                std::process::exit(1);
            }

        }
        
        // Worker mode.
        if self.configuration.worker {
            // @TODO: support running in both manager and worker mode.
            if self.configuration.manager {
                error!("You can only run in manager or worker mode, not both.");
                std::process::exit(1);
            }

            if self.configuration.expect_workers > 0 {
                error!("The --expect-workers option is only available to the manager");
                std::process::exit(1);
            }

            if self.configuration.host != "" {
                error!("The --host option is only available to the manager");
                std::process::exit(1);
            }

            if self.configuration.manager_bind_host != "0.0.0.0" {
                error!("The --manager-bind-host option is only available to the manager");
                std::process::exit(1);
            }

            let default_port: u16 = DEFAULT_PORT.to_string().parse().unwrap();
            if self.configuration.manager_bind_port != default_port {
                error!("The --manager-bind-port option is only available to the manager");
                std::process::exit(1);
            }

            if self.configuration.no_stats {
                error!("The --no-stats option is only available to the manager");
                std::process::exit(1);
            }

            if self.configuration.only_summary {
                error!("The --only-summary option is only available to the manager");
                std::process::exit(1);
            }

            if self.configuration.status_codes {
                error!("The --status-codes option is only available to the manager");
                std::process::exit(1);
            }

            if self.configuration.no_hash_check {
                error!("The --no-hash-check option is only available to the manager");
                std::process::exit(1);
            }
        }

        if !self.configuration.manager && !self.configuration.worker {
            if self.configuration.no_hash_check {
                error!("The --no-hash-check option is only available when running in manager mode");
                std::process::exit(1);
            }
        }

        // Configure number of client threads to launch per second, defaults to 1.
        let hatch_rate = self.configuration.hatch_rate;
        if hatch_rate < 1 {
            error!("Hatch rate must be greater than 0, or no clients will launch.");
            std::process::exit(1);
        }
        if hatch_rate > 1 && self.configuration.worker {
            error!("The --hatch-rate option is only available to the manager");
            std::process::exit(1);
        }
        debug!("hatch_rate = {}", hatch_rate);

        // Confirm there's either a global host, or each task set has a host defined.
        if self.configuration.host.len() == 0 {
            for task_set in &self.task_sets {
                match &task_set.host {
                    Some(h) => {
                        if is_valid_host(h) {
                            info!("host for {} configured: {}", task_set.name, h);
                        }
                    }
                    None => {
                        match &self.host {
                            Some(h) => {
                                if is_valid_host(h) {
                                    info!("host for {} configured: {}", task_set.name, h);
                                }
                            }
                            None => {
                                if ! self.configuration.worker {
                                    error!("Host must be defined globally or per-TaskSet. No host defined for {}.", task_set.name);
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                }
            }
        }
        else {
            if is_valid_host(&self.configuration.host) {
                info!("global host configured: {}", self.configuration.host);
            }
        }

        // Apply weights to tasks in each task set.
        for task_set in &mut self.task_sets {
            let (weighted_on_start_tasks, weighted_tasks, weighted_on_stop_tasks) = weight_tasks(&task_set);
            task_set.weighted_on_start_tasks = weighted_on_start_tasks;
            task_set.weighted_tasks = weighted_tasks;
            task_set.weighted_on_stop_tasks = weighted_on_stop_tasks;
            debug!("weighted {} on_start: {:?} tasks: {:?} on_stop: {:?}", task_set.name, task_set.weighted_on_start_tasks, task_set.weighted_tasks, task_set.weighted_on_stop_tasks);
        }

        // Allocate a state for each of the clients we are about to start.
        if !self.configuration.worker {
            self.weighted_clients = self.weight_task_set_clients();
        }

        // Calculate a unique hash for the current load test.
        let mut s = DefaultHasher::new();
        self.task_sets.hash(&mut s);
        self.task_sets_hash = s.finish();
        debug!("task_sets_hash: {}", self.task_sets_hash);

        // Our load test is officially starting.
        let started = time::Instant::now();
        // Spawn clients at hatch_rate per second, or one every 1 / hatch_rate fraction of a second.
        let sleep_float = 1.0 / hatch_rate as f32;
        let sleep_duration = time::Duration::from_secs_f32(sleep_float);

        // Start goose in manager mode.
        if self.configuration.manager {
            #[cfg(feature = "gaggle")]
            {
                self = manager::manager_main(self);
            }

            #[cfg(not(feature = "gaggle"))]
            {
                error!("goose must be recompiled with `--features gaggle` to start in manager mode");
                std::process::exit(1);
            }
        }
        // Start goose in worker mode.
        else if self.configuration.worker {
            #[cfg(feature = "gaggle")]
            {
                worker::worker_main(&self);
            }

            #[cfg(not(feature = "gaggle"))]
            {
                error!("goose must be recompiled with `--features gaggle` to start in worker mode");
                std::process::exit(1);
            }
        }
        // Start goose in single-process mode.
        else {
            let mut rt = tokio::runtime::Runtime::new().unwrap();
            self = rt.block_on(self.launch_clients(started, sleep_duration, None));
        }

        if !self.configuration.no_stats && !self.configuration.worker {
            stats::print_final_stats(&self, started.elapsed().as_secs() as usize);
        }
    }

    /// Called internally in local-mode and gaggle-mode.
    async fn launch_clients(mut self, mut started: time::Instant, sleep_duration: time::Duration, socket: Option<Socket>) -> GooseAttack {
        trace!("launch clients: started({:?}) sleep_duration({:?}) socket({:?})", started, sleep_duration, socket);
        // Collect client threads in a vector for when we want to stop them later.
        let mut clients = vec![];
        // Collect client thread channels in a vector so we can talk to the client threads.
        let mut client_channels = vec![];
        // Create a single channel allowing all Goose child threads to sync state back to parent
        let (all_threads_sender, parent_receiver): (mpsc::Sender<GooseClient>, mpsc::Receiver<GooseClient>) = mpsc::channel();
        // Spawn clients, each with their own weighted task_set.
        for mut thread_client in self.weighted_clients.clone() {
            // Stop launching threads if the run_timer has expired.
            if util::timer_expired(started, self.run_time) {
                break;
            }

            // Copy weighted tasks and weighted on start tasks into the client thread.
            thread_client.weighted_tasks = self.task_sets[thread_client.task_sets_index].weighted_tasks.clone();
            thread_client.weighted_on_start_tasks = self.task_sets[thread_client.task_sets_index].weighted_on_start_tasks.clone();
            thread_client.weighted_on_stop_tasks = self.task_sets[thread_client.task_sets_index].weighted_on_stop_tasks.clone();
            // Remember which task group this client is using.
            thread_client.weighted_clients_index = self.active_clients;

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
                let thread_task_set = self.task_sets[thread_client.task_sets_index].clone();

                // We number threads from 1 as they're human-visible (in the logs), whereas active_clients starts at 0.
                let thread_number = self.active_clients + 1;

                let is_worker = self.configuration.worker;

                // Launch a new client.
                let client = tokio::spawn(
                    client::client_main(thread_number, thread_task_set, thread_client, thread_receiver, thread_sender, is_worker)
                );

                clients.push(client);
                self.active_clients += 1;
                debug!("sleeping {:?} milliseconds...", sleep_duration);
                thread::sleep(sleep_duration);
            }
            else {
                warn!("no tasks for thread {} to run", self.task_sets[thread_client.task_sets_index].name);
            }
        }
        // Restart the timer now that all threads are launched.
        started = time::Instant::now();
        if self.configuration.worker {
            info!("[{}] launched {} clients...", get_worker_id(), self.active_clients);
        }
        else {
            info!("launched {} clients...", self.active_clients);
        }

        // Ensure we have request statistics when we're displaying running statistics.
        if !self.configuration.no_stats && !self.configuration.only_summary {
            for (index, send_to_client) in client_channels.iter().enumerate() {
                match send_to_client.send(GooseClientCommand::SYNC) {
                    Ok(_) => {
                        debug!("telling client {} to sync stats", index);
                    }
                    Err(e) => {
                        warn!("failed to tell client {} to sync stats: {}", index, e);
                    }
                }
            }
        }

        // Track whether or not we've (optionally) reset the statistics after all clients started.
        let mut statistics_reset: bool = false;

        // Catch ctrl-c to allow clean shutdown to display statistics.
        let canceled = Arc::new(AtomicBool::new(false));
        util::setup_ctrlc_handler(&canceled);

        // Determine when to display running statistics (if enabled).
        let mut statistics_timer = time::Instant::now();
        let mut display_running_statistics = false;

        loop {
            // When displaying running statistics, sync data from client threads first.
            if !self.configuration.no_stats {
                // Synchronize statistics from client threads into parent.
                if util::timer_expired(statistics_timer, RUNNING_STATS_EVERY) {
                    statistics_timer = time::Instant::now();
                    for (index, send_to_client) in client_channels.iter().enumerate() {
                        match send_to_client.send(GooseClientCommand::SYNC) {
                            Ok(_) => {
                                debug!("telling client {} to sync stats", index);
                            }
                            Err(e) => {
                                warn!("failed to tell client {} to sync stats: {}", index, e);
                            }
                        }
                    }
                    if !self.configuration.only_summary {
                        display_running_statistics = true;
                        // Give client threads time to send statstics.
                        let pause = time::Duration::from_millis(100);
                        thread::sleep(pause);
                    }
                }

                // Load messages from client threads until the receiver queue is empty.
                let mut received_message = false;
                let mut message = parent_receiver.try_recv();
                while message.is_ok() {
                    received_message = true;
                    // Messages contain per-client statistics: merge them into the global statistics.
                    let unwrapped_message = message.unwrap();
                    let weighted_clients_index = unwrapped_message.weighted_clients_index;
                    self.weighted_clients[weighted_clients_index].mode = unwrapped_message.mode;
                    // Syncronize client requests
                    for (request_key, request) in unwrapped_message.requests {
                        trace!("request_key: {}", request_key);
                        let merged_request;
                        if let Some(parent_request) = self.merged_requests.get(&request_key) {
                            merged_request = merge_from_client(parent_request, &request, &self.configuration);
                        }
                        else {
                            // First time seeing this request, simply insert it.
                            merged_request = request.clone();
                        }
                        self.merged_requests.insert(request_key.to_string(), merged_request);
                    }
                    message = parent_receiver.try_recv();
                }

                // As worker, push statistics up to manager.
                if self.configuration.worker && received_message {
                    #[cfg(feature = "gaggle")]
                    {
                        // Push all statistics to manager process.
                        if !worker::push_stats_to_manager(&socket.clone().unwrap(), &self.merged_requests.clone(), true) {
                            // EXIT received, cancel.
                            canceled.store(true, Ordering::SeqCst);
                        }
                        // The manager has all our statistics, reset locally.
                        self.merged_requests = HashMap::new();
                    }
                }

                // Flush statistics collected prior to all client threads running
                if self.configuration.reset_stats && !statistics_reset {
                    info!("statistics reset...");
                    for (client_index, client) in self.weighted_clients.clone().iter().enumerate() {
                        let mut reset_client = client.clone();
                        // Start again with an empty requests hashmap.
                        reset_client.requests = HashMap::new();
                        self.weighted_clients[client_index] = reset_client;
                    }
                    statistics_reset = true;
                }
            }

            if util::timer_expired(started, self.run_time) || canceled.load(Ordering::SeqCst) {
                if self.configuration.worker {
                    info!("[{}] stopping after {} seconds...", get_worker_id(), started.elapsed().as_secs());
                }
                else {
                    info!("stopping after {} seconds...", started.elapsed().as_secs());
                }
                for (index, send_to_client) in client_channels.iter().enumerate() {
                    match send_to_client.send(GooseClientCommand::EXIT) {
                        Ok(_) => {
                            debug!("telling client {} to exit", index);
                        }
                        Err(e) => {
                            warn!("failed to tell client {} to exit: {}", index, e);
                        }
                    }
                }
                if self.configuration.worker {
                    info!("[{}] waiting for clients to exit", get_worker_id());
                }
                else {
                    info!("waiting for clients to exit");
                }
                for client in clients {
                    let _ = client.await;
                }
                debug!("all clients exited");

                // If we're printing statistics, collect the final messages received from clients
                if !self.configuration.no_stats {
                    let mut message = parent_receiver.try_recv();
                    while message.is_ok() {
                        let unwrapped_message = message.unwrap();
                        let weighted_clients_index = unwrapped_message.weighted_clients_index;
                        self.weighted_clients[weighted_clients_index].mode = unwrapped_message.mode;
                        // Syncronize client requests
                        for (request_key, request) in unwrapped_message.requests {
                            trace!("request_key: {}", request_key);
                            let merged_request;
                            if let Some(parent_request) = self.merged_requests.get(&request_key) {
                                merged_request = merge_from_client(parent_request, &request, &self.configuration);
                            }
                            else {
                                // First time seeing this request, simply insert it.
                                merged_request = request.clone();
                            }
                            self.merged_requests.insert(request_key.to_string(), merged_request);
                        }
                        message = parent_receiver.try_recv();
                    }
                }

                #[cfg(feature = "gaggle")]
                {
                    // As worker, push statistics up to manager.
                    if self.configuration.worker {
                        // Push all statistics to manager process.
                        worker::push_stats_to_manager(&socket.clone().unwrap(), &self.merged_requests.clone(), true);
                        // No need to reset local stats, the worker is exiting.
                    }
                }

                // All clients are done, exit out of loop for final cleanup.
                break;
            }

            // If enabled, display running statistics after sync
            if display_running_statistics {
                display_running_statistics = false;
                stats::print_running_stats(&self, started.elapsed().as_secs() as usize);
            }

            let one_second = time::Duration::from_secs(1);
            thread::sleep(one_second);
        }
        self
    }
}

/// CLI options available when launching a Goose load test.
#[derive(StructOpt, Debug, Default, Clone, Serialize, Deserialize)]
#[structopt(name = "client")]
pub struct GooseConfiguration {
    /// Host to load test, for example: http://10.21.32.33
    #[structopt(short = "H", long, required=false, default_value="")]
    host: String,

    /// Number of concurrent Goose users (defaults to available CPUs).
    #[structopt(short, long)]
    clients: Option<usize>,

    /// How many users to spawn per second.
    #[structopt(short = "r", long, required=false, default_value="1")]
    hatch_rate: usize,

    /// Stop after the specified amount of time, e.g. (300s, 20m, 3h, 1h30m, etc.).
    #[structopt(short = "t", long, required=false, default_value="")]
    run_time: String,

    /// Don't print stats in the console
    #[structopt(long)]
    no_stats: bool,

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

    // The number of occurrences of the `v/verbose` flag
    /// Debug level (-v, -vv, -vvv, etc.)
    #[structopt(short = "v", long, parse(from_occurrences))]
    verbose: u8,

    // The number of occurrences of the `g/log-level` flag
    /// Log level (-g, -gg, -ggg, etc.)
    #[structopt(short = "g", long, parse(from_occurrences))]
    log_level: u8,

    /// Log file name
    #[structopt(long, default_value="goose.log")]
    log_file: String,

    /// Enables manager mode
    #[structopt(long)]
    manager: bool,

    /// Ignore worker load test checksum
    #[structopt(long)]
    no_hash_check: bool,

    /// Required when in manager mode, how many workers to expect
    #[structopt(long, required=false, default_value="0")]
    expect_workers: u16,

    /// Define host manager listens on, formatted x.x.x.x
    #[structopt(long, default_value="0.0.0.0")]
    manager_bind_host: String,

    /// Define port manager listens on
    #[structopt(long, default_value=DEFAULT_PORT)]
    manager_bind_port: u16,

    /// Enables worker mode
    #[structopt(long)]
    worker: bool,

    /// Host manager is running on
    #[structopt(long, default_value="127.0.0.1")]
    manager_host: String,

    /// Port manager is listening on
    #[structopt(long, default_value=DEFAULT_PORT)]
    manager_port: u16,
}

/// Returns a sequenced bucket of weighted usize pointers to Goose Tasks
fn weight_tasks(task_set: &GooseTaskSet) -> (Vec<Vec<usize>>, Vec<Vec<usize>>, Vec<Vec<usize>>) {
    trace!("weight_tasks for {}", task_set.name);

    // A BTreeMap of Vectors allows us to group and sort tasks per sequence value.
    let mut sequenced_tasks: BTreeMap <usize, Vec<GooseTask>> = BTreeMap::new();
    let mut sequenced_on_start_tasks: BTreeMap <usize, Vec<GooseTask>> = BTreeMap::new();
    let mut sequenced_on_stop_tasks: BTreeMap <usize, Vec<GooseTask>> = BTreeMap::new();
    let mut unsequenced_tasks: Vec<GooseTask> = Vec::new();
    let mut unsequenced_on_start_tasks: Vec<GooseTask> = Vec::new();
    let mut unsequenced_on_stop_tasks: Vec<GooseTask> = Vec::new();
    let mut u: usize = 0;
    let mut v: usize;
    // Handle ordering of tasks.
    for task in &task_set.tasks {
        if task.sequence > 0 {
            if task.on_start {
                if let Some(sequence) = sequenced_on_start_tasks.get_mut(&task.sequence) {
                    // This is another task with this order value.
                    sequence.push(task.clone());
                }
                else {
                    // This is the first task with this order value.
                    sequenced_on_start_tasks.insert(task.sequence, vec![task.clone()]);
                }
            }
            // Allow a task to be both on_start and on_stop.
            if task.on_stop {
                if let Some(sequence) = sequenced_on_stop_tasks.get_mut(&task.sequence) {
                    // This is another task with this order value.
                    sequence.push(task.clone());
                }
                else {
                    // This is the first task with this order value.
                    sequenced_on_stop_tasks.insert(task.sequence, vec![task.clone()]);
                }
            }
            if !task.on_start && !task.on_stop {
                if let Some(sequence) = sequenced_tasks.get_mut(&task.sequence) {
                    // This is another task with this order value.
                    sequence.push(task.clone());
                }
                else {
                    // This is the first task with this order value.
                    sequenced_tasks.insert(task.sequence, vec![task.clone()]);
                }
            }
        }
        else {
            if task.on_start {
                unsequenced_on_start_tasks.push(task.clone());
            }
            if task.on_stop {
                unsequenced_on_stop_tasks.push(task.clone());
            }
            if !task.on_start && !task.on_stop {
                unsequenced_tasks.push(task.clone());
            }
        }
        // Look for lowest common divisor amongst all tasks of any weight.
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

    // Apply weight to sequenced tasks.
    let mut weighted_tasks: Vec<Vec<usize>> = Vec::new();
    for (_sequence, tasks) in sequenced_tasks.iter() {
        let mut sequence_weighted_tasks = Vec::new();
        for task in tasks {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!("{}: {} has weight of {} (reduced with gcd to {})", task.tasks_index, task.name, task.weight, weight);
            let mut tasks = vec![task.tasks_index; weight];
            sequence_weighted_tasks.append(&mut tasks);
        }
        weighted_tasks.push(sequence_weighted_tasks);
    }
    // Apply weight to unsequenced tasks.
    trace!("created weighted_tasks: {:?}", weighted_tasks);
    let mut weighted_unsequenced_tasks = Vec::new();
    for task in unsequenced_tasks {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!("{}: {} has weight of {} (reduced with gcd to {})", task.tasks_index, task.name, task.weight, weight);
        let mut tasks = vec![task.tasks_index; weight];
        weighted_unsequenced_tasks.append(&mut tasks);
    }
    // Unsequenced tasks come lost.
    weighted_tasks.push(weighted_unsequenced_tasks);

    // Apply weight to on_start sequenced tasks.
    let mut weighted_on_start_tasks: Vec<Vec<usize>> = Vec::new();
    for (_sequence, tasks) in sequenced_on_start_tasks.iter() {
        let mut sequence_on_start_weighted_tasks = Vec::new();
        for task in tasks {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!("{}: {} has weight of {} (reduced with gcd to {})", task.tasks_index, task.name, task.weight, weight);
            let mut tasks = vec![task.tasks_index; weight];
            sequence_on_start_weighted_tasks.append(&mut tasks);
        }
        weighted_on_start_tasks.push(sequence_on_start_weighted_tasks);
    }
    // Apply weight to unsequenced on_start tasks.
    trace!("created weighted_on_start_tasks: {:?}", weighted_tasks);
    let mut weighted_on_start_unsequenced_tasks = Vec::new();
    for task in unsequenced_on_start_tasks {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!("{}: {} has weight of {} (reduced with gcd to {})", task.tasks_index, task.name, task.weight, weight);
        let mut tasks = vec![task.tasks_index; weight];
        weighted_on_start_unsequenced_tasks.append(&mut tasks);
    }
    // Unsequenced tasks come lost.
    weighted_on_start_tasks.push(weighted_on_start_unsequenced_tasks);

    // Apply weight to on_stop sequenced tasks.
    let mut weighted_on_stop_tasks: Vec<Vec<usize>> = Vec::new();
    for (_sequence, tasks) in sequenced_on_stop_tasks.iter() {
        let mut sequence_on_stop_weighted_tasks = Vec::new();
        for task in tasks {
            // divide by greatest common divisor so bucket is as small as possible
            let weight = task.weight / u;
            trace!("{}: {} has weight of {} (reduced with gcd to {})", task.tasks_index, task.name, task.weight, weight);
            let mut tasks = vec![task.tasks_index; weight];
            sequence_on_stop_weighted_tasks.append(&mut tasks);
        }
        weighted_on_stop_tasks.push(sequence_on_stop_weighted_tasks);
    }
    // Apply weight to unsequenced on_stop tasks.
    trace!("created weighted_on_stop_tasks: {:?}", weighted_tasks);
    let mut weighted_on_stop_unsequenced_tasks = Vec::new();
    for task in unsequenced_on_stop_tasks {
        // divide by greatest common divisor so bucket is as small as possible
        let weight = task.weight / u;
        trace!("{}: {} has weight of {} (reduced with gcd to {})", task.tasks_index, task.name, task.weight, weight);
        let mut tasks = vec![task.tasks_index; weight];
        weighted_on_stop_unsequenced_tasks.append(&mut tasks);
    }
    // Unsequenced tasks come last.
    weighted_on_stop_tasks.push(weighted_on_stop_unsequenced_tasks);

    (weighted_on_start_tasks, weighted_tasks, weighted_on_stop_tasks)
}

fn is_valid_host(host: &str) -> bool {
    match Url::parse(host) {
        Ok(_) => true,
        Err(e) => {
            error!("invalid host '{}': {}", host, e);
            std::process::exit(1);
        }
    }
}

/// A helper function that merges together response times.
///
/// Used in `lib.rs` to merge together per-thread response times, and in `stats.rs`
/// to aggregate all response times.
pub fn merge_response_times(
    mut global_response_times: BTreeMap<usize, usize>,
    local_response_times: BTreeMap<usize, usize>,
) -> BTreeMap<usize, usize> {
    // Iterate over client response times, and merge into global response times.
    for (response_time, count) in &local_response_times {
        let counter = match global_response_times.get(&response_time) {
            // We've seen this response_time before, increment counter.
            Some(c) => {
                *c + count
            }
            // First time we've seen this response time, initialize counter.
            None => {
                *count
            }
        };
        global_response_times.insert(*response_time, counter);
    }
    global_response_times
}

// Update global minimum response time based on local resposne time.
fn update_min_response_time(mut global_min: usize, min: usize) -> usize {
    if global_min == 0 || (min > 0 && min < global_min) {
        global_min = min;
    }
    global_min
}

// Update global maximum response time based on local resposne time.
fn update_max_response_time(mut global_max: usize, max: usize) -> usize {
    if global_max < max {
        global_max = max;
    }
    global_max
}

/// Merge per-client-statistics from client thread into global parent statistics
fn merge_from_client(
    parent_request: &GooseRequest,
    client_request: &GooseRequest,
    config: &GooseConfiguration,
) -> GooseRequest {
    // Make a mutable copy where we can merge things
    let mut merged_request = parent_request.clone();

    // Iterate over client response times, and merge into global response times.
    merged_request.response_times = merge_response_times(
        merged_request.response_times,
        client_request.response_times.clone(),
    );
    // Increment total response time counter.
    merged_request.total_response_time += &client_request.total_response_time;
    // Increment count of how many resposne counters we've seen.
    merged_request.response_time_counter += &client_request.response_time_counter;
    // If client had new fastest response time, update global fastest response time.
    merged_request.min_response_time = update_min_response_time(merged_request.min_response_time, client_request.min_response_time);
    // If client had new slowest response time, update global slowest resposne time.
    merged_request.max_response_time = update_max_response_time(merged_request.max_response_time, client_request.max_response_time);
    // Increment total success counter.
    merged_request.success_count += &client_request.success_count;
    // Increment total fail counter.
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
        global_response_times = merge_response_times(global_response_times, local_response_times.clone());
        // @TODO: how can we do useful testing of private method and objects?
        assert_eq!(&global_response_times, &local_response_times);
    }

    #[test]
    fn valid_host() {
        // We can only test valid domains, as we exit on failure.
        // @TODO: rework so we don't exit on failure
        assert_eq!(is_valid_host("http://example.com"), true);
        assert_eq!(is_valid_host("http://example.com/"), true);
        assert_eq!(is_valid_host("https://www.example.com/and/with/path"), true);
        assert_eq!(is_valid_host("foo://example.com"), true);
        assert_eq!(is_valid_host("file:///path/to/file"), true);
    }
}
