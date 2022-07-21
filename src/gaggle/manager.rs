/// Manager-specific code.
use std::time::Duration;

use crate::util;
use crate::{GooseConfiguration, GooseDefaults, GooseError};

/// Optional join handle for manager thread, if enabled.
pub(crate) type ManagerJoinHandle =
    Option<tokio::task::JoinHandle<std::result::Result<(), GooseError>>>;
/// Optional unbounded sender to manager thread, if enabled.
pub(crate) type ManagerTx = Option<flume::Sender<ManagerMessage>>;

#[derive(Debug)]
pub(crate) enum ManagerCommand {
    // Gaggle is starting, wait for all Worker instances to connect.
    WaitForWorkers,
    // Exit
    _Exit,
}

/// This structure is used to control the Manager process.
#[derive(Debug)]
pub(crate) struct ManagerMessage {
    /// The command that is being sent to the Manager.
    pub(crate) command: ManagerCommand,
    /// An optional value that is being sent to the Manager.
    pub(crate) _value: Option<String>,
}

struct ManagerRunState {
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
            idle_status_displayed: false,
            phase: ManagerPhase::Idle,
            drift_timer: tokio::time::Instant::now(),
            controller_rx,
        }
    }
}

/// @TODO: Actually, remove the AttackPhase duplication: that shouldn't be handled differently.
enum ManagerPhase {
    /// No Workers are connected, Gaggle can be configured.
    Idle,
    /// Workers are connecting to the Manager, Gaggle can not be reconfigured.
    WaitForWorkers,
    /// All Workers are connected and the load test is ready.
    _Active,
}

impl GooseConfiguration {
    // @TODO: move Manager configuration here.
    pub(crate) fn configure_manager(&mut self, _defaults: &GooseDefaults) {
        //
    }

    // @TODO: it should be possible for the controller to reconfigure / make changes before load test starts.
    // @TODO: This needs to be its own thread, allowing the controller to end it.
    pub(crate) async fn setup_manager(
        &mut self,
        defaults: &GooseDefaults,
    ) -> Result<(ManagerJoinHandle, ManagerTx), GooseError> {
        // Update the manager configuration, loading defaults if necessasry.
        self.configure_manager(defaults);

        // There's no setup necessary if Manager mode is not enabled.
        if !self.manager {
            return Ok((None, None));
        }

        // Create an unbounded channel to allow the controller to manage the Manager thread.
        let (manager_tx, manager_rx): (
            flume::Sender<ManagerMessage>,
            flume::Receiver<ManagerMessage>,
        ) = flume::unbounded();

        let configuration = self.clone();
        let manager_handle =
            tokio::spawn(async move { configuration.manager_main(manager_rx).await });
        // @TODO: return manager_tx thread to the controller (if there is a controller)
        Ok((Some(manager_handle), Some(manager_tx)))
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
                ManagerPhase::WaitForWorkers => {}
                ManagerPhase::_Active => {}
            }

            // Process messages received from parent or Controller thread.
            let sleep_duration = match manager_run_state.controller_rx.try_recv() {
                Ok(message) => {
                    match message.command {
                        ManagerCommand::WaitForWorkers => {
                            let expect_workers = self.expect_workers.unwrap_or(0);
                            if expect_workers == 1 {
                                info!("Manager is waiting for {} Worker.", expect_workers);
                            } else {
                                info!("Manager is waiting for {} Workers.", expect_workers);
                            }
                            manager_run_state.phase = ManagerPhase::WaitForWorkers;
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
