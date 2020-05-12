use std::{thread, time};

use nng::*;

use crate::GooseState;
use crate::goose::{GooseRequest, GooseClient, GooseClientCommand};
use crate::manager::GooseClientInitializer;

pub fn worker_main(state: &GooseState) {
    // Creates a TCP address. @TODO: add optional support for UDP.
    let address = format!("{}://{}:{}", "tcp", state.configuration.manager_bind_host, state.configuration.manager_bind_port);
    info!("worker connecting to manager at {}", &address);

    // Create a request socket.
    let client = match Socket::new(Protocol::Req0) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to create socket {}: {}.", &address, e);
            std::process::exit(1);
        }
    };

    // Connect to manager.
    match client.dial(&address) {
        Ok(d) => d,
        Err(e) => {
            error!("failed to create socket {}: {}.", &address, e);
            std::process::exit(1);
        }
    }

    // Let manager know we're ready to work.
    let mut buf: Vec<u8> = Vec::new();
    let requests: Vec<GooseRequest> = Vec::new();
    match serde_cbor::to_writer(&mut buf, &requests) {
        Ok(_) => (),
        Err(e) => {
            error!("failed to serialize empty Vec<GooseRequest>: {}", e);
            std::process::exit(1);
        }
    }

    match client.send(&buf) {
        Ok(m) => m,
        Err(e) => {
            error!("communication failure to {}: {:?}.", &address, e);
            std::process::exit(1);
        }
    }

    let mut hatch_rate: Option<f32> = None;

    // Wait for the manager to give us client parameters.
    loop {
        info!("waiting for instructions from manager");
        let msg = match client.recv() {
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

        // Allocate a state for each client that will be spawned.
        info!("initializing client states...");
        let mut weighted_clients = Vec::new();
        for initializer in initializers {
            weighted_clients.push(GooseClient::new(
                weighted_clients.len(),
                initializer.task_sets_index,
                initializer.default_host.clone(),
                initializer.task_set_host.clone(),
                initializer.min_wait,
                initializer.max_wait,
                &initializer.config,
            ));
            if hatch_rate == None {
                hatch_rate = Some(1.0 / (initializer.config.hatch_rate as f32 / (initializer.config.expect_workers as f32)));
                info!("prepared to start 1 client every {:.2} seconds", hatch_rate.unwrap());
            }
        }
        info!("initialized {} client states", weighted_clients.len());
        break;
    }

    info!("waiting for go-ahead from manager");
    // Tell manager we're ready to load test.
    loop {
        match client.send(&buf) {
            Ok(m) => m,
            Err(e) => {
                error!("communication failure to {}: {:?}.", &address, e);
                std::process::exit(1);
            }
        }
        let msg = match client.recv() {
            Ok(m) => m,
            Err(e) => {
                error!("unexpected error receiving manager message: {}", e);
                std::process::exit(1);
            }
        };
        let command: GooseClientCommand = match serde_cbor::from_reader(msg.as_slice()) {
            Ok(c) => c,
            Err(e) => {
                error!("invalid message received: {}", e);
                continue;
            }
        };

        match command {
            GooseClientCommand::RUN => break,
            _ => {
                let sleep_duration = time::Duration::from_secs(1);
                debug!("sleeping {:?} second waiting for manager...", sleep_duration);
                thread::sleep(sleep_duration);
            }
        }
    }
    // @TODO: perform actual work.
    info!("gaggle launching, load test initializing on all workers");
}
