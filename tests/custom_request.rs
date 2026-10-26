//! A custom request counts like any other request: in its row's requests,
//! failures and rates, and in the Aggregated row.

use serial_test::serial;
use std::time::Duration;

use goose::config::GooseConfiguration;
use goose::prelude::*;

/// Record one successful and one failed custom request, as a TCP load test
/// does, without a server.
pub async fn tcp_echo(user: &mut GooseUser) -> TransactionResult {
    user.record_custom_request_duration(
        "TCP",
        "echo",
        Duration::from_micros(400),
        true,
        None,
        None,
    )
    .await?;
    user.record_custom_request_duration(
        "TCP",
        "echo",
        Duration::from_micros(900),
        false,
        None,
        Some("connection refused"),
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn custom_requests_are_counted() {
    let dir = std::env::temp_dir().join(format!("goose-custom-request-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.json").to_string_lossy().into_owned();
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        "tcp://127.0.0.1:9",
        "--users",
        "2",
        "--increase-rate",
        "2",
        "--run-time",
        "2",
        "--no-reset-metrics",
        "--report-file",
        &report,
        "--quiet",
    ])
    .unwrap();
    let goose_metrics = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            scenario!("TcpUser")
                .set_wait_time(Duration::from_millis(10), Duration::from_millis(10))
                .unwrap()
                .register_transaction(transaction!(tcp_echo)),
        )
        .execute()
        .await
        .unwrap();

    let request = &goose_metrics.requests["TCP echo"];
    let runs = goose_metrics.transactions[0][0].success_count;
    assert!(runs > 0);
    assert_eq!(request.success_count, runs);
    assert_eq!(request.fail_count, runs);
    assert_eq!(request.raw_data.counter, 2 * runs);

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    let rows = json["raw_request_metrics"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{:?}", rows);
    for row in rows {
        assert_eq!(row["number_of_requests"], 2 * runs, "{}", row);
        assert_eq!(row["number_of_failures"], runs, "{}", row);
        assert!(
            row["requests_per_second"].as_f64().unwrap() > 0.0,
            "{}",
            row
        );
        assert!(
            row["failures_per_second"].as_f64().unwrap() > 0.0,
            "{}",
            row
        );
    }
    assert_eq!(rows[1]["name"], "Aggregated");

    // The console's per request table counts it too; with one request it has
    // no Aggregated row.
    let printed = goose_metrics.to_string();
    let counted = |name: &str| -> usize {
        let line = printed
            .lines()
            .find(|line| line.trim_start().starts_with(name) && line.contains('%'))
            .unwrap_or_else(|| panic!("no {} row: {}", name, printed));
        line.split('|')
            .nth(1)
            .unwrap()
            .trim()
            .replace(',', "")
            .parse()
            .unwrap()
    };
    assert_eq!(counted("TCP echo"), 2 * runs, "{}", printed);
    std::fs::remove_dir_all(&dir).ok();
}
