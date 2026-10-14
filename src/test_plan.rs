//! Test plan structures and functions.
//!
//! Internally, Goose represents all load tests as a series of Test Plan steps.

use chrono::prelude::*;
use gumdrop::Options;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::str::FromStr;
use std::time;

use crate::config::{timespan, GooseConfiguration};
use crate::util;
use crate::{AttackPhase, GooseAttack, GooseAttackRunState, GooseError};

/// Internal data structure representing a test plan.
#[derive(Options, Debug, Clone, Serialize, Deserialize)]
pub struct TestPlan {
    // A test plan is a vector of tuples each indicating a # of users and milliseconds.
    pub steps: Vec<(usize, usize)>,
    // Which step of the test_plan is currently running.
    pub current: usize,
}

impl Default for TestPlan {
    fn default() -> Self {
        Self::new()
    }
}

/// Automatically represent all load tests internally as a test plan.
///
/// Load tests launched using `--users`, `--increase-time`, `--increase-rate`, and/or `--run-time` are
/// automatically converted to a `Vec<(usize, usize)>` test plan.
impl TestPlan {
    /// Create a new, empty TestPlan structure.
    pub fn new() -> TestPlan {
        TestPlan {
            steps: Vec::new(),
            current: 0,
        }
    }

    /// Build a test plan from current configuration.
    ///
    /// Fails when a configured time span is too long for a `usize` in
    /// milliseconds.
    pub(crate) fn build(configuration: &GooseConfiguration) -> Result<TestPlan, GooseError> {
        if let Some(test_plan) = configuration.test_plan.as_ref() {
            // Test plan was manually defined, clone and return as is.
            return Ok(test_plan.clone());
        }
        let mut steps: Vec<(usize, usize)> = Vec::new();

        // Build a simple test plan from configured options if possible.
        if let Some(users) = configuration.users {
            if configuration.increase_time != "0" {
                // Load test is configured with --increase-time.
                steps.push((
                    users,
                    timespan("increase_time", &configuration.increase_time)? * 1_000,
                ));
            } else {
                // Load test is configured with --increase-rate.
                let increase_rate =
                    if let Some(increase_rate) = configuration.increase_rate.as_ref() {
                        util::parse_rate(Some(increase_rate.to_string()))
                    } else {
                        util::parse_rate(None)
                    };
                // Convert increase_rate to milliseconds.
                let ms_increase_rate = 1.0 / increase_rate * 1_000.0;
                // Finally, multiply the increase rate by the number of users to start.
                let total_time = ms_increase_rate * users as f32;
                steps.push((users, total_time as usize));
            }

            // If a run-time is set, maintain users for the configured duration.
            if configuration.run_time != "0" {
                steps.push((
                    users,
                    timespan("run_time", &configuration.run_time)? * 1_000,
                ));
            }

            // Ramp down using --decrease-time or --decrease-rate if configured.
            // This applies whether or not --run-time is set (e.g. ramp up then
            // immediately ramp down).
            if configuration.decrease_time != "0" {
                // Decrease users over the configured decrease-time.
                steps.push((
                    0,
                    timespan("decrease_time", &configuration.decrease_time)? * 1_000,
                ));
            } else if let Some(decrease_rate) = configuration.decrease_rate.as_ref() {
                // Calculate decrease time from rate.
                let rate = util::parse_rate(Some(decrease_rate.to_string()));
                let ms_decrease_rate = 1.0 / rate * 1_000.0;
                let total_time = ms_decrease_rate * users as f32;
                steps.push((0, total_time as usize));
            } else if configuration.run_time != "0" {
                // No decrease option set but run-time is configured: shut down
                // as quickly as possible once run-time elapses.
                steps.push((0, 0));
            }
        }

        // Define test plan from options.
        Ok(TestPlan { steps, current: 0 })
    }

    // Determine the total number of users required by the test plan.
    pub fn total_users(&self) -> usize {
        self.checked_total_users().unwrap_or(usize::MAX)
    }

    /// The total number of users the test plan starts, `None` if it does not
    /// fit in a `usize`.
    fn checked_total_users(&self) -> Option<usize> {
        let mut previous: usize = 0;
        self.steps
            .iter()
            .try_fold(0usize, |total_users, &(users, _)| {
                // Add to total_users every time there is an increase.
                let increase = users.saturating_sub(previous);
                previous = users;
                total_users.checked_add(increase)
            })
    }
}

/// Implement [`FromStr`] to convert `"users,timespan"` string formatted test plans to Goose's
/// internal representation of Vec<(usize, usize)>.
///
/// Users are represented simply as an integer.
///
/// Time span can be specified as an integer, indicating seconds. Or can use integers together
/// with one or more of "h", "m", and "s", in that order, indicating "hours", "minutes", and
/// "seconds". Valid formats include: 20, 20s, 3m, 2h, 1h20m, 3h30m10s, etc.
impl FromStr for TestPlan {
    type Err = GooseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TestPlan::parse(s).inspect_err(|e| {
            if let GooseError::InvalidOption { value, detail, .. } = e {
                if detail == INVALID_STEP {
                    // Logger isn't initialized yet, provide helpful debug output.
                    eprintln!("ERROR: invalid `configuration.test_plan` value: '{value}'");
                    eprintln!("  Expected format: --test-plan \"{{users}},{{timespan}};{{users}},{{timespan}}\"");
                    eprintln!("    {{users}} must be an integer, ie \"100\"");
                    eprintln!("    {{timespan}} can be integer seconds or \"30s\", \"20m\", \"3h\", \"1h30m\", etc");
                }
            }
        })
    }
}

/// The detail of a test plan step that does not parse.
const INVALID_STEP: &str = "invalid `configuration.test_plan` value.";

impl TestPlan {
    /// Parse a test plan as [`FromStr`] does, without printing anything: the
    /// Controllers parse client text with this.
    pub(crate) fn parse(s: &str) -> Result<TestPlan, GooseError> {
        // Convert string into a TestPlan.
        let mut steps: Vec<(usize, usize)> = Vec::new();
        // Each line of the test plan must be in the format "{users},{timespan}", white space is ignored
        let re =
            Regex::new(r"^\s*([0-9]+)\s*,\s*([0-9]+|(([0-9]+?)h)?(([0-9]+?)m)?(([0-9]+?)s)?)\s*$")
                .unwrap();
        // A test plan can have multiple lines split by the semicolon ";".
        for line in s.split(';') {
            // Users and milliseconds, unless either is too large for a usize.
            let step = re.captures(line).and_then(|cap| {
                Some((
                    cap[1].parse::<usize>().ok()?,
                    util::parse_timespan(&cap[2])? * 1_000,
                ))
            });
            match step {
                Some(step) => steps.push(step),
                None => {
                    return Err(GooseError::InvalidOption {
                        option: "`configuration.test_plan".to_string(),
                        value: line.to_string(),
                        detail: INVALID_STEP.to_string(),
                    })
                }
            }
        }
        let test_plan = TestPlan { steps, current: 0 };
        // A plan with no users never starts: weighting zero users never ends.
        match test_plan.checked_total_users() {
            Some(total) if total > 0 => Ok(test_plan),
            _ => Err(GooseError::InvalidOption {
                option: "`configuration.test_plan".to_string(),
                value: s.to_string(),
                detail: "`configuration.test_plan` must start at least one user, and no more than fit in a usize.".to_string(),
            }),
        }
    }
}

/// A test plan is a series of steps performing one of the following actions.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum TestPlanStepAction {
    /// A test plan step that is increasing the number of GooseUser threads.
    Increasing,
    /// A test plan step that is maintaining the number of GooseUser threads.
    Maintaining,
    /// A test plan step that is decreasing the number of GooseUser threads.
    Decreasing,
    /// A test plan step that is canceling all GooseUser threads.
    Canceling,
    /// The final step indicating that the load test is finished.
    Finished,
}

/// A historical record of a single test plan step, used to generate reports from the metrics.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TestPlanHistory {
    /// What action happend in this step.
    pub action: TestPlanStepAction,
    /// A timestamp of when the step started.
    pub timestamp: DateTime<Utc>,
    /// The number of users when the step started.
    pub users: usize,
}
impl TestPlanHistory {
    /// A helper to record a new test plan step in the historical record.
    pub(crate) fn step(action: TestPlanStepAction, users: usize) -> TestPlanHistory {
        TestPlanHistory {
            action,
            timestamp: Utc::now(),
            users,
        }
    }
}

impl GooseAttack {
    // Advance the active [`GooseAttack`](./struct.GooseAttack.html) to the next TestPlan step.
    pub(crate) fn advance_test_plan(&mut self, goose_attack_run_state: &mut GooseAttackRunState) {
        // Record the instant this new step starts, for use with timers.
        self.step_started = Some(time::Instant::now());

        let action = if self.test_plan.current == self.test_plan.steps.len() - 1 {
            // If this is the last TestPlan step and there are 0 users, shut down.
            if self.test_plan.steps[self.test_plan.current].0 == 0 {
                // @TODO: don't shut down if stopped by a controller...
                self.set_attack_phase(goose_attack_run_state, AttackPhase::Shutdown);
                TestPlanStepAction::Finished
            }
            // Otherwise maintain the number of GooseUser threads until canceled.
            else {
                self.set_attack_phase(goose_attack_run_state, AttackPhase::Maintain);
                TestPlanStepAction::Maintaining
            }
        // If this is not the last TestPlan step, determine what happens next.
        } else if self.test_plan.current < self.test_plan.steps.len() {
            match self.test_plan.steps[self.test_plan.current]
                .0
                .cmp(&self.test_plan.steps[self.test_plan.current + 1].0)
            {
                Ordering::Less => {
                    self.set_attack_phase(goose_attack_run_state, AttackPhase::Increase);
                    TestPlanStepAction::Increasing
                }
                Ordering::Greater => {
                    self.set_attack_phase(goose_attack_run_state, AttackPhase::Decrease);
                    TestPlanStepAction::Decreasing
                }
                Ordering::Equal => {
                    self.set_attack_phase(goose_attack_run_state, AttackPhase::Maintain);
                    TestPlanStepAction::Maintaining
                }
            }
        } else {
            unreachable!("Advanced 2 steps beyond the end of the TestPlan.")
        };

        // Record details about new new TestPlan step that is starting.
        self.metrics.history.push(TestPlanHistory::step(
            action,
            self.test_plan.steps[self.test_plan.current].0,
        ));

        // Always advance the TestPlan step
        self.test_plan.current += 1;
    }
}
