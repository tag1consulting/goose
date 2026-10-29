//! A Controller that can't bind its port logs why, and the load test runs
//! without it. Its own test binary: the Goose log it reads is set up once per
//! process, by the first load test that runs.

use httpmock::{Method::GET, MockServer};

use goose::prelude::*;

mod common;

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_controller_bind_failure_is_logged() {
    const LOG_FILE: &str = "test-controller-bind-failure.log";
    let server = MockServer::start();
    let index = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });

    // Hold a port for each Controller.
    let telnet = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let websocket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let telnet_port = telnet.local_addr().unwrap().port().to_string();
    let websocket_port = websocket.local_addr().unwrap().port().to_string();

    let configuration = common::build_configuration(
        &server,
        vec![
            "--telnet-host",
            "127.0.0.1",
            "--telnet-port",
            &telnet_port,
            "--websocket",
            "--websocket-host",
            "127.0.0.1",
            "--websocket-port",
            &websocket_port,
            "--goose-log",
            LOG_FILE,
            "--run-time",
            "2",
        ],
    );
    let goose_metrics = common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario!("LoadTest").register_transaction(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;

    // The load test ran.
    assert!(index.calls() > 0);
    assert!(goose_metrics.duration >= 1);

    let log = std::fs::read_to_string(LOG_FILE).unwrap();
    common::cleanup_files(vec![LOG_FILE]);
    for (protocol, port) in [("Telnet", &telnet_port), ("WebSocket", &websocket_port)] {
        let line = log
            .lines()
            .find(|line| {
                line.contains("[ERROR]")
                    && line.contains(&format!("the {} Controller", protocol))
                    && line.contains(&format!("127.0.0.1:{}", port))
            })
            .unwrap_or_else(|| panic!("no {} bind error in {}", protocol, log));
        // The OS error.
        assert!(line.contains("in use"), "{}", line);
    }
}
