//! Validate `--threshold`: rules that pass, rules that stop the run, and the
//! Thresholds section of every report.
use httpmock::{Method::GET, Mock, MockServer};
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod common;

use goose::config::GooseConfiguration;
use goose::metrics::GooseMetrics;
use goose::prelude::*;
use goose::threshold::ThresholdResult;

// Paths used in load tests performed during these tests.
const OK_PATH: &str = "/ok";
const FAIL_PATH: &str = "/fail";
const SLOW_PATH: &str = "/slow";

// How long a run lasts unless a threshold stops it.
const LONG_RUN_TIME: &str = "30";

// A run a threshold stopped ends well before LONG_RUN_TIME.
const ABORTED_WITHIN: Duration = Duration::from_secs(15);

// Test transactions.
pub async fn get_ok(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(OK_PATH).await?;
    Ok(())
}

pub async fn get_fail(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(FAIL_PATH).await?;
    Ok(())
}

pub async fn get_slow(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SLOW_PATH).await?;
    Ok(())
}

// When each request of `get_slow_timed` finished.
static FINISHED: Mutex<Vec<chrono::DateTime<chrono::Utc>>> = Mutex::new(Vec::new());

pub async fn get_slow_timed(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get(SLOW_PATH).await?;
    FINISHED.lock().unwrap().push(chrono::Utc::now());
    Ok(())
}

// All tests in this file run against these endpoints.
fn setup_mock_server_endpoints(server: &MockServer) -> Vec<Mock<'_>> {
    vec![
        server.mock(|when, then| {
            when.method(GET).path(OK_PATH);
            then.status(200);
        }),
        server.mock(|when, then| {
            when.method(GET).path(FAIL_PATH);
            then.status(500);
        }),
        server.mock(|when, then| {
            when.method(GET).path(SLOW_PATH);
            then.status(200).delay(Duration::from_millis(300));
        }),
    ]
}

/// A load test of `transactions`, with `args` on the command line.
fn load_test(server: &MockServer, args: Vec<&str>, transactions: Vec<Transaction>) -> GooseAttack {
    let configuration = common::build_configuration(server, args);
    let mut scenario = scenario!("LoadTest");
    for transaction in transactions {
        scenario = scenario.register_transaction(transaction);
    }
    common::build_load_test(configuration, vec![scenario], None, None)
}

/// A load test of `transactions` with only `args`, the host and `--quiet` on the
/// command line, for options the common defaults conflict with, such as
/// `--iterations` and `--test-plan`.
fn bare_load_test(
    server: &MockServer,
    args: Vec<&str>,
    transactions: Vec<Transaction>,
) -> GooseAttack {
    let host = server.base_url();
    let mut all = vec![
        "--host",
        host.as_str(),
        "--co-mitigation",
        "disabled",
        "--quiet",
    ];
    all.extend(args);
    let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
    let mut scenario = scenario!("LoadTest");
    for transaction in transactions {
        scenario = scenario.register_transaction(transaction);
    }
    common::build_load_test(configuration, vec![scenario], None, None)
}

/// How long `goose_attack` ran, and what it returned.
async fn run(goose_attack: GooseAttack) -> (Duration, Result<GooseMetrics, GooseError>) {
    let started = Instant::now();
    let result = goose_attack.execute().await;
    (started.elapsed(), result)
}

/// The failed rules and the metrics of a run that must have failed a threshold.
fn thresholds_failed(
    result: Result<GooseMetrics, GooseError>,
) -> (Vec<ThresholdResult>, GooseMetrics) {
    match result {
        Err(GooseError::ThresholdsFailed { failed, metrics }) => (failed, *metrics.0),
        Err(other) => panic!("expected ThresholdsFailed, got {:?}", other),
        Ok(metrics) => panic!(
            "expected ThresholdsFailed, got Ok: {:?}",
            metrics.thresholds
        ),
    }
}

/// A path for a report in the system's temporary folder.
fn report_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("goose-thresholds-{}-{name}", std::process::id()))
}

#[tokio::test]
#[serial]
/// A passing rule returns Ok, with its result on the metrics.
async fn passing_rules_return_ok() {
    let server = MockServer::start();
    let mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec![
            "--run-time",
            "2",
            "--threshold",
            "error-rate<1%",
            "--threshold",
            "p95<5s",
            "--threshold",
            "rps>0",
        ],
        vec![transaction!(get_ok)],
    );
    let (_, result) = run(goose_attack).await;
    let metrics = result.expect("every rule passes");

    assert!(mocks[0].calls() > 0);
    let rules: Vec<(&str, bool, bool)> = metrics
        .thresholds
        .iter()
        .map(|result| (result.rule.as_str(), result.passed, result.aborted))
        .collect();
    assert_eq!(
        rules,
        [
            ("error-rate<1%", true, false),
            ("p95<5s", true, false),
            ("rps>0", true, false)
        ]
    );
    assert_eq!(metrics.thresholds[0].observed, "0%");
    assert_eq!(metrics.thresholds[0].value, Some(0.0));
}

#[tokio::test]
#[serial]
/// A failing error-rate stops the run well before its run time, and the error
/// holds the metrics.
async fn failing_rule_aborts_the_run() {
    let server = MockServer::start();
    let mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            LONG_RUN_TIME,
            "--threshold",
            "error-rate<1%",
            "--threshold",
            "p95<5s",
        ],
        vec![transaction!(get_fail)],
    );
    let (elapsed, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);

    assert!(elapsed < ABORTED_WITHIN, "ran {:?}", elapsed);
    assert!(mocks[1].calls() > 0);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].rule, "error-rate<1%");
    assert_eq!(failed[0].observed, "100%");
    assert!(failed[0].aborted);
    // The metrics hold every rule's result, and the requests.
    assert_eq!(metrics.thresholds.len(), 2);
    assert!(metrics.thresholds[1].passed);
    assert!(!metrics.thresholds[1].aborted);
    assert!(metrics.requests.contains_key("GET /fail"));
}

#[tokio::test]
#[serial]
/// Under --no-threshold-abort a failing rule runs its full time, then fails the run.
async fn no_threshold_abort_runs_the_full_time() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "3",
            "--threshold",
            "error-rate<1%",
            "--no-threshold-abort",
        ],
        vec![transaction!(get_fail)],
    );
    let (elapsed, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);

    assert!(elapsed >= Duration::from_secs(3), "ran {:?}", elapsed);
    assert!(metrics.duration >= 3);
    assert_eq!(failed.len(), 1);
    assert!(!failed[0].aborted);
}

#[tokio::test]
#[serial]
/// --threshold-abort-delay holds the first check back, then the rule still stops
/// the run well before its run time.
async fn abort_delay_holds_back_the_first_check() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            LONG_RUN_TIME,
            "--threshold",
            "error-rate<1%",
            "--threshold-abort-delay",
            "3",
        ],
        vec![transaction!(get_fail)],
    );
    let (elapsed, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);

    // The metrics reset, and checking starts, once both users run, so the run's
    // duration counts from there.
    assert!(metrics.duration >= 3, "duration {}", metrics.duration);
    assert!(elapsed >= Duration::from_secs(3), "ran {:?}", elapsed);
    assert!(elapsed < ABORTED_WITHIN, "ran {:?}", elapsed);
    assert!(failed[0].aborted);
}

#[tokio::test]
#[serial]
/// A users ramp that resets its metrics is not judged while users start: every
/// user starts before the failing rule stops the run.
async fn nothing_aborts_during_the_increase() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "4",
            "--increase-rate",
            "1",
            "--run-time",
            LONG_RUN_TIME,
            "--threshold",
            "error-rate<1%",
        ],
        vec![transaction!(get_fail)],
    );
    let (elapsed, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);

    assert_eq!(metrics.maximum_users, 4);
    assert!(elapsed >= Duration::from_secs(3), "ran {:?}", elapsed);
    assert!(elapsed < ABORTED_WITHIN, "ran {:?}", elapsed);
    assert!(failed[0].aborted);
}

#[tokio::test]
#[serial]
/// A rule for one request judges only that request: it fails while a rule for
/// every request passes.
async fn rule_for_one_request() {
    let server = MockServer::start();
    let mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "2",
            "--no-threshold-abort",
            "--threshold",
            "max<200ms for GET /slow",
            "--threshold",
            "max<200ms for GET /ok",
            "--threshold",
            "p50<5s",
        ],
        vec![transaction!(get_ok), transaction!(get_slow)],
    );
    let (_, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);

    assert!(mocks[0].calls() > 0);
    assert!(mocks[2].calls() > 0);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].rule, "max<200ms for GET /slow");
    assert!(failed[0].value.unwrap() >= 300.0);
    let passed: Vec<&str> = metrics
        .thresholds
        .iter()
        .filter(|result| result.passed)
        .map(|result| result.rule.as_str())
        .collect();
    assert_eq!(passed, ["max<200ms for GET /ok", "p50<5s"]);
}

#[tokio::test]
#[serial]
/// A rule for a request that never runs is skipped while the run goes, so it
/// stops nothing, and fails at the end with no samples.
async fn rule_for_a_request_that_never_runs() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let goose_attack = load_test(
        &server,
        vec!["--run-time", "3", "--threshold", "p95<1s for GET /never"],
        vec![transaction!(get_ok)],
    );
    let (elapsed, result) = run(goose_attack).await;
    let (failed, _metrics) = thresholds_failed(result);

    assert!(elapsed >= Duration::from_secs(3), "ran {:?}", elapsed);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].observed, "no samples");
    assert_eq!(failed[0].value, None);
    assert!(!failed[0].aborted);
}

#[tokio::test]
#[serial]
/// Without thresholds, no report has a Thresholds section.
async fn no_thresholds_no_section() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let html = report_path("none.html");
    let markdown = report_path("none.md");
    let json = report_path("none.json");
    let goose_attack = load_test(
        &server,
        vec![
            "--report-file",
            html.to_str().unwrap(),
            "--report-file",
            markdown.to_str().unwrap(),
            "--report-file",
            json.to_str().unwrap(),
        ],
        vec![transaction!(get_ok)],
    );
    let (_, result) = run(goose_attack).await;
    let metrics = result.unwrap();
    assert!(metrics.thresholds.is_empty());

    let html_text = std::fs::read_to_string(&html).unwrap();
    let markdown_text = std::fs::read_to_string(&markdown).unwrap();
    let json_text = std::fs::read_to_string(&json).unwrap();
    for path in [&html, &markdown, &json] {
        let _ = std::fs::remove_file(path);
    }
    assert!(html_text.contains("<h2>Request Metrics</h2>"));
    assert!(!html_text.contains("Thresholds"));
    assert!(!html_text.contains("threshold-"));
    assert!(markdown_text.contains("## Request Metrics"));
    assert!(!markdown_text.contains("Thresholds"));
    assert!(!json_text.contains("\"thresholds\""));
}

/// The rows of the HTML and Markdown Thresholds sections of `html` and `markdown`.
fn sections(html: &str, markdown: &str) -> (Vec<String>, Vec<String>) {
    let html_section = html
        .split("<h2>Thresholds</h2>")
        .nth(1)
        .expect("an HTML Thresholds section")
        .split("</table>")
        .next()
        .unwrap();
    let html_rows = html_section
        .split("<tr class=\"")
        .skip(1)
        .map(|row| {
            row.split("<td")
                .skip(1)
                .map(|cell| {
                    let text = cell.split_once('>').unwrap().1;
                    text.split("</td>").next().unwrap().trim().to_string()
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect();
    let markdown_rows = markdown
        .split("## Thresholds")
        .nth(1)
        .expect("a Markdown Thresholds section")
        .lines()
        .skip(4)
        .take_while(|line| line.starts_with('|'))
        .map(str::to_string)
        .collect();
    (html_rows, markdown_rows)
}

#[tokio::test]
#[serial]
/// Every report of a run with thresholds has the section, and a JSON report
/// rendered with --render-report shows the same results.
async fn reports_show_thresholds_and_render_them_again() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let html = report_path("run.html");
    let markdown = report_path("run.md");
    let json = report_path("run.json");
    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            "2",
            "--no-threshold-abort",
            "--threshold",
            "error-rate<1%",
            "--threshold",
            "p95<5s for GET /ok",
            "--report-file",
            html.to_str().unwrap(),
            "--report-file",
            markdown.to_str().unwrap(),
            "--report-file",
            json.to_str().unwrap(),
        ],
        vec![transaction!(get_ok), transaction!(get_fail)],
    );
    let (_, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);
    assert_eq!(failed.len(), 1);
    assert_eq!(metrics.thresholds.len(), 2);

    let (html_rows, markdown_rows) = sections(
        &std::fs::read_to_string(&html).unwrap(),
        &std::fs::read_to_string(&markdown).unwrap(),
    );
    let observed = &metrics.thresholds[0].observed;
    assert_eq!(
        html_rows,
        [
            format!("<code>error-rate&lt;1%</code> | {observed} | &#10007; FAIL"),
            format!(
                "<code>p95&lt;5s for GET /ok</code> | {} | &#10003; PASS",
                metrics.thresholds[1].observed
            ),
        ]
    );
    assert_eq!(
        markdown_rows,
        [
            format!("| error-rate\\<1% | {observed} | **FAIL** |"),
            format!(
                "| p95\\<5s for GET /ok | {} | PASS |",
                metrics.thresholds[1].observed
            ),
        ]
    );

    // Render the JSON report again, as HTML and Markdown.
    let rendered_html = report_path("rendered.html");
    let rendered_markdown = report_path("rendered.md");
    let configuration = GooseConfiguration::parse_args_default(&[
        "--render-report",
        json.to_str().unwrap(),
        "--report-file",
        rendered_html.to_str().unwrap(),
        "--report-file",
        rendered_markdown.to_str().unwrap(),
        "--quiet",
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("Render").register_transaction(transaction!(get_ok)))
        .execute()
        .await
        .expect("rendering a report never fails a threshold");
    let rendered = sections(
        &std::fs::read_to_string(&rendered_html).unwrap(),
        &std::fs::read_to_string(&rendered_markdown).unwrap(),
    );
    for path in [&html, &markdown, &json, &rendered_html, &rendered_markdown] {
        let _ = std::fs::remove_file(path);
    }
    assert_eq!(rendered, (html_rows, markdown_rows));
}

#[tokio::test]
#[serial]
/// Without a metrics reset, under --no-reset-metrics or --iterations, checking
/// still waits for the run to enter Maintain: every user starts before the failing
/// rule stops the run, well before its run time.
async fn checks_wait_for_maintain_without_a_reset() {
    for mode in [
        vec!["--no-reset-metrics", "--run-time", LONG_RUN_TIME],
        vec!["--iterations", "1000000"],
    ] {
        let server = MockServer::start();
        let _mocks = setup_mock_server_endpoints(&server);

        let mut args = vec![
            "--users",
            "4",
            "--increase-rate",
            "2",
            "--threshold",
            "error-rate<1%",
        ];
        args.extend(mode.iter().copied());
        let goose_attack = bare_load_test(&server, args, vec![transaction!(get_fail)]);
        // Under --iterations a run checking never stopped would last for hours.
        let (elapsed, result) = tokio::time::timeout(2 * ABORTED_WITHIN, run(goose_attack))
            .await
            .expect("the threshold stops the run");
        let (failed, metrics) = thresholds_failed(result);

        assert_eq!(metrics.maximum_users, 4, "{mode:?}");
        assert!(
            elapsed >= Duration::from_millis(1_500),
            "{:?}: ran {:?}",
            mode,
            elapsed
        );
        assert!(elapsed < ABORTED_WITHIN, "{:?}: ran {:?}", mode, elapsed);
        assert_eq!(failed[0].rule, "error-rate<1%");
        assert!(failed[0].aborted, "{:?}", mode);
    }
}

#[tokio::test]
#[serial]
/// An rps rule counts from Maintain: a test plan whose hold makes the rate but
/// whose ramp up drags the whole run's average under it passes, without stopping
/// the run.
async fn rps_counts_from_maintain() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    // Each user makes at most about 3.3 requests a second to the 300 ms
    // endpoint, so 10 users hold about 33 a second. Over the whole run, a ramp
    // up of 16 seconds to 10 users, one every 1.6 seconds, is 88 user seconds,
    // and a hold of 4 to 4.5 seconds is at most 45 more: at most
    // 133 * 3.33 / 20, about 22 a second.
    let goose_attack = bare_load_test(
        &server,
        vec!["--test-plan", "10,16s;10,4s;0,0", "--threshold", "rps>=25"],
        vec![transaction!(get_slow)],
    );
    let (_, result) = run(goose_attack).await;
    let metrics = result.expect("the hold makes the rate");

    let rule = &metrics.thresholds[0];
    assert!(rule.passed, "{:?}", rule);
    assert!(!rule.aborted);
    assert!(rule.observed.ends_with(" from Maintain"), "{:?}", rule);
    // The whole run's rate is under the rule's limit.
    let requests: usize = metrics
        .requests
        .values()
        .map(|request| request.success_count + request.fail_count)
        .sum();
    let whole_run = requests as f64 / metrics.duration as f64;
    assert!(whole_run < 25.0, "{}", whole_run);
}

/// The requests made, successful or failed.
fn requests_made(metrics: &GooseMetrics) -> usize {
    metrics
        .requests
        .values()
        .map(|request| request.success_count + request.fail_count)
        .sum()
}

#[tokio::test]
#[serial]
/// Under --no-threshold-abort an rps rule still counts only the hold, as with
/// abort on: its row matches the same run's with abort on, and sits well above
/// the whole run's rate, which a ramp up kept without a metrics reset drags down.
async fn no_threshold_abort_rps_counts_only_the_hold() {
    let mut rates = Vec::new();
    for mode in [&[][..], &["--no-threshold-abort"][..]] {
        let server = MockServer::start();
        let _mocks = setup_mock_server_endpoints(&server);

        // Each user makes at most about 3.3 requests a second to the 300 ms
        // endpoint. A ramp up of 5 seconds to 10 users, 2 a second, is about 25
        // user seconds; a hold of 5 seconds is 50 more. The hold makes about 33
        // requests a second, the whole run about 75 * 3.3 / 10, or 25.
        let mut args = vec![
            "--users",
            "10",
            "--increase-rate",
            "2",
            "--run-time",
            "5",
            "--no-reset-metrics",
            "--threshold",
            "rps>=1",
        ];
        args.extend_from_slice(mode);
        let goose_attack = load_test(&server, args, vec![transaction!(get_slow)]);
        let (_, result) = run(goose_attack).await;
        let metrics = result.expect("the hold makes the rate");

        let rule = &metrics.thresholds[0];
        assert!(rule.passed, "{:?}: {:?}", mode, rule);
        assert!(
            rule.observed.ends_with(" from Maintain"),
            "{:?}: {:?}",
            mode,
            rule
        );
        let held = rule.value.expect("the hold has a rate");
        let whole_run = requests_made(&metrics) as f64 / metrics.duration as f64;
        assert!(
            held > whole_run * 1.15,
            "{:?}: held {} against {} for the whole run",
            mode,
            held,
            whole_run
        );
        rates.push(held);
    }
    let (aborts, judged_at_end) = (rates[0], rates[1]);
    assert!(
        (aborts - judged_at_end).abs() <= aborts * 0.05,
        "{} with abort, {} without",
        aborts,
        judged_at_end
    );
}

#[tokio::test]
#[serial]
/// An rps rule counts every hold of a test plan, and not the Increase step
/// between two of them: its rate is the requests finished in the holds over the
/// seconds held, and not that rate with the Increase step's requests counted.
async fn rps_skips_an_increase_between_holds() {
    use goose::test_plan::TestPlanStepAction;

    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);
    FINISHED.lock().unwrap().clear();

    // A long hold at 1 user, a long Increase step to 20, and a short hold at 20:
    // about 3.3 requests a second for 8 seconds and 66 for 2, so the holds make
    // about 13 a second together, while the Increase step makes about 35.
    let goose_attack = bare_load_test(
        &server,
        vec![
            "--test-plan",
            "1,1s;1,8s;20,8s;20,2s;0,0",
            "--threshold",
            "rps>=1",
        ],
        vec![transaction!(get_slow_timed)],
    );
    let (_, result) = run(goose_attack).await;
    let metrics = result.expect("the holds make the rate");
    let held = metrics.thresholds[0].value.expect("the holds have a rate");

    // Each hold, from the history: from a Maintaining step to the step after it.
    let holds: Vec<_> = metrics
        .history
        .windows(2)
        .filter(|pair| matches!(pair[0].action, TestPlanStepAction::Maintaining))
        .map(|pair| (pair[0].timestamp, pair[1].timestamp))
        .collect();
    assert_eq!(holds.len(), 2, "{:?}", metrics.history);
    let finished = FINISHED.lock().unwrap().clone();
    let within = |from, to| finished.iter().filter(|&&at| at >= from && at < to).count();
    let seconds = |from: chrono::DateTime<chrono::Utc>, to: chrono::DateTime<chrono::Utc>| {
        (to - from).num_microseconds().unwrap() as f64 / 1e6
    };
    let in_holds: usize = holds.iter().map(|&(from, to)| within(from, to)).sum();
    let held_seconds: f64 = holds.iter().map(|&(from, to)| seconds(from, to)).sum();
    let expected = in_holds as f64 / held_seconds;
    assert!(
        (held - expected).abs() <= expected * 0.1,
        "observed {}, the holds made {}",
        held,
        expected
    );
    // Counting the Increase step's requests and seconds would raise it well past.
    let (increase_from, increase_to) = (holds[0].1, holds[1].0);
    let increase = within(increase_from, increase_to);
    assert!(increase > 0);
    let with_increase =
        (in_holds + increase) as f64 / (held_seconds + seconds(increase_from, increase_to));
    assert!(
        with_increase > held * 1.25,
        "observed {}, with the Increase step {}",
        held,
        with_increase
    );
}

#[tokio::test]
#[serial]
/// A test plan that never holds its users steady has no time an rps rule
/// counts: the rule fails at the end as `no samples`, while the other rules are
/// judged over the whole run.
async fn rps_without_a_hold_has_no_samples() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let goose_attack = bare_load_test(
        &server,
        vec![
            "--test-plan",
            "4,3s;0,0",
            "--threshold",
            "rps>0",
            "--threshold",
            "p95<5s",
        ],
        vec![transaction!(get_ok)],
    );
    let (_, result) = run(goose_attack).await;
    let (failed, metrics) = thresholds_failed(result);

    assert!(requests_made(&metrics) > 0);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].rule, "rps>0");
    assert_eq!(failed[0].observed, "no samples");
    assert_eq!(failed[0].value, None);
    assert!(!failed[0].aborted);
    assert!(metrics.thresholds[1].passed, "{:?}", metrics.thresholds[1]);
}

#[tokio::test]
#[serial]
/// The rule that stopped the run says so in the console's Thresholds table and in
/// every report, and a JSON report rendered again says the same.
async fn reports_show_the_rule_that_stopped_the_run() {
    let server = MockServer::start();
    let _mocks = setup_mock_server_endpoints(&server);

    let html = report_path("aborted.html");
    let markdown = report_path("aborted.md");
    let json = report_path("aborted.json");
    let goose_attack = load_test(
        &server,
        vec![
            "--users",
            "2",
            "--increase-rate",
            "2",
            "--run-time",
            LONG_RUN_TIME,
            "--threshold",
            "error-rate<1%",
            "--threshold",
            "p95<5s",
            "--report-file",
            html.to_str().unwrap(),
            "--report-file",
            markdown.to_str().unwrap(),
            "--report-file",
            json.to_str().unwrap(),
        ],
        vec![transaction!(get_fail)],
    );
    let (elapsed, result) = run(goose_attack).await;
    let (_, metrics) = thresholds_failed(result);
    assert!(elapsed < ABORTED_WITHIN, "ran {:?}", elapsed);
    let aborted = &metrics.thresholds[0];
    assert!(aborted.aborted && !aborted.passed);
    assert!(aborted.observed.starts_with("100%"), "{:?}", aborted);
    let passed = &metrics.thresholds[1];

    let console = format!("{metrics}");
    let table = console
        .split(" === THRESHOLDS ===\n")
        .nth(1)
        .expect("a console Thresholds table");
    let rows: Vec<&str> = table.lines().skip(3).take(2).collect();
    assert_eq!(
        rows,
        [
            format!(
                " {:<44} | {:>14} | FAIL, stopped the run",
                "error-rate<1%", aborted.observed
            ),
            format!(" {:<44} | {:>14} | PASS", "p95<5s", passed.observed),
        ]
    );

    let (html_rows, markdown_rows) = sections(
        &std::fs::read_to_string(&html).unwrap(),
        &std::fs::read_to_string(&markdown).unwrap(),
    );
    assert_eq!(
        html_rows,
        [
            format!(
                "<code>error-rate&lt;1%</code> | {} | &#10007; FAIL, stopped the run",
                aborted.observed
            ),
            format!(
                "<code>p95&lt;5s</code> | {} | &#10003; PASS",
                passed.observed
            ),
        ]
    );
    assert_eq!(
        markdown_rows,
        [
            format!(
                "| error-rate\\<1% | {} | **FAIL, stopped the run** |",
                aborted.observed
            ),
            format!("| p95\\<5s | {} | PASS |", passed.observed),
        ]
    );

    // Render the JSON report again, as HTML and Markdown.
    let rendered_html = report_path("aborted-rendered.html");
    let rendered_markdown = report_path("aborted-rendered.md");
    let configuration = GooseConfiguration::parse_args_default(&[
        "--render-report",
        json.to_str().unwrap(),
        "--report-file",
        rendered_html.to_str().unwrap(),
        "--report-file",
        rendered_markdown.to_str().unwrap(),
        "--quiet",
    ])
    .unwrap();
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario!("Render").register_transaction(transaction!(get_ok)))
        .execute()
        .await
        .expect("rendering a report never fails a threshold");
    let rendered = sections(
        &std::fs::read_to_string(&rendered_html).unwrap(),
        &std::fs::read_to_string(&rendered_markdown).unwrap(),
    );
    for path in [&html, &markdown, &json, &rendered_html, &rendered_markdown] {
        let _ = std::fs::remove_file(path);
    }
    assert_eq!(rendered, (html_rows, markdown_rows));
}
