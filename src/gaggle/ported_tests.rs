//! The integration tests' Gaggle variants, rewritten on a manager and two
//! workers in one process. Each once ran copies of a load test as plain
//! processes with no manager, a Gaggle Goose no longer has; each now checks
//! what a Gaggle changes about what it tests: counts merged from every
//! worker equal to what one process counts for the same plan, users given
//! the scenarios one process would give them, logs collected on the
//! manager, hooks run once on the manager, and settings the manager hands
//! its workers.

use super::run_tests::{
    clean, configuration, gaggle, get, lines, mock_ok, scenario_users, scratch, worker_logs,
};
use crate::config::{GooseConfiguration, GooseDefault, GooseDefaultType};
use crate::goose::{GooseUser, Scenario, Transaction, TransactionResult};
use crate::metrics::GooseMetrics;
use crate::{GooseAttack, GooseScheduler};
use httpmock::{Method::GET, MockServer};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

/// How long each test may take.
const WITHIN: Duration = Duration::from_secs(120);

/// `run`, failing past [`WITHIN`].
async fn within<T>(run: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, run)
        .await
        .expect("the test ends within 120 seconds")
}

/// The manager's metrics for a Gaggle of two workers of `build` against
/// `host`, the manager given `args`, each exiting cleanly.
async fn two_workers(
    build: impl Fn(GooseConfiguration, Option<usize>) -> GooseAttack,
    host: &str,
    name: &str,
    args: &[&str],
) -> (GooseMetrics, super::run_tests::Scratch) {
    let folder = scratch(name);
    let metrics = clean(within(gaggle(build, 2, host.to_string(), &folder, args, &[], &[])).await);
    (metrics, folder)
}

/// One process running `build` against `host` with `args`.
async fn one_process(
    build: impl Fn(GooseConfiguration, Option<usize>) -> GooseAttack,
    host: &str,
    args: &[&str],
) -> GooseMetrics {
    let mut all = vec!["--quiet", "--no-telnet", "--host", host];
    all.extend_from_slice(args);
    within(build(configuration(&all), None).execute())
        .await
        .expect("one process runs it")
}

/// Every request name's count, successes and failures together.
fn counts(metrics: &GooseMetrics) -> BTreeMap<String, usize> {
    metrics
        .requests
        .iter()
        .map(|(name, request)| (name.clone(), request.success_count + request.fail_count))
        .collect()
}

/// Every transaction's successes and failures, by scenario and transaction.
fn transaction_counts(metrics: &GooseMetrics) -> Vec<Vec<(usize, usize)>> {
    metrics
        .transactions
        .iter()
        .map(|scenario| {
            scenario
                .iter()
                .map(|transaction| (transaction.success_count, transaction.fail_count))
                .collect()
        })
        .collect()
}

/// A server answering every GET with 200.
async fn ok_server() -> MockServer {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET);
            then.status(200).body("ok");
        })
        .await;
    server
}

/// Deterministic counts: every user runs its scenario `iterations` times,
/// none of it reset away, as a run of `--iterations` never resets its
/// metrics (and refuses `--no-reset-metrics`).
fn iterations<'a>(users: &'a str, iterations: &'a str) -> Vec<&'a str> {
    vec![
        "--users",
        users,
        "--increase-rate",
        "20",
        "--iterations",
        iterations,
    ]
}

// scheduler.rs

/// Scenarios A, B and C, weighted 3, 2 and 1, each GETting its own path,
/// scheduled with `scheduler`.
fn weighted(configuration: GooseConfiguration, scheduler: &GooseScheduler) -> GooseAttack {
    let scenario = |name: &str, weight: usize, path: &'static str| {
        Scenario::new(name)
            .set_weight(weight)
            .unwrap()
            .register_transaction(get(path))
    };
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(scenario("A", 3, "/a"))
        .register_scenario(scenario("B", 2, "/b"))
        .register_scenario(scenario("C", 1, "/c"))
        .set_scheduler(scheduler.clone())
}

/// Six users across two workers run the scenarios one process running all
/// six gives them with `scheduler`: the same users in each for the round
/// robin and serial schedulers, and the weights' share of the users for the
/// random one. With `--iterations`, every scenario's requests, merged, are
/// what one process counts.
async fn scheduled(scheduler: GooseScheduler, iterated: bool) {
    let server = ok_server().await;
    let host = server.base_url();
    let args = match iterated {
        true => iterations("6", "2"),
        false => vec!["--users", "6", "--increase-rate", "20", "--run-time", "2"],
    };
    let build = |configuration, _| weighted(configuration, &scheduler);
    let (merged, _folder) = two_workers(build, &host, "scheduler", &args).await;
    let single = one_process(build, &host, &args).await;
    let users = scenario_users(&merged);
    let sizes: Vec<usize> = users.iter().map(BTreeSet::len).collect();
    assert_eq!(sizes, vec![3, 2, 1], "{users:?}");
    let all: BTreeSet<usize> = users.iter().flatten().copied().collect();
    assert_eq!(
        all,
        (0..6).collect::<BTreeSet<usize>>(),
        "every user once: {users:?}"
    );
    if scheduler != GooseScheduler::Random {
        assert_eq!(users, scenario_users(&single), "{scheduler:?}");
    }
    if iterated {
        assert_eq!(counts(&merged), counts(&single), "{scheduler:?}");
        assert_eq!(
            counts(&merged),
            BTreeMap::from([
                ("GET /a".to_string(), 6),
                ("GET /b".to_string(), 4),
                ("GET /c".to_string(), 2)
            ])
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn round_robin_scenarios_across_workers() {
    scheduled(GooseScheduler::RoundRobin, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn serial_scenarios_across_workers() {
    scheduled(GooseScheduler::Serial, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn random_scenarios_across_workers() {
    scheduled(GooseScheduler::Random, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn round_robin_iterations_across_workers() {
    scheduled(GooseScheduler::RoundRobin, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn serial_iterations_across_workers() {
    scheduled(GooseScheduler::Serial, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn random_iterations_across_workers() {
    scheduled(GooseScheduler::Random, true).await;
}

// scenarios.rs

/// Scenarios a1, a2, b1 and b2, each GETting its own path; with `default`,
/// only `scenariob*` set as a default in code.
fn four(configuration: GooseConfiguration, default: bool) -> GooseAttack {
    let mut attack = GooseAttack::initialize_with_config(configuration).unwrap();
    for (name, path) in [
        ("scenarioa1", "/a1"),
        ("scenarioa2", "/a2"),
        ("scenariob1", "/b1"),
        ("scenariob2", "/b2"),
    ] {
        attack = attack.register_scenario(Scenario::new(name).register_transaction(get(path)));
    }
    if default {
        attack = *attack
            .set_default(GooseDefault::Scenarios, "scenariob*")
            .unwrap();
    }
    attack
}

/// The manager's scenario choice, from `--scenarios` or a default in code,
/// is what every worker runs: no worker's user runs a scenario left out,
/// and the merged scenarios hold users only in those chosen.
async fn chosen_scenarios(default: bool) {
    let server = ok_server().await;
    let host = server.base_url();
    let mut args = iterations("4", "2");
    if !default {
        args.extend_from_slice(&["--scenarios", "scenariob*"]);
    }
    let build = |configuration, _| four(configuration, default);
    let (merged, _folder) = two_workers(build, &host, "scenarios", &args).await;
    let single = one_process(build, &host, &args).await;
    assert_eq!(counts(&merged), counts(&single));
    assert_eq!(
        counts(&merged).keys().cloned().collect::<Vec<_>>(),
        vec!["GET /b1".to_string(), "GET /b2".to_string()]
    );
    assert_eq!(counts(&merged).values().sum::<usize>(), 8);
    for scenario in &merged.scenarios {
        if scenario.name.starts_with("scenarioa") {
            assert!(scenario.users.is_empty(), "{}", scenario.name);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn scenarios_option_reaches_every_worker() {
    chosen_scenarios(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn scenarios_default_reaches_every_worker() {
    chosen_scenarios(true).await;
}

// sequence.rs

/// One scenario of three transactions, GET /one, /two (weight 2) and
/// /three, sequenced 1, 2 and unsequenced when `sequenced`.
fn sequence(
    configuration: GooseConfiguration,
    sequenced: bool,
    scheduler: &GooseScheduler,
) -> GooseAttack {
    let mut one = get("/one");
    let mut two = get("/two").set_weight(2).unwrap();
    if sequenced {
        one = one.set_sequence(1);
        two = two.set_sequence(2);
    }
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("Sequence")
                .register_transaction(get("/three"))
                .register_transaction(two)
                .register_transaction(one),
        )
        .set_scheduler(scheduler.clone())
}

/// Every transaction's merged counts are what one process counts for the
/// same plan.
async fn sequenced(sequenced: bool, scheduler: GooseScheduler) {
    let server = ok_server().await;
    let host = server.base_url();
    let args = iterations("4", "2");
    let build = |configuration, _| sequence(configuration, sequenced, &scheduler);
    let (merged, _folder) = two_workers(build, &host, "sequence", &args).await;
    let single = one_process(build, &host, &args).await;
    assert_eq!(transaction_counts(&merged), transaction_counts(&single));
    assert_eq!(counts(&merged), counts(&single));
    assert_eq!(counts(&merged).get("GET /two"), Some(&16));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn unsequenced_transactions_merge() {
    sequenced(false, GooseScheduler::RoundRobin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn sequenced_round_robin_transactions_merge() {
    sequenced(true, GooseScheduler::RoundRobin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn sequenced_serial_transactions_merge() {
    sequenced(true, GooseScheduler::Serial).await;
}

// logs.rs, error.rs

/// GET /index, then GET /error, which fails: each failure is logged to
/// the debug log.
async fn get_error(user: &mut GooseUser) -> TransactionResult {
    let mut goose = user.get("/error").await?;
    if let Ok(response) = goose.response {
        if !response.status().is_success() {
            let headers = response.headers().clone();
            return user.set_failure(
                "loaded /error and got non-200 message",
                &mut goose.request,
                Some(&headers),
                None,
            );
        }
    }
    Ok(())
}

/// A load test of GET /index and a failing GET /error.
fn failing(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("LoadTest")
                .register_transaction(get("/index"))
                .register_transaction(Transaction::new(Arc::new(|user| {
                    Box::pin(async move { get_error(user).await })
                }))),
        )
}

/// A transaction that returns a custom error every time: a transaction
/// error, with no request.
async fn boom(_user: &mut GooseUser) -> TransactionResult {
    Err(Box::new(crate::goose::TransactionError::Custom(
        "boom".to_string(),
    )))
}

/// [`failing`], and a transaction error each iteration as well.
fn failing_both(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("LoadTest")
                .register_transaction(get("/index"))
                .register_transaction(Transaction::new(Arc::new(|user| {
                    Box::pin(async move { get_error(user).await })
                })))
                .register_transaction(Transaction::new(Arc::new(|user| {
                    Box::pin(async move { boom(user).await })
                }))),
        )
}

/// A server answering /index with 200 and /error with 503.
async fn failing_server() -> MockServer {
    let server = MockServer::start_async().await;
    mock_ok(&server, "/index").await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/error");
            then.status(503);
        })
        .await;
    server
}

/// The entries in each collected log `file`, every worker's together:
/// lines, less a CSV log's header.
fn entries(folder: &std::path::Path, file: &str, csv: bool) -> usize {
    let workers = folder.join("manager").join("workers");
    let logs: Vec<std::path::PathBuf> = std::fs::read_dir(&workers)
        .unwrap()
        .map(|entry| entry.unwrap().path().join(file))
        .collect();
    assert_eq!(logs.len(), 2, "a {file} from each worker");
    logs.iter().map(|log| lines(log) - usize::from(csv)).sum()
}

/// Each log `kind` set on the manager, in `format`, is written on every
/// worker and collected into the manager's run folder, the workers' entries
/// together as many as the merged metrics count: 2 users, 3 iterations,
/// each a request to /index and one to /error that fails.
async fn collected(kinds: &[(&str, &str, &str)]) {
    let server = failing_server().await;
    let host = server.base_url();
    let mut args = iterations("2", "3");
    let mut names = Vec::new();
    for (kind, format, _) in kinds {
        names.push((
            format!("--{kind}-log"),
            format!("{kind}.log"),
            format!("--{kind}-format"),
            format.to_string(),
        ));
    }
    for (option, file, format_option, format) in &names {
        args.extend_from_slice(&[
            option.as_str(),
            file.as_str(),
            format_option.as_str(),
            format.as_str(),
        ]);
    }
    let (merged, folder) = two_workers(failing, &host, "logs", &args).await;
    let requests = counts(&merged).values().sum::<usize>();
    assert_eq!(requests, 12);
    for ((kind, format, _), (_, file, _, _)) in kinds.iter().zip(&names) {
        let csv = *format == "csv";
        if *format == "pretty" {
            for log in worker_logs(&folder, file) {
                assert!(!log.is_empty(), "{}", kind);
            }
            continue;
        }
        let expected = match *kind {
            "request" => requests,
            "transaction" => 12,
            "scenario" => 6,
            "error" | "debug" => 6,
            _ => unreachable!(),
        };
        assert_eq!(entries(&folder, file, csv), expected, "{kind} {format}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn request_logs_are_collected() {
    collected(&[("request", "json", "")]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn transaction_logs_are_collected() {
    collected(&[("transaction", "csv", "")]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn scenario_logs_are_collected() {
    collected(&[("scenario", "raw", "")]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn error_logs_are_collected() {
    collected(&[("error", "json", "")]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn debug_logs_are_collected() {
    collected(&[("debug", "csv", "")]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn every_log_is_collected() {
    collected(&[
        ("request", "pretty", ""),
        ("transaction", "raw", ""),
        ("scenario", "csv", ""),
        ("error", "raw", ""),
        ("debug", "json", ""),
    ])
    .await;
}

/// Every worker's errors are merged into one summary, as one process
/// counts them; `--no-error-summary` on the manager keeps none, and still
/// counts every failure.
async fn error_summary(summary: bool) {
    let server = failing_server().await;
    let host = server.base_url();
    let mut args = iterations("4", "2");
    if !summary {
        args.push("--no-error-summary");
    }
    let (merged, _folder) = two_workers(failing_both, &host, "errors", &args).await;
    let single = one_process(failing_both, &host, &args).await;
    assert_eq!(merged.requests["GET /error"].fail_count, 8);
    let occurrences = |metrics: &GooseMetrics| -> Vec<usize> {
        metrics
            .errors
            .values()
            .map(|error| error.occurrences)
            .collect()
    };
    assert_eq!(occurrences(&merged), occurrences(&single));
    match summary {
        true => assert_eq!(occurrences(&merged), vec![8]),
        false => assert!(merged.errors.is_empty(), "{:?}", merged.errors),
    }
    let transaction_occurrences = |metrics: &GooseMetrics| -> Vec<usize> {
        metrics
            .transaction_errors
            .values()
            .map(|error| error.occurrences)
            .collect()
    };
    assert_eq!(
        transaction_occurrences(&merged),
        transaction_occurrences(&single)
    );
    match summary {
        true => assert_eq!(transaction_occurrences(&merged), vec![8]),
        false => assert!(
            merged.transaction_errors.is_empty(),
            "{:?}",
            merged.transaction_errors
        ),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn error_summary_merges_every_worker() {
    error_summary(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn no_error_summary_reaches_the_merge() {
    error_summary(false).await;
}

// redirect.rs, defaults.rs, one_taskset.rs

/// Each user: on start, GET /pre; then GET /a twice; with `defaults`,
/// sticky follow set as a default in code.
fn redirected(configuration: GooseConfiguration, defaults: bool) -> GooseAttack {
    let attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("Redirect")
                .register_transaction(get("/pre").set_on_start())
                .register_transaction(get("/a"))
                .register_transaction(get("/a")),
        );
    if !defaults {
        return attack;
    }
    *attack
        .set_default(GooseDefault::StickyFollow, true)
        .unwrap()
}

/// A mock on `server` answering a GET of `path` with `status`, sending
/// `location` with it when given.
async fn answer<'a>(
    server: &'a MockServer,
    path: &str,
    status: u16,
    location: Option<String>,
) -> httpmock::Mock<'a> {
    server
        .mock_async(|when, then| {
            when.method(GET).path(path);
            let then = then.status(status);
            if let Some(location) = location {
                then.header("Location", location.as_str());
            }
        })
        .await
}

/// 2 users, each running its scenario twice: deterministic counts, as a
/// run of `--iterations` never resets its metrics.
fn two_users_twice() -> Vec<&'static str> {
    vec!["--users", "2", "--increase-rate", "20", "--iterations", "2"]
}

/// A chain of redirects on one server, followed by every worker: the merged
/// counts are one process's, and the server sees every request once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn redirects_are_followed_on_every_worker() {
    let server = MockServer::start_async().await;
    let pre = answer(&server, "/pre", 200, None).await;
    let a = answer(&server, "/a", 302, Some("/redirect".to_string())).await;
    let hop = answer(&server, "/redirect", 301, Some("/final".to_string())).await;
    let last = answer(&server, "/final", 200, None).await;
    let host = server.base_url();
    let args = two_users_twice();
    let build = |configuration, _| redirected(configuration, false);
    let (merged, _folder) = two_workers(build, &host, "redirect", &args).await;
    let seen = (pre.calls_async().await, a.calls_async().await);
    assert_eq!(seen, (2, 8));
    assert_eq!(hop.calls_async().await, 8);
    assert_eq!(last.calls_async().await, 8);
    let single = one_process(build, &host, &args).await;
    assert_eq!(counts(&merged), counts(&single));
    assert_eq!(counts(&merged).values().sum::<usize>(), 10);
}

/// /a on the first server redirects to the second's /b. Sticky, each
/// user's later requests go to the second server: checked on the servers,
/// which see what every worker did.
async fn sticky(args: &[&str], defaults: bool, sticky: bool) {
    let (first, second) = (
        MockServer::start_async().await,
        MockServer::start_async().await,
    );
    let pre = answer(&first, "/pre", 200, None).await;
    let first_a = answer(&first, "/a", 302, Some(format!("{}/b", second.base_url()))).await;
    let second_a = answer(&second, "/a", 200, None).await;
    let second_b = answer(&second, "/b", 200, None).await;
    let host = first.base_url();
    let build = |configuration, _| redirected(configuration, defaults);
    let (merged, _folder) = two_workers(build, &host, "sticky", args).await;
    // Each user's first /a reaches the first server; sticky, its other
    // three go to the second.
    let seen = (
        first_a.calls_async().await,
        second_a.calls_async().await,
        second_b.calls_async().await,
    );
    match sticky {
        true => assert_eq!(seen, (2, 6, 2)),
        false => assert_eq!(seen, (8, 0, 8)),
    }
    // Every /pre stays counted: a run of --iterations never resets.
    assert_eq!(pre.calls_async().await, 2);
    let counted = merged
        .requests
        .get("GET /pre")
        .map_or(0, |pre| pre.success_count);
    assert_eq!(counted, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn sticky_redirects_reach_every_worker() {
    let mut args = two_users_twice();
    args.push("--sticky-follow");
    sticky(&args, false, true).await;
    // Without it, every /a starts on the first server.
    sticky(&two_users_twice(), false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn defaults_set_in_code_reach_every_worker() {
    sticky(&two_users_twice(), true, true).await;
}

/// `--no-reset-metrics` set as a default in code reaches every worker:
/// with two users on each worker, a reset would clear at least one /pre on
/// each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn no_reset_metrics_default_reaches_every_worker() {
    let server = MockServer::start_async().await;
    let pre = answer(&server, "/pre", 200, None).await;
    answer(&server, "/a", 200, None).await;
    let args = ["--users", "4", "--increase-rate", "8", "--run-time", "2"];
    let build = |configuration, place| {
        *ramped(configuration, place)
            .set_default(GooseDefault::NoResetMetrics, true)
            .unwrap()
    };
    let (merged, _folder) = two_workers(build, &server.base_url(), "reset-default", &args).await;
    assert_eq!(pre.calls_async().await, 4);
    let counted = merged
        .requests
        .get("GET /pre")
        .map_or(0, |pre| pre.success_count);
    assert_eq!(counted, 4);
}

// no_normal_tasks.rs

/// A scenario of only on_start and on_stop: every user on every worker runs
/// each once, the on_stop when the manager's run ends, and the merged
/// metrics count them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn on_stop_runs_once_per_user_on_every_worker() {
    let server = MockServer::start_async().await;
    let login = answer(&server, "/login", 200, None).await;
    let logout = answer(&server, "/logout", 200, None).await;
    let build = |configuration, _: Option<usize>| {
        GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(
                Scenario::new("LoadTest")
                    .register_transaction(get("/login").set_on_start())
                    .register_transaction(get("/logout").set_on_stop()),
            )
    };
    let args = [
        "--users",
        "4",
        "--increase-rate",
        "20",
        "--run-time",
        "2",
        "--no-reset-metrics",
    ];
    let (merged, _folder) = two_workers(build, &server.base_url(), "on-stop", &args).await;
    let calls = (login.calls_async().await, logout.calls_async().await);
    assert_eq!(calls, (4, 4));
    assert_eq!(counts(&merged).get("GET /logout"), Some(&4));
}

/// Each user: on start GET /pre, then GET /a, for 2 seconds.
fn ramped(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(
            Scenario::new("One")
                .register_transaction(get("/pre").set_on_start())
                .register_transaction(get("/a")),
        )
}

/// The manager's metrics reset, when the ramp ends, reaches every worker:
/// with `--no-reset-metrics` every user's /pre stays counted, and without
/// it the ramp's are gone from the merged metrics.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn metrics_reset_reaches_every_worker() {
    for reset in [false, true] {
        let server = MockServer::start_async().await;
        let pre = answer(&server, "/pre", 200, None).await;
        answer(&server, "/a", 200, None).await;
        let mut args = vec!["--users", "4", "--increase-rate", "8", "--run-time", "2"];
        if !reset {
            args.push("--no-reset-metrics");
        }
        let (merged, _folder) = two_workers(ramped, &server.base_url(), "reset", &args).await;
        let counted = merged
            .requests
            .get("GET /pre")
            .map_or(0, |pre| pre.success_count);
        assert_eq!(pre.calls_async().await, 4);
        // Every worker took the reset: none is left out of the merge.
        let a = counts(&merged).get("GET /a").copied().unwrap_or(0);
        assert!(a > 0, "every worker's /a is merged: {:?}", counts(&merged));
        match reset {
            false => assert_eq!(counted, 4),
            // Only each worker's last user may race the reset with its /pre.
            true => assert!(counted <= 2, "the reset clears the ramp: {}", counted),
        }
    }
}

// setup_teardown.rs, throttle.rs

/// GET /a, with test_start GET /start and test_stop GET /stop as asked.
fn hooked(configuration: GooseConfiguration, start: bool, stop: bool) -> GooseAttack {
    let mut attack = GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(Scenario::new("A").register_transaction(get("/a")));
    if start {
        attack = attack.test_start(get("/start"));
    }
    if stop {
        attack = attack.test_stop(get("/stop"));
    }
    attack
}

/// test_start and test_stop, as registered, run once each, on the manager,
/// never on a worker.
async fn hooks(start: bool, stop: bool) {
    let server = MockServer::start_async().await;
    let starts = answer(&server, "/start", 200, None).await;
    let stops = answer(&server, "/stop", 200, None).await;
    let a = answer(&server, "/a", 200, None).await;
    let build = |configuration, _| hooked(configuration, start, stop);
    let _ = two_workers(build, &server.base_url(), "hooks", &two_users_twice()).await;
    assert_eq!(starts.calls_async().await, usize::from(start));
    assert_eq!(stops.calls_async().await, usize::from(stop));
    assert_eq!(a.calls_async().await, 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn test_start_runs_once_on_the_manager() {
    hooks(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn test_stop_runs_once_on_the_manager() {
    hooks(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn test_start_and_stop_run_once_on_the_manager() {
    hooks(true, true).await;
}

/// The throttle is the whole Gaggle's, split between its workers: two
/// workers under a throttle of 10 requests a second make about 10 a second
/// together, not 10 each, and five times the throttle makes more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn throttle_is_split_between_workers() {
    let mut made = Vec::new();
    for throttle in ["10", "50"] {
        let server = MockServer::start_async().await;
        let a = answer(&server, "/a", 200, None).await;
        let args = [
            "--users",
            "4",
            "--increase-rate",
            "40",
            "--run-time",
            "3",
            "--throttle-requests",
            throttle,
        ];
        let started = std::time::Instant::now();
        let _ = two_workers(ramped_a, &server.base_url(), "throttle", &args).await;
        let ran = started.elapsed().as_secs_f64();
        made.push((a.calls_async().await, ran));
    }
    let (slow, ran) = made[0];
    // At most the throttle over the whole time the run took, and well short
    // of twice it, which a throttle each worker took whole would make.
    assert!(slow as f64 <= 10.0 * ran + 5.0, "{} in {:.1} s", slow, ran);
    assert!((20..=45).contains(&slow), "{} in {:.1} s", slow, ran);
    assert!(made[1].0 > slow * 2, "{:?}", made);
}

/// Each user GETs /a.
fn ramped_a(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(Scenario::new("A").register_transaction(get("/a")))
}

// discard_body.rs

/// A keep-alive HTTP/1.1 server answering every request with a 256 KiB
/// body, too large to arrive with the headers. Returns its base URL and the
/// connections it accepted.
async fn body_server() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    const BODY_BYTES: usize = 256 * 1024;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    tokio::spawn(async move {
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {BODY_BYTES}\r\n\r\n");
        let body = vec![b'x'; BODY_BYTES];
        while let Ok((mut socket, _)) = listener.accept().await {
            accepted.fetch_add(1, Ordering::SeqCst);
            let (head, body) = (head.clone(), body.clone());
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buffer[..read]);
                    while let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        request.drain(..end + 4);
                        if socket.write_all(head.as_bytes()).await.is_err()
                            || socket.write_all(&body).await.is_err()
                        {
                            return;
                        }
                    }
                }
            });
        }
    });
    (format!("http://{address}"), connections)
}

/// Each user GETs /body and drops the response with its body unread.
fn drops_bodies(configuration: GooseConfiguration, _: Option<usize>) -> GooseAttack {
    GooseAttack::initialize_with_config(configuration)
        .unwrap()
        .register_scenario(Scenario::new("Body").register_transaction(Transaction::new(
            Arc::new(|user| {
                Box::pin(async move {
                    let _goose = user.get("/body").await?;
                    Ok(())
                })
            }),
        )))
}

/// A Gaggle worker's user reads the bodies its transactions drop, as one
/// process does: one user's 10 requests open at most 2 connections, the
/// second when a request starts before the last connection is back in the
/// pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn worker_reads_dropped_bodies() {
    let (host, connections) = body_server().await;
    let folder = scratch("unread-bodies");
    let metrics = clean(
        within(gaggle(
            drops_bodies,
            1,
            host,
            &folder,
            &iterations("1", "10"),
            &[],
            &[],
        ))
        .await,
    );
    assert_eq!(counts(&metrics).values().sum::<usize>(), 10);
    let opened = connections.load(std::sync::atomic::Ordering::SeqCst);
    assert!(opened <= 2, "10 requests opened {} connections", opened);
}
