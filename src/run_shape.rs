//! The shape of a running users ramp.
//!
//! A load test started with `--users` and the ramp options runs as test plan
//! steps, bare `(users, milliseconds)` pairs in which a hold looks like any
//! other step of equal users. Changing a running test by patching step
//! indexes goes wrong, so the main loop keeps the run's shape here instead:
//! the target user count, the hold and the ramp down. When something changes
//! mid run, the steps still ahead are generated again from the shape and the
//! current user count. The ramp in progress is the step being run, its
//! target and its end instant (when it started, plus its duration); only a
//! Users change replaces it.
//!
//! A test plan has no shape: its steps are the plan.

use std::convert::TryFrom;
use std::time::{Duration, Instant};

use crate::config::GooseConfiguration;
use crate::util;

/// How a users ramp holds its users after the ramp up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// Not started yet: how long it will hold, zero for no hold.
    Pending(Duration),
    /// Started, with a fixed end.
    Timed { start: Instant, end: Instant },
    /// Holds until stopped, once `started`.
    UntilStopped { started: bool },
}

/// How users stop once the hold ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RampDown {
    /// All at once.
    None,
    /// At this many users per second.
    Rate(f32),
    /// Over this long.
    Time(Duration),
}

/// What a step of a users ramp's plan is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// The user count when the plan was generated again, which the next
    /// step paces its ramp from.
    Lead,
    /// A ramp to the target user count.
    Ramp,
    /// The hold at the target user count.
    Hold,
    /// The ramp down to no users.
    RampDown,
}

/// Where a run is, as the snapshot's `settings.stage` names it.
#[cfg(any(test, feature = "dashboard"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Idle,
    RampUp,
    Hold,
    HoldUntilStopped,
    RampingDown,
    TestPlan,
    Stopping,
}

#[cfg(any(test, feature = "dashboard"))]
impl Stage {
    /// The name the snapshot gives the stage.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Stage::Idle => "idle",
            Stage::RampUp => "ramp_up",
            Stage::Hold => "hold",
            Stage::HoldUntilStopped => "hold_until_stopped",
            Stage::RampingDown => "ramping_down",
            Stage::TestPlan => "test_plan",
            Stage::Stopping => "stopping",
        }
    }
}

/// The shape of a running users ramp; see the module documentation.
#[derive(Clone, Debug)]
pub(crate) struct RunShape {
    /// The user count the run ramps to and holds.
    pub(crate) target: usize,
    /// The hold after the ramp up.
    pub(crate) hold: Hold,
    /// How users stop when the hold ends.
    pub(crate) ramp_down: RampDown,
    /// Whether the configured ramp down has started.
    pub(crate) ramp_down_started: bool,
    /// Whether a decrease rate set on this run added the ramp down, where
    /// the run started with all users stopping at once.
    pub(crate) ramp_down_added: bool,
    /// What each step of the run's test plan is, step for step.
    pub(crate) kinds: Vec<StepKind>,
}

/// Milliseconds in `duration`, as a test plan step holds them.
fn ms(duration: Duration) -> usize {
    usize::try_from(duration.as_millis()).unwrap_or(usize::MAX)
}

/// Milliseconds to change `users` users at `rate` users per second, as
/// [`TestPlan::build`](crate::test_plan::TestPlan) computes them.
pub(crate) fn ramp_ms(users: usize, rate: f32) -> usize {
    (1.0 / rate * 1_000.0 * users as f32) as usize
}

impl RunShape {
    /// The shape of a run started from `configuration`, derived as
    /// [`TestPlan::build`](crate::test_plan::TestPlan) builds its steps,
    /// defaults included: with no run time and a ramp down the run ramps up
    /// and straight back down; with neither it holds until stopped. `None`
    /// for a test plan, or with no users.
    pub(crate) fn for_configuration(configuration: &GooseConfiguration) -> Option<RunShape> {
        if configuration.test_plan.is_some() {
            return None;
        }
        let target = configuration.users?;
        let run_time = configuration.run_time_secs();
        let decrease_time = configuration.decrease_time_secs();
        let ramp_down = if decrease_time > 0 {
            RampDown::Time(Duration::from_secs(decrease_time as u64))
        } else if let Some(rate) = configuration.decrease_rate.as_ref() {
            RampDown::Rate(util::parse_rate(Some(rate.to_string())))
        } else {
            RampDown::None
        };
        let hold = if run_time > 0 {
            Hold::Pending(Duration::from_secs(run_time as u64))
        } else if ramp_down != RampDown::None {
            Hold::Pending(Duration::ZERO)
        } else {
            Hold::UntilStopped { started: false }
        };
        let mut kinds = vec![StepKind::Ramp];
        if run_time > 0 {
            kinds.push(StepKind::Hold);
        }
        if !matches!(hold, Hold::UntilStopped { .. }) {
            kinds.push(StepKind::RampDown);
        }
        Some(RunShape {
            target,
            hold,
            ramp_down,
            ramp_down_started: false,
            ramp_down_added: false,
            kinds,
        })
    }

    /// Where a running users ramp is, while it is not stopping: ramping up,
    /// holding, holding until stopped, or ramping down.
    #[cfg(any(test, feature = "dashboard"))]
    pub(crate) fn stage(&self) -> Stage {
        if self.ramp_down_started {
            Stage::RampingDown
        } else {
            match self.hold {
                Hold::Timed { .. } => Stage::Hold,
                Hold::UntilStopped { started: true } => Stage::HoldUntilStopped,
                _ => Stage::RampUp,
            }
        }
    }

    /// Whole seconds, rounded up, until the hold ends, once it has a fixed
    /// end; 0 while a ramp runs past it. `None` before the hold starts, once
    /// the ramp down has started, and while holding until stopped.
    #[cfg(any(test, feature = "dashboard"))]
    pub(crate) fn hold_ends_in_secs(&self, now: Instant) -> Option<u64> {
        match self.hold {
            Hold::Timed { end, .. } if !self.ramp_down_started => {
                let left = end.saturating_duration_since(now);
                Some(left.as_millis().div_ceil(1_000) as u64)
            }
            _ => None,
        }
    }

    /// The step machine has entered step `current` of `steps` (or passed
    /// the last one) at `now`: start the hold, or record that the ramp down
    /// has started. A hold's duration is set as it starts, from its fixed
    /// end once it has one.
    pub(crate) fn entered(&mut self, current: usize, steps: &mut [(usize, usize)], now: Instant) {
        match self.kinds.get(current) {
            Some(StepKind::Hold) => {
                let left = match self.hold {
                    Hold::Pending(duration) => {
                        self.hold = Hold::Timed {
                            start: now,
                            end: now + duration,
                        };
                        duration
                    }
                    Hold::Timed { end, .. } => end.saturating_duration_since(now),
                    Hold::UntilStopped { .. } => return,
                };
                if let Some(step) = steps.get_mut(current) {
                    step.1 = ms(left);
                }
            }
            Some(StepKind::RampDown) => self.ramp_down_started = true,
            Some(_) => {}
            // Past the last step, a run holding until stopped starts its hold.
            None => {
                if let Hold::UntilStopped { started } = &mut self.hold {
                    *started = true;
                }
            }
        }
    }

    /// The steps after a ramp (`after_ramp`) or after the hold, with their
    /// kinds: the hold, if it has an end and is still ahead, then the ramp
    /// down for the target count, or `(0, 0)` with none. Nothing follows a
    /// hold until stopped.
    fn tail(&self, after_ramp: bool) -> Vec<((usize, usize), StepKind)> {
        let mut tail = Vec::new();
        if after_ramp {
            match self.hold {
                Hold::Pending(duration) if !duration.is_zero() => {
                    tail.push(((self.target, ms(duration)), StepKind::Hold));
                }
                // Its duration is what is left when it is entered.
                Hold::Timed { .. } => tail.push(((self.target, 0), StepKind::Hold)),
                _ => {}
            }
        }
        if !matches!(self.hold, Hold::UntilStopped { .. }) {
            let down = match self.ramp_down {
                RampDown::Rate(rate) => ramp_ms(self.target, rate),
                RampDown::Time(duration) => ms(duration),
                RampDown::None => 0,
            };
            tail.push(((0, down), StepKind::RampDown));
        }
        tail
    }

    /// A Users change to `users`, from `active` users `elapsed` milliseconds
    /// into the step in progress, ramping over `ramp` milliseconds: the plan
    /// to run from now, which the caller enters by advancing past its first
    /// step. During the ramp down, the ramp down is abandoned and the run
    /// holds at the new count until stopped.
    pub(crate) fn retarget(
        &mut self,
        active: usize,
        elapsed: usize,
        users: usize,
        ramp: usize,
    ) -> Vec<(usize, usize)> {
        if self.ramp_down_started {
            self.hold = Hold::UntilStopped { started: true };
            self.ramp_down_started = false;
        }
        self.target = users;
        let mut steps = vec![(active, elapsed), (users, ramp)];
        self.kinds = vec![StepKind::Lead, StepKind::Ramp];
        for (step, kind) in self.tail(true) {
            steps.push(step);
            self.kinds.push(kind);
        }
        steps
    }

    /// A decrease rate set on the running ramp: the ramp down still ahead
    /// is generated again at `rate`, or added where all users would stop at
    /// once. A run holding until stopped keeps holding: the rate is for the
    /// next Users decrease, a later run time, and the next Start.
    pub(crate) fn set_decrease_rate(
        &mut self,
        rate: f32,
        steps: &mut Vec<(usize, usize)>,
        current: usize,
    ) {
        if self.ramp_down == RampDown::None {
            self.ramp_down_added = true;
        }
        self.ramp_down = RampDown::Rate(rate);
        self.regenerate_tail(steps, current);
    }

    /// The decrease rate cleared on the running ramp, which the caller
    /// allows only where this run added the ramp down, or none is ahead:
    /// all users stop at once again.
    pub(crate) fn clear_decrease_rate(&mut self, steps: &mut Vec<(usize, usize)>, current: usize) {
        if matches!(self.ramp_down, RampDown::Rate(_)) {
            self.ramp_down = RampDown::None;
        }
        self.ramp_down_added = false;
        self.regenerate_tail(steps, current);
    }

    /// Whether clearing the decrease rate would remove a ramp down still
    /// ahead that the run started with.
    pub(crate) fn keeps_decrease_rate(&self) -> bool {
        !matches!(self.hold, Hold::UntilStopped { .. })
            && matches!(self.ramp_down, RampDown::Rate(_))
            && !self.ramp_down_added
    }

    /// The run time changed on the running ramp at `now`, to hold for
    /// `run_time` after the ramp up, or cleared with `None` to hold until
    /// stopped, which the caller allows only with no ramp down.
    ///
    /// During a timed hold the hold ends at its start plus the new run
    /// time, at once if that has passed. A run holding until stopped holds
    /// from `now` for the new run time, then ramps down. Before the hold
    /// starts, the new run time replaces the hold ahead.
    ///
    /// Returns true when a run holding past its last step has entered a new
    /// hold step at `current`, which the caller starts timing from `now`.
    pub(crate) fn set_run_time(
        &mut self,
        run_time: Option<Duration>,
        steps: &mut Vec<(usize, usize)>,
        current: usize,
        step_started: Option<Instant>,
        now: Instant,
    ) -> bool {
        if self.ramp_down_started {
            return false;
        }
        match (run_time, self.hold) {
            (None, Hold::UntilStopped { .. }) => return false,
            (None, Hold::Pending(_)) => self.hold = Hold::UntilStopped { started: false },
            (None, Hold::Timed { .. }) => self.hold = Hold::UntilStopped { started: true },
            (Some(run_time), Hold::Pending(_)) => self.hold = Hold::Pending(run_time),
            (Some(run_time), Hold::UntilStopped { started: false }) => {
                self.hold = Hold::Pending(run_time);
            }
            (Some(run_time), Hold::Timed { start, .. }) => {
                let end = start + run_time;
                self.hold = Hold::Timed { start, end };
                self.end_hold_step(end, steps, current, step_started);
                return false;
            }
            (Some(run_time), Hold::UntilStopped { started: true }) => {
                let end = now + run_time;
                self.hold = Hold::Timed { start: now, end };
                // A timed hold cleared while its step ran is still on that
                // step, so it ends at the new end too.
                self.end_hold_step(end, steps, current, step_started);
                if current >= steps.len() {
                    // Past the last step: the hold is a new step, entered now.
                    steps.push((self.target, ms(run_time)));
                    self.kinds.push(StepKind::Hold);
                    for (step, kind) in self.tail(false) {
                        steps.push(step);
                        self.kinds.push(kind);
                    }
                    return true;
                }
            }
        }
        self.regenerate_tail(steps, current);
        false
    }

    /// The hold step in progress, if step `current` is one, ends at `end`;
    /// a hold still ahead takes what is left of it when it is entered.
    fn end_hold_step(
        &self,
        end: Instant,
        steps: &mut [(usize, usize)],
        current: usize,
        step_started: Option<Instant>,
    ) {
        if self.kinds.get(current) == Some(&StepKind::Hold) {
            if let (Some(step), Some(started)) = (steps.get_mut(current), step_started) {
                step.1 = ms(end.saturating_duration_since(started));
            }
        }
    }

    /// Generate the steps after step `current` again, as a ramp down change
    /// needs; the step in progress, and so a ramp in progress, is kept.
    /// Nothing changes once the ramp down has started, or past the last
    /// step.
    pub(crate) fn regenerate_tail(&mut self, steps: &mut Vec<(usize, usize)>, current: usize) {
        if self.ramp_down_started || current >= steps.len() || current >= self.kinds.len() {
            return;
        }
        let after_ramp = self.kinds[current] == StepKind::Ramp;
        steps.truncate(current + 1);
        self.kinds.truncate(current + 1);
        for (step, kind) in self.tail(after_ramp) {
            steps.push(step);
            self.kinds.push(kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_plan::TestPlan;

    fn configuration(args: &[&str]) -> GooseConfiguration {
        GooseConfiguration::parse_args_default(args).unwrap()
    }

    /// The shape a start derives has one kind per step `TestPlan::build`
    /// builds, and the defaults `TestPlan::build` has.
    #[test]
    fn shape_matches_build() {
        let secs = Duration::from_secs;
        for (args, hold, ramp_down, kinds) in [
            (
                &["--users", "10"][..],
                Hold::UntilStopped { started: false },
                RampDown::None,
                &[StepKind::Ramp][..],
            ),
            (
                &["--users", "10", "--run-time", "60"][..],
                Hold::Pending(secs(60)),
                RampDown::None,
                &[StepKind::Ramp, StepKind::Hold, StepKind::RampDown][..],
            ),
            (
                &["--users", "10", "--decrease-rate", "2"][..],
                Hold::Pending(secs(0)),
                RampDown::Rate(2.0),
                &[StepKind::Ramp, StepKind::RampDown][..],
            ),
            (
                &[
                    "--users",
                    "10",
                    "--increase-time",
                    "5",
                    "--run-time",
                    "1m",
                    "--decrease-time",
                    "30",
                ][..],
                Hold::Pending(secs(60)),
                RampDown::Time(secs(30)),
                &[StepKind::Ramp, StepKind::Hold, StepKind::RampDown][..],
            ),
        ] {
            let configuration = configuration(args);
            let shape = RunShape::for_configuration(&configuration).unwrap();
            let plan = TestPlan::build(&configuration).unwrap();
            assert_eq!(shape.hold, hold, "{:?}", args);
            assert_eq!(shape.ramp_down, ramp_down, "{:?}", args);
            assert_eq!(shape.kinds, kinds, "{:?}", args);
            assert_eq!(shape.kinds.len(), plan.steps.len(), "{:?}", args);
            assert_eq!(shape.stage(), Stage::RampUp, "{:?}", args);
        }
        assert!(RunShape::for_configuration(&configuration(&["--test-plan", "1,1s"])).is_none());
        assert!(RunShape::for_configuration(&configuration(&[])).is_none());
    }

    /// The hold starts when it is entered, and ends at a fixed instant that
    /// a Users ramp during it does not move.
    #[test]
    fn hold_keeps_its_end() {
        let configuration = configuration(&["--users", "10", "--run-time", "60"]);
        let mut shape = RunShape::for_configuration(&configuration).unwrap();
        let mut steps = TestPlan::build(&configuration).unwrap().steps;
        let now = Instant::now();
        assert_eq!(shape.hold_ends_in_secs(now), None);
        shape.entered(1, &mut steps, now);
        assert_eq!(shape.stage(), Stage::Hold);
        assert_eq!(shape.hold_ends_in_secs(now), Some(60));
        assert_eq!(steps[1], (10, 60_000));

        // Twenty seconds in, 5 more users over 2 seconds.
        let later = now + Duration::from_secs(20);
        let mut steps = shape.retarget(10, 20_000, 15, 2_000);
        assert_eq!(steps, vec![(10, 20_000), (15, 2_000), (15, 0), (0, 0)]);
        assert_eq!(shape.stage(), Stage::Hold);
        // The hold's step is what is left of it when the ramp ends.
        shape.entered(2, &mut steps, later + Duration::from_secs(2));
        assert_eq!(steps[2], (15, 38_000));
        assert_eq!(shape.hold_ends_in_secs(later), Some(40));
        // A ramp that runs past the end leaves no hold, and the countdown at 0.
        assert_eq!(
            shape.hold_ends_in_secs(now + Duration::from_secs(61)),
            Some(0)
        );
        shape.entered(3, &mut steps, now + Duration::from_secs(61));
        assert_eq!(shape.stage(), Stage::RampingDown);
        assert_eq!(shape.hold_ends_in_secs(now), None);
    }
}
