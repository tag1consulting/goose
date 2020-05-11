use nng::*;

use crate::GooseState;
use crate::goose::{GooseRequest, GooseMethod};


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
    let ready = GooseRequest::new("/", GooseMethod::GET);
    serde_cbor::to_writer(&mut buf, &ready).unwrap();
    match client.send(&buf) {
        Ok(m) => m,
        Err(e) => {
            error!("communication failure to {}: {:?}.", &address, e);
            std::process::exit(1);
        }
    }

}
