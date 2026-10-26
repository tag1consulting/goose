//! Users start and stop at the rate asked for, also when the interval
//! between them is longer than the 500 ms the main loop sleeps at most.

use httpmock::{Method::GET, MockServer};
use serial_test::serial;
use std::collections::HashMap;
use std::time::Duration;

mod common;

use goose::prelude::*;

const INDEX_PATH: &str = "/";

pub async fn get_index(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(INDEX_PATH).await?;
    Ok(())
}

/// The first and last transaction log times, in milliseconds since the run
/// started, of each user of a run of 9 users with `args`.
async fn user_times(args: Vec<&str>) -> Vec<(u64, u64)> {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path(INDEX_PATH);
        then.status(200);
    });
    let log = std::env::temp_dir().join(format!(
        "goose-user-rate-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let log_path = log.to_string_lossy().into_owned();
    let mut args = args;
    args.extend_from_slice(&["--users", "9", "--transaction-log", &log_path]);
    let configuration = common::build_configuration(&server, args);
    common::run_load_test(
        common::build_load_test(
            configuration,
            vec![scenario!("LoadTest")
                // Each user logs a transaction every 50 ms, from when it starts
                // until it stops.
                .set_wait_time(Duration::from_millis(50), Duration::from_millis(50))
                .unwrap()
                .register_transaction(transaction!(get_index))],
            None,
            None,
        ),
        None,
    )
    .await;

    let mut times: HashMap<u64, (u64, u64)> = HashMap::new();
    for line in std::fs::read_to_string(&log).unwrap().lines() {
        let entry: serde_json::Value = serde_json::from_str(line).unwrap();
        let user = entry["user"].as_u64().unwrap();
        let elapsed = entry["elapsed"].as_u64().unwrap();
        let time = times.entry(user).or_insert((elapsed, elapsed));
        time.0 = time.0.min(elapsed);
        time.1 = time.1.max(elapsed);
    }
    std::fs::remove_file(&log).ok();
    assert_eq!(times.len(), 9, "{:?}", times);
    times.into_values().collect()
}

/// At `--increase-rate 1.5` a user starts every 667 ms, so the ninth starts
/// 8 intervals, 5.3 s, after the first; rounded up to whole half seconds it
/// was 8 s.
#[tokio::test]
#[serial]
async fn users_start_at_the_increase_rate() {
    let times = user_times(vec!["--increase-rate", "1.5", "--run-time", "1"]).await;
    let first = times.iter().map(|time| time.0).min().unwrap();
    let ninth = times.iter().map(|time| time.0).max().unwrap();
    let span = ninth - first;
    assert!(
        (5_000..=6_000).contains(&span),
        "the ninth user started {} ms after the first: {:?}",
        span,
        times
    );
}

/// At `--decrease-rate 1.5` a user stops every 667 ms, so the ninth stops
/// 8 intervals, 5.3 s, after the first.
#[tokio::test]
#[serial]
async fn users_stop_at_the_decrease_rate() {
    let times = user_times(vec![
        "--increase-rate",
        "100",
        "--run-time",
        "1",
        "--decrease-rate",
        "1.5",
    ])
    .await;
    let first = times.iter().map(|time| time.1).min().unwrap();
    let ninth = times.iter().map(|time| time.1).max().unwrap();
    let span = ninth - first;
    assert!(
        (5_000..=6_000).contains(&span),
        "the ninth user stopped {} ms after the first: {:?}",
        span,
        times
    );
}
