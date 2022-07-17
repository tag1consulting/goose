/// Worker-specific code.
use std::time::Duration;
use tokio::time::sleep;

use crate::{GooseConfiguration, GooseDefaults, GooseError};

/// Optional join handle for worker thread, if enabled.
pub(crate) type WorkerJoinHandle =
    Option<tokio::task::JoinHandle<std::result::Result<(), GooseError>>>;
/// Optional unbounded sender to worker thread, if enabled.
pub(crate) type WorkerTx = Option<flume::Sender<Option<WorkerMessage>>>;

#[derive(Debug)]
pub(crate) enum WorkerCommand {}

/// This structure is used to control the Worker process.
#[derive(Debug)]
pub(crate) struct WorkerMessage {
    /// The command that is being sent to the Worker.
    pub command: WorkerCommand,
    /// An optional value that is being sent to the Worker.
    pub value: Option<String>,
}

impl GooseConfiguration {
    // @TODO: move Worker configuration here.
    pub(crate) fn configure_worker(&mut self, defaults: &GooseDefaults) {
        //
    }

    // @TODO: it should be possible for the controller to reconfigure / make changes before load test starts.
    // @TODO: This needs to be its own thread, allowing the controller to end it.
    pub(crate) async fn setup_worker(
        &mut self,
        defaults: &GooseDefaults,
    ) -> Result<(WorkerJoinHandle, WorkerTx), GooseError> {
        // Update the Worker configuration, loading defaults if necessasry.
        self.configure_manager(defaults);

        // There's no setup necessary if Worker mode is not enabled.
        if !self.worker {
            return Ok((None, None));
        }

        // Create an unbounded channel to allow the controller to manage the Worker thread.
        let (worker_tx, worker_rx): (
            flume::Sender<Option<WorkerMessage>>,
            flume::Receiver<Option<WorkerMessage>>,
        ) = flume::unbounded();

        let configuration = self.clone();
        let worker_handle = tokio::spawn(async move { configuration.worker_main(worker_rx).await });
        // @TODO: return worker_tx thread to the controller (if there is a controller)
        Ok((Some(worker_handle), Some(worker_tx)))
    }

    /// Worker thread, coordiantes with Manager instanec.
    pub(crate) async fn worker_main(
        self: GooseConfiguration,
        receiver: flume::Receiver<Option<WorkerMessage>>,
    ) -> Result<(), GooseError> {
        loop {
            debug!("top of worker loop...");
            sleep(Duration::from_millis(250)).await;
        }

        Ok(())
    }
}
