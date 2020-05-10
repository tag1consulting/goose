use crate::GooseState;

use std::convert::TryInto;

use libzmq::{prelude::*, *};

pub fn manager_main(state: &GooseState) {
    // Build configured address and port.
    let addr: TcpAddr = match format!("{}:{}", state.configuration.manager_bind_host, state.configuration.manager_bind_port).try_into() {
        Ok(a) => a,
        Err(e) => {
            error!("failed to parse address '{}' and port '{}': {}.", state.configuration.manager_bind_host, state.configuration.manager_bind_port, e);
            std::process::exit(1);
        }
    };
    // Bind to configured address and port.
    let mananger = match ServerBuilder::new().bind(&addr).build() {
        Ok(s) => s,
        Err(e) => {
            error!("manager failed to bind to '{}': {}.", &addr, e);
            std::process::exit(1);
        }
    };
    info!("manager waiting for {} workers on {}:{}", state.configuration.expect_workers, state.configuration.manager_bind_host, state.configuration.manager_bind_port);
    // Confirm bound address, primarily for debug.
    let bound = match mananger.last_endpoint() {
        Ok(b) => b,
        Err(e) => {
            error!("manager failed to get bound address: {}.", e);
            std::process::exit(1);
        }
    };
    debug!("bound: {:?}", bound);
    let msg = mananger.recv_msg().unwrap();
    info!("msg: {:?}", msg);
}