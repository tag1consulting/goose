use std::collections::{HashMap, HashSet};
use std::{thread, time};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use lazy_static::lazy_static;
use nng::*;
use serde::{Serialize, Deserialize};

use crate::{GooseAttack, GooseConfiguration, GooseClientCommand};
use crate::goose::GooseRequest;
use crate::stats;
use crate::util;

/// How long the manager will wait for all workers to stop after the load test ends.
const GRACEFUL_SHUTDOWN_TIMEOUT: usize = 30;

/// All elements required to initialize a client in a worker process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseClientInitializer {
    /// An index into the internal `GooseTest.task_sets` vector, indicating which GooseTaskSet is running.
    pub task_sets_index: usize,
    /// The global GooseAttack host.
    pub default_host: Option<String>,
    /// The GooseTaskSet.host.
    pub task_set_host: Option<String>,
    /// Minimum amount of time to sleep after running a task.
    pub min_wait: usize,
    /// Maximum amount of time to sleep after running a task.
    pub max_wait: usize,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
    /// Numerical identifier for worker.
    pub worker_id: usize,
}

// Mutable singleton globally tracking how many workers are currently being managed.
lazy_static! {
    static ref ACTIVE_WORKERS: Mutex<AtomicUsize> = Mutex::new(AtomicUsize::new(0));
}

fn pipe_closed(_pipe: Pipe, event: PipeEvent) {
    match event {
        PipeEvent::AddPost => {
            debug!("worker pipe added");
            ACTIVE_WORKERS.lock().unwrap().fetch_add(1, Ordering::SeqCst);
        },
        PipeEvent::RemovePost => {
            let active_workers = ACTIVE_WORKERS.lock().unwrap().fetch_sub(1, Ordering::SeqCst);
            info!("worker {} exited", active_workers);
        },
        _ => {},
    }
}

pub fn manager_main(mut goose_attack: GooseAttack) -> GooseAttack {
    // Creates a TCP address.
    let address = format!("tcp://{}:{}", goose_attack.configuration.manager_bind_host, goose_attack.configuration.manager_bind_port);
    info!("worker connecting to manager at {}", &address);

    // Create a Rep0 reply socket.
    let server = match Socket::new(Protocol::Rep0) {
        Ok(s) => s,
        Err(e) => {
            error!("failed to create socket: {}.", e);
            std::process::exit(1);
        }
    };

    // Set up callback function to receive pipe event notifications.
    match server.pipe_notify(pipe_closed) {
        Ok(_) => (),
        Err(e) => {
            error!("failed to set up pipe handler: {}", e);
            std::process::exit(1);
        }
    }

    // Listen for connections.
    match server.listen(&address) {
        Ok(s) => (s),
        Err(e) => {
            error!("failed to bind to socket {}: {}.", address, e);
            std::process::exit(1);
        }
    }
    info!("manager listening on {}, waiting for {} workers", &address, goose_attack.configuration.expect_workers);

    // Calculate how many clients each worker will be responsible for.
    let split_clients = goose_attack.clients / (goose_attack.configuration.expect_workers as usize);
    let mut split_clients_remainder = goose_attack.clients % (goose_attack.configuration.expect_workers as usize);
    if split_clients_remainder > 0 {
        info!("each worker to start {} clients, assigning 1 extra to {} workers", split_clients, split_clients_remainder);
    }
    else {
        info!("each worker to start {} clients", split_clients);
    }

    // A mutable bucket of clients to be assigned to workers.
    let mut available_clients = goose_attack.weighted_clients.clone();

    // Track how many workers we've seen.
    let mut workers: HashSet<Pipe> = HashSet::new();

    // Track start time, we'll reset this when the test actually starts.
    let mut started = time::Instant::now();
    let mut running_statistics_timer = time::Instant::now();
    let mut exit_timer = time::Instant::now();
    let mut load_test_running = false;
    let mut load_test_finished = false;

    // Catch ctrl-c to allow clean shutdown to display statistics.
    let canceled = Arc::new(AtomicBool::new(false));
    util::setup_ctrlc_handler(&canceled);

    // Worker control loop.
    loop {
        // While running load test, check if any workers go away.
        if !load_test_finished {
            // If ACTIVE_WORKERS is less than the total workers seen, a worker went away.
            if ACTIVE_WORKERS.lock().unwrap().load(Ordering::SeqCst) < workers.len() {
                // If worked goes away during load test, exit gracefully.
                if load_test_running {
                    info!("worker went away, stopping gracefully afer {} seconds...", started.elapsed().as_secs());
                    load_test_finished = true;
                    exit_timer = time::Instant::now();
                }
                // If a worker goes away during start up, exit immediately.
                else { 
                    warn!("worker went away, stopping immediately...");
                    break;
                }
            }
        }
        if load_test_running {
            if !load_test_finished {
                // Test ran to completion or was canceled with ctrl-c.
                if util::timer_expired(started, goose_attack.run_time) || canceled.load(Ordering::SeqCst) {
                    info!("stopping after {} seconds...", started.elapsed().as_secs());
                    load_test_finished = true;
                    exit_timer = time::Instant::now();
                }
            }

            // Aborting graceful shutdown, workers took too long to shut down.
            if load_test_finished && util::timer_expired(exit_timer, GRACEFUL_SHUTDOWN_TIMEOUT) {
                warn!("graceful shutdown timer expired, exiting...");
                break;
            }
        
            // When displaying running statistics, sync data from client threads first.
            if !goose_attack.configuration.only_summary &&
                util::timer_expired(running_statistics_timer, crate::RUNNING_STATS_EVERY
            ) { 
                // Reset timer each time we display statistics.
                running_statistics_timer = time::Instant::now();
                stats::print_running_stats(&goose_attack, started.elapsed().as_secs() as usize);
            }
        }
        else if canceled.load(Ordering::SeqCst) {
            info!("load test canceled, exiting");
            std::process::exit(1);
        }

        // Check for messages from workers.
        match server.try_recv() {
            Ok(mut msg) => {
                // Message received, grab the pipe to determine which worker it is.
                let pipe = match msg.pipe() {
                    Some(p) => p,
                    None => {
                        error!("unexpected fatal error reading worker pipe");
                        std::process::exit(1);
                    }
                };

                // Workers always send a HashMap<String, GooseRequest>.
                let requests: HashMap<String, GooseRequest> = serde_cbor::from_reader(msg.as_slice()).unwrap();
                debug!("requests statistics received: {:?}", requests.len());

                // If workers already contains this pipe, we've seen this worker before.
                if workers.contains(&pipe) {
                    let mut buf: Vec<u8> = Vec::new();
                    // All workers are running load test, sending statistics.
                    if workers.len() == goose_attack.configuration.expect_workers as usize {
                        // Requests statistics received, merge them into our local copy.
                        if requests.len() > 0 {
                            debug!("requests statistics received: {:?}", requests.len());
                            for (request_key, request) in requests {
                                trace!("request_key: {}", request_key);
                                let merged_request;
                                if let Some(parent_request) = goose_attack.merged_requests.get(&request_key) {
                                    merged_request = crate::merge_from_client(parent_request, &request, &goose_attack.configuration);
                                }
                                else {
                                    // First time seeing this request, simply insert it.
                                    merged_request = request.clone();
                                }
                                goose_attack.merged_requests.insert(request_key.to_string(), merged_request);
                            }
                        }
                        // Notify the worker that the load test is over and to exit.
                        if load_test_finished {
                            debug!("telling worker to exit");
                            match serde_cbor::to_writer(&mut buf, &GooseClientCommand::EXIT) {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("failed to serialize client command: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                        // Notify the worker that the load test is still running.
                        else {
                            match serde_cbor::to_writer(&mut buf, &GooseClientCommand::RUN) {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("failed to serialize client command: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                    // All workers are not yet running, tell worker to wait.
                    else { 
                        match serde_cbor::to_writer(&mut buf, &GooseClientCommand::WAIT) {
                            Ok(_) => (),
                            Err(e) => {
                                error!("failed to serialize client command: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    let message: Message = buf.as_slice().into();
                    match server.try_send(message) {
                        Ok(_) => (),
                        // Determine why there was an error.
                        Err((_, e)) => {
                            match e {
                                // A worker went away, this happens during shutdown.
                                Error::TryAgain => {
                                    if ACTIVE_WORKERS.lock().unwrap().load(Ordering::SeqCst) == 0 {
                                        info!("all workers have exited");
                                        break;
                                    }
                                },
                                // An unexpected error.
                                _ => {
                                    error!("communication failure: {:?}", e);
                                    std::process::exit(1);
                                },
                            }
                        }
                    }
                }
                // This is the first time we've seen this worker.
                else {
                    // Make sure we're not already connected to all of our workers.
                    if workers.len() >= goose_attack.configuration.expect_workers as usize {
                        // We already have enough workers, tell this extra one to EXIT.
                        let mut buf: Vec<u8> = Vec::new();
                        match serde_cbor::to_writer(&mut buf, &GooseClientCommand::EXIT) {
                            Ok(_) => (),
                            Err(e) => {
                                error!("failed to serialize client command: {}", e);
                                std::process::exit(1);
                            }
                        }
                        let message: Message = buf.as_slice().into();
                        match server.try_send(message) {
                            Ok(_) => (),
                            // Determine why our send failed.
                            Err((_, e)) => {
                                match e {
                                    Error::TryAgain => {
                                        if ACTIVE_WORKERS.lock().unwrap().load(Ordering::SeqCst) == 0 {
                                            info!("all workers have exited");
                                            break;
                                        }
                                    },
                                    _ => {
                                        error!("communication failure: {:?}", e);
                                        std::process::exit(1);
                                    },
                                }
                            }
                        }
                    }
                    // We need another worker, accept the connection.
                    else {
                        // Validate worker load test hash.
                        match requests.get("load_test_hash") {
                            Some(r) => {
                                if r.load_test_hash != goose_attack.task_sets_hash {
                                    if goose_attack.configuration.no_hash_check {
                                        warn!("worker is running a different load test, ignoring")
                                    }
                                    else {
                                        error!("worker is running a different load test, set --no-hash-check to ignore");
                                        std::process::exit(1);
                                    }
                                }

                            },
                            None => {
                                if goose_attack.configuration.no_hash_check {
                                    warn!("worker is running a different load test, ignoring")
                                }
                                else {
                                    error!("worker is running a different load test, set --no-hash-check to ignore");
                                    std::process::exit(1);
                                }
                            }
                        };

                        workers.insert(pipe);
                        info!("worker {} of {} connected", workers.len(), goose_attack.configuration.expect_workers);

                        // Send new worker a batch of clients.
                        let mut client_batch = split_clients;
                        // If remainder, put extra client in this batch.
                        if split_clients_remainder > 0 {
                            split_clients_remainder -= 1;
                            client_batch += 1;
                        }
                        let mut clients = Vec::new();

                        // Pop clients from available_clients vector and build worker initializer.
                        for _ in 1..=client_batch {
                            let client = match available_clients.pop() {
                                Some(c) => c,
                                None => {
                                    error!("not enough available clients!?");
                                    std::process::exit(1);
                                }
                            };
                            // Build a vector of GooseClient initializers for next worker.
                            clients.push(GooseClientInitializer{
                                task_sets_index: client.task_sets_index,
                                default_host: client.default_host.clone(),
                                task_set_host: client.task_set_host.clone(),
                                min_wait: client.min_wait,
                                max_wait: client.max_wait,
                                config: client.config.clone(),
                                worker_id: workers.len(),
                            });
                        }

                        // Send vector of client initializers to worker.
                        let mut buf: Vec<u8> = Vec::new();
                        match serde_cbor::to_writer(&mut buf, &clients) {
                            Ok(_) => (),
                            Err(e) => {
                                error!("failed to serialize client initializers: {}", e);
                                std::process::exit(1);
                            }
                        }
                        info!("sending {} clients to worker {}", clients.len(), workers.len());
                        let message: Message = buf.as_slice().into();
                        match server.try_send(message) {
                            Ok(_) => (),
                            Err((_, e)) => {
                                match e {
                                    Error::TryAgain => {
                                        if ACTIVE_WORKERS.lock().unwrap().load(Ordering::SeqCst) == 0 {
                                            info!("all workers have exited");
                                            break;
                                        }
                                    },
                                    _ => {
                                        error!("communication failure: {:?}", e);
                                        std::process::exit(1);
                                    },
                                }
                            }
                        }

                        if workers.len() == goose_attack.configuration.expect_workers as usize {
                            info!("gaggle distributed load test started");
                            // Reset start time, the distributed load test is truly starting now.
                            started = time::Instant::now();
                            running_statistics_timer = time::Instant::now();
                            load_test_running = true;
                        }
                    }
                }
            }
            Err(e) => {
                if e == Error::TryAgain {
                    if workers.len() > 0 {
                        if ACTIVE_WORKERS.lock().unwrap().load(Ordering::SeqCst) == 0 {
                            info!("all workers have exited");
                            break;
                        }
                    }
                    if !load_test_finished {
                        // Sleep half a second then return to the loop.
                        thread::sleep(time::Duration::from_millis(500));
                    }
                }
                else {
                    error!("unexpected error receiving client message: {}", e);
                    std::process::exit(1);
                }
            }
        }

    }
    goose_attack
}
