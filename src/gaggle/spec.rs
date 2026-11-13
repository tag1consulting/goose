//! What a manager hands each worker: its share of the load test, built here,
//! and sent postcard encoded in a `Prepare`. An explicit struct, never a
//! serialized `GooseConfiguration`, whose serde output hides the dashboard
//! token and is no faithful copy.

use crate::client::CommandLineClientOptions;
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
    /// Whether the worker writes it gzipped: whether `manager_file` ends in
    /// `.gz`, as one process gzips a log of that name and no other.
    pub(crate) gzip: bool,
}

impl LogSpec {
    /// The name a worker gives this log in its own run folder.
    pub(crate) fn worker_file(&self) -> String {
        let mut file = log_file_name(LOG_NAMES[self.kind as usize], self.format.as_ref());
        if self.gzip {
            file.push_str(".gz");
        }
        file
    }
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
    /// Its users ramp, when the manager runs one and every worker has a
    /// user: it then runs `test_plan` as one process runs a users ramp, so
    /// a Controller can change its users, rates and run time as it runs.
    pub(crate) ramp: Option<WorkerRamp>,
    /// Whether the whole load test's test plan holds until stopped, its last
    /// step running users: a worker whose share of that step is none then
    /// waits for its manager rather than ending its run.
    pub(crate) holds: bool,
    pub(crate) iterations: Option<usize>,
    pub(crate) co_mitigation: Option<GooseCoordinatedOmissionMitigation>,
    pub(crate) timeout: Option<String>,
    pub(crate) no_gzip: bool,
    pub(crate) sticky_follow: bool,
    pub(crate) drop_unread_bodies: bool,
    pub(crate) accept_invalid_certs: bool,
    pub(crate) http1_only: bool,
    /// Which of the timeout, gzip, certificate and HTTP/1.1 only options the
    /// manager's command line gave: they win over the worker's client builder,
    /// as on one process.
    pub(crate) client_command_line: CommandLineClientOptions,
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

/// A worker's share of a users ramp: its users, each rate scaled by its
/// share of the users, and the same times, each as the command line gives
/// it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkerRamp {
    pub(crate) users: usize,
    pub(crate) increase_rate: Option<String>,
    pub(crate) increase_time: Option<String>,
    pub(crate) decrease_rate: Option<String>,
    pub(crate) decrease_time: Option<String>,
    pub(crate) run_time: Option<String>,
}

impl WorkerRamp {
    /// Each worker's ramp for a manager configured with `configuration`,
    /// running `users` each: `None` with a test plan, or when a worker has
    /// no users, which a users ramp can not start.
    fn shares(configuration: &GooseConfiguration, users: &[usize]) -> Option<Vec<WorkerRamp>> {
        if configuration.test_plan.is_some() || configuration.users.is_none() {
            return None;
        }
        if users.contains(&0) {
            return None;
        }
        let rates = |rate: &Option<String>| {
            rate.as_deref()
                .and_then(|rate| rate.parse::<f32>().ok())
                .map(|rate| super::control::rate_shares(rate, users))
        };
        // With neither an increase rate nor time, one process ramps at 1
        // user a second: each worker takes its share of that rate.
        let increase_rate = match (&configuration.increase_rate, &configuration.increase_time) {
            (None, None) => Some("1".to_string()),
            (rate, _) => rate.clone(),
        };
        let increase = rates(&increase_rate);
        let decrease = rates(&configuration.decrease_rate);
        Some(
            users
                .iter()
                .enumerate()
                .map(|(worker, &users)| WorkerRamp {
                    users,
                    increase_rate: increase.as_ref().map(|rates| rates[worker].to_string()),
                    increase_time: configuration.increase_time.clone(),
                    decrease_rate: decrease.as_ref().map(|rates| rates[worker].to_string()),
                    decrease_time: configuration.decrease_time.clone(),
                    run_time: configuration.run_time.clone(),
                })
                .collect(),
        )
    }
}

/// The most test plan steps a worker runs: a plan of more fails `Prepare`.
pub(crate) const MAX_PLAN_STEPS: usize = 100_000;

/// A share past what a worker runs.
#[derive(Debug, PartialEq)]
pub(crate) enum WorkerBound {
    /// More users than a delta may count.
    Users(usize),
    /// More test plan steps than [`MAX_PLAN_STEPS`].
    PlanSteps(usize),
}

impl std::fmt::Display for WorkerBound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerBound::Users(users) => write!(
                f,
                "{users} users, more than the {} users a worker runs",
                super::delta::DeltaLimits::default().max_users
            ),
            WorkerBound::PlanSteps(steps) => write!(
                f,
                "{steps} test plan steps, more than the {MAX_PLAN_STEPS} steps a worker runs"
            ),
        }
    }
}

/// Whether a worker runs a share of `users` users over a test plan of
/// `plan_steps` steps: the worker checks its run spec with this, and the
/// manager its largest share, before it waits for any worker.
pub(crate) fn worker_bounds(users: usize, plan_steps: usize) -> Result<(), WorkerBound> {
    if users as u64 > super::delta::DeltaLimits::default().max_users {
        return Err(WorkerBound::Users(users));
    }
    if plan_steps > MAX_PLAN_STEPS {
        return Err(WorkerBound::PlanSteps(plan_steps));
    }
    Ok(())
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

/// The users worker 0 of `workers` starts over a load test of `steps`: the
/// largest share, as [`split`] gives the first workers the remainder.
pub(crate) fn largest_share(steps: &[(usize, usize)], workers: usize) -> usize {
    let first: Vec<(usize, usize)> = steps
        .iter()
        .map(|&(users, ms)| (split(users, workers, 0), ms))
        .collect();
    users_started(&first)
}

/// The most users a test plan runs at once.
pub(crate) fn peak_users(steps: &[(usize, usize)]) -> usize {
    steps.iter().map(|&(users, _)| users).max().unwrap_or(0)
}

/// `throttle` split in proportion to `users`, the remainder spread: every
/// worker with users gets at least 1, and a worker with none gets 0, so the
/// shares add up to `throttle` whenever any worker has users and `throttle`
/// is at least the number that do. Each share is
/// its proportion rounded down, raised to 1; the rest goes one at a time
/// to the largest remainders, the first worker first among equals; and
/// what raising shares to 1 put past `throttle` is taken back one at a time
/// from the largest share, the first worker first among equals, never below
/// one. So beside many workers of one user, a large worker gives up what
/// their floors cost it, and starves none of them.
pub(crate) fn throttle_shares(throttle: usize, users: &[usize]) -> Vec<usize> {
    let workers = users.len();
    let total: u128 = users.iter().map(|&u| u as u128).sum();
    if throttle == 0 || total == 0 {
        return vec![0; workers];
    }
    let mut shares: Vec<usize> = Vec::with_capacity(workers);
    let mut remainders: Vec<(u128, usize)> = Vec::with_capacity(workers);
    for (worker, &u) in users.iter().enumerate() {
        if u == 0 {
            shares.push(0);
            continue;
        }
        let exact = throttle as u128 * u as u128;
        shares.push(((exact / total) as usize).max(1));
        remainders.push((exact % total, worker));
    }
    // Largest remainder first, then the first workers; only workers with
    // users take any.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut sum: usize = shares.iter().sum();
    let mut at = 0;
    while sum < throttle {
        shares[remainders[at % remainders.len()].1] += 1;
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
    configuration
        .logs()
        .iter()
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
                gzip: manager_file.ends_with(".gz"),
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
    client_command_line: CommandLineClientOptions,
    steps: &[(usize, usize)],
    order: &[usize],
    workers: usize,
    records_graph: bool,
) -> Vec<WorkerRunSpec> {
    let plans = plans(steps, workers);
    let users: Vec<usize> = plans.iter().map(|plan| users_started(plan)).collect();
    // The throttle follows the users each worker runs at once, at its peak:
    // for a plan that goes up, down and up again, the users it starts over
    // the run would count some twice.
    let peaks: Vec<usize> = plans.iter().map(|plan| peak_users(plan)).collect();
    // Every worker gets a share, one with no users too, as a running
    // throttle can change but not turn on.
    let throttles = match configuration.throttle_requests() {
        0 => vec![0; workers],
        throttle => super::control::live_throttle_shares(throttle, &peaks),
    };
    let mut ramps = WorkerRamp::shares(configuration, &users).map(Vec::into_iter);
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
                ramp: ramps.as_mut().and_then(Iterator::next),
                holds: steps.last().is_some_and(|&(users, _)| users > 0),
                iterations: configuration.iterations,
                co_mitigation: configuration.co_mitigation.clone(),
                timeout: configuration.timeout.clone(),
                no_gzip: configuration.no_gzip,
                sticky_follow: configuration.sticky_follow,
                drop_unread_bodies: configuration.drop_unread_bodies,
                accept_invalid_certs: configuration.accept_invalid_certs,
                http1_only: configuration.http1_only,
                client_command_line,
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
    /// Apply the spec to a worker's configuration. A worker refuses these
    /// options on its command line, so any it holds were set in the load
    /// test's code: each is replaced, and every log is off unless the
    /// manager asked for it, so a log set in code is never written where the
    /// manager would not collect it.
    pub(crate) fn configure(&self, configuration: &mut GooseConfiguration) {
        configuration.host = self.host.clone();
        configuration.users = Some(self.users);
        configuration.increase_rate = None;
        configuration.increase_time = None;
        configuration.run_time = None;
        configuration.decrease_rate = None;
        configuration.decrease_time = None;
        match &self.ramp {
            Some(ramp) => {
                configuration.users = Some(ramp.users);
                configuration.increase_rate.clone_from(&ramp.increase_rate);
                configuration.increase_time.clone_from(&ramp.increase_time);
                configuration.decrease_rate.clone_from(&ramp.decrease_rate);
                configuration.decrease_time.clone_from(&ramp.decrease_time);
                configuration.run_time.clone_from(&ramp.run_time);
                configuration.test_plan = None;
            }
            None => {
                configuration.test_plan = Some(crate::test_plan::TestPlan {
                    steps: self.test_plan.clone(),
                    current: 0,
                })
            }
        }
        configuration.scenarios = (!self.scenarios.is_empty()).then(|| crate::config::Scenarios {
            active: self.scenarios.clone(),
        });
        configuration.throttle_requests = Some(self.throttle_requests);
        configuration.iterations = self.iterations;
        configuration.co_mitigation = self.co_mitigation.clone();
        configuration.timeout = self.timeout.clone();
        configuration.no_gzip = self.no_gzip;
        configuration.sticky_follow = self.sticky_follow;
        configuration.drop_unread_bodies = self.drop_unread_bodies;
        configuration.accept_invalid_certs = self.accept_invalid_certs;
        configuration.http1_only = self.http1_only;
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
        let mut gzip = [false; LOG_NAMES.len()];
        for log in &self.logs {
            if let Some(gzipped) = gzip.get_mut(log.kind as usize) {
                *gzipped = log.gzip;
            }
        }
        configuration.gaggle_log_gzip = Some(gzip);
        for (log, format) in configuration.logs_mut() {
            *log = None;
            *format = None;
        }
        for log in &self.logs {
            // Of a kind `validate` checked.
            if let Some((path, format)) =
                IntoIterator::into_iter(configuration.logs_mut()).nth(log.kind as usize)
            {
                *path = Some(RUN_FOLDER.to_string());
                format.clone_from(&log.format);
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
            .map(|log| (log.manager_file.clone(), folder.file(&log.worker_file())))
            .collect()
    }

    /// Why a worker whose load test has `scenarios` scenarios cannot run
    /// this spec, when it cannot: a log of no kind a worker writes, or one
    /// asked for twice, a user given a scenario the load test lacks, or a
    /// spec past a size bound: more users than a delta may count, more test
    /// plan steps than [`MAX_PLAN_STEPS`], or custom data past
    /// [`MAX_WORKER_DATA`](crate::MAX_WORKER_DATA). A worker's manager is
    /// trusted no more than its network, so a bad spec fails `Prepare`
    /// rather than the worker.
    pub(crate) fn validate(&self, scenarios: usize) -> Result<(), String> {
        worker_bounds(self.users, self.test_plan.len())
            .map_err(|bound| format!("the run spec gives {bound}"))?;
        if self.custom_data.len() > crate::MAX_WORKER_DATA {
            return Err(format!(
                "the run spec's custom data is {} bytes, more than the {} bytes allowed",
                self.custom_data.len(),
                crate::MAX_WORKER_DATA
            ));
        }
        if let Some(log) = self
            .logs
            .iter()
            .find(|log| log.kind as usize >= LOG_NAMES.len())
        {
            return Err(format!(
                "the run spec asks for a log of kind {}, and only {} kinds exist",
                log.kind,
                LOG_NAMES.len()
            ));
        }
        for (n, log) in self.logs.iter().enumerate() {
            if self.logs[..n].iter().any(|before| before.kind == log.kind) {
                return Err(format!(
                    "the run spec asks for the {} log twice",
                    LOG_NAMES[log.kind as usize]
                ));
            }
        }
        // The worker starts a user for each entry of the slice.
        if self.users != self.slice.len() {
            return Err(format!(
                "the run spec gives {} users and scenarios for {}",
                self.users,
                self.slice.len()
            ));
        }
        if self.user_offset.checked_add(self.slice.len()).is_none() {
            return Err(format!(
                "the run spec's {} users from ordinal {} pass the largest ordinal",
                self.slice.len(),
                self.user_offset
            ));
        }
        if let Some(&scenario) = self.slice.iter().find(|&&scenario| scenario >= scenarios) {
            return Err(format!(
                "the run spec gives a user scenario {scenario}, and this load test has {scenarios}"
            ));
        }
        Ok(())
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
        for workers in 1..7 {
            for total in 0..40 {
                let parts: Vec<usize> = (0..workers).map(|w| split(total, workers, w)).collect();
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
    }

    /// Every step's users add up to the global step's, and the users the
    /// workers start add up to the users the plan starts, for a plan that
    /// goes up, down and up again.
    #[test]
    fn plans_add_up_at_every_step() {
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
    }

    /// One worker of 100 users beside many of 1, and idle workers: the
    /// shares add up to the throttle, every busy worker gets at least 1,
    /// no idle worker gets any, and the large worker keeps the rest.
    #[test]
    fn throttle_shares_starve_no_small_worker() {
        for small in 1..=30 {
            for idle in 0..=2 {
                let mut users = vec![100];
                users.extend(std::iter::repeat_n(1, small));
                users.extend(std::iter::repeat_n(0, idle));
                for throttle in (small + 1)..=300 {
                    let shares = throttle_shares(throttle, &users);
                    let why = format!("{throttle} {users:?}: {shares:?}");
                    assert_eq!(shares.iter().sum::<usize>(), throttle, "{}", why);
                    for (share, &u) in shares.iter().zip(&users) {
                        assert_eq!(*share == 0, u == 0, "{}", why);
                    }
                    let others: usize = shares[1..].iter().sum();
                    assert_eq!(shares[0], throttle - others, "{}", why);
                    assert!(shares[0] >= shares[1], "{}", why);
                }
            }
        }
    }

    #[test]
    fn throttle_shares_add_up_and_skip_workers_without_users() {
        assert_eq!(
            throttle_shares(3_000, &[300, 300, 300]),
            [1_000, 1_000, 1_000]
        );
        assert_eq!(throttle_shares(10, &[2, 1, 1]), [5, 3, 2]);
        assert_eq!(throttle_shares(0, &[5, 5]), [0, 0]);
        // A worker with no users gets none: its share goes to the others.
        assert_eq!(throttle_shares(4, &[10, 0]), [4, 0]);
        assert_eq!(throttle_shares(4, &[0, 0]), [0, 0]);
        // Every small input: the shares add up whenever any worker has
        // users, and no worker with users gets 0, nor one without any.
        fn each(workers: usize, f: &mut dyn FnMut(&[usize])) {
            let mut users = vec![0; workers];
            loop {
                f(&users);
                let Some(at) = users.iter().position(|&u| u < 4) else {
                    return;
                };
                users[at] += 1;
                for earlier in &mut users[..at] {
                    *earlier = 0;
                }
            }
        }
        for workers in 1..=4 {
            each(workers, &mut |users| {
                let busy = users.iter().filter(|&&u| u > 0).count();
                for throttle in busy.max(1)..=24 {
                    let shares = throttle_shares(throttle, users);
                    let sum: usize = shares.iter().sum();
                    if busy > 0 {
                        assert_eq!(sum, throttle, "{throttle} {users:?}: {shares:?}");
                    } else {
                        assert_eq!(sum, 0, "{throttle} {users:?}: {shares:?}");
                    }
                    for (share, &u) in shares.iter().zip(users) {
                        assert_eq!(*share == 0, u == 0, "{throttle} {users:?}: {shares:?}");
                    }
                }
            });
        }
        // A plan that goes up, down and up again: each worker's share
        // follows the users it runs at its peak, 4, 4 and 3, giving 8, 8
        // and 6, never the users it starts over the run, 7, 6 and 5,
        // which would give 9, 7 and 6.
        let configuration = GooseConfiguration::parse_args_default(&[
            "--host",
            "http://h",
            "--throttle-requests",
            "22",
        ])
        .unwrap();
        let steps = [(10, 1_000), (3, 1_000), (11, 1_000), (0, 0)];
        let specs = worker_specs(
            &configuration,
            CommandLineClientOptions::default(),
            &steps,
            &[0; 18],
            3,
            true,
        );
        let shares: Vec<usize> = specs.iter().map(|spec| spec.throttle_requests).collect();
        assert_eq!(shares, [8, 8, 6]);
        // A plain ramp: the peak is the users started.
        let ramp = [(9, 1_000), (9, 5_000), (0, 0)];
        for plan in plans(&ramp, 3) {
            assert_eq!(peak_users(&plan), users_started(&plan));
        }
    }

    #[test]
    fn worker_specs_cover_every_user_once() {
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
        let specs = worker_specs(
            &configuration,
            CommandLineClientOptions::default(),
            &steps,
            &order,
            3,
            true,
        );
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
    }

    /// A spec with a log of no kind, or a user given a scenario the load
    /// test lacks, is refused with the reason.
    #[test]
    fn a_bad_spec_is_refused() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--host", "http://h", "--debug-log"]).unwrap();
        let spec = worker_specs(
            &configuration,
            CommandLineClientOptions::default(),
            &[(4, 1_000), (0, 0)],
            &[0, 1, 0, 1],
            2,
            true,
        )
        .remove(0);
        assert_eq!(spec.validate(2), Ok(()));
        let mut kind = spec.clone();
        kind.logs[0].kind = LOG_NAMES.len() as u8;
        let why = kind.validate(2).unwrap_err();
        assert!(why.contains("log of kind 5"), "{}", why);
        let why = spec.validate(1).unwrap_err();
        assert!(why.contains("scenario 1"), "{}", why);
        let mut offset = spec.clone();
        offset.user_offset = usize::MAX;
        let why = offset.validate(2).unwrap_err();
        assert!(why.contains("largest ordinal"), "{}", why);
        // Users that disagree with the slice, which decides how many start.
        let mut users = offset.clone();
        users.users = 0;
        let why = users.validate(2).unwrap_err();
        assert!(why.contains("0 users and scenarios for 2"), "{}", why);
    }

    /// A spec past a size bound is refused, naming it, and one at each
    /// bound is not: its users, its test plan's steps, a log asked for
    /// twice, and its custom data.
    #[test]
    fn a_spec_past_a_size_bound_is_refused() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--host", "http://h", "--debug-log"]).unwrap();
        let spec = worker_specs(
            &configuration,
            CommandLineClientOptions::default(),
            &[(4, 1_000), (0, 0)],
            &[0, 1, 0, 1],
            2,
            true,
        )
        .remove(0);
        let refused = |spec: &WorkerRunSpec, named: &str| {
            let why = spec.validate(2).unwrap_err();
            assert!(why.contains(named), "{}", why);
        };
        let max_users = crate::gaggle::delta::DeltaLimits::default().max_users as usize;
        let users = |users: usize| {
            let mut spec = spec.clone();
            spec.users = users;
            spec.slice = vec![0; users];
            spec
        };
        assert_eq!(users(max_users).validate(2), Ok(()));
        refused(&users(max_users + 1), "more than the 1000000 users");
        let steps = |steps: usize| {
            let mut spec = spec.clone();
            spec.test_plan = vec![(0, 1); steps];
            spec
        };
        assert_eq!(steps(100_000).validate(2), Ok(()));
        refused(&steps(100_001), "more than the 100000 steps");
        let mut twice = spec.clone();
        twice.logs.push(twice.logs[0].clone());
        refused(&twice, "the debug log twice");
        let data = |bytes: usize| {
            let mut spec = spec.clone();
            spec.custom_data = vec![0; bytes];
            spec
        };
        assert_eq!(data(crate::MAX_WORKER_DATA).validate(2), Ok(()));
        refused(
            &data(crate::MAX_WORKER_DATA + 1),
            &format!("more than the {} bytes", crate::MAX_WORKER_DATA),
        );
    }

    /// A spec configures every option it carries, so a worker runs exactly
    /// its share.
    #[test]
    fn spec_configures_a_worker() {
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
        let spec = worker_specs(
            &manager,
            CommandLineClientOptions::default(),
            &[(5, 1_000), (0, 0)],
            &[0; 5],
            2,
            true,
        )
        .remove(1);
        // What the load test may have set in code, as configuring a worker
        // with its defaults gives it.
        let mut worker = GooseConfiguration {
            request_log: Some("in-code.log".to_string()),
            request_format: Some(GooseLogFormat::Csv),
            debug_log: Some("debug-in-code.log".to_string()),
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
        // Logs the load test set in code, which the manager did not ask
        // for, are off.
        assert_eq!(
            (
                worker.request_log.as_deref(),
                worker.request_format.as_ref()
            ),
            (None, None)
        );
        assert!(worker.debug_log.is_none());
        assert!(worker.transaction_log.is_none() && worker.error_log.is_none());
        assert!(worker.no_reset_metrics);
        assert!(worker.gaggle_records_graph);
        // A users ramp's worker runs a users ramp of its own.
        assert_eq!(spec.ramp.as_ref().map(|ramp| ramp.users), Some(2));
        assert!(worker.test_plan.is_none());
        // A test plan's worker runs its share of the plan.
        let manager = GooseConfiguration::parse_args_default(&[
            "--host",
            "http://example.com",
            "--test-plan",
            "5,1s;0,0s",
        ])
        .unwrap();
        let spec = worker_specs(
            &manager,
            CommandLineClientOptions::default(),
            &[(5, 1_000), (0, 0)],
            &[0; 5],
            2,
            true,
        )
        .remove(1);
        assert!(spec.ramp.is_none());
        spec.configure(&mut worker);
        assert_eq!(
            worker.test_plan.as_ref().map(|plan| plan.steps.clone()),
            Some(vec![(2, 1_000), (0, 0)])
        );
    }

    /// A users ramp's workers each run their share of the users, each rate
    /// scaled by that share and the same times; none does once a worker
    /// would have no users.
    #[test]
    fn a_users_ramp_is_shared_as_ramps() {
        let manager = GooseConfiguration::parse_args_default(&[
            "--host",
            "http://example.com",
            "--users",
            "9",
            "--increase-rate",
            "3",
            "--decrease-time",
            "10s",
            "--run-time",
            "1m",
            "--throttle-requests",
            "10",
        ])
        .unwrap();
        let steps = [(9, 3_000), (9, 60_000), (0, 10_000)];
        let specs = worker_specs(
            &manager,
            CommandLineClientOptions::default(),
            &steps,
            &[0; 9],
            3,
            true,
        );
        let ramps: Vec<WorkerRamp> = specs
            .iter()
            .map(|spec| spec.ramp.clone().unwrap())
            .collect();
        assert!(ramps.iter().all(|ramp| ramp.users == 3));
        assert!(ramps
            .iter()
            .all(|ramp| ramp.increase_rate.as_deref() == Some("1")));
        assert!(ramps
            .iter()
            .all(|ramp| ramp.decrease_time.as_deref() == Some("10s")));
        assert!(ramps
            .iter()
            .all(|ramp| ramp.run_time.as_deref() == Some("1m")));
        let throttles: Vec<usize> = specs.iter().map(|spec| spec.throttle_requests).collect();
        assert_eq!(throttles.iter().sum::<usize>(), 10);
        // With neither an increase rate nor time, the workers share one
        // user a second.
        let default =
            GooseConfiguration::parse_args_default(&["--host", "http://h", "--users", "4"])
                .unwrap();
        let specs = worker_specs(
            &default,
            CommandLineClientOptions::default(),
            &[(4, 4_000)],
            &[0; 4],
            4,
            true,
        );
        let rate: f32 = specs
            .iter()
            .map(|spec| {
                spec.ramp
                    .as_ref()
                    .unwrap()
                    .increase_rate
                    .as_deref()
                    .unwrap()
                    .parse::<f32>()
                    .unwrap()
            })
            .sum();
        assert!((rate - 1.0).abs() < 1e-5, "{}", rate);
        // Four workers for three users: the shares run test plans.
        let specs = worker_specs(
            &manager,
            CommandLineClientOptions::default(),
            &[(3, 1_000)],
            &[0; 3],
            4,
            true,
        );
        assert!(specs.iter().all(|spec| spec.ramp.is_none()));
        // A worker with no users still takes a share of the throttle.
        assert!(specs.iter().all(|spec| spec.throttle_requests > 0));
    }
}
