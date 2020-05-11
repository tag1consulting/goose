use crate::GooseState;

use nng::*;

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
    let ready: Message = "READY".as_bytes().into();
    match client.send(ready) {
        Ok(m) => m,
        Err(e) => {
            error!("communication failure to {}: {:?}.", &address, e);
            std::process::exit(1);
        }
    }
}
