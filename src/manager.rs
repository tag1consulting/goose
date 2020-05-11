use nng::*;

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

    // Currently loops forever receiving/printing utf8 messages.
    let mut msg;
    loop {
        msg = server.recv().unwrap();
        let test: GooseRequest = serde_cbor::from_reader(msg.as_slice()).unwrap();
        println!("{:?}", test);
    }
}
