//! `--iterations`: every user runs its scenario the configured number of
//! times, and a ramp down, which would stop users before they finish, is
//! refused with it.

use httpmock::{Method::GET, MockServer};
use serial_test::serial;

mod common;

use goose::config::GooseConfiguration;
use goose::prelude::*;

const INDEX_PATH: &str = "/";
const ABOUT_PATH: &str = "/about/";

// Each iteration of the scenario makes one request to each path.
const REQUESTS_PER_ITERATION: usize = 2;

const USERS: usize = 4;
const ITERATIONS: usize = 3;

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

pub async fn get_about(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(ABOUT_PATH).await?;
    Ok(())
}

/// A load test of 4 users, started at 4 a second, each running 3 iterations,
/// with `args` added to the command line.
fn load_test(server: &MockServer, args: &[&str]) -> GooseAttack {
    let host = server.base_url();
    let users = USERS.to_string();
    let iterations = ITERATIONS.to_string();
    let mut all = vec![
        "--host",
        host.as_str(),
        "--users",
        users.as_str(),
        "--increase-rate",
        "4",
        "--iterations",
        iterations.as_str(),
        "--co-mitigation",
        "disabled",
        "--quiet",
    ];
    all.extend_from_slice(args);
    let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
    let scenario = scenario!("LoadTest")
        .register_transaction(transaction!(get_index))
        .register_transaction(transaction!(get_about));
    common::build_load_test(configuration, vec![scenario], None, None)
}

#[tokio::test]
#[serial]
/// Without a ramp down, every user runs every iteration before the run ends.
async fn every_user_runs_every_iteration() {
    let server = MockServer::start();
    let index = server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    });
    let about = server.mock(|when, then| {
        when.method(GET).path(ABOUT_PATH);
        then.status(200);
    });

    let metrics = load_test(&server, &[]).execute().await.unwrap();

    assert_eq!(metrics.scenarios.len(), 1);
    assert_eq!(metrics.scenarios[0].run_times.counter, USERS * ITERATIONS);
    let requests: usize = metrics
        .requests
        .values()
        .map(|request| request.success_count + request.fail_count)
        .sum();
    assert_eq!(requests, USERS * ITERATIONS * REQUESTS_PER_ITERATION);
    assert_eq!(index.calls(), USERS * ITERATIONS);
    assert_eq!(about.calls(), USERS * ITERATIONS);
}

#[tokio::test]
#[serial]
/// A ramp down would stop users before they finish their iterations, so
/// `--decrease-rate` and `--decrease-time` are refused with `--iterations`
/// before any user starts.
async fn a_ramp_down_is_refused() {
    for (args, option) in [
        (["--decrease-rate", "4"], "`configuration.decrease_rate`"),
        (["--decrease-time", "1"], "`configuration.decrease_time`"),
    ] {
        let server = MockServer::start();
        let index = server.mock(|when, then| {
            when.method(GET).path(INDEX_PATH);
            then.status(200);
        });

        match load_test(&server, &args).execute().await {
            Err(GooseError::InvalidOption {
                option: refused,
                detail,
                ..
            }) => {
                assert_eq!(refused, option, "{:?}", args);
                assert_eq!(
                    detail,
                    format!("{} can not be set with `configuration.iterations`.", option),
                    "{:?}",
                    args
                );
            }
            other => panic!("{:?}: expected InvalidOption, got {:?}", args, other),
        }
        assert_eq!(index.calls(), 0, "{:?}", args);
    }
}
