//! Without `--dashboard` a run serves no dashboard, whether or not Goose was
//! built with the `dashboard` feature, which is one of its default features:
//! nothing binds the dashboard's default address, `127.0.0.1:5118`.
//!
//! The test holds that address while a load test runs, or finds another
//! program already holding it. Either way a dashboard that tried to listen
//! there would fail to bind, and a failed dashboard bind ends the run with an
//! error, so a run that succeeds never tried. The unit
//! test `setup_dashboard_without_the_option_binds_nothing` in
//! `src/dashboard.rs` shows the same at the function that decides it, which
//! returns before it binds or spawns anything.

use httpmock::{Method::GET, MockServer};

use goose::prelude::*;

use crate::common;

/// The dashboard's default address, which `--dashboard` binds unless
/// `--dashboard-host` or `--dashboard-port` say otherwise.
const DEFAULT_DASHBOARD_ADDRESS: &str = "127.0.0.1:5118";

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn a_run_without_the_option_binds_no_dashboard_port() {
    // Held for the whole run. No other test binds this address: each takes a
    // free port of its own for the dashboard. When the bind fails, another
    // program already holds the address, which serves the test as well.
    let held = std::net::TcpListener::bind(DEFAULT_DASHBOARD_ADDRESS).ok();
    if held.is_none() {
        assert!(
            std::net::TcpStream::connect(DEFAULT_DASHBOARD_ADDRESS).is_ok(),
            "{} can be neither bound nor reached",
            DEFAULT_DASHBOARD_ADDRESS
        );
    }

    let server = MockServer::start();
    let index = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let configuration = common::build_configuration(
        &server,
        vec!["--users", "2", "--run-time", "2", "--no-telnet"],
    );
    assert!(!configuration.dashboard);

    let goose_metrics = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("Index").register_transaction(transaction!(get_index)))
        .execute()
        .await
        .expect("a run without --dashboard does not bind the dashboard's port");

    assert!(index.calls() > 0);
    assert!(!goose_metrics.requests.is_empty());
    // The test's own listener, when it holds one, is still on the address.
    if let Some(held) = held {
        assert_eq!(
            held.local_addr().unwrap().to_string(),
            DEFAULT_DASHBOARD_ADDRESS
        );
    }
}
