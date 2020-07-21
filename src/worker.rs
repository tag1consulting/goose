use nng::*;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::{thread, time};
use url::Url;

use crate::goose::{GooseMethod, GooseRequest, GooseUser, GooseUserCommand};
use crate::manager::GooseUserInitializer;
use crate::util;
use crate::{get_worker_id, GooseAttack, GooseConfiguration, WORKER_ID};

// If pipe closes unexpectedly, exit.
fn pipe_closed(_pipe: Pipe, event: PipeEvent) {
    if event == PipeEvent::RemovePost {
        warn!("[{}] manager went away, exiting", get_worker_id());
        std::process::exit(1);
    }
}

// If pipe closes during shutdown, just log it.
fn pipe_closed_during_shutdown(_pipe: Pipe, event: PipeEvent) {
    if event == PipeEvent::RemovePost {
        info!("[{}] manager went away", get_worker_id());
    }
}

pub async fn worker_main(goose_attack: &GooseAttack) -> GooseAttack {
    // Creates a TCP address.
    let address = format!(
        "tcp://{}:{}",
        goose_attack.configuration.manager_host, goose_attack.configuration.manager_port
    );
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
                debug!(
                    "sleeping {:?} milliseconds waiting for manager...",
                    sleep_duration
                );
                thread::sleep(sleep_duration);
                retries += 1;
            }
        }
    }

    // Let manager know we're ready to work -- push empty HashMap.
    let mut requests: HashMap<String, GooseRequest> = HashMap::new();
    // "Fake" request for manager to validate this worker's load test hash.
    requests.insert(
        "load_test_hash".to_string(),
        GooseRequest::new("none", GooseMethod::GET, goose_attack.task_sets_hash),
    );
    debug!(
        "sending load test hash to manager: {}",
        goose_attack.task_sets_hash
    );
    push_stats_to_manager(&manager, &requests, false);

    // Only send load_test_hash one time.
    requests = HashMap::new();

    let mut hatch_rate: Option<f32> = None;
    let mut config: GooseConfiguration = GooseConfiguration::default();
    let mut weighted_users: Vec<GooseUser> = Vec::new();

    // Wait for the manager to send user parameters.
    loop {
        info!("waiting for instructions from manager");
        let msg = match manager.recv() {
            Ok(m) => m,
            Err(e) => {
                error!("unexpected error receiving manager message: {}", e);
                std::process::exit(1);
            }
        };
        let initializers: Vec<GooseUserInitializer> = match serde_cbor::from_reader(msg.as_slice())
        {
            Ok(i) => i,
            Err(_) => {
                let command: GooseUserCommand = match serde_cbor::from_reader(msg.as_slice()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("invalid message received: {}", e);
                        continue;
                    }
                };
                match command {
                    GooseUserCommand::EXIT => {
                        warn!("received EXIT command from manager");
                        std::process::exit(0);
                    }
                    other => {
                        info!("received unknown command from manager: {:?}", other);
                    }
                }
                continue;
            }
        };

        let mut worker_id: usize = 0;
        // Allocate a state for each user that will be spawned.
        info!("initializing user states...");
        for initializer in initializers {
            if worker_id == 0 {
                worker_id = initializer.worker_id;
            }
            let user = match GooseUser::new(
                initializer.task_sets_index,
                Url::parse(&initializer.base_url).unwrap(),
                initializer.min_wait,
                initializer.max_wait,
                &initializer.config,
                goose_attack.task_sets_hash,
            ) {
                Ok(u) => u,
                Err(e) => {
                    error!("[{}] failed to create GooseUser: {}", get_worker_id(), e);
                    std::process::exit(1);
                }
            };
            weighted_users.push(user);
            if hatch_rate == None {
                hatch_rate = Some(
                    1.0 / (initializer.config.hatch_rate as f32
                        / (initializer.config.expect_workers as f32)),
                );
                config = initializer.config;
                info!(
                    "[{}] prepared to start 1 user every {:.2} seconds",
                    worker_id,
                    hatch_rate.unwrap()
                );
            }
        }
        WORKER_ID.store(worker_id, Ordering::Relaxed);
        info!(
            "[{}] initialized {} user states",
            get_worker_id(),
            weighted_users.len()
        );
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
                error!(
                    "[{}] unexpected error receiving manager message: {}",
                    get_worker_id(),
                    e
                );
                std::process::exit(1);
            }
        };
        let command: GooseUserCommand = match serde_cbor::from_reader(msg.as_slice()) {
            Ok(c) => c,
            Err(e) => {
                error!("[{}] invalid message received: {}", get_worker_id(), e);
                continue;
            }
        };

        match command {
            // Break out of loop and start the load test.
            GooseUserCommand::RUN => break,
            // Exit worker process immediately.
            GooseUserCommand::EXIT => {
                warn!("[{}] received EXIT command from manager", get_worker_id());
                std::process::exit(0);
            }
            // Sleep and then loop again.
            _ => {
                let sleep_duration = time::Duration::from_secs(1);
                debug!(
                    "[{}] sleeping {:?} second waiting for manager...",
                    get_worker_id(),
                    sleep_duration
                );
                thread::sleep(sleep_duration);
            }
        }
    }

    // Worker is officially starting the load test.
    info!(
        "[{}] entering gaggle mode, starting load test",
        get_worker_id()
    );
    let sleep_duration = time::Duration::from_secs_f32(hatch_rate.unwrap());

    let mut worker_goose_attack = GooseAttack::initialize_with_config(config.clone());
    worker_goose_attack.started = Some(time::Instant::now());
    worker_goose_attack.task_sets = goose_attack.task_sets.clone();
    if config.run_time != "" {
        worker_goose_attack.run_time = util::parse_timespan(&config.run_time);
        info!(
            "[{}] run_time = {}",
            get_worker_id(),
            worker_goose_attack.run_time
        );
    } else {
        worker_goose_attack.run_time = 0;
    }
    worker_goose_attack.weighted_users = weighted_users;
    worker_goose_attack.configuration.worker = true;
    match worker_goose_attack
        .launch_users(sleep_duration, Some(manager))
        .await
    {
        Ok(w) => w,
        Err(e) => {
            error!("[{}] failed to launch GooseAttack: {}", get_worker_id(), e);
            std::process::exit(1);
        }
    }
}

pub fn push_stats_to_manager(
    manager: &Socket,
    requests: &HashMap<String, GooseRequest>,
    get_response: bool,
) -> bool {
    debug!(
        "[{}] pushing stats to manager: {}",
        get_worker_id(),
        requests.len()
    );
    let mut message = Message::new().unwrap();
    match serde_cbor::to_writer(&mut message, requests) {
        Ok(_) => (),
        Err(e) => {
            error!(
                "[{}] failed to serialize empty Vec<GooseRequest>: {}",
                get_worker_id(),
                e
            );
            std::process::exit(1);
        }
    }
    match manager.try_send(message) {
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
                error!(
                    "[{}] unexpected error receiving manager message: {}",
                    get_worker_id(),
                    e
                );
                std::process::exit(1);
            }
        };
        let command: GooseUserCommand = match serde_cbor::from_reader(msg.as_slice()) {
            Ok(c) => c,
            Err(e) => {
                error!("[{}] invalid message received: {}", get_worker_id(), e);
                std::process::exit(1);
            }
        };

        if command == GooseUserCommand::EXIT {
            info!("[{}] received EXIT command from manager", get_worker_id());
            // Shutting down, register shutdown pipe handler.
            match manager.pipe_notify(pipe_closed_during_shutdown) {
                Ok(_) => (),
                Err(e) => {
                    error!("failed to set up new pipe handler: {}", e);
                    std::process::exit(1);
                }
            }
            return false;
        }
    }
    true
}
