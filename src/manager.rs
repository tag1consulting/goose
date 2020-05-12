use nng::*;

use std::collections::HashSet;

use crate::GooseState;
use crate::goose::GooseRequest;

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

    let mut workers: HashSet<Pipe> = HashSet::new();

    // Loop accepted connetions unitl we hear from all expected workers.
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

        let request: Vec<GooseRequest> = serde_cbor::from_reader(msg.as_slice()).unwrap();
        debug!("{:?}", request);

        if !workers.contains(&pipe) {
            workers.insert(pipe);
            info!("worker {} of {} connected", workers.len(), state.configuration.expect_workers);
        }

        if workers.len() == state.configuration.expect_workers as usize {
            info!("all workers have connected, starting load test..");
            // @TODO: start load test.
            break;
        }
    }
}
