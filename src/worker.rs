use std::{thread, time};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use nng::*;

use crate::{GooseAttack, GooseConfiguration, get_worker_id, WORKER_ID};
use crate::goose::{GooseRequest, GooseClient, GooseClientCommand, GooseMethod};
use crate::manager::GooseClientInitializer;
use crate::util;

fn pipe_closed(_pipe: Pipe, event: PipeEvent) {
    if event == PipeEvent::RemovePost {
        warn!("[{}] manager went away, exiting", get_worker_id());
        std::process::exit(1);
    }
}

pub fn worker_main(goose_attack: &GooseAttack) {
    // Creates a TCP address.
    let address = format!("tcp://{}:{}", goose_attack.configuration.manager_host, goose_attack.configuration.manager_port);
    info!("worker connecting to manager at {}", &address);

    // Create a request socket.
    let manager = match Socket::new(Protocol::Req0) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to create socket {}: {}.", &address, e);
            std::process::exit(1);
        }
    };
    match manager.pipe_notify(pipe_closed) {
        Ok(_) => (),
        Err(e) => {
            error!("failed to set up pipe handler: {}", e);
            std::process::exit(1);
        }
    }

    // Pause 1/10 of a second in case we're blocking on a cargo lock.
    thread::sleep(time::Duration::from_millis(100));
    // Connect to manager.
    let mut retries = 0;
    loop {
        match manager.dial(&address) {
            Ok(_) => break,
            Err(e) => {
                if retries >= 5 {
                    error!("failed to communicate with manager at {}: {}.", &address, e);
                    std::process::exit(1);
                }
                debug!("failed to communicate with manager at {}: {}.", &address, e);
                let sleep_duration = time::Duration::from_millis(500);
                debug!("sleeping {:?} milliseconds waiting for manager...", sleep_duration);
                thread::sleep(sleep_duration);
                retries += 1;
            }
        }
    }

    // Let manager know we're ready to work -- push empty HashMap.
    let mut requests: HashMap<String, GooseRequest> = HashMap::new();
    // "Fake" request for manager to validate this worker's load test hash.
    requests.insert("load_test_hash".to_string(), GooseRequest::new(
        "none",
        GooseMethod::GET,
        goose_attack.task_sets_hash,
    ));
    debug!("sending load test hash to manager: {}", goose_attack.task_sets_hash);
    push_stats_to_manager(&manager, &requests, false);

    // Only send load_test_hash one time.
    requests = HashMap::new();

    let mut hatch_rate: Option<f32> = None;
    let mut config: GooseConfiguration = GooseConfiguration::default();
    let mut weighted_clients: Vec<GooseClient> = Vec::new();

    // Wait for the manager to send client parameters.
    loop {
        info!("waiting for instructions from manager");
        let msg = match manager.recv() {
            Ok(m) => m,
            Err(e) => {
                error!("unexpected error receiving manager message: {}", e);
                std::process::exit(1);
            }
        };
        let initializers: Vec<GooseClientInitializer> = match serde_cbor::from_reader(msg.as_slice()) {
            Ok(i) => i,
            Err(_) => {
                let command: GooseClientCommand = match serde_cbor::from_reader(msg.as_slice()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("invalid message received: {}", e);
                        continue;
                    }
                };
                match command {
                    GooseClientCommand::EXIT => {
                        warn!("received EXIT command from manager");
                        std::process::exit(0);
                    },
                    other => {
                        info!("received unknown command from manager: {:?}", other);
                    }
                }
                continue;
            }
        };

        let mut worker_id: usize = 0;
        // Allocate a state for each client that will be spawned.
        info!("initializing client states...");
        for initializer in initializers {
            if worker_id == 0 {
                worker_id = initializer.worker_id;
            }
            weighted_clients.push(GooseClient::new(
                weighted_clients.len(),
                initializer.task_sets_index,
                initializer.default_host.clone(),
                initializer.task_set_host.clone(),
                initializer.min_wait,
                initializer.max_wait,
                &initializer.config,
                goose_attack.task_sets_hash,
            ));
            if hatch_rate == None {
                hatch_rate = Some(1.0 / (initializer.config.hatch_rate as f32 / (initializer.config.expect_workers as f32)));
                config = initializer.config;
                info!("[{}] prepared to start 1 client every {:.2} seconds", worker_id, hatch_rate.unwrap());
            }
        }
        WORKER_ID.lock().unwrap().store(worker_id, Ordering::Relaxed);
        info!("[{}] initialized {} client states", get_worker_id(), weighted_clients.len());
        break;
    }

    info!("[{}] waiting for go-ahead from manager", get_worker_id());

    // Wait for the manager to send go-ahead to start the load test.
    loop {
        // Push statistics to manager to force a reply, waiting for RUN.
        push_stats_to_manager(&manager, &requests, false);
        let msg = match manager.recv() {
            Ok(m) => m,
            Err(e) => {
                error!("[{}] unexpected error receiving manager message: {}", get_worker_id(), e);
                std::process::exit(1);
            }
        };
        let command: GooseClientCommand = match serde_cbor::from_reader(msg.as_slice()) {
            Ok(c) => c,
            Err(e) => {
                error!("[{}] invalid message received: {}", get_worker_id(), e);
                continue;
            }
        };

        match command {
            // Break out of loop and start the load test.
            GooseClientCommand::RUN => break,
            // Exit worker process immediately.
            GooseClientCommand::EXIT => {
                warn!("[{}] received EXIT command from manager", get_worker_id());
                std::process::exit(0);
            },
            // Sleep and then loop again.
            _ => {
                let sleep_duration = time::Duration::from_secs(1);
                debug!("[{}] sleeping {:?} second waiting for manager...", get_worker_id(), sleep_duration);
                thread::sleep(sleep_duration);
            }
        }
    }

    // Worker is officially starting the load test.
    let started = time::Instant::now();
    info!("[{}] entering gaggle mode, starting load test", get_worker_id());
    let sleep_duration = time::Duration::from_secs_f32(hatch_rate.unwrap());

    let mut worker_goose_attack = GooseAttack::initialize_with_config(config.clone());
    worker_goose_attack.task_sets = goose_attack.task_sets.clone();
    if config.run_time != "" {
        worker_goose_attack.run_time = util::parse_timespan(&config.run_time);
        info!("[{}] run_time = {}", get_worker_id(), worker_goose_attack.run_time);
    }
    else {
        worker_goose_attack.run_time = 0;
    }
    worker_goose_attack.weighted_clients = weighted_clients;
    worker_goose_attack.configuration.worker = true;
    let mut rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(worker_goose_attack.launch_clients(started, sleep_duration, Some(manager)));
}

pub fn push_stats_to_manager(manager: &Socket, requests: &HashMap<String, GooseRequest>, get_response: bool) -> bool {
    debug!("[{}] pushing stats to manager: {}", get_worker_id(), requests.len());
    let mut buf: Vec<u8> = Vec::new();
    match serde_cbor::to_writer(&mut buf, requests) {
        Ok(_) => (),
        Err(e) => {
            error!("[{}] failed to serialize empty Vec<GooseRequest>: {}", get_worker_id(), e);
            std::process::exit(1);
        }
    }
    match manager.try_send(&buf) {
        Ok(m) => m,
        Err(e) => {
            error!("[{}] communication failure: {:?}.", get_worker_id(), e);
            std::process::exit(1);
        }
    }

    if get_response {
        // Wait for server to reply.
        let msg = match manager.recv() {
            Ok(m) => m,
            Err(e) => {
                error!("[{}] unexpected error receiving manager message: {}", get_worker_id(), e);
                std::process::exit(1);
            }
        };
        let command: GooseClientCommand = match serde_cbor::from_reader(msg.as_slice()) {
            Ok(c) => c,
            Err(e) => {
                error!("[{}] invalid message received: {}", get_worker_id(), e);
                std::process::exit(1);
            }
        };

        match command {
            GooseClientCommand::EXIT => {
                warn!("[{}] received EXIT command from manager", get_worker_id());
                return false;
            },
            _ => (),
        }
    }
    true
}
