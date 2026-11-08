//! The users a change starts: weighted users built into [`GooseUser`]s in
//! parts, on threads off the main loop, and the batch a test plan step
//! starts them from, with the [`GooseAttack`] steps that ask for them.

use std::io;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;

use crate::config::GooseConfiguration;
use crate::goose::{self, GooseUser};
use crate::{client, GooseAttack, GooseAttackRunState, GooseError};

/// What happened to users, in order, as the tests see it.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UserEvent {
    /// A part of a batch of this many users was built: also the one user a
    /// new test plan builds to check its client can be built.
    Built(usize),
    /// The user with this ordinal started.
    Started(usize),
    /// A dropped batch handed this many built users to a blocking thread to
    /// drop, off the main loop.
    DroppedOffLoop(usize),
}

/// What the tests count, slow down and fail as users are built and started,
/// one set for each [`GooseAttack`], so tests running at once never share
/// them. Outside the tests it holds nothing, and each hook does nothing:
/// these hooks are the only test code the code that builds and starts users
/// calls, one call for each event.
#[derive(Clone, Default)]
pub(crate) struct UserTestHooks {
    /// Every user's client built (with a shared client, each user's clone of
    /// it), counted as it is built: one for each user a batch builds, plus
    /// the one user a new test plan builds, and drops, to check its client
    /// can be built.
    #[cfg(test)]
    clients_built: Arc<std::sync::atomic::AtomicUsize>,
    /// How long building each user's client takes, on top of building it.
    #[cfg(test)]
    build_delay: Duration,
    /// Set to make building every user's client from now on fail.
    #[cfg(test)]
    fail_builds: Arc<std::sync::atomic::AtomicBool>,
    /// Each part built and each user started, with when.
    #[cfg(test)]
    events: Arc<Mutex<Vec<(UserEvent, std::time::Instant)>>>,
    /// Each part built: its users, the threads it was built on, and when.
    #[cfg(test)]
    parts_built_on: Arc<Mutex<Vec<(usize, usize, std::time::Instant)>>>,
}

impl UserTestHooks {
    /// A user's client was built: under test, counted, then slowed by
    /// `build_delay`, and failed once `fail_builds` is set.
    fn client_built(&self) -> Result<(), GooseError> {
        #[cfg(test)]
        {
            std::thread::sleep(self.build_delay);
            self.clients_built
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.fail_builds.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(GooseError::Io(io::Error::other(
                    "the test failed this build",
                )));
            }
        }
        Ok(())
    }

    /// A part of `_users` users was built on `_threads` threads, before it
    /// is handed on.
    fn part_built(&self, _users: usize, _threads: usize) {
        #[cfg(test)]
        {
            self.record(UserEvent::Built(_users));
            self.parts_built_on
                .lock()
                .unwrap()
                .push((_users, _threads, std::time::Instant::now()));
        }
    }

    /// The user with ordinal `_ordinal` started.
    pub(crate) fn user_started(&self, _ordinal: usize) {
        #[cfg(test)]
        self.record(UserEvent::Started(_ordinal));
    }

    /// A dropped batch handed `_users` built users to a blocking thread to
    /// drop.
    fn dropped_off_loop(&self, _users: usize) {
        #[cfg(test)]
        self.record(UserEvent::DroppedOffLoop(_users));
    }

    #[cfg(test)]
    fn record(&self, event: UserEvent) {
        self.events
            .lock()
            .unwrap()
            .push((event, std::time::Instant::now()));
    }
}

/// A user the load test will start: the scenario it runs and the base URL
/// its requests are relative to. Weighting builds no client: a batch builds
/// the users a change starts, before they start.
pub(crate) struct WeightedUser {
    /// The index of the scenario this user runs.
    pub(crate) scenarios_index: usize,
    /// The machine name of the scenario this user runs.
    pub(crate) machine_name: Arc<str>,
    /// The base URL this user's requests are relative to.
    pub(crate) base_url: url::Url,
}

/// What a batch of users is built from, as it was when the batch was asked
/// for.
struct UserBuild {
    configuration: GooseConfiguration,
    load_test_hash: u64,
    client_strategy: Option<client::ClientStrategy>,
    user_test: UserTestHooks,
}

/// The threads users are built on: one for each available CPU, but for a
/// batch built ahead of its step (see [`ahead_threads`]).
fn build_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// The threads a batch built ahead of its step is built on until its step
/// begins: one fewer than [`build_threads`]. The users running while it
/// builds still share every CPU with it; fewer build threads take less of
/// them.
fn ahead_threads() -> usize {
    build_threads().saturating_sub(1).max(1)
}

/// Build `users` in their order, each with its own client unless one is
/// shared, on `threads` threads: the only way a user is built. `None` when
/// `cancel` is set before every client is built, and what was built is
/// dropped.
fn build_users(
    users: &[WeightedUser],
    build: &UserBuild,
    threads: usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Option<Vec<GooseUser>>, GooseError> {
    let num_cpus = threads.max(1);
    let chunk_size = users.len().div_ceil(num_cpus).max(1);
    // Clients built by every thread, for the log of a canceled build.
    let clients = std::sync::atomic::AtomicUsize::new(0);
    let clients = &clients;

    // Building a client is the expensive part. Chunks are collected back in
    // order to keep the order users were weighted in.
    let results: Vec<Result<Option<Vec<GooseUser>>, GooseError>> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(num_cpus);
        for (chunk_idx, chunk) in users.chunks(chunk_size).enumerate() {
            let base = chunk_idx * chunk_size;
            handles.push(s.spawn(move || {
                let mut built = Vec::with_capacity(chunk.len());
                for (i, user) in chunk.iter().enumerate() {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return Ok(None);
                    }
                    debug!(
                        "creating user state: {} ({})",
                        base + i,
                        user.scenarios_index
                    );
                    let reqwest_client = match &build.client_strategy {
                        None => goose::create_reqwest_client(&build.configuration)?,
                        Some(strategy) => strategy.user_client()?,
                    };
                    build.user_test.client_built()?;
                    clients.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    built.push(GooseUser::new(
                        user.scenarios_index,
                        user.machine_name.clone(),
                        user.base_url.clone(),
                        &build.configuration,
                        build.load_test_hash,
                        Some(reqwest_client),
                    )?);
                }
                Ok::<Option<Vec<GooseUser>>, GooseError>(Some(built))
            }));
        }
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    Err(GooseError::Io(io::Error::other(
                        "user initialization thread panicked",
                    )))
                })
            })
            .collect()
    });

    let mut built = Vec::with_capacity(users.len());
    for result in results {
        match result? {
            Some(chunk) => built.extend(chunk),
            None => {
                debug!(
                    "[users]: build of {} users canceled after building {} clients",
                    users.len(),
                    clients.load(std::sync::atomic::Ordering::Relaxed)
                );
                return Ok(None);
            }
        }
    }
    debug!("[scheduler]: built {} users", built.len());
    Ok(Some(built))
}

/// One part of a batch, as built: its users, the next to start last. A part
/// canceled while it builds is never handed on.
type BuiltPart = Result<Vec<GooseUser>, GooseError>;

/// The most users one part of a batch builds: 4 for each build thread, or
/// one second of the step's rate, whichever is larger, so users start as
/// soon as the first part is built and the parts after it keep up with the
/// rate wherever building can. `rate` is users a second, infinite for a step
/// that starts all its users at once.
fn part_size(rate: f64) -> usize {
    let threads = build_threads();
    let second = if rate.is_finite() {
        rate.ceil() as usize
    } else {
        usize::MAX
    };
    (threads * 4).max(second)
}

/// Build `users` in the order they start, in parts of at most `largest`
/// users, handing each part to `deliver` as soon as it is built.
///
/// Without `taken`, every part is `largest`: a batch built ahead of its
/// step, or at setup. With it, the batch's step is running, and the batch
/// says on `taken` each time it takes a part, and whether it waited for it:
/// the first part is one user a build thread, and each later part is built
/// only once the batch took the one before, so it builds at most one part
/// ahead of the users it starts and a change that drops it leaves few built;
/// a part doubles, up to `largest`, after the batch waited for one, and is
/// one user a build thread again after it did not.
///
/// Each part is built on as many threads as `threads` says when it begins:
/// a batch built ahead of its step starts with fewer, and has every thread
/// once its step begins.
///
/// Stops after a part that failed or was canceled, when `deliver` says
/// nobody takes parts, or when the batch is dropped.
fn build_parts(
    users: &[WeightedUser],
    build: &UserBuild,
    largest: usize,
    threads: &std::sync::atomic::AtomicUsize,
    cancel: &std::sync::atomic::AtomicBool,
    taken: Option<std::sync::mpsc::Receiver<bool>>,
    mut deliver: impl FnMut(BuiltPart) -> bool,
) {
    let part_threads = || threads.load(std::sync::atomic::Ordering::Relaxed).max(1);
    let mut size = if taken.is_some() {
        part_threads().min(largest)
    } else {
        largest
    };
    // The next user to start is the last, so parts start from the end.
    let mut rest = users;
    while !rest.is_empty() {
        let (left, part) = rest.split_at(rest.len() - size.min(rest.len()));
        let building = std::time::Instant::now();
        let on = part_threads();
        let built = match build_users(part, build, on, cancel) {
            // Canceled: the batch was dropped, and what was built with it.
            Ok(None) => return,
            Ok(Some(users)) => {
                build.user_test.part_built(users.len(), on);
                Ok(users)
            }
            Err(e) => Err(e),
        };
        let build_ms = building.elapsed().as_millis();
        let more = built.is_ok();
        if !deliver(built) || !more {
            return;
        }
        rest = left;
        let waiting = std::time::Instant::now();
        if let (Some(taken), false) = (&taken, rest.is_empty()) {
            size = match taken.recv() {
                Ok(true) => size.saturating_mul(2).min(largest),
                Ok(false) => part_threads().min(largest),
                // The batch was dropped.
                Err(_) => return,
            };
        }
        debug!(
            "[users]: built a part of {} users on {on} threads in {build_ms} ms, then waited {} ms for it to be taken, {} left, next part {}",
            part.len(),
            waiting.elapsed().as_millis(),
            rest.len(),
            size.min(rest.len())
        );
    }
}

/// The users built, or being built, for the users a test plan step starts,
/// in parts. Dropping it stops a build still running, and drops what it
/// built.
pub(crate) struct UserBatch {
    /// The index of the test plan step that starts these users.
    pub(crate) step: usize,
    /// How many users it builds.
    size: usize,
    /// How many of them have not started.
    left: usize,
    user_test: UserTestHooks,
    /// Set to stop a build still running.
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// The threads each part still to build is built on: see [`build_parts`].
    threads: Arc<std::sync::atomic::AtomicUsize>,
    /// Every build thread, read once, as its step builds on them all.
    all_threads: usize,
    /// Each part as it is built, in the order the users start.
    parts: tokio::sync::mpsc::UnboundedReceiver<BuiltPart>,
    /// While its step runs, where the batch says it took a part, and whether
    /// it waited for it, so the next part is built: see [`build_parts`].
    taken: Option<std::sync::mpsc::Sender<bool>>,
    /// The part users start from now, the next last.
    next: Vec<GooseUser>,
    /// Whether the next user was due before its part was built, so its start
    /// is late and the users after it are due from it, never sooner.
    pub(crate) stalled: bool,
    /// How long the main loop has waited for the part it waits for now.
    waited: Duration,
    /// Whether building a part failed, so its step starts no more users.
    pub(crate) failed: bool,
}

/// How the current increase step's users have started, for the log.
pub(crate) struct StepStarts {
    /// The test plan step.
    step: usize,
    /// The second since its boundary that the last user started in.
    second: u64,
    /// The users started in that second.
    in_second: usize,
}

impl UserBatch {
    /// A batch of `size` users built from `build`, whose parts come on
    /// `parts`.
    fn new(
        step: usize,
        size: usize,
        parts: tokio::sync::mpsc::UnboundedReceiver<BuiltPart>,
        taken: Option<std::sync::mpsc::Sender<bool>>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        build: &UserBuild,
    ) -> Self {
        let all_threads = build_threads();
        UserBatch {
            step,
            size,
            left: size,
            user_test: build.user_test.clone(),
            cancel,
            threads: Arc::new(std::sync::atomic::AtomicUsize::new(all_threads)),
            all_threads,
            parts,
            taken,
            next: Vec::new(),
            stalled: false,
            waited: Duration::ZERO,
            failed: false,
        }
    }

    /// Build `users` for `step`, at `rate` users a second, in one blocking
    /// task, so building never blocks the main loop: each part goes on the
    /// channel as soon as it is built. `running` when the step runs now, so
    /// building keeps pace with the users it starts.
    fn build(
        step: usize,
        rate: f64,
        users: Vec<WeightedUser>,
        build: UserBuild,
        running: bool,
    ) -> Self {
        let size = users.len();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (parts_tx, parts) = tokio::sync::mpsc::unbounded_channel();
        let (taken_tx, taken) = if running {
            let (tx, rx) = std::sync::mpsc::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let batch = UserBatch::new(step, size, parts, taken_tx, cancel.clone(), &build);
        // A batch built ahead of its step builds on fewer threads until its
        // step begins.
        let threads = batch.threads.clone();
        if !running {
            threads.store(ahead_threads(), std::sync::atomic::Ordering::Relaxed);
        }
        tokio::task::spawn_blocking(move || {
            // Nobody takes the parts of a batch that was dropped.
            build_parts(
                &users,
                &build,
                part_size(rate),
                &threads,
                &cancel,
                taken,
                |built| parts_tx.send(built).is_ok(),
            );
        });
        batch
    }

    /// The next user to start, waiting up to `wait` for its part: `None`
    /// while its part builds, an error when building it failed.
    async fn next_user(&mut self, wait: Duration) -> Result<Option<GooseUser>, String> {
        if self.next.is_empty() {
            // A part already on the channel is taken without waiting.
            let part = match self.parts.try_recv() {
                Ok(part) => Some(part),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => None,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    // The next user was due before its part was built.
                    self.stalled = true;
                    let waiting = std::time::Instant::now();
                    let part = tokio::time::timeout(wait, self.parts.recv()).await;
                    self.waited += waiting.elapsed();
                    match part {
                        Ok(part) => part,
                        Err(_) => return Ok(None),
                    }
                }
            };
            match part {
                Some(Ok(users)) => {
                    let waited = std::mem::take(&mut self.waited);
                    debug!(
                        "[users]: step {} took a part of {} users after waiting {} ms for it, stalled {}",
                        self.step,
                        users.len(),
                        waited.as_millis(),
                        self.stalled
                    );
                    self.next = users;
                    // The next part is built from now: a builder that stopped
                    // takes no more.
                    if let Some(taken) = &self.taken {
                        let _ = taken.send(self.stalled);
                    }
                }
                Some(Err(e)) => return Err(e.to_string()),
                None => return Err("its build stopped".to_string()),
            }
        }
        let user = self.next.pop();
        if user.is_some() {
            self.left -= 1;
        }
        Ok(user)
    }
}

impl Drop for UserBatch {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Dropping many users' clients takes long enough to hold up the main
        // loop, so a blocking thread drops them when there is a runtime: the
        // part users start from, and every part built and not yet taken. A
        // part still building drops what it built on its own thread.
        // Gathered as parts, not one Vec, so the main loop moves no user.
        let mut parts = vec![std::mem::take(&mut self.next)];
        // Closed first, so a part sent after the drain is refused and its
        // builder drops it; only a send already under way as it closes can
        // still be missed here.
        self.parts.close();
        while let Ok(part) = self.parts.try_recv() {
            if let Ok(part) = part {
                parts.push(part);
            }
        }
        let users: usize = parts.iter().map(Vec::len).sum();
        debug!(
            "[users]: dropping the batch of {} users for step {}: {} built and not started",
            self.size, self.step, users
        );
        if users == 0 {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            self.user_test.dropped_off_loop(users);
            runtime.spawn_blocking(move || drop(parts));
        }
    }
}

impl GooseAttack {
    /// What users built now are built from.
    fn user_build(&self) -> UserBuild {
        UserBuild {
            configuration: self.configuration.clone(),
            load_test_hash: self.metrics.hash,
            client_strategy: self.client_strategy.clone(),
            user_test: self.user_test.clone(),
        }
    }

    /// Take the next `users` weighted users to start, in their order: those
    /// left, when fewer are. Every caller asks for at most those left.
    fn take_weighted_users(&mut self, users: usize) -> Vec<WeightedUser> {
        let first = self.weighted_users.len().saturating_sub(users);
        self.weighted_users.split_off(first)
    }

    /// The index of the first test plan step after `step` (from the first,
    /// for `None`) that raises the users, and how many users it adds.
    fn next_increase(&self, step: Option<usize>) -> Option<(usize, usize)> {
        let steps = &self.test_plan.steps;
        let first = step.map_or(0, |step| step + 1);
        (first..steps.len()).find_map(|index| {
            let previous = if index == 0 { 0 } else { steps[index - 1].0 };
            (steps[index].0 > previous).then(|| (index, steps[index].0 - previous))
        })
    }

    /// The users a second `step` starts: infinite when it starts them all at
    /// once.
    pub(crate) fn step_rate(&self, step: usize) -> f64 {
        self.step_users_added(step) as f64 / self.test_plan.steps[step].1 as f64 * 1_000.0
    }

    /// The users `step` adds to the step before it, 0 unless it raises them.
    pub(crate) fn step_users_added(&self, step: usize) -> usize {
        let steps = &self.test_plan.steps;
        let previous = if step == 0 { 0 } else { steps[step - 1].0 };
        steps[step].0.saturating_sub(previous)
    }

    /// Build the users the first increase starts, before the load test
    /// starts, so a client that can not be built fails it now.
    pub(crate) fn build_first_users(&mut self) -> Result<(), GooseError> {
        self.user_batch = None;
        if let Some((step, users)) = self.next_increase(None) {
            let users = self.take_weighted_users(users);
            let build = self.user_build();
            let (parts_tx, parts) = tokio::sync::mpsc::unbounded_channel();
            let not_canceled = std::sync::atomic::AtomicBool::new(false);
            let mut failed = None;
            build_parts(
                &users,
                &build,
                part_size(self.step_rate(step)),
                &std::sync::atomic::AtomicUsize::new(build_threads()),
                &not_canceled,
                None,
                |built| match built {
                    // The receiver is held below, so the send succeeds.
                    Ok(built) => parts_tx.send(Ok(built)).is_ok(),
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                },
            );
            if let Some(e) = failed {
                return Err(e);
            }
            self.user_batch = Some(UserBatch::new(
                step,
                users.len(),
                parts,
                None,
                Arc::default(),
                &build,
            ));
        }
        Ok(())
    }

    /// Build one user as the new test plan's users will be built, and drop
    /// it, so a client that can not be built fails the change at once.
    pub(crate) fn check_user_client(&self) -> Result<(), GooseError> {
        if let Some(user) = self.weighted_users.last() {
            let not_canceled = std::sync::atomic::AtomicBool::new(false);
            build_users(
                std::slice::from_ref(user),
                &self.user_build(),
                1,
                &not_canceled,
            )?;
            self.user_test.part_built(1, 1);
        }
        Ok(())
    }

    /// Once a step's users have all started or stopped: when a test plan
    /// raises the users in a later step, build the users it adds now, and
    /// hold them until it begins. Only a test plan's steps are known ahead.
    pub(crate) fn build_ahead(&mut self) {
        if self.user_batch.is_some() || self.run_shape.is_some() || self.is_gaggle_manager() {
            return;
        }
        let Some((step, users)) = self.next_increase(Some(self.test_plan.current)) else {
            return;
        };
        if users > self.weighted_users.len() {
            return;
        }
        debug!("[users]: building the {users} users test plan step {step} starts");
        let users = self.take_weighted_users(users);
        let rate = self.step_rate(step);
        self.user_batch = Some(UserBatch::build(
            step,
            rate,
            users,
            self.user_build(),
            false,
        ));
    }

    /// The next user the current step starts, once its part is built: asks
    /// for a batch of the users the step still starts when there is none,
    /// and waits up to 100 ms for a part still building. `None` while its
    /// part builds, or when building failed and no more start.
    pub(crate) async fn next_user(
        &mut self,
        goose_attack_run_state: &GooseAttackRunState,
    ) -> Option<GooseUser> {
        let step = self.test_plan.current;
        let batch = match self.user_batch.take() {
            Some(batch) if batch.step == step && (batch.left > 0 || batch.failed) => batch,
            other => {
                // After a batch that failed, fewer users may be left than the
                // step still starts: it starts those left.
                let needed = (self.test_plan.steps[step].0 - goose_attack_run_state.active_users)
                    .min(self.weighted_users.len());
                let users = self.take_weighted_users(needed);
                let rate = self.step_rate(step);
                let mut batch = UserBatch::build(step, rate, users, self.user_build(), true);
                if needed == 0 {
                    error!("[users]: no users are left for test plan step {step} to start, as building earlier users failed");
                    batch.failed = true;
                }
                // Any other batch is dropped once this one is asked for.
                drop(other);
                batch
            }
        };
        let batch = self.user_batch.insert(batch);
        // Its step runs: a batch built ahead builds what is left of it on
        // every thread.
        batch
            .threads
            .store(batch.all_threads, std::sync::atomic::Ordering::Relaxed);
        if batch.failed {
            return None;
        }
        match batch.next_user(Duration::from_millis(100)).await {
            Ok(user) => user,
            Err(e) => {
                let (first, left) = (self.metrics.total_users + 1, batch.left);
                error!(
                    "[users]: failed to build users {first} to {}, starting none of them: {e}",
                    first + left - 1
                );
                batch.failed = true;
                // A Gaggle worker's users that never start leave their ordinals.
                #[cfg(feature = "gaggle")]
                self.user_ordinals
                    .drain(..left.min(self.user_ordinals.len()));
                None
            }
        }
    }

    /// Log how the current increase step's users start against its `rate`,
    /// `users` a second: its first new user's delay from the boundary, then
    /// each second that started fewer than the rate. Called as each user
    /// starts, and with `last` once the step's users have all started.
    pub(crate) fn log_step_starts(&mut self, rate: f64, users: usize, last: bool) {
        let step = self.test_plan.current;
        let Some(boundary) = self.step_started else {
            return;
        };
        let elapsed = boundary.elapsed();
        let starts = match self.step_starts.take() {
            Some(starts) if starts.step == step => starts,
            other if last => {
                self.step_starts = other;
                return;
            }
            _ => {
                let at = chrono::Utc::now()
                    - chrono::Duration::from_std(elapsed)
                        .unwrap_or_else(|_| chrono::Duration::zero());
                info!(
                    "[users]: test plan step {step} began at {}, its first new user started {} ms after it",
                    at.format("%H:%M:%S%.3f"),
                    elapsed.as_millis()
                );
                StepStarts {
                    step,
                    second: 0,
                    in_second: 0,
                }
            }
        };
        let starts = self.step_starts.insert(starts);
        // Each second ended since the last start, then the one in progress
        // once the last user started.
        let second = elapsed.as_secs();
        while starts.second < second || (last && starts.second == second) {
            if rate.is_finite() {
                // The first user is due at the boundary.
                let due_by = |second: u64| ((rate * second as f64).ceil() as usize).min(users);
                let due = due_by(starts.second + 1) - due_by(starts.second);
                if starts.in_second < due {
                    info!(
                        "[users]: test plan step {step} started {} of the {due} users due in second {}",
                        starts.in_second, starts.second
                    );
                }
            }
            starts.second += 1;
            starts.in_second = 0;
        }
        if last {
            self.step_starts = None;
        } else {
            starts.in_second += 1;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::*;

    /// A load test of `scenarios` against `host` with `args` on its command
    /// line, prepared and started as `execute` does, with no user started
    /// yet. Each user's client takes `delay` longer to build.
    pub(crate) async fn started_attack(
        host: &str,
        args: &[&str],
        scenarios: Vec<Scenario>,
        delay: Duration,
    ) -> (GooseAttack, GooseAttackRunState) {
        let mut line = vec!["--no-telnet", "--no-websocket", "--quiet", "--host", host];
        line.extend_from_slice(args);
        let configuration = GooseConfiguration::parse_args_default(&line).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        goose_attack.user_test.build_delay = delay;
        for scenario in scenarios {
            goose_attack = goose_attack.register_scenario(scenario);
        }
        let defaults = goose_attack.defaults.clone();
        goose_attack.configuration.configure(&defaults).unwrap();
        goose_attack.test_plan = TestPlan::build(&goose_attack.configuration).unwrap();
        goose_attack.run_shape =
            run_shape::RunShape::for_configuration(&goose_attack.configuration);
        goose_attack
            .build_client_strategy(client::CommandLineClientOptions::default())
            .unwrap();
        goose_attack.prepare_load_test().unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        goose_attack.reset_run_state(&mut run_state).await.unwrap();
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Increase);
        (goose_attack, run_state)
    }

    /// `--users` `users`, started 1,000 a second, held for an hour.
    async fn users_attack(
        host: &str,
        users: usize,
        scenarios: Vec<Scenario>,
        delay: Duration,
    ) -> (GooseAttack, GooseAttackRunState) {
        let users = users.to_string();
        let args = [
            "--users",
            &users,
            "--increase-rate",
            "1000",
            "--decrease-rate",
            "1000",
            "--run-time",
            "1h",
        ];
        started_attack(host, &args, scenarios, delay).await
    }

    /// Run one turn of the step machine, as the main loop does, and return
    /// whether the load test now holds its users.
    async fn step(goose_attack: &mut GooseAttack, run_state: &mut GooseAttackRunState) -> bool {
        match goose_attack.attack_phase {
            AttackPhase::Increase => goose_attack.increase_attack(run_state).await.unwrap(),
            AttackPhase::Decrease => goose_attack.decrease_attack(run_state).await.unwrap(),
            AttackPhase::Maintain => return true,
            phase => panic!("unexpected phase {:?}", phase),
        }
        goose_attack.attack_phase == AttackPhase::Maintain
    }

    /// Run the step machine until every user a change starts or stops has
    /// been, and the load test holds.
    async fn ramp_to_hold(goose_attack: &mut GooseAttack, run_state: &mut GooseAttackRunState) {
        while !step(goose_attack, run_state).await {}
    }

    /// Requests `/` from the mock server, then waits `wait_ms`.
    pub(crate) fn index_scenario(name: &str, wait_ms: u64) -> Scenario {
        async fn get_index(user: &mut GooseUser) -> crate::goose::TransactionResult {
            let _goose = user.get("/").await?;
            Ok(())
        }
        scenario!(name)
            .set_wait_time(
                Duration::from_millis(wait_ms),
                Duration::from_millis(wait_ms),
            )
            .unwrap()
            .register_transaction(transaction!(get_index))
    }

    /// A mock server answering `/`.
    pub(crate) fn index_server() -> httpmock::MockServer {
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/");
            then.status(200);
        });
        server
    }

    /// Wait, as the main loop holds, until `done` or 10 seconds have passed.
    async fn hold_until(
        goose_attack: &mut GooseAttack,
        run_state: &mut GooseAttackRunState,
        done: impl Fn(&GooseAttack, &GooseAttackRunState) -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done(goose_attack, run_state) && std::time::Instant::now() < deadline {
            goose_attack.maintain_attack(run_state).await.unwrap();
        }
    }

    /// How many clients have been built for users so far.
    fn clients_built(goose_attack: &GooseAttack) -> usize {
        goose_attack
            .user_test
            .clients_built
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Every batch built and user started so far, in order.
    fn user_events(goose_attack: &GooseAttack) -> Vec<(UserEvent, std::time::Instant)> {
        goose_attack.user_test.events.lock().unwrap().clone()
    }

    /// How many users have started so far.
    fn users_started(goose_attack: &GooseAttack) -> usize {
        user_events(goose_attack)
            .iter()
            .filter(|(event, _)| matches!(event, UserEvent::Started(_)))
            .count()
    }

    /// Weighting users builds no client: a change builds only the clients of
    /// the users it starts, so a ramp of 5 builds 5.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_weighting_builds_no_client_starting_builds_one() {
        let server = index_server();
        let (mut goose_attack, mut run_state) = users_attack(
            &server.base_url(),
            5,
            vec![index_scenario("Lazy", 10)],
            Duration::ZERO,
        )
        .await;
        // Setup builds the clients of the 5 users the ramp starts.
        assert_eq!(clients_built(&goose_attack), 5, "setup built other than 5");
        // Weighting many more, as a Controller increase does, builds none.
        let more = goose_attack.weight_scenario_users(100).unwrap();
        assert_eq!(more.len(), 100);
        assert_eq!(
            clients_built(&goose_attack),
            5,
            "weighting 100 users built clients"
        );
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        assert_eq!(goose_attack.metrics.total_users, 5);
        assert_eq!(
            clients_built(&goose_attack),
            5,
            "5 users built other than 5"
        );
    }

    /// A storm of `users` changes, each landing before the last finished, as
    /// the Controller sends them: no replaced change's batch finishes, so
    /// each builds at most one client on each build thread beyond the users
    /// started, and only the running users' tasks are held (releasing
    /// stopped users' tasks is the cycles test's).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_users_storm_builds_and_holds_only_started_users() {
        use crate::controller::tests::control;

        let server = index_server();
        let (mut goose_attack, mut run_state) = users_attack(
            &server.base_url(),
            10,
            vec![index_scenario("Storm", 10)],
            // Well over the 100 ms a turn waits for a batch: each change's
            // batch is still on its first client per thread when the next lands.
            Duration::from_millis(500),
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        let changes = 20;
        for _ in 0..changes {
            for users in ["users 1010", "users 10"] {
                assert!(control(&mut goose_attack, &mut run_state, users).await);
                // One turn of the step machine before the next change lands.
                step(&mut goose_attack, &mut run_state).await;
            }
        }
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        assert_eq!(run_state.active_users, 10);
        hold_until(&mut goose_attack, &mut run_state, |_, run_state| {
            run_state.users.len() == run_state.active_users
        })
        .await;
        let started = goose_attack.metrics.total_users;
        assert!(
            clients_built(&goose_attack) <= started + changes * build_threads(),
            "{} clients built for {} users started",
            clients_built(&goose_attack),
            started
        );
        assert_eq!(
            run_state.users.len(),
            run_state.active_users,
            "user tasks held, against users running"
        );
    }

    /// Changes landing every 300 ms while a batch builds far faster than
    /// its users may start, 10 a second: a batch for a running step builds
    /// little ahead of the users it starts, so the changes leave few clients
    /// built that never start.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_a_storm_builds_little_beyond_the_users_it_starts() {
        use crate::controller::tests::control;

        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--users", "10", "--increase-rate", "10", "--run-time", "1h"],
            vec![index_scenario("Paced", 10)],
            Duration::ZERO,
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        let (built, started) = (
            clients_built(&goose_attack),
            goose_attack.metrics.total_users,
        );
        let changes = 10;
        for _ in 0..changes {
            for users in ["users 1010", "users 10"] {
                assert!(control(&mut goose_attack, &mut run_state, users).await);
                let until = std::time::Instant::now() + Duration::from_millis(300);
                while std::time::Instant::now() < until {
                    if step(&mut goose_attack, &mut run_state).await {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            }
        }
        let built = clients_built(&goose_attack) - built;
        let started = goose_attack.metrics.total_users - started;
        assert!(
            built <= started + changes * 5 * build_threads(),
            "{} clients built for {} users started over {} increases",
            built,
            started,
            changes
        );
    }

    /// Users changes that each run to completion, with no hold between them
    /// in which the main loop prunes: stopped users' tasks are still released
    /// as new users start, so the tasks held stay bounded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_users_cycles_release_stopped_tasks_as_users_start() {
        use crate::controller::tests::control;

        let server = index_server();
        // A user told to exit ends once its request is answered, so a wait
        // long enough not to flood the mock server keeps stopping users short.
        let (mut goose_attack, mut run_state) = users_attack(
            &server.base_url(),
            10,
            vec![index_scenario("Cycle", 100)],
            Duration::ZERO,
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        let mut most_held = 0;
        for _ in 0..20 {
            for users in ["users 50", "users 10"] {
                assert!(control(&mut goose_attack, &mut run_state, users).await);
                ramp_to_hold(&mut goose_attack, &mut run_state).await;
                most_held = most_held.max(run_state.users.len());
            }
        }
        assert_eq!(goose_attack.metrics.total_users, 810);
        // Without releasing stopped users' tasks, every one of the 810 is
        // held; a stopping user against the mock server can take seconds to
        // end, so some are still held.
        assert!(
            most_held < 810,
            "{} user tasks held for at most 50 running users",
            most_held
        );
    }

    /// Users of a weighted load test with several scenarios start in the
    /// order, with the ordinals and scenarios, main gave them, at setup and
    /// on a Controller increase.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_weighted_users_start_in_base_order() {
        use crate::controller::tests::control;

        let server = index_server();
        let scenarios = vec![
            index_scenario("Heavy", 10).set_weight(3).unwrap(),
            index_scenario("Middle", 10).set_weight(2).unwrap(),
            index_scenario("Light", 10),
        ];
        let (mut goose_attack, mut run_state) =
            users_attack(&server.base_url(), 8, scenarios, Duration::ZERO).await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        assert!(control(&mut goose_attack, &mut run_state, "users 14").await);
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        // (scenario index, user ordinal) of each user, in the order started.
        let started: Vec<(usize, usize)> = run_state.running_users.clone();
        assert_eq!(started, BASE_ORDER);
    }

    /// The order `test_weighted_users_start_in_base_order` recorded on main
    /// at 23d8d409, when every weighted user's client was built at once.
    const BASE_ORDER: &[(usize, usize)] = &[
        (1, 0),
        (0, 1),
        (0, 2),
        (1, 3),
        (0, 4),
        (2, 5),
        (1, 6),
        (0, 7),
        (0, 8),
        (1, 9),
        (0, 10),
        (2, 11),
        (1, 12),
        (0, 13),
    ];

    /// With clients that take 20 ms each to build, a ramp of 50 users holds,
    /// and resets its metrics, only once all 50 run, and its Maintain row
    /// says 50 users; and so does a Controller increase of 50 more, whose
    /// users are built as it runs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_ramp_holds_once_every_user_runs() {
        use crate::controller::tests::control;

        let server = index_server();
        let (mut goose_attack, mut run_state) = users_attack(
            &server.base_url(),
            50,
            vec![index_scenario("Slow", 10)],
            Duration::from_millis(20),
        )
        .await;
        while !step(&mut goose_attack, &mut run_state).await {
            if run_state.all_users_spawned {
                break;
            }
        }
        // The metrics reset and Maintain both come with every user running.
        assert!(run_state.all_users_spawned, "the metrics were not reset");
        assert_eq!(goose_attack.attack_phase, AttackPhase::Maintain);
        assert_eq!(
            users_started(&goose_attack),
            50,
            "users running at Maintain"
        );
        assert_eq!(run_state.active_users, 50);
        let maintain = goose_attack.metrics.history.last().unwrap();
        assert!(matches!(maintain.action, TestPlanStepAction::Maintaining));
        assert_eq!(maintain.users, 50);

        // 50 more, built after the change is answered.
        assert!(control(&mut goose_attack, &mut run_state, "users 100").await);
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        assert_eq!(
            users_started(&goose_attack),
            100,
            "users running at Maintain"
        );
        assert_eq!(run_state.active_users, 100);
        let maintain = goose_attack.metrics.history.last().unwrap();
        assert!(matches!(maintain.action, TestPlanStepAction::Maintaining));
        assert_eq!(maintain.users, 100);
    }

    /// No user starts before every client of its part is built: the 10 a
    /// ramp starts, then the 20 a Controller increase adds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_no_user_starts_before_its_batch_is_built() {
        use crate::controller::tests::control;

        let server = index_server();
        let (mut goose_attack, mut run_state) = users_attack(
            &server.base_url(),
            10,
            vec![index_scenario("Batch", 10)],
            Duration::from_millis(20),
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        assert!(control(&mut goose_attack, &mut run_state, "users 30").await);
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        // Users start in order, so each starts after the part it is in
        // when the users started never outrun the users built.
        let (mut built, mut started) = (0, 0);
        for (event, _) in user_events(&goose_attack) {
            match event {
                UserEvent::Built(part) => built += part,
                UserEvent::Started(ordinal) => {
                    started += 1;
                    assert!(
                        started <= built,
                        "user {} started before its part was built",
                        ordinal
                    );
                }
                UserEvent::DroppedOffLoop(_) => {}
            }
        }
        assert_eq!(built, 30);
        assert_eq!(users_started(&goose_attack), 30);
    }

    /// A test plan's increase after a maintain step has its batch built
    /// ahead, before the step begins, and its users start at the step's rate
    /// from its start.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_test_plan_builds_the_next_increase_ahead() {
        let server = index_server();
        // 5 users at once, held 2 s, then 10 more over 2 s: 5 a second.
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", "5,0s;5,2s;15,2s;15,1h"],
            vec![index_scenario("Plan", 10)],
            Duration::from_millis(20),
        )
        .await;
        // Run the step machine, noting when each step began.
        let mut step_began = std::collections::BTreeMap::new();
        loop {
            let current = goose_attack.test_plan.current;
            step_began.entry(current).or_insert_with(|| {
                goose_attack
                    .step_started
                    .unwrap_or_else(std::time::Instant::now)
            });
            if goose_attack.attack_phase == AttackPhase::Maintain && current >= 3 {
                break;
            }
            match goose_attack.attack_phase {
                AttackPhase::Increase => {
                    goose_attack.increase_attack(&mut run_state).await.unwrap()
                }
                AttackPhase::Maintain => {
                    goose_attack.maintain_attack(&mut run_state).await.unwrap()
                }
                phase => panic!("unexpected phase {:?}", phase),
            }
        }
        assert_eq!(run_state.active_users, 15);
        let events = user_events(&goose_attack);
        let built: Vec<(usize, std::time::Instant)> = events
            .iter()
            .filter_map(|&(event, at)| match event {
                UserEvent::Built(users) => Some((users, at)),
                _ => None,
            })
            .collect();
        // Step 2 is the increase after the maintain step: every part of its
        // 10 users, after setup's 5, was built before it began.
        let increase_began = step_began[&2];
        let ahead: usize = built
            .iter()
            .skip(1)
            .filter(|(_, at)| *at < increase_began)
            .map(|(users, _)| users)
            .sum();
        assert_eq!(
            ahead, 10,
            "the increase's users were not all built before its step began"
        );
        // Its 10 users start from the step's start at 5 a second: the last
        // no sooner than 1.6 s after the first.
        let starts: Vec<std::time::Instant> = events
            .iter()
            .filter_map(|&(event, at)| match event {
                UserEvent::Started(_) => Some(at),
                _ => None,
            })
            .skip(5)
            .collect();
        assert_eq!(starts.len(), 10);
        assert!(starts[0] >= increase_began);
        let spread = starts[9] - starts[0];
        assert!(
            spread >= Duration::from_millis(1_600),
            "10 users started over {:?} at 5 a second",
            spread
        );
    }

    /// A step whose users take longer to build than it lasts starts them as
    /// each part is built, from its boundary: before its last part is built,
    /// never before a user's own part, and never faster than its rate.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_a_step_starts_users_as_its_parts_are_built() {
        let server = index_server();
        // 4 users a second for each build thread, so each part is 4 users a
        // thread, 200 ms at 50 ms a client: 5 parts, about 1 s to build.
        let rate = 4 * build_threads();
        let users = 5 * rate;
        let plan = format!("2,0s;{0},5s;{0},1h", 2 + users);
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", &plan],
            vec![index_scenario("Parts", 10)],
            Duration::from_millis(50),
        )
        .await;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !(goose_attack.attack_phase == AttackPhase::Maintain
            && goose_attack.test_plan.current >= 2)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the step's users did not all start"
            );
            match goose_attack.attack_phase {
                AttackPhase::Increase => {
                    goose_attack.increase_attack(&mut run_state).await.unwrap()
                }
                AttackPhase::Maintain => {
                    goose_attack.maintain_attack(&mut run_state).await.unwrap()
                }
                phase => panic!("unexpected phase {:?}", phase),
            }
        }
        assert_eq!(run_state.active_users, 2 + users);
        // Setup built the first 2; the rest are the step's.
        let events: Vec<(UserEvent, std::time::Instant)> =
            user_events(&goose_attack).into_iter().skip(1).collect();
        let (mut built, mut started, mut starts, mut last_built) = (0, 0, Vec::new(), None);
        for (at, &(event, when)) in events.iter().enumerate() {
            match event {
                UserEvent::Built(part) => {
                    built += part;
                    last_built = Some(at);
                }
                UserEvent::Started(ordinal) if ordinal >= 2 => {
                    started += 1;
                    starts.push((at, when));
                    assert!(
                        started <= built,
                        "user {} started before its part was built",
                        ordinal
                    );
                }
                _ => {}
            }
        }
        assert_eq!(built, users);
        assert!(
            starts[0].0 < last_built.unwrap(),
            "no user started before the step's last part was built"
        );
        let interval = Duration::from_secs(1) / rate as u32;
        for (k, &(_, when)) in starts.iter().enumerate() {
            assert!(
                when - starts[0].1 + Duration::from_millis(50) >= interval * k as u32,
                "user {} of the step started faster than {} a second",
                k,
                rate
            );
        }
    }

    /// A part that lands while the next user is due, inside the wait for
    /// it, stalls the batch as one that lands after the wait does.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_a_part_landing_inside_the_wait_stalls_the_batch() {
        let server = index_server();
        let (mut goose_attack, _run_state) = users_attack(
            &server.base_url(),
            1,
            vec![index_scenario("Wait", 10)],
            Duration::ZERO,
        )
        .await;
        let build = goose_attack.user_build();
        let weighted = goose_attack.weight_scenario_users(1).unwrap();
        let not_canceled = std::sync::atomic::AtomicBool::new(false);
        let users = build_users(&weighted, &build, build_threads(), &not_canceled)
            .unwrap()
            .unwrap();
        let (parts_tx, parts) = tokio::sync::mpsc::unbounded_channel();
        let mut batch = UserBatch::new(1, 1, parts, None, Arc::default(), &build);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = parts_tx.send(Ok(users));
        });
        let user = batch.next_user(Duration::from_secs(10)).await.unwrap();
        assert!(user.is_some(), "the part did not land");
        assert!(
            batch.stalled,
            "a part that landed inside the wait did not stall the batch"
        );
    }

    /// A user due before its part was built starts once the part lands, and
    /// restarts the schedule from its own start: the next user is due one
    /// interval after now, not one interval after the time the late user
    /// was due, which would start every user behind it at once to catch up.
    /// One thread, so the part is sent only after the stalled turn returns.
    #[tokio::test(flavor = "current_thread")]
    async fn test_a_stalled_batch_restarts_the_schedule() {
        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--users", "3", "--increase-rate", "1", "--run-time", "1h"],
            vec![index_scenario("Stall", 10)],
            Duration::ZERO,
        )
        .await;
        // The first user starts from the batch setup built.
        goose_attack.increase_attack(&mut run_state).await.unwrap();
        assert_eq!(run_state.active_users, 1);
        // The other two come in a part the test sends.
        let build = goose_attack.user_build();
        let weighted = goose_attack.weight_scenario_users(2).unwrap();
        let not_canceled = std::sync::atomic::AtomicBool::new(false);
        let users = build_users(&weighted, &build, build_threads(), &not_canceled)
            .unwrap()
            .unwrap();
        let (parts_tx, parts) = tokio::sync::mpsc::unbounded_channel();
        goose_attack.user_batch = Some(UserBatch::new(
            goose_attack.test_plan.current,
            2,
            parts,
            None,
            Arc::default(),
            &build,
        ));
        // The next user was due a minute ago: far enough back that one
        // interval after it is still past, so a schedule that catches up
        // instead of restarting leaves the next user due at once.
        run_state.adjust_user_timer = std::time::Instant::now() - Duration::from_secs(60);
        // Its part is not built: nothing starts, and the batch stalls.
        goose_attack.increase_attack(&mut run_state).await.unwrap();
        assert_eq!(run_state.active_users, 1, "a user started with no part");
        assert!(
            goose_attack
                .user_batch
                .as_ref()
                .is_some_and(|batch| batch.stalled),
            "an empty channel did not stall the batch"
        );
        // The part lands, and the late user starts.
        parts_tx.send(Ok(users)).unwrap();
        goose_attack.increase_attack(&mut run_state).await.unwrap();
        assert_eq!(run_state.active_users, 2, "the late user did not start");
        assert!(
            run_state.adjust_user_timer > std::time::Instant::now(),
            "the user after a late one is due at once: the schedule did not restart"
        );
        assert!(!run_state.adjust_due());
    }

    /// A Controller increase, then a second change before its batch is
    /// built: the first batch starts no user after the change drops it, and once dropped builds at
    /// most one more client on each build thread. A batch held for a test
    /// plan step is dropped when the Controller replaces the test plan.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_a_change_drops_a_batch_building_or_held() {
        use crate::controller::tests::control;

        let server = index_server();
        let (mut goose_attack, mut run_state) = users_attack(
            &server.base_url(),
            2,
            vec![index_scenario("Cancel", 10)],
            Duration::from_millis(50),
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        // 1,000 users at 50 ms each are far from built when the next change
        // drops them, whatever its first part started.
        assert!(control(&mut goose_attack, &mut run_state, "users 1002").await);
        step(&mut goose_attack, &mut run_state).await;
        assert!(control(&mut goose_attack, &mut run_state, "users 2").await);
        let (dropped, started) = (
            clients_built(&goose_attack),
            goose_attack.metrics.total_users,
        );
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        hold_until(&mut goose_attack, &mut run_state, |_, _| false).await;
        assert_eq!(
            goose_attack.metrics.total_users, started,
            "a user of the dropped batch started"
        );
        assert!(
            clients_built(&goose_attack) - dropped <= build_threads(),
            "{} clients built after the batch was dropped",
            clients_built(&goose_attack) - dropped
        );

        // A test plan holds its next increase's batch while it maintains.
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", "2,0s;2,1h;400,1s;400,1h"],
            vec![index_scenario("Held", 10)],
            Duration::from_millis(50),
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        // The new plan holds 2 users for a second at step 1, then raises them
        // at step 2, the held batch's step too, so only dropping the batch
        // with the change keeps step 2 from starting its users. A 0 ms hold
        // would never end.
        assert!(
            control(
                &mut goose_attack,
                &mut run_state,
                "test_plan 2,1s;3,1s;3,1h"
            )
            .await
        );
        let dropped = clients_built(&goose_attack);
        hold_until(&mut goose_attack, &mut run_state, |goose_attack, _| {
            goose_attack.attack_phase != AttackPhase::Maintain
        })
        .await;
        assert_eq!(goose_attack.test_plan.current, 2, "step 2 did not begin");
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        let held = std::time::Instant::now();
        hold_until(&mut goose_attack, &mut run_state, |_, _| {
            held.elapsed() > Duration::from_secs(2)
        })
        .await;
        assert_eq!(goose_attack.metrics.total_users, 3);
        // The held batch's 398 users are never all built, and none starts.
        assert!(
            !user_events(&goose_attack)
                .iter()
                .any(|(event, _)| *event == UserEvent::Built(398)),
            "the dropped batch was built"
        );
        assert!(
            // One user started. The new plan's check of its client ran
            // before `dropped` was read, so it is not counted here.
            clients_built(&goose_attack) - dropped <= 1 + build_threads(),
            "{} clients built after the held batch was dropped, for 1 user started",
            clients_built(&goose_attack) - dropped
        );
    }

    /// A test plan whose increase fails to build its users starts none of
    /// them, keeps its running users, and goes on to its later steps
    /// without a panic: a later increase, whose build fails too, starts none either.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_a_failed_batch_starts_none_and_the_plan_goes_on() {
        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", "5,0s;5,1s;15,1s;15,1s;25,1s;25,1h"],
            vec![index_scenario("Fail", 10)],
            Duration::ZERO,
        )
        .await;
        // The first 5 are built at setup; every build from now fails.
        goose_attack
            .user_test
            .fail_builds
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !(goose_attack.attack_phase == AttackPhase::Maintain
            && goose_attack.test_plan.current >= 5)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the plan did not reach its last step"
            );
            match goose_attack.attack_phase {
                AttackPhase::Increase => {
                    goose_attack.increase_attack(&mut run_state).await.unwrap()
                }
                AttackPhase::Maintain => {
                    goose_attack.maintain_attack(&mut run_state).await.unwrap()
                }
                phase => panic!("unexpected phase {:?}", phase),
            }
        }
        assert_eq!(
            run_state.active_users, 5,
            "users started from failed batches"
        );
        assert_eq!(users_started(&goose_attack), 5);
        assert!(
            goose_attack
                .metrics
                .history
                .iter()
                .all(|row| row.users <= 5),
            "a history row counts users that never started"
        );
    }

    /// After an increase fails to build, the next asks only for the users
    /// left, and starts them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_after_a_failed_batch_the_next_increase_starts_those_left() {
        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", "5,0s;15,1s;25,1s;25,1h"],
            vec![index_scenario("Left", 10)],
            Duration::ZERO,
        )
        .await;
        // Step 1's 10 users fail to build; step 2 builds the 10 left.
        let fail = goose_attack.user_test.fail_builds.clone();
        fail.store(true, std::sync::atomic::Ordering::Relaxed);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !(goose_attack.attack_phase == AttackPhase::Maintain
            && goose_attack.test_plan.current >= 3)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the plan did not reach its last step"
            );
            if goose_attack.test_plan.current == 2 {
                fail.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            match goose_attack.attack_phase {
                AttackPhase::Increase => {
                    goose_attack.increase_attack(&mut run_state).await.unwrap()
                }
                AttackPhase::Maintain => {
                    goose_attack.maintain_attack(&mut run_state).await.unwrap()
                }
                phase => panic!("unexpected phase {:?}", phase),
            }
        }
        assert_eq!(run_state.active_users, 15);
        assert_eq!(users_started(&goose_attack), 15);
    }

    /// A finished batch built ahead, never asked for and then dropped, has
    /// its users dropped off the main loop, as a built batch does.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_a_dropped_ahead_batch_drops_its_users_off_the_main_loop() {
        use crate::controller::tests::control;

        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", "2,0s;2,1h;52,1s;52,1h"],
            vec![index_scenario("Ahead", 10)],
            Duration::ZERO,
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        // The next increase's 50 users are built ahead while the plan holds,
        // and wait on the batch's channel: `Built` is recorded before the
        // part is handed on.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !(user_events(&goose_attack)
            .iter()
            .any(|(event, _)| *event == UserEvent::Built(50))
            && goose_attack
                .user_batch
                .as_ref()
                .is_some_and(|batch| !batch.parts.is_empty()))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the ahead batch was not built"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(control(&mut goose_attack, &mut run_state, "test_plan 3,1s;3,1h").await);
        assert!(
            user_events(&goose_attack)
                .iter()
                .any(|(event, _)| *event == UserEvent::DroppedOffLoop(50)),
            "the dropped batch's 50 users were not handed off the main loop"
        );
    }

    /// The parts built so far: users, the threads each was built on, when.
    fn parts_built_on(goose_attack: &GooseAttack) -> Vec<(usize, usize, std::time::Instant)> {
        goose_attack
            .user_test
            .parts_built_on
            .lock()
            .unwrap()
            .clone()
    }

    /// A batch built ahead of its step builds on fewer threads than every
    /// build thread, so it takes less of the CPUs the running users share; setup
    /// builds on every thread.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_an_ahead_batch_builds_on_fewer_threads() {
        // On one CPU an ahead batch has the one thread every batch has.
        if build_threads() == 1 {
            return;
        }
        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", "2,0s;2,1h;52,1s;52,1h"],
            vec![index_scenario("Fewer", 10)],
            Duration::ZERO,
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while parts_built_on(&goose_attack)
            .iter()
            .map(|(users, _, _)| users)
            .sum::<usize>()
            < 52
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the ahead batch was not built"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let parts = parts_built_on(&goose_attack);
        assert_eq!(parts[0].0, 2, "setup's part comes first");
        assert_eq!(parts[0].1, build_threads(), "setup built on every thread");
        for &(users, threads, _) in &parts[1..] {
            assert_eq!(
                threads,
                ahead_threads(),
                "a part of {} users built ahead on {} of {} threads",
                users,
                threads,
                build_threads()
            );
        }
    }

    /// A batch built ahead for a step that starts its users at once, a 0 s
    /// ramp, and that takes longer to build than the hold before it: its
    /// step starts the users already built while the rest still build, not
    /// once the whole batch is built.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_an_ahead_batch_for_a_0s_ramp_starts_users_as_its_parts_are_built() {
        let server = index_server();
        // 40 users a build thread at 50 ms a client take at least 2 s to
        // build, longer than the 1 s hold they are built in.
        let users = 40 * build_threads();
        let plan = format!("2,0s;2,1s;{0},0s;{0},1h", 2 + users);
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", &plan],
            vec![index_scenario("Ramp", 10)],
            Duration::from_millis(50),
        )
        .await;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !(goose_attack.attack_phase == AttackPhase::Maintain
            && goose_attack.test_plan.current >= 3)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the step's users did not all start"
            );
            match goose_attack.attack_phase {
                AttackPhase::Increase => {
                    goose_attack.increase_attack(&mut run_state).await.unwrap()
                }
                AttackPhase::Maintain => {
                    goose_attack.maintain_attack(&mut run_state).await.unwrap()
                }
                phase => panic!("unexpected phase {:?}", phase),
            }
        }
        assert_eq!(run_state.active_users, 2 + users);
        let events = user_events(&goose_attack);
        let last_built = events
            .iter()
            .rposition(|(event, _)| matches!(event, UserEvent::Built(_)))
            .unwrap();
        assert!(
            events[..last_built].iter().any(
                |(event, _)| matches!(event, UserEvent::Started(ordinal) if *ordinal >= 2)
            ),
            "no user of the step started before its batch's last part was built"
        );
    }

    /// When a batch built ahead is not done as its step begins, the step
    /// begins on time, its first new user starts within a second of the
    /// boundary, and the parts still to build use every build thread.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // Builds clients on every CPU: never beside a real time throttle test.
    #[serial_test::parallel(cpu)]
    async fn test_an_ahead_batch_builds_on_every_thread_once_its_step_begins() {
        // On one CPU an ahead batch has the one thread every batch has.
        if build_threads() == 1 {
            return;
        }
        // 10 parts of 4 users a build thread, at 4 a second a build thread,
        // 100 ms a client: about 600 ms a part ahead on one thread fewer, so a
        // 2 s hold ends with most of the batch still to build.
        let added = 40 * build_threads();
        let plan = format!("2,0s;2,2s;{},10s;{},1h", 2 + added, 2 + added);
        let server = index_server();
        let (mut goose_attack, mut run_state) = started_attack(
            &server.base_url(),
            &["--test-plan", &plan],
            vec![index_scenario("Switch", 10)],
            Duration::from_millis(100),
        )
        .await;
        ramp_to_hold(&mut goose_attack, &mut run_state).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while goose_attack.test_plan.current < 2 {
            assert!(std::time::Instant::now() < deadline, "the hold did not end");
            goose_attack.maintain_attack(&mut run_state).await.unwrap();
        }
        let boundary = goose_attack.step_started.unwrap();
        // Run the step until two parts were built after it began.
        while parts_built_on(&goose_attack)
            .iter()
            .filter(|(_, _, at)| *at > boundary)
            .count()
            < 2
        {
            assert!(
                std::time::Instant::now() < deadline,
                "no part was built after the step began"
            );
            goose_attack.increase_attack(&mut run_state).await.unwrap();
        }
        let first = user_events(&goose_attack)
            .into_iter()
            .find_map(|(event, at)| match event {
                UserEvent::Started(ordinal) if ordinal >= 2 => Some(at),
                _ => None,
            })
            .expect("no user of the step started");
        assert!(
            first.duration_since(boundary) < Duration::from_secs(1),
            "the step's first new user started {:?} after its boundary",
            first.duration_since(boundary)
        );
        let parts = parts_built_on(&goose_attack);
        let before: usize = parts[1..]
            .iter()
            .filter(|(_, _, at)| *at <= boundary)
            .map(|(users, _, _)| users)
            .sum();
        assert!(before < added, "the ahead batch was done before its step");
        // The second part done after the boundary was begun after it.
        let (users, threads, _) = parts
            .iter()
            .filter(|(_, _, at)| *at > boundary)
            .nth(1)
            .copied()
            .unwrap();
        assert_eq!(
            threads,
            build_threads(),
            "a part of {} users built after its step began on {} of {} threads",
            users,
            threads,
            build_threads()
        );
    }
}
