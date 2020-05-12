use std::{thread, time};

use nng::*;

use crate::GooseState;
use crate::goose::GooseRequest;

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

    // Wait for the manager to give us client parameters.
    info!("waiting for instructions from manager");
    let _msg = match client.recv() {
        Ok(m) => m,
        Err(e) => {
            error!("unexpected error receiving manager message: {}", e);
            std::process::exit(1);
        }
    };

    // @TODO: perform actual work.
    loop {
        let sleep_duration = time::Duration::from_secs(5);
        info!("client sleeping {:?} second...", sleep_duration);
        thread::sleep(sleep_duration);
    }
}
