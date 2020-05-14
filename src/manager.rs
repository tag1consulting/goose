use nng::*;
use serde::{Serialize, Deserialize};

use std::collections::{HashMap, HashSet};

use crate::{GooseState, GooseConfiguration, GooseClientCommand};
use crate::goose::GooseRequest;

/// All elements required to initialize a client in a worker process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GooseClientInitializer {
    /// An index into the internal `GooseTest.task_sets` vector, indicating which GooseTaskSet is running.
    pub task_sets_index: usize,
    /// The global GooseState host.
    pub default_host: Option<String>,
    /// The GooseTaskSet.host.
    pub task_set_host: Option<String>,
    /// Minimum amount of time to sleep after running a task.
    pub min_wait: usize,
    /// Maximum amount of time to sleep after running a task.
    pub max_wait: usize,
    /// A local copy of the global GooseConfiguration.
    pub config: GooseConfiguration,
}

pub fn manager_main(state: &GooseState) {
    // Creates a TCP address. @TODO: add optional support for UDP.
    let address = format!("{}://{}:{}", "tcp", state.configuration.manager_bind_host, state.configuration.manager_bind_port);

    // Create a reply socket.
    let server = match Socket::new(Protocol::Rep0) {
        Ok(s) => s,
        Err(e) => {
            error!("failed to create {}://{}:{} socket: {}.", "tcp", state.configuration.manager_bind_host, state.configuration.manager_bind_port, e);
            std::process::exit(1);
        }
    };
    // Listen for connections.
    match server.listen(&address) {
        Ok(s) => (s),
        Err(e) => {
            error!("failed to bind to socket {}://{}:{}: {}.", "tcp", state.configuration.manager_bind_host, state.configuration.manager_bind_port, e);
            std::process::exit(1);
        }
    }
    info!("manager listening on {}, waiting for {} workers", &address, state.configuration.expect_workers);

    // Calculate how many clients each worker will be responsible for.
    let split_clients = state.clients / (state.configuration.expect_workers as usize);
    let mut split_clients_remainder = state.clients % (state.configuration.expect_workers as usize);
    if split_clients_remainder > 0 {
        info!("each worker to start {} clients, assigning 1 extra to {} workers", split_clients, split_clients_remainder);
    }
    else {
        info!("each worker to start {} clients", split_clients);
    }

    // A mutable bucket of clients to be assigned to workers.
    let mut available_clients = state.weighted_clients.clone();

    // Track how many workers we've seen.
    let mut workers: HashSet<Pipe> = HashSet::new();

    // Worker control loop.
    let mut msg;
    loop {
        msg = match server.recv() {
            Ok(m) => m,
            Err(e) => {
                error!("unexpected error receiving client message: {}", e);
                std::process::exit(1);
            }
        };

        let pipe = match msg.pipe() {
            Some(p) => p,
            None => {
                error!("unexpected fatal error reading worker pipe");
                std::process::exit(1);
            }
        };

        let requests: HashMap<String, GooseRequest> = serde_cbor::from_reader(msg.as_slice()).unwrap();
        debug!("requests statistics received: {:?}", requests.len());

        // We've seen this worker before.
        if workers.contains(&pipe) {
            let mut buf: Vec<u8> = Vec::new();
            // All workers are running load test, sending statistics.
            if workers.len() == state.configuration.expect_workers as usize {
                // @TODO: merge in statistics
                if requests.len() > 0 {
                    info!("requests statistics received: {:?}", requests.len());
                }
                match serde_cbor::to_writer(&mut buf, &GooseClientCommand::RUN) {
                    Ok(_) => (),
                    Err(e) => {
                        error!("failed to serialize client command: {}", e);
                        std::process::exit(1);
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
            match server.send(message) {
                Ok(m) => m,
                Err(e) => {
                    error!("communication failure: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        // This is the first time we've seen this worker.
        else {
            // Make sure we're not already conneted to all of our workers.
            if workers.len() >= state.configuration.expect_workers as usize {
                // We already have enough workers, tell this one to EXIT.
                let mut buf: Vec<u8> = Vec::new();
                match serde_cbor::to_writer(&mut buf, &GooseClientCommand::EXIT) {
                    Ok(_) => (),
                    Err(e) => {
                        error!("failed to serialize client command: {}", e);
                        std::process::exit(1);
                    }
                }
                let message: Message = buf.as_slice().into();
                match server.send(message) {
                    Ok(m) => m,
                    Err(e) => {
                        error!("communication failure: {:?}", e);
                        std::process::exit(1);
                    }
                }
            }
            // We need another worker, accept the connection.
            else {
                workers.insert(pipe);
                if workers.len() == state.configuration.expect_workers as usize {
                    info!("all {} workers have connected", state.configuration.expect_workers);
                }
                else {
                    info!("worker {} of {} connected", workers.len(), state.configuration.expect_workers);
                }

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
                info!("sending worker-{} {} clients", workers.len(), clients.len());
                let message: Message = buf.as_slice().into();
                match server.send(message) {
                    Ok(m) => m,
                    Err(e) => {
                        error!("communication failure: {:?}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
    }
}
