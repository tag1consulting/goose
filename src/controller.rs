//! Optional telnet and WebSocket Controller threads.
//!
//! By default, Goose launches both a telnet Controller and a WebSocket Controller, allowing
//! real-time control of the running load test.

use crate::config::GooseConfiguration;
use crate::metrics::GooseMetrics;
use crate::test_plan::{TestPlan, TestPlanHistory, TestPlanStepAction};
use crate::util;
use crate::{AttackPhase, GooseAttack, GooseAttackRunState, GooseError};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::str::{self, FromStr};
use std::time::Duration;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// All commands recognized by the Goose Controllers.
///
/// Commands are not case sensitive. When sending commands to the WebSocket Controller,
/// they must be formatted as json as defined by
/// [ControllerWebSocketRequest](./struct.ControllerWebSocketRequest.html).
///
/// GOOSE DEVELOPER NOTE: The following steps are required to add a new command:
///  1. Define the new command here in the ControllerCommand enum.
///      - Commands will be displayed in the help screen in the order defined here, so
///        they should be logically grouped.
///  2. Add the new command to `ControllerCommand::details` and populate all
///    `ControllerCommandDetails`, using other commands as an implementation reference.
///      - The `regex` is used to identify the command, and optionally to extract a
///        value (for example see `Hatchrate` and `Users`)
///      - If additional validation is required beyond the regular expression, add
///        the necessary logic to `ControllerCommand::validate_value`.
///  3. Add any necessary parent process logic for the command to
///    `GooseAttack::handle_controller_requests` (also in this file).
///  4. Add a test for the new command in tests/controller.rs.
#[derive(Clone, Debug, EnumIter, PartialEq, Eq)]
pub enum ControllerCommand {
    /// Displays a list of all commands supported by the Controller.
    ///
    /// # Example
    /// Returns the a list of all supported Controller commands.
    /// ```notest
    /// help
    /// ```
    ///
    /// This command can be run at any time.
    Help,
    /// Disconnect from the Controller.
    ///
    /// # Example
    /// Disconnects from the Controller.
    /// ```notest
    /// exit
    /// ```
    ///
    /// This command can be run at any time.
    Exit,
    /// Start an idle test.
    ///
    /// # Example
    /// Starts an idle load test.
    /// ```notest
    /// start
    /// ```
    ///
    /// Goose must be idle to process this command.
    Start,
    /// Stop a running test, putting it into an idle state.
    ///
    /// # Example
    /// Stops a running (or stating) load test.
    /// ```notest
    /// stop
    /// ```
    ///
    /// Goose must be running (or starting) to process this command.
    Stop,
    /// Tell the load test to shut down (which will disconnect the controller).
    ///
    /// # Example
    /// Terminates the Goose process, cleanly shutting down the load test if running.
    /// ```notest
    /// shutdown
    /// ```
    ///
    /// Goose can process this command at any time.
    Shutdown,
    /// Configure the host to load test.
    ///
    /// # Example
    /// Tells Goose to generate load against <http://example.com/>.
    /// ```notest
    /// host http://example.com/
    /// ```
    ///
    /// Goose must be idle to process this command.
    Host,
    /// Configure how quickly new [`GooseUser`](../goose/struct.GooseUser.html)s are launched.
    ///
    /// # Example
    /// Tells Goose to launch a new user every 1.25 seconds.
    /// ```notest
    /// hatchrate 1.25
    /// ```
    ///
    /// Goose can be idle or running when processing this command.
    HatchRate,
    /// Configure how long to take to launch all [`GooseUser`](../goose/struct.GooseUser.html)s.
    ///
    /// # Example
    /// Tells Goose to launch a new user every 1.25 seconds.
    /// ```notest
    /// startuptime 1.25
    /// ```
    ///
    /// Goose must be idle to process this command.
    StartupTime,
    /// Configure how many [`GooseUser`](../goose/struct.GooseUser.html)s to launch.
    ///
    /// # Example
    /// Tells Goose to simulate 100 concurrent users.
    /// ```notest
    /// users 100
    /// ```
    ///
    /// Can be configured on an idle or running load test.
    Users,
    /// Configure how long the load test should run before stopping and returning to an idle state.
    ///
    /// # Example
    /// Tells Goose to run the load test for 1 minute, before automatically stopping.
    /// ```notest
    /// runtime 60
    /// ```
    ///
    /// This can be configured when Goose is idle as well as when a Goose load test is running.
    RunTime,
    /// Define a load test plan. This will replace the previously configured test plan, if any.
    ///
    /// # Example
    /// Tells Goose to launch 10 users in 5 seconds, maintain them for 30 seconds, and then spend 5 seconds
    /// stopping them.
    /// ```notest
    /// testplan "10,5s;10,30s;0,5s"
    /// ```
    ///
    /// Can be configured on an idle or running load test.
    TestPlan,
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s.
    ///
    /// # Example
    /// Returns the current Goose configuration.
    /// ```notest
    /// config
    /// ```
    Config,
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s in json format.
    ///
    /// # Example
    /// Returns the current Goose configuration in json format.
    /// ```notest
    /// configjson
    /// ```
    ///
    /// This command can be run at any time.
    ConfigJson,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s.
    ///
    /// # Example
    /// Returns the current Goose metrics.
    /// ```notest
    /// metrics
    /// ```
    ///
    /// This command can be run at any time.
    Metrics,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s in json format.
    ///
    /// # Example
    /// Returns the current Goose metrics in json format.
    /// ```notest
    /// metricsjson
    /// ```
    ///
    /// This command can be run at any time.
    MetricsJson,
}

/// Defines details around identifying and processing ControllerCommands.
///
/// As order doesn't matter here, it's preferred to define commands in alphabetical order.
impl ControllerCommand {
    /// Each ControllerCommand must define complete ControllerCommentDetails.
    ///  - `help` returns a ControllerHelp struct that is used to provide inline help.
    ///  - `regex` returns an &str defining the regular expression used to match the command
    ///    (and optionally to grab a value being set).
    ///  - `process_response` contains a boxed closure that receives the parent process
    ///    response to the command and responds to the controller appropriately.
    fn details(&self) -> ControllerCommandDetails {
        match self {
            ControllerCommand::Config => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "config",
                    description: "display load test configuration\n",
                },
                regex: r"(?i)^config$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Config(config) = response {
                        Ok(format!("{:#?}", config))
                    } else {
                        Err("error loading configuration".to_string())
                    }
                }),
            },
            ControllerCommand::ConfigJson => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "config-json",
                    description: "display load test configuration in json format\n",
                },
                regex: r"(?i)^(configjson|config-json)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Config(config) = response {
                        Ok(serde_json::to_string(&config).expect("unexpected serde failure"))
                    } else {
                        Err("error loading configuration".to_string())
                    }
                }),
            },
            ControllerCommand::Exit => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "exit",
                    description: "exit controller\n\n",
                },
                regex: r"(?i)^(exit|quit|q)$",
                process_response: Box::new(|_| {
                    let e = "received an impossible EXIT command";
                    error!("{}", e);
                    Err(e.to_string())
                }),
            },
            ControllerCommand::HatchRate => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "hatchrate FLOAT",
                    description: "set per-second rate users hatch\n",
                },
                regex: r"(?i)^(hatchrate|hatch_rate|hatch-rate) ([0-9]*(\.[0-9]*)?){1}$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("hatch_rate configured".to_string())
                    } else {
                        Err("failed to configure hatch_rate".to_string())
                    }
                }),
            },
            ControllerCommand::Help => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "help",
                    description: "this help\n",
                },
                regex: r"(?i)^(help|\?)$",
                process_response: Box::new(|_| {
                    let e = "received an impossible HELP command";
                    error!("{}", e);
                    Err(e.to_string())
                }),
            },
            ControllerCommand::Host => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "host HOST",
                    description: "set host to load test, (ie https://web.site/)\n",
                },
                regex: r"(?i)^(host|hostname|host_name|host-name) ((https?)://.+)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("host configured".to_string())
                    } else {
                        Err("failed to reconfigure host, be sure host is valid and load test is idle".to_string())
                    }
                }),
            },
            ControllerCommand::Metrics => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "metrics",
                    description: "display metrics for current load test\n",
                },
                regex: r"(?i)^(metrics|stats)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Metrics(metrics) = response {
                        Ok(metrics.to_string())
                    } else {
                        Err("error loading metrics".to_string())
                    }
                }),
            },
            ControllerCommand::MetricsJson => {
                ControllerCommandDetails {
                    help: ControllerHelp {
                        name: "metrics-json",
                        // No new-line as this is the last line of the help screen.
                        description: "display metrics for current load test in json format",
                    },
                    regex: r"(?i)^(metricsjson|metrics-json|statsjson|stats-json)$",
                    process_response: Box::new(|response| {
                        if let ControllerResponseMessage::Metrics(metrics) = response {
                            Ok(serde_json::to_string(&metrics).expect("unexpected serde failure"))
                        } else {
                            Err("error loading metrics".to_string())
                        }
                    }),
                }
            }
            ControllerCommand::RunTime => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "runtime TIME",
                    description: "set how long to run test, (ie 1h30m5s)\n",
                },
                regex: r"(?i)^(run|runtime|run_time|run-time|) (\d+|((\d+?)h)?((\d+?)m)?((\d+?)s)?)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("run_time configured".to_string())
                    } else {
                        Err("failed to configure run_time".to_string())
                    }
                }),
            },
            ControllerCommand::Shutdown => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "shutdown",
                    description: "shutdown load test and exit controller\n\n",
                },
                regex: r"(?i)^shutdown$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("load test shut down".to_string())
                    } else {
                        Err("failed to shut down load test".to_string())
                    }
                }),
            },
            ControllerCommand::Start => {
                ControllerCommandDetails {
                    help: ControllerHelp {
                        name: "start",
                        description: "start an idle load test\n",
                    },
                    regex: r"(?i)^start$",
                    process_response: Box::new(|response| {
                        if let ControllerResponseMessage::Bool(true) = response {
                            Ok("load test started".to_string())
                        } else {
                            Err("unable to start load test, be sure it is idle and host is configured".to_string())
                        }
                    }),
                }
            }
            ControllerCommand::StartupTime => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "startup-time TIME",
                    description: "set total time to take starting users\n",
                },
                regex: r"(?i)^(starttime|start_time|start-time|startup|startuptime|startup_time|startup-time) (\d+|((\d+?)h)?((\d+?)m)?((\d+?)s)?)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("startup_time configured".to_string())
                    } else {
                        Err(
                            "failed to configure startup_time, be sure load test is idle"
                                .to_string(),
                        )
                    }
                }),
            },
            ControllerCommand::Stop => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "stop",
                    description: "stop a running load test and return to idle state\n",
                },
                regex: r"(?i)^stop$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("load test stopped".to_string())
                    } else {
                        Err("load test not running, failed to stop".to_string())
                    }
                }),
            },
            ControllerCommand::TestPlan => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "test-plan PLAN",
                    description: "define or replace test-plan, (ie 10,5m;10,1h;0,30s)\n\n",
                },
                regex: r"(?i)^(testplan|test_plan|test-plan|plan) (((\d+)\s*,\s*(\d+|((\d+?)h)?((\d+?)m)?((\d+?)s)?)*;*)+)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("test-plan configured".to_string())
                    } else {
                        Err("failed to configure test-plan, be sure test-plan is valid".to_string())
                    }
                }),
            },
            ControllerCommand::Users => ControllerCommandDetails {
                help: ControllerHelp {
                    name: "users INT",
                    description: "set number of simulated users\n",
                },
                regex: r"(?i)^(users?) (\d+)$",
                process_response: Box::new(|response| {
                    if let ControllerResponseMessage::Bool(true) = response {
                        Ok("users configured".to_string())
                    } else {
                        Err("load test not idle, failed to reconfigure users".to_string())
                    }
                }),
            },
        }
    }

    // Optionally perform validation beyond what is possible with a regular expression.
    //
    // Return Some(value) if the value is valid, otherwise return None.
    fn validate_value(&self, value: &str) -> Option<String> {
        // The Regex that captures the host only validates that the host starts with
        // http:// or https://. Now use a library to properly validate that this is
        // a valid host before sending to the parent process.
        if self == &ControllerCommand::Host {
            if util::is_valid_host(value).is_ok() {
                Some(value.to_string())
            } else {
                None
            }
        } else if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }

    // If the regular expression that matches this command also matches a value, get and validate
    // the value.
    //
    // Returns Some(value) if the value is valid, otherwise returns None.
    fn get_value(&self, command_string: &str) -> Option<String> {
        let regex = Regex::new(self.details().regex)
            .expect("ControllerCommand::details().regex returned invalid regex [2]");
        let caps = regex.captures(command_string).unwrap();
        let value = caps.get(2).map_or("", |m| m.as_str());
        self.validate_value(value)
    }

    // Builds a help screen displayed when a controller receives the `help` command.
    fn display_help() -> String {
        let mut help_text = Vec::new();
        writeln!(
            &mut help_text,
            "{} {} controller commands:",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        )
        .expect("failed to write to buffer");
        // Builds help screen in the order commands are defined in the ControllerCommand enum.
        for command in ControllerCommand::iter() {
            write!(
                &mut help_text,
                "{:<18} {}",
                command.details().help.name,
                command.details().help.description
            )
            .expect("failed to write to buffer");
        }
        String::from_utf8(help_text).expect("invalid utf-8 in help text")
    }
}

/// The parent process side of the Controller functionality.
impl GooseAttack {
    /// Handle Controller requests.
    pub(crate) async fn handle_controller_requests(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        // If the controller is enabled, check if we've received any
        // messages.
        if let Some(c) = goose_attack_run_state.controller_channel_rx.as_ref() {
            match c.try_recv() {
                Ok(message) => {
                    info!(
                        "request from controller client {}: {:?}",
                        message.client_id, message.request
                    );
                    // As order is not important here, commands should be defined in alphabetical order.
                    match &message.request.command {
                        // Send back a copy of the running configuration.
                        ControllerCommand::Config | ControllerCommand::ConfigJson => {
                            self.reply_to_controller(
                                message,
                                ControllerResponseMessage::Config(Box::new(
                                    self.configuration.clone(),
                                )),
                            );
                        }
                        // Send back a copy of the running metrics.
                        ControllerCommand::Metrics | ControllerCommand::MetricsJson => {
                            self.reply_to_controller(
                                message,
                                ControllerResponseMessage::Metrics(Box::new(self.metrics.clone())),
                            );
                        }
                        // Start the load test, and acknowledge command.
                        ControllerCommand::Start => {
                            // We can only start an idle load test.
                            if self.attack_phase == AttackPhase::Idle {
                                self.test_plan = TestPlan::build(&self.configuration);
                                if self.prepare_load_test().is_ok() {
                                    // Rebuild test plan in case any parameters have been changed.
                                    self.set_attack_phase(
                                        goose_attack_run_state,
                                        AttackPhase::Increase,
                                    );
                                    self.reply_to_controller(
                                        message,
                                        ControllerResponseMessage::Bool(true),
                                    );
                                    // Reset the run state when starting a new load test.
                                    self.reset_run_state(goose_attack_run_state).await?;
                                    self.metrics.history.push(TestPlanHistory::step(
                                        TestPlanStepAction::Increasing,
                                        0,
                                    ));
                                } else {
                                    // Do not move to Starting phase if unable to prepare load test.
                                    self.reply_to_controller(
                                        message,
                                        ControllerResponseMessage::Bool(false),
                                    );
                                }
                            } else {
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        // Stop the load test, and acknowledge command.
                        ControllerCommand::Stop => {
                            // We can only stop a starting or running load test.
                            if [AttackPhase::Increase, AttackPhase::Maintain]
                                .contains(&self.attack_phase)
                            {
                                // Don't shutdown when load test is stopped by controller, remain idle instead.
                                goose_attack_run_state.shutdown_after_stop = false;
                                // Don't automatically restart the load test.
                                self.configuration.no_autostart = true;
                                self.cancel_attack(goose_attack_run_state).await?;
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(true),
                                );
                            } else {
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        // Stop the load test, and acknowledge request.
                        ControllerCommand::Shutdown => {
                            // If load test is Idle, there are no metrics to display.
                            if self.attack_phase == AttackPhase::Idle {
                                self.metrics.display_metrics = false;
                                self.set_attack_phase(
                                    goose_attack_run_state,
                                    AttackPhase::Decrease,
                                );
                            } else {
                                self.cancel_attack(goose_attack_run_state).await?;
                            }

                            // Shutdown after stopping.
                            goose_attack_run_state.shutdown_after_stop = true;
                            // Confirm shut down to Controller.
                            self.reply_to_controller(
                                message,
                                ControllerResponseMessage::Bool(true),
                            );

                            // Give the controller thread time to send the response.
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        ControllerCommand::Host => {
                            if self.attack_phase == AttackPhase::Idle {
                                // The controller uses a regular expression to validate that
                                // this is a valid hostname, so simply use it with further
                                // validation.
                                if let Some(host) = &message.request.value {
                                    info!(
                                        "changing host from {:?} to {}",
                                        self.configuration.host, host
                                    );
                                    self.configuration.host = host.to_string();
                                    self.reply_to_controller(
                                        message,
                                        ControllerResponseMessage::Bool(true),
                                    );
                                } else {
                                    debug!(
                                        "controller didn't provide host: {:#?}",
                                        &message.request
                                    );
                                    self.reply_to_controller(
                                        message,
                                        ControllerResponseMessage::Bool(false),
                                    );
                                }
                            } else {
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        ControllerCommand::Users => {
                            // The controller uses a regular expression to validate that
                            // this is a valid integer, so simply use it with further
                            // validation.
                            if let Some(users) = &message.request.value {
                                // Use expect() as Controller uses regex to validate this is an integer.
                                let new_users = usize::from_str(users)
                                    .expect("failed to convert string to usize");
                                // If setting users, any existing configuration for a test plan isn't valid.
                                self.configuration.test_plan = None;

                                match self.attack_phase {
                                    // If the load test is idle, simply update the configuration.
                                    AttackPhase::Idle => {
                                        let current_users = if !self.test_plan.steps.is_empty() {
                                            self.test_plan.steps[self.test_plan.current].0
                                        } else if let Some(users) = self.configuration.users {
                                            users
                                        } else {
                                            0
                                        };
                                        info!(
                                            "changing users from {:?} to {}",
                                            current_users, new_users
                                        );
                                        self.configuration.users = Some(new_users);
                                        self.reply_to_controller(
                                            message,
                                            ControllerResponseMessage::Bool(true),
                                        );
                                    }
                                    // If the load test is running, rebuild the active test plan.
                                    AttackPhase::Increase
                                    | AttackPhase::Decrease
                                    | AttackPhase::Maintain => {
                                        info!(
                                            "changing users from {} to {}",
                                            goose_attack_run_state.active_users, new_users
                                        );
                                        // Determine how long has elapsed since this step started.
                                        let elapsed = self.step_elapsed() as usize;

                                        // Determine how quickly to adjust user account.
                                        let hatch_rate = if let Some(hatch_rate) =
                                            self.configuration.hatch_rate.as_ref()
                                        {
                                            util::get_hatch_rate(Some(hatch_rate.to_string()))
                                        } else {
                                            util::get_hatch_rate(None)
                                        };
                                        // Convert hatch_rate to milliseconds.
                                        let ms_hatch_rate = 1.0 / hatch_rate * 1_000.0;
                                        // Determine how many users to increase or decrease by.
                                        let user_difference = (goose_attack_run_state.active_users
                                            as isize
                                            - new_users as isize)
                                            .abs();
                                        // Multiply the user difference by the hatch rate to get the total_time required.
                                        let total_time =
                                            (ms_hatch_rate * user_difference as f32) as usize;

                                        // Reset the test_plan to adjust to the newly specified users.
                                        self.test_plan.steps = vec![
                                            // Record how many active users there are currently.
                                            (goose_attack_run_state.active_users, elapsed),
                                            // Configure the new user count.
                                            (new_users, total_time),
                                        ];

                                        // Reset the current step to what was happening when reconfiguration happened.
                                        self.test_plan.current = 0;

                                        // Allocate more users if increasing users.
                                        if new_users > goose_attack_run_state.active_users {
                                            self.weighted_users = self
                                                .weight_scenario_users(user_difference as usize)?;
                                        }

                                        // Also update the running configurtion (this impacts if the test is stopped and then
                                        // restarted through the controller).
                                        self.configuration.users = Some(new_users);

                                        // Finally, advance to the next step to adjust user count.
                                        self.advance_test_plan(goose_attack_run_state);

                                        self.reply_to_controller(
                                            message,
                                            ControllerResponseMessage::Bool(true),
                                        );
                                    }
                                    _ => {
                                        self.reply_to_controller(
                                            message,
                                            ControllerResponseMessage::Bool(false),
                                        );
                                    }
                                }
                            } else {
                                warn!("Controller didn't provide users: {:#?}", &message.request);
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        ControllerCommand::HatchRate => {
                            // The controller uses a regular expression to validate that
                            // this is a valid float, so simply use it with further
                            // validation.
                            if let Some(hatch_rate) = &message.request.value {
                                // If startup_time was already set, unset it first.
                                if !self.configuration.startup_time.is_empty() {
                                    info!(
                                        "resetting startup_time from {} to 0",
                                        self.configuration.startup_time
                                    );
                                    self.configuration.startup_time = "0".to_string();
                                }
                                info!(
                                    "changing hatch_rate from {:?} to {}",
                                    self.configuration.hatch_rate, hatch_rate
                                );
                                self.configuration.hatch_rate = Some(hatch_rate.clone());
                                // If setting hatch_rate, any existing configuration for a test plan isn't valid.
                                self.configuration.test_plan = None;
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(true),
                                );
                            } else {
                                warn!(
                                    "Controller didn't provide hatch_rate: {:#?}",
                                    &message.request
                                );
                            }
                        }
                        ControllerCommand::StartupTime => {
                            if self.attack_phase == AttackPhase::Idle {
                                // The controller uses a regular expression to validate that
                                // this is a valid startup time, so simply use it with further
                                // validation.
                                if let Some(startup_time) = &message.request.value {
                                    // If hatch_rate was already set, unset it first.
                                    if let Some(hatch_rate) = &self.configuration.hatch_rate {
                                        info!("resetting hatch_rate from {} to None", hatch_rate);
                                        self.configuration.hatch_rate = None;
                                    }
                                    info!(
                                        "changing startup_rate from {} to {}",
                                        self.configuration.startup_time, startup_time
                                    );
                                    self.configuration.startup_time.clone_from(startup_time);
                                    // If setting startup_time, any existing configuration for a test plan isn't valid.
                                    self.configuration.test_plan = None;
                                    self.reply_to_controller(
                                        message,
                                        ControllerResponseMessage::Bool(true),
                                    );
                                } else {
                                    warn!(
                                        "Controller didn't provide startup_time: {:#?}",
                                        &message.request
                                    );
                                    self.reply_to_controller(
                                        message,
                                        ControllerResponseMessage::Bool(false),
                                    );
                                }
                            } else {
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        ControllerCommand::RunTime => {
                            // The controller uses a regular expression to validate that
                            // this is a valid run time, so simply use it with further
                            // validation.
                            if let Some(run_time) = &message.request.value {
                                info!(
                                    "changing run_time from {:?} to {}",
                                    self.configuration.run_time, run_time
                                );
                                self.configuration.run_time.clone_from(run_time);
                                // If setting run_time, any existing configuration for a test plan isn't valid.
                                self.configuration.test_plan = None;
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(true),
                                );
                            } else {
                                warn!(
                                    "Controller didn't provide run_time: {:#?}",
                                    &message.request
                                );
                            }
                        }
                        ControllerCommand::TestPlan => {
                            if let Some(value) = &message.request.value {
                                match value.parse::<TestPlan>() {
                                    Ok(t) => {
                                        // Switch the configuration to use the test plan.
                                        self.configuration.test_plan = Some(t.clone());
                                        self.configuration.users = None;
                                        self.configuration.hatch_rate = None;
                                        self.configuration.startup_time = "0".to_string();
                                        self.configuration.run_time = "0".to_string();
                                        match self.attack_phase {
                                            // If the load test is idle, just update the configuration.
                                            AttackPhase::Idle => {
                                                self.reply_to_controller(
                                                    message,
                                                    ControllerResponseMessage::Bool(true),
                                                );
                                            }
                                            // If the load test is running, rebuild the active test plan.
                                            AttackPhase::Increase
                                            | AttackPhase::Decrease
                                            | AttackPhase::Maintain => {
                                                // Rebuild the active test plan.
                                                self.test_plan = t;

                                                // Reallocate users.
                                                self.weighted_users = self.weight_scenario_users(
                                                    self.test_plan.total_users(),
                                                )?;

                                                // Determine how long the current step has been running.
                                                let elapsed = self.step_elapsed() as usize;

                                                // Insert the current state of the test plan before the new test plan.
                                                self.test_plan.steps.insert(
                                                    0,
                                                    (goose_attack_run_state.active_users, elapsed),
                                                );

                                                // Finally, advance to the next step to adjust user count.
                                                self.advance_test_plan(goose_attack_run_state);

                                                // The load test is successfully reconfigured.
                                                self.reply_to_controller(
                                                    message,
                                                    ControllerResponseMessage::Bool(true),
                                                );
                                            }
                                            _ => {
                                                unreachable!("Controller used in impossible phase.")
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Controller provided invalid test_plan: {}", e);
                                        self.reply_to_controller(
                                            message,
                                            ControllerResponseMessage::Bool(false),
                                        );
                                    }
                                }
                            } else {
                                warn!(
                                    "Controller didn't provide test_plan: {:#?}",
                                    &message.request
                                );
                                self.reply_to_controller(
                                    message,
                                    ControllerResponseMessage::Bool(false),
                                );
                            }
                        }
                        // These messages shouldn't be received here.
                        ControllerCommand::Help | ControllerCommand::Exit => {
                            warn!("Unexpected command: {:?}", &message.request);
                        }
                    }
                }
                Err(e) => {
                    // Errors can be ignored, they happen any time there are no messages.
                    debug!("error receiving message: {}", e);
                }
            }
        };
        Ok(())
    }

    /// Use the provided oneshot channel to reply to a controller client request.
    pub(crate) fn reply_to_controller(
        &mut self,
        request: ControllerRequest,
        response: ControllerResponseMessage,
    ) {
        if let Some(oneshot_tx) = request.response_channel {
            if oneshot_tx
                .send(ControllerResponse {
                    _client_id: request.client_id,
                    response,
                })
                .is_err()
            {
                warn!("failed to send response to controller via one-shot channel")
            }
        }
    }
}

/// The control loop listens for connections on the configured TCP port. Each connection
/// spawns a new thread so multiple clients can connect. Handles incoming connections for
/// both telnet and WebSocket clients.
///  -  @TODO: optionally limit how many controller connections are allowed
///  -  @TODO: optionally require client authentication
///  -  @TODO: optionally ssl-encrypt client communication
pub(crate) async fn controller_main(
    // Expose load test configuration to controller thread.
    configuration: GooseConfiguration,
    // For sending requests to the parent process.
    channel_tx: flume::Sender<ControllerRequest>,
    // Which type of controller to launch.
    protocol: ControllerProtocol,
) -> io::Result<()> {
    // Build protocol-appropriate address.
    let address = match &protocol {
        ControllerProtocol::Telnet => format!(
            "{}:{}",
            configuration.telnet_host, configuration.telnet_port
        ),
        ControllerProtocol::WebSocket => format!(
            "{}:{}",
            configuration.websocket_host, configuration.websocket_port
        ),
    };

    // All controllers use a TcpListener port.
    debug!(
        "preparing to bind {:?} controller to: {}",
        protocol, address
    );
    let listener = TcpListener::bind(&address).await?;
    info!("{:?} controller listening on: {}", protocol, address);

    // Counter increments each time a controller client connects with this protocol.
    let mut thread_id: u32 = 0;

    // Wait for a connection.
    while let Ok((stream, _)) = listener.accept().await {
        thread_id += 1;

        // Identify the client ip and port, used primarily for debug logging.
        let peer_address = stream
            .peer_addr()
            .map_or("UNKNOWN ADDRESS".to_string(), |p| p.to_string());

        // Create a per-client Controller state.
        let controller_state = ControllerState {
            thread_id,
            peer_address,
            channel_tx: channel_tx.clone(),
            protocol: protocol.clone(),
        };

        // Spawn a new thread to communicate with a client. The returned JoinHandle is
        // ignored as the thread simply runs until the client exits or Goose shuts down.
        // Don't .await the tokio::spawn or Goose can't handle multiple simultaneous
        // connections.
        let _ignored_joinhandle = tokio::spawn(controller_state.accept_connections(stream));
    }

    Ok(())
}

/// Implement [`FromStr`] to convert controller commands and optional values to the proper enum
/// representation.
impl FromStr for ControllerCommand {
    type Err = GooseError;

    // Use regular expressions to convert controller input to ControllerCommands.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Load all ControllerCommand regex into a set.
        let mut regex_set: Vec<String> = Vec::new();
        let mut keys = Vec::new();
        for t in ControllerCommand::iter() {
            keys.push(t.clone());
            regex_set.push(t.details().regex.to_string());
        }
        let commands = RegexSet::new(regex_set)
            .expect("ControllerCommand::details().regex returned invalid regex");
        let matches: Vec<_> = commands.matches(s).into_iter().collect();
        // This happens any time the controller receives an invalid command.
        if matches.is_empty() {
            Err(GooseError::InvalidControllerCommand {
                detail: format!("unrecognized controller command: '{}'.", s),
            })
        // This shouldn't ever happen, but if it does report all available information.
        } else if matches.len() > 1 {
            let mut matched_commands = Vec::new();
            for index in matches {
                matched_commands.push(keys[index].clone())
            }
            Err(GooseError::InvalidControllerCommand {
                detail: format!(
                    "matched multiple controller commands: '{}' ({:?}).",
                    s, matched_commands
                ),
            })
        // Only one command matched.
        } else {
            Ok(keys[*matches.first().unwrap()].clone())
        }
    }
}

/// Send a message to the client TcpStream, no prompt or line feed.
async fn write_to_socket_raw(socket: &mut tokio::net::TcpStream, message: &str) {
    if socket
        // Add a linefeed to the end of the message.
        .write_all(message.as_bytes())
        .await
        .is_err()
    {
        warn!("failed to write data to socket");
    }
}

/// Goose supports two different Controller protocols: telnet and WebSocket.
#[derive(Clone, Debug)]
pub(crate) enum ControllerProtocol {
    /// Allows control of Goose via telnet.
    Telnet,
    /// Allows control of Goose via a WebSocket.
    WebSocket,
}

/// All commands define their own ControllerHelp to create a help screen.
#[derive(Clone, Debug)]
pub(crate) struct ControllerHelp<'a> {
    // The name of the controller command.
    name: &'a str,
    // A description of the contorller command.
    description: &'a str,
}

/// Defines the regular expression used to identify a command and optionall the associated
/// value, as well as the logic for letting the controller know whether or not the
/// recognized command worked correctly.
pub(crate) struct ControllerCommandDetails<'a> {
    // The name and description of the controller command.
    help: ControllerHelp<'a>,
    // A [regular expression](https://docs.rs/regex/1.5.5/regex/struct.Regex.html) for
    // matching the command and option value.
    regex: &'a str,
    // The response sent by the controller after handling the incoming controller
    // request.
    process_response: Box<dyn Fn(ControllerResponseMessage) -> Result<String, String>>,
}

/// This structure is used to send commands and values to the parent process.
#[derive(Debug)]
pub(crate) struct ControllerRequestMessage {
    /// The command that is being sent to the parent.
    pub command: ControllerCommand,
    /// An optional value that is being sent to the parent.
    pub value: Option<String>,
}

/// An enumeration of all messages the parent can reply back to the controller thread.
#[derive(Debug)]
pub(crate) enum ControllerResponseMessage {
    /// A response containing a boolean value.
    Bool(bool),
    /// A response containing the load test configuration.
    Config(Box<GooseConfiguration>),
    /// A response containing current load test metrics.
    Metrics(Box<GooseMetrics>),
}

/// The request that's passed from the controller to the parent thread.
#[derive(Debug)]
pub(crate) struct ControllerRequest {
    /// Optional one-shot channel if a reply is required.
    pub response_channel: Option<tokio::sync::oneshot::Sender<ControllerResponse>>,
    /// An integer identifying which controller client is making the request.
    pub client_id: u32,
    /// The actual request message.
    pub request: ControllerRequestMessage,
}

/// The response that's passed from the parent to the controller.
#[derive(Debug)]
pub(crate) struct ControllerResponse {
    /// An integer identifying which controller the parent is responding to.
    pub _client_id: u32,
    /// The actual response message.
    pub response: ControllerResponseMessage,
}

/// This structure defines the required json format of any request sent to the WebSocket
/// Controller.
///
/// Requests must be made in the following format:
/// ```json
/// {
///     "request": String,
/// }
///
/// ```
///
/// The request "String" value must be a valid
/// [`ControllerCommand`](./enum.ControllerCommand.html).
///
/// # Example
/// The following request will shut down the load test:
/// ```json
/// {
///     "request": "shutdown",
/// }
/// ```
///
/// Responses will be formatted as defined in
/// [ControllerWebSocketResponse](./struct.ControllerWebSocketResponse.html).
#[derive(Debug, Deserialize, Serialize)]
pub struct ControllerWebSocketRequest {
    /// A valid command string.
    pub request: String,
}

/// This structure defines the json format of any response returned from the WebSocket
/// Controller.
///
/// Responses are in the following format:
/// ```json
/// {
///     "response": String,
///     "success": bool,
/// }
/// ```
///
/// # Example
/// The following response will be returned when a request is made to shut down the
/// load test:
/// ```json
/// {
///     "response": "load test shut down",
///     "success": true
/// }
/// ```
///
/// Requests must be formatted as defined in
/// [ControllerWebSocketRequest](./struct.ControllerWebSocketRequest.html).
#[derive(Debug, Deserialize, Serialize)]
pub struct ControllerWebSocketResponse {
    /// The response from the controller.
    pub response: String,
    /// Whether the request was successful or not.
    pub success: bool,
}

/// Return type to indicate whether or not to exit the Controller thread.
type ControllerExit = bool;

/// The telnet Controller message buffer.
type ControllerTelnetMessage = [u8; 1024];

/// The WebSocket Controller message buffer.
type ControllerWebSocketMessage = std::result::Result<
    tokio_tungstenite::tungstenite::Message,
    tokio_tungstenite::tungstenite::Error,
>;

/// Simplify the ControllerExecuteCommand trait definition for WebSockets.
type ControllerWebSocketSender = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    tokio_tungstenite::tungstenite::Message,
>;

/// This state object is created in the main Controller thread and then passed to the specific
/// per-client thread.
pub(crate) struct ControllerState {
    /// Track which controller-thread this is.
    thread_id: u32,
    /// Track the ip and port of the connected TCP client.
    peer_address: String,
    /// A shared channel for communicating with the parent process.
    channel_tx: flume::Sender<ControllerRequest>,
    /// Which protocol this Controller understands.
    protocol: ControllerProtocol,
}
// Defines functions shared by all Controllers.
impl ControllerState {
    async fn accept_connections(self, mut socket: tokio::net::TcpStream) {
        info!(
            "{:?} client [{}] connected from {}",
            self.protocol, self.thread_id, self.peer_address
        );
        match self.protocol {
            ControllerProtocol::Telnet => {
                let mut buf: ControllerTelnetMessage = [0; 1024];

                // Display initial goose> prompt.
                write_to_socket_raw(&mut socket, "goose> ").await;

                loop {
                    // Process data received from the client in a loop.
                    let n = match socket.read(&mut buf).await {
                        Ok(data) => data,
                        Err(_) => {
                            info!(
                                "Telnet client [{}] disconnected from {}",
                                self.thread_id, self.peer_address
                            );
                            break;
                        }
                    };

                    // Invalid request, exit.
                    if n == 0 {
                        info!(
                            "Telnet client [{}] disconnected from {}",
                            self.thread_id, self.peer_address
                        );
                        break;
                    }

                    // Extract the command string in a protocol-specific way.
                    if let Ok(command_string) = self.get_command_string(buf).await {
                        // Extract the command and value in a generic way.
                        if let Ok(request_message) = self.get_match(command_string.trim()).await {
                            // Act on the commmand received.
                            if self.execute_command(&mut socket, request_message).await {
                                // If execute_command returns true, it's time to exit.
                                info!(
                                    "Telnet client [{}] disconnected from {}",
                                    self.thread_id, self.peer_address
                                );
                                break;
                            }
                        } else {
                            self.write_to_socket(
                                &mut socket,
                                Err("unrecognized command".to_string()),
                            )
                            .await;
                        }
                    } else {
                        // Corrupted request from telnet client, exit.
                        info!(
                            "Telnet client [{}] disconnected from {}",
                            self.thread_id, self.peer_address
                        );
                        break;
                    }
                }
            }
            ControllerProtocol::WebSocket => {
                let stream = match tokio_tungstenite::accept_async(socket).await {
                    Ok(s) => s,
                    Err(e) => {
                        info!("invalid WebSocket handshake: {}", e);
                        return;
                    }
                };
                let (mut ws_sender, mut ws_receiver) = stream.split();

                loop {
                    // Wait until the client sends a command.
                    let data = match ws_receiver.next().await {
                        Some(d) => d,
                        None => {
                            // Returning with no data means the client disconnected.
                            info!(
                                "WebSocket client [{}] disconnected from {}",
                                self.thread_id, self.peer_address
                            );
                            break;
                        }
                    };

                    // Extract the command string in a protocol-specific way.
                    if let Ok(command_string) = self.get_command_string(data).await {
                        // Extract the command and value in a generic way.
                        if let Ok(request_message) = self.get_match(command_string.trim()).await {
                            if self.execute_command(&mut ws_sender, request_message).await {
                                // If execute_command() returns true, it's time to exit.
                                info!(
                                    "WebSocket client [{}] disconnected from {}",
                                    self.thread_id, self.peer_address
                                );
                                break;
                            }
                        } else {
                            self.write_to_socket(
                                &mut ws_sender,
                                Err(
                                    "unrecognized command, see Goose book https://book.goose.rs/controller/websocket.html"
                                        .to_string(),
                                ),
                            )
                            .await;
                        }
                    } else {
                        self.write_to_socket(
                            &mut ws_sender,
                            Err(
                                "invalid json, see Goose book https://book.goose.rs/controller/websocket.html"
                                    .to_string(),
                            ),
                        )
                        .await;
                    }
                }
            }
        }
    }

    // Both Controllers use a common function to identify commands.
    async fn get_match(
        &self,
        command_string: &str,
    ) -> Result<ControllerRequestMessage, GooseError> {
        // Use FromStr to convert &str to ControllerCommand.
        let command: ControllerCommand = ControllerCommand::from_str(command_string)?;
        // Extract value if there is one, otherwise will be None.
        let value: Option<String> = command.get_value(command_string);

        Ok(ControllerRequestMessage { command, value })
    }

    /// Process a request entirely within the Controller thread, without sending a message
    /// to the parent thread.
    fn process_local_command(&self, request_message: &ControllerRequestMessage) -> Option<String> {
        match request_message.command {
            ControllerCommand::Help => Some(ControllerCommand::display_help()),
            ControllerCommand::Exit => Some("goodbye!".to_string()),
            // All other commands require sending the request to the parent thread.
            _ => None,
        }
    }

    /// Send a message to parent thread, with or without an optional value, and wait for
    /// a reply.
    async fn process_command(
        &self,
        request: ControllerRequestMessage,
    ) -> Result<ControllerResponseMessage, String> {
        // Create a one-shot channel to allow the parent to reply to our request. As flume
        // doesn't implement a one-shot channel, we use tokio for this temporary channel.
        let (response_tx, response_rx): (
            tokio::sync::oneshot::Sender<ControllerResponse>,
            tokio::sync::oneshot::Receiver<ControllerResponse>,
        ) = tokio::sync::oneshot::channel();

        if self
            .channel_tx
            .try_send(ControllerRequest {
                response_channel: Some(response_tx),
                client_id: self.thread_id,
                request,
            })
            .is_err()
        {
            return Err("parent process has closed the controller channel".to_string());
        }

        // Await response from parent.
        match response_rx.await {
            Ok(value) => Ok(value.response),
            Err(e) => Err(format!("one-shot channel dropped without reply: {}", e)),
        }
    }
}

/// Controller-protocol-specific functions, necessary to manage the different way each
/// Controller protocol communicates with a client.
#[async_trait]
trait Controller<T> {
    // Extract the command string from a Controller client request.
    async fn get_command_string(&self, raw_value: T) -> Result<String, String>;
}
#[async_trait]
impl Controller<ControllerTelnetMessage> for ControllerState {
    // Extract the command string from a telnet Controller client request.
    async fn get_command_string(
        &self,
        raw_value: ControllerTelnetMessage,
    ) -> Result<String, String> {
        let command_string = match str::from_utf8(&raw_value) {
            Ok(m) => {
                if let Some(c) = m.lines().next() {
                    c
                } else {
                    ""
                }
            }
            Err(e) => {
                let error = format!("ignoring unexpected input from telnet controller: {}", e);
                info!("{}", error);
                return Err(error);
            }
        };

        Ok(command_string.to_string())
    }
}
#[async_trait]
impl Controller<ControllerWebSocketMessage> for ControllerState {
    // Extract the command string from a WebSocket Controller client request.
    async fn get_command_string(
        &self,
        raw_value: ControllerWebSocketMessage,
    ) -> Result<String, String> {
        if let Ok(request) = raw_value {
            if request.is_text() {
                if let Ok(request) = request.into_text() {
                    debug!("websocket request: {:?}", request.trim());
                    let command_string: ControllerWebSocketRequest =
                        match serde_json::from_str(&request) {
                            Ok(c) => c,
                            Err(_) => {
                                return Err("invalid json, see Goose book https://book.goose.rs/controller/websocket.html"
                                    .to_string())
                            }
                        };
                    return Ok(command_string.request);
                } else {
                    // Failed to consume the WebSocket message and convert it to a String.
                    return Err("unsupported string format".to_string());
                }
            } else {
                // Received a non-text WebSocket message.
                return Err("unsupported format, requests must be sent as text".to_string());
            }
        }
        // Improper WebSocket handshake.
        Err("WebSocket handshake error".to_string())
    }
}
#[async_trait]
trait ControllerExecuteCommand<T> {
    // Run the command received from a Controller request. Returns a boolean, if true exit.
    async fn execute_command(
        &self,
        socket: &mut T,
        request_message: ControllerRequestMessage,
    ) -> ControllerExit;

    // Send response to Controller client. The response is wrapped in a Result to indicate
    // if the request was successful or not.
    async fn write_to_socket(&self, socket: &mut T, response_message: Result<String, String>);
}
#[async_trait]
impl ControllerExecuteCommand<tokio::net::TcpStream> for ControllerState {
    // Run the command received from a telnet Controller request.
    async fn execute_command(
        &self,
        socket: &mut tokio::net::TcpStream,
        request_message: ControllerRequestMessage,
    ) -> ControllerExit {
        // First handle commands that don't require interaction with the parent process.
        if let Some(message) = self.process_local_command(&request_message) {
            self.write_to_socket(socket, Ok(message)).await;
            // If Exit was received return true to exit, otherwise return false.
            return request_message.command == ControllerCommand::Exit;
        }

        // Retain a copy of the command used when processing the parent response.
        let command = request_message.command.clone();

        // Now handle commands that require interaction with the parent process.
        let response = match self.process_command(request_message).await {
            Ok(r) => r,
            Err(e) => {
                // Receiving an error here means the parent closed the communication
                // channel. Write the error to the Controller client and then return
                // true to exit.
                self.write_to_socket(socket, Err(e)).await;
                return true;
            }
        };

        // If Shutdown command was received return true to exit, otherwise return false.
        let exit_controller = command == ControllerCommand::Shutdown;

        // Write the response to the Controller client socket.
        let processed_response = (command.details().process_response)(response);
        self.write_to_socket(socket, processed_response).await;

        // Return true if it's time to exit the Controller.
        exit_controller
    }

    // Send response to telnet Controller client.
    async fn write_to_socket(
        &self,
        socket: &mut tokio::net::TcpStream,
        message: Result<String, String>,
    ) {
        // Send result to telnet Controller client, whether Ok() or Err().
        let response_message = match message {
            Ok(m) => m,
            Err(e) => e,
        };
        if socket
            // Add a linefeed to the end of the message, followed by a prompt.
            .write_all([&response_message, "\ngoose> "].concat().as_bytes())
            .await
            .is_err()
        {
            warn!("failed to write data to socker");
        };
    }
}
#[async_trait]
impl ControllerExecuteCommand<ControllerWebSocketSender> for ControllerState {
    // Run the command received from a WebSocket Controller request.
    async fn execute_command(
        &self,
        socket: &mut ControllerWebSocketSender,
        request_message: ControllerRequestMessage,
    ) -> ControllerExit {
        // First handle commands that don't require interaction with the parent process.
        if let Some(message) = self.process_local_command(&request_message) {
            self.write_to_socket(socket, Ok(message)).await;

            // If Exit was received return true to exit, otherwise return false.
            let exit_controller = request_message.command == ControllerCommand::Exit;
            // If exiting, notify the WebSocket client that this connection is closing.
            if exit_controller
                && socket
                    .send(Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                        reason: std::borrow::Cow::Borrowed("exit"),
                    })))
                    .await
                    .is_err()
            {
                warn!("failed to write data to stream");
            }

            return exit_controller;
        }

        // WebSocket Controller always returns JSON, convert command where necessary.
        let command = match request_message.command {
            ControllerCommand::Config => ControllerCommand::ConfigJson,
            ControllerCommand::Metrics => ControllerCommand::MetricsJson,
            _ => request_message.command.clone(),
        };

        // Now handle commands that require interaction with the parent process.
        let response = match self.process_command(request_message).await {
            Ok(r) => r,
            Err(e) => {
                // Receiving an error here means the parent closed the communication
                // channel. Write the error to the Controller client and then return
                // true to exit.
                self.write_to_socket(socket, Err(e)).await;
                return true;
            }
        };

        // If Shutdown command was received return true to exit, otherwise return false.
        let exit_controller = command == ControllerCommand::Shutdown;

        // Write the response to the Controller client socket.
        let processed_response = (command.details().process_response)(response);
        self.write_to_socket(socket, processed_response).await;

        // If exiting, notify the WebSocket client that this connection is closing.
        if exit_controller
            && socket
                .send(Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: std::borrow::Cow::Borrowed("shutdown"),
                })))
                .await
                .is_err()
        {
            warn!("failed to write data to stream");
        }

        // Return true if it's time to exit the Controller.
        exit_controller
    }

    // Send a json-formatted response to the WebSocket.
    async fn write_to_socket(
        &self,
        socket: &mut ControllerWebSocketSender,
        response_result: Result<String, String>,
    ) {
        let success;
        let response = match response_result {
            Ok(m) => {
                success = true;
                m
            }
            Err(e) => {
                success = false;
                e
            }
        };
        if let Err(e) = socket
            .send(Message::Text(
                match serde_json::to_string(&ControllerWebSocketResponse {
                    response,
                    // Success is true if there is no error, false if there is an error.
                    success,
                }) {
                    Ok(json) => json,
                    Err(e) => {
                        warn!("failed to json encode response: {}", e);
                        return;
                    }
                },
            ))
            .await
        {
            info!("failed to write data to websocket: {}", e);
        }
    }
}
