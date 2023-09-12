/// Manager-specific code.
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use crate::config::{GooseConfigure, GooseValue};
use crate::util;
use crate::{GooseConfiguration, GooseDefaults, GooseError};

/// Optional join handle for manager thread, if enabled.
pub(crate) type ManagerJoinHandle = tokio::task::JoinHandle<std::result::Result<(), GooseError>>;
/// Optional unbounded sender to manager thread, if enabled.
pub(crate) type ManagerTx = flume::Sender<ManagerMessage>;

// Tracks the join_handle and send socket for Worker instance.
#[derive(Debug)]
pub(crate) struct ManagerConnection {
    pub(crate) _join_handle: ManagerJoinHandle,
    pub(crate) tx: ManagerTx,
}

#[derive(Debug)]
pub(crate) enum ManagerCommand {
    // Gaggle is starting, wait for all Worker instances to connect.
    WaitForWorkers,
    // Worker is requesting to join the Gaggle.
    WorkerJoinRequest,
    // Exit
    _Exit,
}

/// This structure is used to control the Manager process.
#[derive(Debug)]
pub(crate) struct ManagerMessage {
    /// The command that is being sent to the Manager.
    pub(crate) command: ManagerCommand,
    /// An optional socket if this is a Worker connecting to a Manager.
    pub(crate) value: Option<tokio::net::TcpStream>,
}

struct ManagerRunState {
    /// Workers
    workers: Vec<tokio::net::TcpStream>,
    /// Whether or not a message has been displayed indicating the Manager is currently idle.
    idle_status_displayed: bool,
    /// Which phase the Manager is currently operating in.
    phase: ManagerPhase,
    /// This variable accounts for time spent doing things which is then subtracted from
    /// the time sleeping to avoid an unintentional drift in events that are supposed to
    /// happen regularly.
    drift_timer: tokio::time::Instant,
    /// Receive messages from the Controller.
    controller_rx: flume::Receiver<ManagerMessage>,
}
impl ManagerRunState {
    fn new(controller_rx: flume::Receiver<ManagerMessage>) -> ManagerRunState {
        ManagerRunState {
            workers: Vec::new(),
            idle_status_displayed: false,
            phase: ManagerPhase::Idle,
            drift_timer: tokio::time::Instant::now(),
            controller_rx,
        }
    }
}

enum ManagerPhase {
    /// No Workers are connected, Gaggle can be configured.
    Idle,
    /// Workers are connecting to the Manager, Gaggle can not be reconfigured.
    WaitForWorkers,
    /// All Workers are connected and the load test is ready.
    Active,
}

impl GooseConfiguration {
    pub(crate) fn configure_manager(&mut self, defaults: &GooseDefaults) {
        // Determine how many CPUs are available.
        let default_users = match std::thread::available_parallelism() {
            Ok(ap) => Some(ap.get()),
            Err(e) => {
                // Default to 1 user if unable to detect number of CPUs.
                info!("failed to detect available_parallelism: {}", e);
                Some(1)
            }
        };

        // Re-configure `users`, in case the AttackMode was changed.
        self.users = self.get_value(vec![
            // Use --users if set and not on Worker.
            GooseValue {
                value: self.users,
                filter: self.worker,
                message: "--users",
            },
            // Otherwise use GooseDefault if set and not on Worker.
            GooseValue {
                value: defaults.users,
                filter: defaults.users.is_none() || self.worker,
                message: "default users",
            },
            // Otherwise use detected number of CPUs if not on Worker.
            GooseValue {
                value: default_users,
                filter: self.worker || self.test_plan.is_some(),
                message: "users defaulted to number of CPUs",
            },
        ]);
        // Configure `expect_workers`.
        self.expect_workers = self.get_value(vec![
            // Use --expect-workers if configured.
            GooseValue {
                value: self.expect_workers,
                filter: self.expect_workers.is_none(),
                message: "expect_workers",
            },
            // Use GooseDefault if not already set and not Worker.
            GooseValue {
                value: defaults.expect_workers,
                filter: self.expect_workers.is_none() && self.worker,
                message: "expect_workers",
            },
        ]);

        // Configure `no_hash_check`.
        self.no_hash_check = self
            .get_value(vec![
                // Use --no-hash_check if set.
                GooseValue {
                    value: Some(self.no_hash_check),
                    filter: !self.no_hash_check,
                    message: "no_hash_check",
                },
                // Use GooseDefault if not already set and not Worker.
                GooseValue {
                    value: defaults.no_hash_check,
                    filter: defaults.no_hash_check.is_none() || self.worker,
                    message: "no_hash_check",
                },
            ])
            .unwrap_or(false);
    }

    /// Validate configured [`GooseConfiguration`] values.
    pub(crate) fn validate_manager(&self) -> Result<(), GooseError> {
        // Validate nothing incompatible is enabled with --manager.
        if self.manager {
            // Don't allow --manager and --worker together.
            // @TODO: Implement this!
            if self.worker {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager` && `configuration.worker`".to_string(),
                    value: "true".to_string(),
                    detail: "Goose can not run as both Manager and Worker".to_string(),
                });
            } else if !self.debug_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.debug_log`".to_string(),
                    value: self.debug_log.clone(),
                    detail: "`configuration.debug_log` can not be set on the Manager.".to_string(),
                });
            } else if !self.error_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.error_log`".to_string(),
                    value: self.error_log.clone(),
                    detail: "`configuration.error_log` can not be set on the Manager.".to_string(),
                });
            } else if !self.request_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.request_log`".to_string(),
                    value: self.request_log.clone(),
                    detail: "`configuration.request_log` can not be set on the Manager."
                        .to_string(),
                });
            } else if !self.transaction_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.transaction_log`".to_string(),
                    value: self.transaction_log.clone(),
                    detail: "`configuration.transaction_log` can not be set on the Manager."
                        .to_string(),
                });
            } else if !self.scenario_log.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.scenario_log`".to_string(),
                    value: self.scenario_log.clone(),
                    detail: "`configuration.scenario_log` can not be set on the Manager."
                        .to_string(),
                });
            } else if !self.report_file.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.report_file`".to_string(),
                    value: self.report_file.to_string(),
                    detail: "`configuration.report_file` can not be set on the Manager."
                        .to_string(),
                });
            } else if self.no_granular_report {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_granular_report`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_granular_report` can not be set on the Manager."
                        .to_string(),
                });
            } else if self.no_debug_body {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_debug_body`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_debug_body` can not be set on the Manager."
                        .to_string(),
                });
            // Can not set `throttle_requests` on Manager.
            } else if self.throttle_requests > 0 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.throttle_requests`".to_string(),
                    value: self.throttle_requests.to_string(),
                    detail: "`configuration.throttle_requests` can not be set on the Manager."
                        .to_string(),
                });
            }
            if let Some(expect_workers) = self.expect_workers.as_ref() {
                // Must expect at least 1 Worker when running as Manager.
                if expect_workers == &0 {
                    return Err(GooseError::InvalidOption {
                        option: "`configuration.expect_workers`".to_string(),
                        value: expect_workers.to_string(),
                        detail: "`configuration.expect_workers must be set to at least 1."
                            .to_string(),
                    });
                }

                // Must be at least 1 user per worker.
                if let Some(users) = self.users.as_ref() {
                    if expect_workers > users {
                        return Err(GooseError::InvalidOption {
                            option: "`configuration.expect_workers`".to_string(),
                            value: expect_workers.to_string(),
                            detail: "`configuration.expect_workers can not be set to a value larger than `configuration.users`.".to_string(),
                        });
                    }
                } else {
                    return Err(GooseError::InvalidOption {
                        option: "`configuration.expect_workers`".to_string(),
                        value: expect_workers.to_string(),
                        detail: "`configuration.expect_workers can not be set without setting `configuration.users`.".to_string(),
                    });
                }
            } else {
                return Err(GooseError::InvalidOption {
                    option: "configuration.manager".to_string(),
                    value: true.to_string(),
                    detail: "Manager mode requires --expect-workers be configured".to_string(),
                });
            }
        } else {
            // Don't allow `expect_workers` if not running as Manager.
            if let Some(expect_workers) = self.expect_workers.as_ref() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.expect_workers`".to_string(),
                    value: expect_workers.to_string(),
                    detail: "`configuration.expect_workers` can not be set unless on the Manager."
                        .to_string(),
                });
            }
        }

        Ok(())
    }

    // Spawn a Manager thread, provide a channel so it can be controlled by parent and/or Control;er thread.
    pub(crate) async fn setup_manager(&mut self) -> Option<(ManagerJoinHandle, ManagerTx)> {
        // There's no setup necessary if Manager mode is not enabled.
        if !self.manager {
            return None;
        }

        // Create an unbounded channel to allow the controller to manage the Manager thread.
        let (manager_tx, manager_rx): (
            flume::Sender<ManagerMessage>,
            flume::Receiver<ManagerMessage>,
        ) = flume::unbounded();

        let configuration = self.clone();
        let manager_handle =
            tokio::spawn(async move { configuration.manager_main(manager_rx).await });

        // Return manager_tx thread for the (optional) controller thread.
        Some((manager_handle, manager_tx))
    }

    /// Manager thread, coordinates Worker threads.
    pub(crate) async fn manager_main(
        self: GooseConfiguration,
        receiver: flume::Receiver<ManagerMessage>,
    ) -> Result<(), GooseError> {
        // Initialze the Manager run state, used for the lifetime of this Manager instance.
        let mut manager_run_state = ManagerRunState::new(receiver);

        loop {
            debug!("top of manager loop...");

            match manager_run_state.phase {
                // Display message when entering ManagerPhase::Idle, otherwise sleep waiting for a
                // message from Parent or Controller thread.
                ManagerPhase::Idle => {
                    if !manager_run_state.idle_status_displayed {
                        info!("Gaggle mode enabled, Manager is idle.");
                        manager_run_state.idle_status_displayed = true;
                    }
                }
                ManagerPhase::WaitForWorkers => {
                    // @TODO: Keepalive? Timeout?
                }
                ManagerPhase::Active => {
                    // @TODO: Actually start the load test.
                }
            }

            // Process messages received from parent or Controller thread.
            let sleep_duration = match manager_run_state.controller_rx.try_recv() {
                Ok(message) => {
                    match message.command {
                        ManagerCommand::WaitForWorkers => {
                            let expect_workers = self.expect_workers.unwrap_or(0);
                            if expect_workers == 1 {
                                info!("Manager is waiting for 1 Worker.");
                            } else {
                                info!("Manager is waiting for {} Workers.", expect_workers);
                            }
                            manager_run_state.phase = ManagerPhase::WaitForWorkers;
                        }
                        ManagerCommand::WorkerJoinRequest => {
                            let mut socket = message.value.expect("failed to unwrap TcpSocket");
                            if socket.write_all("OK\r\n".as_bytes()).await.is_err() {
                                warn!("failed to write data to socker");
                            }
                            // Store Worker socket for ongoing communications.
                            manager_run_state.workers.push(socket);

                            if let Some(expect_workers) = self.expect_workers {
                                if manager_run_state.workers.len() == self.expect_workers.unwrap() {
                                    info!(
                                        "All {} Workers have connected, starting the load test.",
                                        expect_workers
                                    );
                                    manager_run_state.phase = ManagerPhase::Active;
                                }
                            }
                        }
                        ManagerCommand::_Exit => {
                            info!("Manager is exiting.");
                            break;
                        }
                    }
                    // Message received, fall through but do not sleep.
                    Duration::ZERO
                }
                // No message, fall through and sleep to try again later.
                Err(_) => Duration::from_millis(500),
            };

            // Wake up twice a second to handle messages and allow for a quick shutdown if the
            // load test is canceled during startup.
            debug!("sleeping {:?}...", sleep_duration);
            manager_run_state.drift_timer =
                util::sleep_minus_drift(sleep_duration, manager_run_state.drift_timer).await;
        }

        Ok(())
    }
}
