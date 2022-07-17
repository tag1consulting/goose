/// Manager-specific code.
use std::time::Duration;
use tokio::time::sleep;

use crate::{GooseConfiguration, GooseDefaults, GooseError};

/// Optional join handle for manager thread, if enabled.
pub(crate) type ManagerJoinHandle =
    Option<tokio::task::JoinHandle<std::result::Result<(), GooseError>>>;
/// Optional unbounded sender to manager thread, if enabled.
pub(crate) type ManagerTx = Option<flume::Sender<Option<ManagerMessage>>>;

#[derive(Debug)]
pub(crate) enum ManagerCommand {}

/// This structure is used to control the Manager process.
#[derive(Debug)]
pub(crate) struct ManagerMessage {
    /// The command that is being sent to the Manager.
    pub command: ManagerCommand,
    /// An optional value that is being sent to the Manager.
    pub value: Option<String>,
}

impl GooseConfiguration {
    // @TODO: move Manager configuration here.
    pub(crate) fn configure_manager(&mut self, defaults: &GooseDefaults) {
        //
    }

    // @TODO: it should be possible for the controller to reconfigure / make changes before load test starts.
    // @TODO: This needs to be its own thread, allowing the controller to end it.
    pub(crate) async fn setup_manager(
        &mut self,
        defaults: &GooseDefaults,
    ) -> Result<(ManagerJoinHandle, ManagerTx), GooseError> {
        //) -> Result<(GooseLoggerJoinHandle, GooseLoggerTx), GooseError> {
        // There's no setup necessary if Manager mode is not enabled.
        if !self.manager {
            return Ok((None, None));
        }

        // Update the manager configuration, loading defaults if necessasry.
        self.configure_manager(defaults);

        // Create an unbounded channel to allow the controller to manage the manager thread.
        let (manager_tx, manager_rx): (
            flume::Sender<Option<ManagerMessage>>,
            flume::Receiver<Option<ManagerMessage>>,
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
        receiver: flume::Receiver<Option<ManagerMessage>>,
    ) -> Result<(), GooseError> {
        loop {
            debug!("top of manager loop...");
            sleep(Duration::from_millis(250)).await;
        }

        Ok(())
    }
}
