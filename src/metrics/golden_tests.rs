//! A run of one process writes the same HTML, Markdown, JSON and console
//! reports, byte for byte, as Goose did before Gaggle worker health: the
//! files in `goldens/` were written by `main` at 3dcc817e from this same
//! test, with `GOOSE_UPDATE_GOLDENS=1`.

use super::*;
use crate::goose::{GooseMethod, TransactionName};
use std::sync::Arc;

/// Two requests, a transaction and a scenario, with a fixed run record and
/// no plan history, so no report holds a time of day.
fn metrics() -> GooseMetrics {
    let mut metrics = GooseMetrics {
        duration: 60,
        ..Default::default()
    };
    for (path, times) in [
        ("/", &[450u64, 520, 560, 590, 590, 640, 700, 820, 950][..]),
        ("/about", &[380, 610, 680, 730, 1_240][..]),
    ] {
        let mut aggregate = GooseRequestMetricAggregate::new(path, GooseMethod::Get, 0, "");
        for &us in times {
            aggregate.record_time_us(us, false);
        }
        aggregate.success_count = times.len() - 1;
        aggregate.fail_count = 1;
        metrics.requests.insert(format!("GET {path}"), aggregate);
    }
    let mut transaction = TransactionMetricAggregate::new(
        0,
        Arc::from("Scenario"),
        0,
        TransactionName::TransactionOnly(Arc::from("index")),
    );
    for us in [590, 640, 720] {
        transaction.set_time_us(us, true);
    }
    metrics.transactions = vec![vec![transaction]];
    let mut scenario = ScenarioMetricAggregate::new(0, Arc::from("Scenario"));
    scenario.update_us(1_230, 0);
    scenario.update_us(1_870, 1);
    metrics.scenarios = vec![scenario];
    metrics.display_metrics = true;
    metrics
}

/// Every report of [`metrics`], by file name.
fn reports() -> Vec<(&'static str, Vec<u8>)> {
    let metrics = metrics();
    let configuration = crate::GooseConfiguration::default();
    let record = crate::runs::RunRecord {
        goose_version: "golden".to_string(),
        test: "golden".to_string(),
        started: Some("2026-11-03T10:00:00Z".to_string()),
        ended: Some("2026-11-03T10:01:00Z".to_string()),
        ..Default::default()
    };
    let graph_data = crate::graph::GraphData::new();
    let source = ReportSource {
        configuration: &configuration,
        metrics: &metrics,
        graph_data: &graph_data,
        graphs: false,
        heatmap: None,
        loaded: None,
        record: &record,
        graph_data_in_json: false,
    };
    let mut json = Vec::new();
    source.write_json(&mut json, None).unwrap();
    let mut markdown = Vec::new();
    source.write_markdown(&mut markdown, None).unwrap();
    let html = source.html(None).unwrap().into_bytes();
    let console = metrics.to_string().into_bytes();
    vec![
        ("standalone.json", json),
        ("standalone.md", markdown),
        ("standalone.html", html),
        ("standalone.txt", console),
    ]
}

#[test]
fn standalone_reports_are_unchanged() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/metrics/goldens");
    let update = std::env::var_os("GOOSE_UPDATE_GOLDENS").is_some();
    for (name, bytes) in reports() {
        let path = folder.join(name);
        if update {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            continue;
        }
        let golden = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        assert!(
            golden == bytes,
            "{} differs from its golden:\n{}",
            name,
            String::from_utf8_lossy(&bytes)
        );
    }
}
