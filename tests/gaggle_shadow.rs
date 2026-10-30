//! `--gaggle-shadow-merge`: a run that merges its own deltas ends without
//! an error, which it returns on any difference or an unfinished check.
#![cfg(feature = "gaggle")]

use httpmock::{Method::GET, MockServer};

mod common;

use goose::prelude::*;

async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/").await?;
    Ok(())
}

#[tokio::test]
async fn test_gaggle_shadow_merge_matches() {
    let server = MockServer::start();
    let index = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200);
    });
    let configuration = common::build_configuration(
        &server,
        vec![
            "--users",
            "4",
            "--increase-rate",
            "4",
            "--run-time",
            "3",
            "--no-telnet",
            "--gaggle-shadow-merge",
        ],
    );
    let scenario = scenario!("Index").register_transaction(transaction!(get_index));
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        common::run_load_test(
            common::build_load_test(configuration, vec![scenario], None, None),
            None,
        ),
    )
    .await
    .expect("the run ends");
    assert!(index.calls() > 0);
}
