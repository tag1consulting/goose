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

use crate::config::GooseConfiguration;
use crate::util;
use crate::{AttackPhase, GooseAttack, GooseAttackRunState, GooseError};

/// Internal data structure representing a test plan.
#[derive(Options, Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TestPlan {
    // A test plan is a vector of tuples each indicating a # of users and milliseconds.
    pub(crate) steps: Vec<(usize, usize)>,
    // Which step of the test_plan is currently running.
    pub(crate) current: usize,
}

/// Automatically represent all load tests internally as a test plan.
///
/// Load tests launched using `--users`, `--startup-time`, `--hatch-rate`, and/or `--run-time` are
/// automatically converted to a `Vec<(usize, usize)>` test plan.
impl TestPlan {
    /// Create a new, empty TestPlan structure.
    pub(crate) fn new() -> TestPlan {
        TestPlan {
            steps: Vec::new(),
            current: 0,
        }
    }

    /// Build a test plan from current configuration.
    pub(crate) fn build(configuration: &GooseConfiguration) -> TestPlan {
        if let Some(test_plan) = configuration.test_plan.as_ref() {
            // Test plan was manually defined, clone and return as is.
            test_plan.clone()
        } else {
            let mut steps: Vec<(usize, usize)> = Vec::new();

            // Build a simple test plan from configured options if possible.
            if let Some(users) = configuration.users {
                if configuration.startup_time != "0" {
                    // Load test is configured with --startup-time.
                    steps.push((
                        users,
                        util::parse_timespan(&configuration.startup_time) * 1_000,
                    ));
                } else {
                    // Load test is configured with --hatch-rate.
                    let hatch_rate = if let Some(hatch_rate) = configuration.hatch_rate.as_ref() {
                        util::get_hatch_rate(Some(hatch_rate.to_string()))
                    } else {
                        util::get_hatch_rate(None)
                    };
                    // Convert hatch_rate to milliseconds.
                    let ms_hatch_rate = 1.0 / hatch_rate * 1_000.0;
                    // Finally, multiply the hatch rate by the number of users to hatch.
                    let total_time = ms_hatch_rate * users as f32;
                    steps.push((users, total_time as usize));
                }

                // A run-time is set, configure the load plan to run for the specified time then shut down.
                if configuration.run_time != "0" {
                    // Maintain the configured number of users for the configured run-time.
                    steps.push((users, util::parse_timespan(&configuration.run_time) * 1_000));
                    // Then shut down the load test as quickly as possible.
                    steps.push((0, 0));
                }
            }

            // Define test plan from options.
            TestPlan { steps, current: 0 }
        }
    }

    // Determine the total number of users required by the test plan.
    pub(crate) fn total_users(&self) -> usize {
        let mut total_users: usize = 0;
        let mut previous: usize = 0;
        for step in &self.steps {
            // Add to total_users every time there is an increase.
            if step.0 > previous {
                total_users += step.0 - previous;
            }
            previous = step.0
        }
        total_users
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
        // Convert string into a TestPlan.
        let mut steps: Vec<(usize, usize)> = Vec::new();
        // Each line of the test plan must be in the format "{users},{timespan}", white space is ignored
        let re = Regex::new(r"^\s*(\d+)\s*,\s*(\d+|((\d+?)h)?((\d+?)m)?((\d+?)s)?)\s*$").unwrap();
        // A test plan can have multiple lines split by the semicolon ";".
        let lines = s.split(';');
        for line in lines {
            if let Some(cap) = re.captures(line) {
                let left = cap[1]
                    .parse::<usize>()
                    .expect("failed to convert \\d to usize");
                let right = util::parse_timespan(&cap[2]) * 1_000;
                steps.push((left, right));
            } else {
                // Logger isn't initialized yet, provide helpful debug output.
                eprintln!("ERROR: invalid `configuration.test_plan` value: '{}'", line);
                eprintln!("  Expected format: --test-plan \"{{users}},{{timespan}};{{users}},{{timespan}}\"");
                eprintln!("    {{users}} must be an integer, ie \"100\"");
                eprintln!("    {{timespan}} can be integer seconds or \"30s\", \"20m\", \"3h\", \"1h30m\", etc");
                return Err(GooseError::InvalidOption {
                    option: "`configuration.test_plan".to_string(),
                    value: line.to_string(),
                    detail: "invalid `configuration.test_plan` value.".to_string(),
                });
            }
        }
        // The steps are only valid if the logic gets this far.
        Ok(TestPlan { steps, current: 0 })
    }
}

/// A test plan is a series of steps performing one of the following actions.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
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
