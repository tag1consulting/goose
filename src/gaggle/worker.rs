/// Worker-specific code.
use std::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};

use crate::config::{GooseConfigure, GooseValue};
use crate::metrics::GooseCoordinatedOmissionMitigation;
use crate::util;
use crate::{GooseConfiguration, GooseDefaults, GooseError};

/// Optional join handle for worker thread, if enabled.
pub(crate) type WorkerJoinHandle = tokio::task::JoinHandle<std::result::Result<(), GooseError>>;
/// Optional unbounded sender to worker thread, if enabled.
pub(crate) type WorkerTx = flume::Sender<WorkerMessage>;

type SocketBuffer = [u8; 1024];

const MAX_CONNECTION_ATTEMPTS: u8 = 25;

#[derive(Debug)]
pub(crate) enum WorkerCommand {
    ConnectToManager,
    Stop,
}

/// This structure is used to control the Worker process.
#[derive(Debug)]
pub(crate) struct WorkerMessage {
    /// The command that is being sent to the Worker.
    pub(crate) command: WorkerCommand,
    /// An optional value that is being sent to the Worker.
    pub(crate) _value: Option<String>,
}

// Tracks the join_handle and send socket for Worker instance.
#[derive(Debug)]
pub(crate) struct WorkerConnection {
    pub(crate) _join_handle: WorkerJoinHandle,
    pub(crate) tx: WorkerTx,
}

enum ConnectionState {
    WaitForPrompt,
    WaitForOk,
    Connected,
}

struct WorkerRunState {
    /// Whether or not a message has been displayed indicating the Worker is currently idle.
    idle_status_displayed: bool,
    /// Whether or Worker has successfully connected to Manager instance.
    connected_to_manager: bool,
    /// @TODO: Connection status
    connection_state: Option<ConnectionState>,
    /// A counter tracking how many times the Worker has attempted to connect to the Manager.
    connection_attempts: u8,
    /// Which phase the Worker is currently operating in.
    phase: WorkerPhase,
    /// Whether or not a message has been displayed indicating the Worker is ready and waiting.
    waiting_status_displayed: bool,
    /// This variable accounts for time spent doing things which is then subtracted from
    /// the time sleeping to avoid an unintentional drift in events that are supposed to
    /// happen regularly.
    drift_timer: tokio::time::Instant,
    /// Receive messages from the Controller.
    controller_rx: flume::Receiver<WorkerMessage>,
    /// Connection to Manager.
    stream: Option<TcpStream>,
}
impl WorkerRunState {
    fn new(controller_rx: flume::Receiver<WorkerMessage>) -> WorkerRunState {
        WorkerRunState {
            idle_status_displayed: false,
            connected_to_manager: false,
            connection_state: None,
            connection_attempts: 0,
            phase: WorkerPhase::Idle,
            waiting_status_displayed: false,
            drift_timer: tokio::time::Instant::now(),
            controller_rx,
            stream: None,
        }
    }
}

enum WorkerPhase {
    /// Not connected to Manager, Worker instance is stand-alone and idle.
    Idle,
    /// Trying to connect to the Manager instance.
    ConnectingToManager,
    /// Connected to Manager instance, waiting for the go-ahead to start load test.
    WaitingForManager,
    /// Active load test.
    _Active,
    Exit,
}

impl GooseConfiguration {
    pub(crate) fn configure_worker(&mut self, defaults: &GooseDefaults) {
        // Set `manager_host` on Worker.
        self.manager_host = self
            .get_value(vec![
                // Use --manager-host if configured.
                GooseValue {
                    value: Some(self.manager_host.to_string()),
                    filter: self.manager_host.is_empty(),
                    message: "manager_host",
                },
                // Otherwise use default if set and on Worker.
                GooseValue {
                    value: defaults.manager_host.clone(),
                    filter: defaults.manager_host.is_none() || !self.worker,
                    message: "manager_host",
                },
                // Otherwise default to 127.0.0.1 if on Worker.
                GooseValue {
                    value: Some("127.0.0.1".to_string()),
                    filter: !self.worker,
                    message: "manager_host",
                },
            ])
            .unwrap_or_default();

        // Set `manager_port` on Worker.
        self.manager_port = self
            .get_value(vec![
                // Use --manager-port if configured.
                GooseValue {
                    value: Some(self.manager_port),
                    filter: self.manager_port == 0,
                    message: "manager_port",
                },
                // Otherwise use default if set and on Worker.
                GooseValue {
                    value: defaults.manager_port,
                    filter: defaults.manager_port.is_none() || !self.worker,
                    message: "manager_port",
                },
                // Otherwise default to DEFAULT_GAGGLE_PORT if on Worker.
                GooseValue {
                    value: Some(crate::DEFAULT_TELNET_PORT.to_string().parse().unwrap()),
                    filter: !self.worker,
                    message: "manager_port",
                },
            ])
            .unwrap_or(0);
    }

    /// Validate configured [`GooseConfiguration`] values.
    pub(crate) fn validate_worker(&self) -> Result<(), GooseError> {
        // Validate nothing incompatible is enabled with --worker.
        if self.worker {
            // Can't set `users` on Worker.
            if self.users.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "configuration.users".to_string(),
                    value: self.users.as_ref().unwrap().to_string(),
                    detail: "`configuration.users` can not be set together with the `configuration.worker`.".to_string(),
                });
            // Can't set `startup_time` on Worker.
            } else if self.startup_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.startup_time".to_string(),
                    value: self.startup_time.to_string(),
                    detail: "`configuration.startup_time` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `run_time` on Worker.
            } else if self.run_time != "0" {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.run_time".to_string(),
                    value: self.run_time.to_string(),
                    detail: "`configuration.run_time` can not be set in Worker mode.".to_string(),
                });
            // Can't set `hatch_rate` on Worker.
            } else if self.hatch_rate.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.hatch_rate`".to_string(),
                    value: self.hatch_rate.as_ref().unwrap().to_string(),
                    detail: "`configuration.hatch_rate` can not be set in Worker mode.".to_string(),
                });
            // Can't set `timeout` on Worker.
            } else if self.timeout.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.timeout`".to_string(),
                    value: self.timeout.as_ref().unwrap().to_string(),
                    detail: "`configuration.timeout` can not be set in Worker mode.".to_string(),
                });
            // Can't set `running_metrics` on Worker.
            } else if self.running_metrics.is_some() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.running_metrics".to_string(),
                    value: self.running_metrics.as_ref().unwrap().to_string(),
                    detail: "`configuration.running_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_reset_metrics` on Worker.
            } else if self.no_reset_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_reset_metrics".to_string(),
                    value: self.no_reset_metrics.to_string(),
                    detail: "`configuration.no_reset_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_metrics` on Worker.
            } else if self.no_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_metrics".to_string(),
                    value: self.no_metrics.to_string(),
                    detail: "`configuration.no_metrics` can not be set in Worker mode.".to_string(),
                });
            // Can't set `no_transaction_metrics` on Worker.
            } else if self.no_transaction_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_transaction_metrics".to_string(),
                    value: self.no_transaction_metrics.to_string(),
                    detail: "`configuration.no_transaction_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_scenario_metrics` on Worker.
            } else if self.no_scenario_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_scenario_metrics".to_string(),
                    value: self.no_scenario_metrics.to_string(),
                    detail: "`configuration.no_scenario_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_print_metrics` on Worker.
            } else if self.no_print_metrics {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_print_metrics".to_string(),
                    value: self.no_print_metrics.to_string(),
                    detail: "`configuration.no_print_metrics` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_error_summary` on Worker.
            } else if self.no_error_summary {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_error_summary".to_string(),
                    value: self.no_error_summary.to_string(),
                    detail: "`configuration.no_error_summary` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_status_codes` on Worker.
            } else if self.no_status_codes {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_status_codes".to_string(),
                    value: self.no_status_codes.to_string(),
                    detail: "`configuration.no_status_codes` can not be set in Worker mode."
                        .to_string(),
                });
            // Can't set `no_gzip` on Worker.
            } else if self.no_gzip {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_gzip`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_gzip` can not be set in Worker mode.".to_string(),
                });
            } else if self
                .co_mitigation
                .as_ref()
                .unwrap_or(&GooseCoordinatedOmissionMitigation::Disabled)
                != &GooseCoordinatedOmissionMitigation::Disabled
            {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.co_mitigation`".to_string(),
                    value: format!("{:?}", self.co_mitigation.as_ref().unwrap()),
                    detail: "`configuration.co_mitigation` can not be set in Worker mode."
                        .to_string(),
                });
            // Must set `manager_host` on Worker.
            } else if self.manager_host.is_empty() {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager_host`".to_string(),
                    value: self.manager_host.clone(),
                    detail: "`configuration.manager_host` must be set when in Worker mode."
                        .to_string(),
                });
            // Must set `manager_port` on Worker.
            } else if self.manager_port == 0 {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.manager_port`".to_string(),
                    value: self.manager_port.to_string(),
                    detail: "`configuration.manager_port` must be set when in Worker mode."
                        .to_string(),
                });
            // Can not set `sticky_follow` on Worker.
            } else if self.sticky_follow {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.sticky_follow`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.sticky_follow` can not be set in Worker mode."
                        .to_string(),
                });
            // Can not set `no_hash_check` on Worker.
            } else if self.no_hash_check {
                return Err(GooseError::InvalidOption {
                    option: "`configuration.no_hash_check`".to_string(),
                    value: true.to_string(),
                    detail: "`configuration.no_hash_check` can not be set in Worker mode."
                        .to_string(),
                });
            }
        }

        Ok(())
    }

    // Spawn a Worker thread, provide a channel so it can be controlled by parent and/or Control;er thread.
    pub(crate) async fn setup_worker(&mut self, hash: u64) -> Option<(WorkerJoinHandle, WorkerTx)> {
        // There's no setup necessary if Worker mode is not enabled.
        if !self.worker {
            return None;
        }

        // Create an unbounded channel to allow the controller to manage the Worker thread.
        let (worker_tx, worker_rx): (flume::Sender<WorkerMessage>, flume::Receiver<WorkerMessage>) =
            flume::unbounded();

        let configuration = self.clone();
        let worker_handle =
            tokio::spawn(async move { configuration.worker_main(worker_rx, hash).await });

        // Return worker_tx thread for the (optional) controller thread.
        Some((worker_handle, worker_tx))
    }

    /// Worker thread, coordiantes with Manager instanec.
    pub(crate) async fn worker_main(
        self: GooseConfiguration,
        receiver: flume::Receiver<WorkerMessage>,
        hash: u64,
    ) -> Result<(), GooseError> {
        // Initialze the Worker run state, used for the lifetime of this Worker instance.
        let mut worker_run_state = WorkerRunState::new(receiver);

        // Sleep 1 second to give Manager time to start, if started at the same time.
        sleep(Duration::from_secs(1)).await;

        loop {
            debug!("top of worker loop...");

            // @TODO: How to detect that the socket is dropped?
            // @TODO: Add a timeout.

            match worker_run_state.phase {
                // Display message when entering WorkerPhase::Idle, otherwise sleep waiting for a
                // message from Parent or Controller thread.
                WorkerPhase::Idle => {
                    if !worker_run_state.idle_status_displayed {
                        info!("Gaggle mode enabled, Worker is idle.");
                        worker_run_state.idle_status_displayed = true;
                    }
                }
                WorkerPhase::ConnectingToManager => {
                    if !worker_run_state.connected_to_manager {
                        if worker_run_state.connection_attempts == 0
                            || worker_run_state.connection_attempts % 5 == 0
                        {
                            info!(
                                "Worker connecting to {}:{}.",
                                self.manager_host, self.manager_port
                            );
                        }

                        if worker_run_state.connection_attempts >= MAX_CONNECTION_ATTEMPTS {
                            // @TODO: If --no-autostart go back to idle mode.
                            warn!("failed to connect to Manager");
                            break;
                        }

                        // Only try so many times before giving up.
                        worker_run_state.connection_attempts += 1;

                        // Actually try to connect.
                        worker_run_state.stream = match TcpStream::connect(format!(
                            "{}:{}",
                            self.manager_host, self.manager_port
                        ))
                        .await
                        {
                            Ok(s) => {
                                worker_run_state.connected_to_manager = true;
                                worker_run_state.connection_state =
                                    Some(ConnectionState::WaitForPrompt);
                                Some(s)
                            }
                            Err(e) => {
                                if worker_run_state.connection_attempts % 5 == 0 {
                                    warn!(
                                        "Worker failed to connect to Manager ({} of {} attempts): {}",
                                        worker_run_state.connection_attempts,
                                        MAX_CONNECTION_ATTEMPTS,
                                        e
                                    );
                                }
                                None
                            }
                        };
                    }
                    if let Some(stream) = worker_run_state.stream.as_mut() {
                        if let Ok(Some(message)) = read_buffer(stream) {
                            if let Some(connection_state) =
                                worker_run_state.connection_state.as_ref()
                            {
                                match connection_state {
                                    ConnectionState::WaitForPrompt => {
                                        if message.starts_with("goose>") {
                                            info!("Got `goose>` prompt.");
                                            worker_run_state.connection_state =
                                                Some(ConnectionState::WaitForOk);
                                            stream
                                                .write_all(
                                                    format!("WORKER-CONNECT {}\n", hash).as_bytes(),
                                                )
                                                .await?;
                                        } else {
                                            panic!("Failed to get `goose>` prompt: @TODO: handle this more gracefully.");
                                        }
                                    }
                                    ConnectionState::WaitForOk => {
                                        if message.starts_with("OK") {
                                            info!("Got OK.");
                                            worker_run_state.connection_state =
                                                Some(ConnectionState::Connected);
                                            worker_run_state.phase = WorkerPhase::WaitingForManager;
                                        } else {
                                            panic!("Failed to get OK: @TODO: handle this more gracefully.");
                                        }
                                    }
                                    _ => {
                                        unreachable!("We should not be here.");
                                    }
                                }
                            }
                        }
                    };
                }
                WorkerPhase::WaitingForManager => {
                    if !worker_run_state.waiting_status_displayed {
                        info!("Standing by, waiting for Manager to start the load test...");
                        worker_run_state.waiting_status_displayed = true;
                    }
                }
                WorkerPhase::_Active => {
                    info!("Let's get this party started!");
                }
                WorkerPhase::Exit => {
                    info!("Worker is exiting.");
                    break;
                }
            }

            // Process messages received from parent or Controller thread.
            let sleep_duration = match worker_run_state.controller_rx.try_recv() {
                Ok(message) => {
                    match message.command {
                        WorkerCommand::ConnectToManager => {
                            worker_run_state.phase = WorkerPhase::ConnectingToManager;
                        }
                        WorkerCommand::Stop => {
                            worker_run_state.phase = WorkerPhase::Exit;
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
            worker_run_state.drift_timer =
                util::sleep_minus_drift(sleep_duration, worker_run_state.drift_timer).await;
        }

        Ok(())
    }
}

fn read_buffer(stream: &TcpStream) -> Result<Option<String>, &str> {
    let mut socket_buffer: SocketBuffer = [0; 1024];

    match stream.try_read(&mut socket_buffer) {
        Ok(n) => {
            if n == 0 {
                return Err("Worker disconnected");
            }
            let message = match std::str::from_utf8(&socket_buffer) {
                Ok(m) => {
                    if let Some(c) = m.lines().next() {
                        c
                    } else {
                        ""
                    }
                }
                Err(e) => {
                    info!("ignoring unexpected input from Manager: {}", e);
                    ""
                }
            };
            Ok(Some(message.to_string()))
        }
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
            Err("io::ErrorKind::WouldBlock - @TODO wtf is this?")
        }
        Err(e) => {
            warn!("unexpected read error: {}", e);
            Ok(None)
        }
    }
}
