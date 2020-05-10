use crate::GooseState;

use std::convert::TryInto;

use libzmq::{prelude::*, *};

pub fn worker_main(state: &GooseState) {
    info!("worker connecting to manager at {}:{}", state.configuration.manager_host, state.configuration.manager_port);
    // Build configured address and port.
    let addr: TcpAddr = match format!("{}:{}", state.configuration.manager_host, state.configuration.manager_port).try_into() {
        Ok(a) => a,
        Err(e) => {
            error!("failed to parse address '{}' and port '{}': {}.", state.configuration.manager_host, state.configuration.manager_port, e);
            std::process::exit(1);
        }
    };
    // Connect to manager.
    let client = match ClientBuilder::new().connect(addr).build() {
        Ok(c) => c,
        Err(e) => {
            error!("Worker failed to connect to manager: {}.", e);
            std::process::exit(1);
        }
    };
    debug!("client: {:?}", &client);
    let foo = client.send("READY");
    println!("Ok?({:?})", foo);
}