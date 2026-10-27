//! Without autostart, a run whose Controllers all fail to bind could never be
//! started, so it is refused instead of waiting.

use httpmock::MockServer;

use goose::prelude::*;

mod common;

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_no_autostart_without_a_listening_controller_is_refused() {
    let server = MockServer::start();
    let telnet = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let websocket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let telnet_port = telnet.local_addr().unwrap().port().to_string();
    let websocket_port = websocket.local_addr().unwrap().port().to_string();
    let configuration = common::build_configuration(
        &server,
        vec![
            "--no-autostart",
            "--telnet-host",
            "127.0.0.1",
            "--telnet-port",
            &telnet_port,
            "--websocket",
            "--websocket-host",
            "127.0.0.1",
            "--websocket-port",
            &websocket_port,
        ],
    );
    let goose_attack = common::build_load_test(
        configuration,
        vec![scenario!("LoadTest").register_transaction(transaction!(get_index))],
        None,
        None,
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), goose_attack.execute())
        .await
        .expect("the load test waited for a start that can not come");
    match result {
        Err(GooseError::InvalidOption { option, .. }) => {
            assert_eq!(option, "`configuration.no_autostart`")
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
}
