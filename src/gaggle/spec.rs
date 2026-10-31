//! What a manager hands each worker: its share of the load test, built here,
//! and sent postcard encoded in a `Prepare`. An explicit struct, never a
//! serialized `GooseConfiguration`, whose serde output hides the dashboard
//! token and is no faithful copy.

use crate::config::{GooseConfiguration, RUN_FOLDER};
use crate::logger::GooseLogFormat;
use crate::metrics::GooseCoordinatedOmissionMitigation;
use crate::runs::{log_file_name, LOG_NAMES};
use serde::{Deserialize, Serialize};

/// One log a worker writes, in its own run folder, and streams to the
/// manager at the end of the run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LogSpec {
    /// Which log: an index into [`LOG_NAMES`].
    pub(crate) kind: u8,
    pub(crate) format: Option<GooseLogFormat>,
    /// The file name the manager keeps it under, as its log flag gives it.
    pub(crate) manager_file: String,
}

/// A worker's share of the load test.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkerRunSpec {
    /// This worker's index, from 0.
    pub(crate) worker: u32,
    pub(crate) workers: u32,
    pub(crate) host: Option<String>,
    /// The `--scenarios` patterns, empty when every scenario runs.
    pub(crate) scenarios: Vec<String>,
    /// The users this worker starts over the run.
    pub(crate) users: usize,
    /// The global ordinal of its first user.
    pub(crate) user_offset: usize,
    /// The index in the load test's scenarios of each of its users, by
    /// ordinal: its slice of the one allocation the manager drew for every
    /// user, under any scheduler.
    pub(crate) slice: Vec<usize>,
    /// Its test plan: each step's users, and the step's milliseconds.
    pub(crate) test_plan: Vec<(usize, usize)>,
    /// Its share of `--throttle-requests`, 0 without a throttle.
    pub(crate) throttle_requests: usize,
    pub(crate) iterations: Option<usize>,
    pub(crate) co_mitigation: Option<GooseCoordinatedOmissionMitigation>,
    pub(crate) timeout: Option<String>,
    pub(crate) no_gzip: bool,
    pub(crate) sticky_follow: bool,
    pub(crate) accept_invalid_certs: bool,
    pub(crate) no_metrics: bool,
    pub(crate) no_transaction_metrics: bool,
    pub(crate) no_scenario_metrics: bool,
    pub(crate) no_status_codes: bool,
    pub(crate) no_percentiles_over_time: bool,
    /// Whether per second graph data is kept: whatever the manager's
    /// reports and dashboard need.
    pub(crate) records_graph: bool,
    pub(crate) request_body: bool,
    pub(crate) no_debug_body: bool,
    pub(crate) compress: Option<u32>,
    pub(crate) logs: Vec<LogSpec>,
    /// The bytes the load test's worker data hook returned for it.
    pub(crate) custom_data: Vec<u8>,
}

/// `total` split as evenly as it goes between `workers`, the remainder one
/// each to the first: each worker's part never falls as `total` rises, so
/// every worker moves the way the total does.
pub(crate) fn split(total: usize, workers: usize, worker: usize) -> usize {
    let base = total / workers;
    base + usize::from(worker < total % workers)
}

/// Each worker's test plan: every step's users split between the workers,
/// so each step's users add up to the global step's, at the same times.
pub(crate) fn plans(steps: &[(usize, usize)], workers: usize) -> Vec<Vec<(usize, usize)>> {
    (0..workers)
        .map(|worker| {
            steps
                .iter()
                .map(|&(users, ms)| (split(users, workers, worker), ms))
                .collect()
        })
        .collect()
}

/// The users a test plan starts over the run: every increase added up.
pub(crate) fn users_started(steps: &[(usize, usize)]) -> usize {
    let mut previous = 0usize;
    steps.iter().fold(0usize, |total, &(users, _)| {
        let increase = users.saturating_sub(previous);
        previous = users;
        total.saturating_add(increase)
    })
}

/// `throttle` split in proportion to `users`, the remainder spread, and no
/// worker below 1: the shares add up to `throttle`, which must be at least
/// the number of workers.
pub(crate) fn throttle_shares(throttle: usize, users: &[usize]) -> Vec<usize> {
    let workers = users.len();
    if throttle == 0 || workers == 0 {
        return vec![0; workers];
    }
    let total: u128 = users.iter().map(|&u| u as u128).sum();
    let mut shares: Vec<usize> = Vec::with_capacity(workers);
    let mut remainders: Vec<(u128, usize)> = Vec::with_capacity(workers);
    for (worker, &u) in users.iter().enumerate() {
        let exact = throttle as u128 * u as u128;
        let (share, remainder) = match exact.checked_div(total) {
            Some(share) => (share as usize, exact % total),
            // No users anywhere: an even split.
            None => (throttle / workers, 0),
        };
        shares.push(share.max(1));
        remainders.push((remainder, worker));
    }
    // Largest remainder first, then the first workers.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut sum: usize = shares.iter().sum();
    let mut at = 0;
    while sum < throttle {
        shares[remainders[at % workers].1] += 1;
        sum += 1;
        at += 1;
    }
    // Raising a share to 1 can pass the total: take back from the largest.
    while sum > throttle {
        let largest = (0..workers)
            .max_by_key(|&worker| (shares[worker], std::cmp::Reverse(worker)))
            .unwrap_or(0);
        if shares[largest] <= 1 {
            break;
        }
        shares[largest] -= 1;
        sum -= 1;
    }
    shares
}

/// The logs the manager's flags ask for, as each worker writes them.
pub(crate) fn log_specs(configuration: &GooseConfiguration) -> Vec<LogSpec> {
    let logs = [
        (&configuration.request_log, &configuration.request_format),
        (
            &configuration.transaction_log,
            &configuration.transaction_format,
        ),
        (&configuration.scenario_log, &configuration.scenario_format),
        (&configuration.error_log, &configuration.error_format),
        (&configuration.debug_log, &configuration.debug_format),
    ];
    logs.iter()
        .enumerate()
        .filter_map(|(kind, (log, format))| {
            let log = log.as_deref().filter(|log| !log.is_empty())?;
            let manager_file = if log == RUN_FOLDER {
                let mut file = log_file_name(LOG_NAMES[kind], format.as_ref());
                if configuration.compress.is_some() {
                    file.push_str(".gz");
                }
                file
            } else {
                std::path::Path::new(log)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| LOG_NAMES[kind].to_string())
            };
            Some(LogSpec {
                kind: kind as u8,
                format: (*format).clone(),
                manager_file,
            })
        })
        .collect()
}

/// Every worker's share of a load test of `steps` users, whose users take
/// the scenarios `order` gives by ordinal, for a manager configured with
/// `configuration`. `records_graph` is whether the manager keeps per second
/// graph data. The custom data is added by the caller.
pub(crate) fn worker_specs(
    configuration: &GooseConfiguration,
    steps: &[(usize, usize)],
    order: &[usize],
    workers: usize,
    records_graph: bool,
) -> Vec<WorkerRunSpec> {
    let plans = plans(steps, workers);
    let users: Vec<usize> = plans.iter().map(|plan| users_started(plan)).collect();
    let throttles = throttle_shares(configuration.throttle_requests(), &users);
    let logs = log_specs(configuration);
    let mut offset = 0usize;
    plans
        .into_iter()
        .enumerate()
        .map(|(worker, test_plan)| {
            let count = users[worker];
            let start = offset.min(order.len());
            let end = offset.saturating_add(count).min(order.len());
            let spec = WorkerRunSpec {
                worker: worker as u32,
                workers: workers as u32,
                host: configuration.host.clone().filter(|host| !host.is_empty()),
                scenarios: configuration.scenarios().to_vec(),
                users: count,
                user_offset: offset,
                slice: order[start..end].to_vec(),
                test_plan,
                throttle_requests: throttles[worker],
                iterations: configuration.iterations,
                co_mitigation: configuration.co_mitigation.clone(),
                timeout: configuration.timeout.clone(),
                no_gzip: configuration.no_gzip,
                sticky_follow: configuration.sticky_follow,
                accept_invalid_certs: configuration.accept_invalid_certs,
                no_metrics: configuration.no_metrics,
                no_transaction_metrics: configuration.no_transaction_metrics,
                no_scenario_metrics: configuration.no_scenario_metrics,
                no_status_codes: configuration.no_status_codes,
                no_percentiles_over_time: configuration.no_percentiles_over_time,
                records_graph,
                request_body: configuration.request_body,
                no_debug_body: configuration.no_debug_body,
                compress: configuration.compress,
                logs: logs.clone(),
                custom_data: Vec::new(),
            };
            offset = offset.saturating_add(count);
            spec
        })
        .collect()
}

impl WorkerRunSpec {
    /// Apply the spec to a worker's configuration, whose own load options
    /// are all unset (a worker refuses them).
    pub(crate) fn configure(&self, configuration: &mut GooseConfiguration) {
        configuration.host = self.host.clone();
        configuration.users = Some(self.users);
        configuration.increase_rate = None;
        configuration.increase_time = None;
        configuration.run_time = None;
        configuration.decrease_rate = None;
        configuration.decrease_time = None;
        configuration.test_plan = Some(crate::test_plan::TestPlan {
            steps: self.test_plan.clone(),
            current: 0,
        });
        configuration.scenarios = (!self.scenarios.is_empty()).then(|| crate::config::Scenarios {
            active: self.scenarios.clone(),
        });
        configuration.throttle_requests = Some(self.throttle_requests);
        configuration.iterations = self.iterations;
        configuration.co_mitigation = self.co_mitigation.clone();
        configuration.timeout = self.timeout.clone();
        configuration.no_gzip = self.no_gzip;
        configuration.sticky_follow = self.sticky_follow;
        configuration.accept_invalid_certs = self.accept_invalid_certs;
        configuration.no_metrics = self.no_metrics;
        configuration.no_transaction_metrics = self.no_transaction_metrics;
        configuration.no_scenario_metrics = self.no_scenario_metrics;
        configuration.no_status_codes = self.no_status_codes;
        configuration.no_percentiles_over_time = self.no_percentiles_over_time;
        configuration.gaggle_records_graph = self.records_graph;
        // The manager owns the metrics reset, judges the thresholds and writes
        // the reports: any the load test set in code are not the worker's.
        configuration.no_reset_metrics = true;
        configuration.threshold.clear();
        configuration.thresholds.clear();
        configuration.no_threshold_abort = true;
        configuration.threshold_abort_delay = None;
        configuration.report_file.clear();
        configuration.pdf_print_html = None;
        configuration.running_metrics = None;
        configuration.request_body = self.request_body;
        configuration.no_debug_body = self.no_debug_body;
        configuration.compress = self.compress;
        for log in &self.logs {
            let run_folder = Some(RUN_FOLDER.to_string());
            match log.kind {
                0 => {
                    configuration.request_log = run_folder;
                    configuration.request_format.clone_from(&log.format);
                }
                1 => {
                    configuration.transaction_log = run_folder;
                    configuration.transaction_format.clone_from(&log.format);
                }
                2 => {
                    configuration.scenario_log = run_folder;
                    configuration.scenario_format.clone_from(&log.format);
                }
                3 => {
                    configuration.error_log = run_folder;
                    configuration.error_format.clone_from(&log.format);
                }
                _ => {
                    configuration.debug_log = run_folder;
                    configuration.debug_format.clone_from(&log.format);
                }
            }
        }
    }

    /// Where the worker writes each log, in its run folder `folder`, with
    /// the name the manager keeps it under.
    pub(crate) fn log_files(
        &self,
        folder: &crate::runs::RunFolder,
    ) -> Vec<(String, std::path::PathBuf)> {
        self.logs
            .iter()
            .map(|log| {
                let mut file = log_file_name(LOG_NAMES[log.kind as usize % 5], log.format.as_ref());
                if self.compress.is_some() {
                    file.push_str(".gz");
                }
                (log.manager_file.clone(), folder.file(&file))
            })
            .collect()
    }

    /// The spec, postcard encoded.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    /// A spec read back from `bytes`.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_adds_up_and_follows_the_total() {
        crate::gaggle::within(30, || {
            for workers in 1..7 {
                for total in 0..40 {
                    let parts: Vec<usize> =
                        (0..workers).map(|w| split(total, workers, w)).collect();
                    assert_eq!(parts.iter().sum::<usize>(), total);
                    assert!(parts.windows(2).all(|pair| pair[0] >= pair[1]));
                    for (worker, &part) in parts.iter().enumerate() {
                        assert!(split(total + 1, workers, worker) >= part);
                    }
                }
            }
            assert_eq!(
                (0..3).map(|w| split(10, 3, w)).collect::<Vec<_>>(),
                [4, 3, 3]
            );
        });
    }

    /// Every step's users add up to the global step's, and the users the
    /// workers start add up to the users the plan starts, for a plan that
    /// goes up, down and up again.
    #[test]
    fn plans_add_up_at_every_step() {
        crate::gaggle::within(30, || {
            let steps = [(10, 1_000), (10, 5_000), (3, 1_000), (11, 2_000), (0, 0)];
            for workers in 1..6 {
                let plans = plans(&steps, workers);
                for (step, &(users, ms)) in steps.iter().enumerate() {
                    assert_eq!(plans.iter().map(|plan| plan[step].0).sum::<usize>(), users);
                    assert!(plans.iter().all(|plan| plan[step].1 == ms));
                }
                let started: usize = plans.iter().map(|plan| users_started(plan)).sum();
                assert_eq!(started, users_started(&steps));
            }
            // More workers than users: a worker may hold 0 users.
            let plans = plans(&[(2, 1_000), (0, 0)], 3);
            assert_eq!(plans[2], [(0, 1_000), (0, 0)]);
        });
    }

    #[test]
    fn throttle_shares_add_up_and_never_reach_zero() {
        crate::gaggle::within(30, || {
            assert_eq!(
                throttle_shares(3_000, &[300, 300, 300]),
                [1_000, 1_000, 1_000]
            );
            assert_eq!(throttle_shares(10, &[2, 1, 1]), [5, 3, 2]);
            assert_eq!(throttle_shares(0, &[5, 5]), [0, 0]);
            // A worker with no users still gets 1.
            assert_eq!(throttle_shares(4, &[10, 0]), [3, 1]);
            for throttle in 3..50 {
                for users in [[1, 1, 1], [7, 0, 2], [0, 0, 0], [100, 1, 1]] {
                    let shares = throttle_shares(throttle, &users);
                    assert_eq!(
                        shares.iter().sum::<usize>(),
                        throttle,
                        "{throttle} {users:?}"
                    );
                    assert!(shares.iter().all(|&share| share >= 1));
                }
            }
        });
    }

    #[test]
    fn worker_specs_cover_every_user_once() {
        crate::gaggle::within(30, || {
            let configuration = GooseConfiguration::parse_args_default(&[
                "--host",
                "http://127.0.0.1",
                "--throttle-requests",
                "100",
                "--request-log",
                "--transaction-log",
                "/tmp/somewhere/t.log",
                "--compress",
            ])
            .unwrap();
            let steps = [(7, 1_000), (7, 10_000), (0, 0)];
            let order: Vec<usize> = (0..7).map(|user| user % 3).collect();
            let specs = worker_specs(&configuration, &steps, &order, 3, true);
            assert_eq!(
                specs
                    .iter()
                    .map(|s| (s.user_offset, s.users))
                    .collect::<Vec<_>>(),
                [(0, 3), (3, 2), (5, 2)]
            );
            let slices: Vec<usize> = specs.iter().flat_map(|s| s.slice.clone()).collect();
            assert_eq!(slices, order);
            assert_eq!(
                specs.iter().map(|s| s.throttle_requests).sum::<usize>(),
                100
            );
            assert_eq!(specs[0].logs.len(), 2);
            assert_eq!(specs[0].logs[0].manager_file, "requests.json.gz");
            assert_eq!(specs[0].logs[1].manager_file, "t.log");
            let decoded = WorkerRunSpec::decode(&specs[1].encode().unwrap()).unwrap();
            assert_eq!(decoded, specs[1]);
        });
    }

    /// A spec configures every option it carries, so a worker runs exactly
    /// its share.
    #[test]
    fn spec_configures_a_worker() {
        crate::gaggle::within(30, || {
            let manager = GooseConfiguration::parse_args_default(&[
                "--host",
                "http://example.com",
                "--users",
                "5",
                "--iterations",
                "3",
                "--co-mitigation",
                "average",
                "--timeout",
                "2",
                "--scenario-log",
            ])
            .unwrap();
            let spec = worker_specs(&manager, &[(5, 1_000), (0, 0)], &[0; 5], 2, true).remove(1);
            // What the load test may have set in code, as configuring a worker
            // with its defaults gives it.
            let mut worker = GooseConfiguration {
                threshold: vec!["p95<1ms".to_string()],
                thresholds: vec!["p95<1ms".parse().unwrap()],
                report_file: vec!["r.html".to_string()],
                pdf_print_html: Some("p.html".to_string()),
                running_metrics: Some(1),
                ..Default::default()
            };
            spec.configure(&mut worker);
            // The manager judges the thresholds and writes the reports.
            assert!(worker.threshold.is_empty() && worker.thresholds.is_empty());
            assert!(!worker.aborts_on_thresholds());
            assert!(worker.report_file.is_empty());
            assert!(worker.pdf_print_html.is_none() && worker.running_metrics.is_none());
            assert_eq!(worker.host.as_deref(), Some("http://example.com"));
            assert_eq!(worker.users, Some(2));
            assert_eq!(worker.iterations, Some(3));
            assert_eq!(
                worker.co_mitigation,
                Some(GooseCoordinatedOmissionMitigation::Average)
            );
            assert_eq!(worker.timeout.as_deref(), Some("2"));
            assert_eq!(worker.scenario_log.as_deref(), Some(RUN_FOLDER));
            assert!(worker.no_reset_metrics);
            assert!(worker.gaggle_records_graph);
            assert_eq!(
                worker.test_plan.as_ref().map(|plan| plan.steps.clone()),
                Some(vec![(2, 1_000), (0, 0)])
            );
        });
    }
}
