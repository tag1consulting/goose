//! What a worker, or the manager's disk, does wrong on purpose in Goose's
//! own unit tests, and how: compiled into unit tests only, and never set
//! from outside them. Each place a run takes a fault checks it in one short
//! line that calls into here.

use super::link::{Link, LinkEvent, Outbox, LOG_CHUNK};
use super::spec::WorkerRunSpec;
use crate::metrics::MetricsCommand;
use crate::GooseError;
use std::sync::Arc;
use std::time::Duration;

/// What a worker does wrong on purpose, in unit tests.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TestFaults {
    /// Ignore the manager's metrics reset.
    pub(crate) skip_reset: bool,
    /// Send a first delta the manager refuses.
    pub(crate) refused_delta: bool,
    /// Send only deltas the manager refuses from this sequence number on,
    /// 0 for none.
    pub(crate) refuse_from: u64,
    /// The metrics processor never sends the final metrics.
    pub(crate) silent_final: bool,
    /// The metrics processor never answers a reset.
    pub(crate) silent_reset: bool,
    /// What the worker does with its `Prepare`.
    pub(crate) prepare: PrepareFault,
    /// What the worker sends when asked for its logs.
    pub(crate) logs: LogsFault,
    /// End with an error a second into the run, its link dropped.
    pub(crate) exit_mid_run: bool,
    /// Send a final delta the manager refuses, and stop as told.
    pub(crate) refused_final: bool,
    /// Trigger the killswitch, as Ctrl-C on the manager, as `Stop` arrives,
    /// at [`LOG_FAULT_AT`].
    pub(crate) cancel_on_stop: bool,
    /// Never say its users stopped, nor send its final metrics: wait to be
    /// shut down. With `cancel_on_stop`, the killswitch is triggered a
    /// second after `Stop` arrives.
    pub(crate) withhold_final: bool,
    /// A second into the run, say it stopped for this reason, as sent, and
    /// run on.
    pub(crate) stop_reason: Option<i32>,
    /// The metrics processor gets the final metrics command this late.
    pub(crate) final_after: Option<Duration>,
    /// Make the delta of this sequence number larger than a message may
    /// be, 0 for none.
    pub(crate) oversize_delta: u64,
    /// Make the final delta larger than a message may be.
    pub(crate) oversize_final: bool,
}

impl TestFaults {
    /// What the worker makes of the run spec it decoded, `decoded`, as
    /// [`Self::prepare`] says: what it answers, or the error it ends with.
    pub(crate) async fn on_prepare(
        &self,
        link: &Link,
        decoded: Result<WorkerRunSpec, String>,
    ) -> Result<Result<WorkerRunSpec, String>, GooseError> {
        let failed = |detail: &str| GooseError::Gaggle {
            detail: detail.to_string(),
        };
        match self.prepare {
            PrepareFault::None => Ok(decoded),
            PrepareFault::Refuse => Ok(Err("refused on purpose".to_string())),
            PrepareFault::Vanish => Err(failed("went away on purpose")),
            PrepareFault::Stall => loop {
                if let Some(LinkEvent::Shutdown | LinkEvent::Lost(_)) =
                    link.next_event(Duration::from_secs(1)).await
                {
                    return Err(failed("stalled on purpose"));
                }
            },
        }
    }

    /// Have `outbox` corrupt or oversize the deltas these faults name.
    pub(crate) fn arm(&self, outbox: &Outbox) {
        use std::sync::atomic::Ordering::Relaxed;
        if self.refused_delta {
            outbox.corrupt_next.store(true, Relaxed);
        }
        outbox.corrupt_final.store(self.refused_final, Relaxed);
        outbox.corrupt_from.store(self.refuse_from, Relaxed);
        outbox.oversize_seq.store(self.oversize_delta, Relaxed);
        outbox.oversize_final.store(self.oversize_final, Relaxed);
    }

    /// Say once, through `link`, that the worker stopped for
    /// [`Self::stop_reason`], `sent` set once it has.
    pub(crate) fn stop_once(&self, link: &Link, sent: &mut bool) {
        if let (Some(reason), false) = (self.stop_reason, *sent) {
            *sent = true;
            link.stopped(super::proto::Stopped {
                reason,
                canceled: "on purpose".to_string(),
                run_folder: String::new(),
            });
        }
    }

    /// With [`Self::withhold_final`], wait to be shut down, never saying
    /// the users stopped, and with [`Self::cancel_on_stop`] trigger the
    /// killswitch once the manager took another worker's `Stopped`, which
    /// follows its final metrics: the error the worker then ends with.
    /// `None` without it.
    pub(crate) async fn withhold_final(&self, link: &Link) -> Option<GooseError> {
        if !self.withhold_final {
            return None;
        }
        loop {
            match link.next_event(Duration::from_secs(1)).await {
                Some(LinkEvent::Shutdown | LinkEvent::Lost(_)) => {
                    return Some(GooseError::Gaggle {
                        detail: "withheld its final metrics on purpose".to_string(),
                    })
                }
                Some(LinkEvent::Stop(_)) if self.cancel_on_stop => {
                    tokio::spawn(async {
                        let until = std::time::Instant::now() + Duration::from_secs(10);
                        while STOPPED_SEEN.load(std::sync::atomic::Ordering::SeqCst) == 0
                            && std::time::Instant::now() < until
                        {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        *LOG_FAULT_AT.lock().unwrap() = Some(std::time::Instant::now());
                        crate::trigger_killswitch("a test's Ctrl-C as the run stops");
                    });
                }
                _ => {}
            }
        }
    }

    /// The final metrics `command`, to send to `metrics` now, or `None`
    /// when [`Self::final_after`] sends it itself, that much later.
    pub(crate) fn late_final(
        &self,
        metrics: &flume::Sender<MetricsCommand>,
        command: MetricsCommand,
    ) -> Option<MetricsCommand> {
        let Some(delay) = self.final_after else {
            return Some(command);
        };
        let metrics = metrics.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = metrics.send(command);
        });
        None
    }
}

/// Armed by a test: the manager's processor holds its first threshold check
/// with requests to judge until the manager is stopping, or 20 seconds at
/// most, and 300 ms more, blocking the thread it runs on meanwhile.
pub(crate) static HOLD_THRESHOLD_CHECK: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set as the manager starts to stop its workers.
pub(crate) static MANAGER_STOPPING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Hold a threshold check, as [`HOLD_THRESHOLD_CHECK`] says, once there are
/// requests to judge.
pub(crate) fn hold_threshold_check(has_requests: bool) {
    use std::sync::atomic::Ordering::SeqCst;
    if !has_requests || !HOLD_THRESHOLD_CHECK.swap(false, SeqCst) {
        return;
    }
    // Off the runtime's own threads, so the manager's main loop runs on.
    tokio::task::block_in_place(|| {
        let until = std::time::Instant::now() + Duration::from_secs(20);
        while !MANAGER_STOPPING.load(SeqCst) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        HELD_TO_STOP.store(MANAGER_STOPPING.load(SeqCst), SeqCst);
        std::thread::sleep(Duration::from_millis(300));
    });
}

/// The held check was released as the manager stopped, not at its 20 second
/// cap.
pub(crate) static HELD_TO_STOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The workers' `Stopped`, which follows their final metrics, the manager
/// has taken.
pub(crate) static STOPPED_SEEN: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// What a worker sends in place of its logs, in unit tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum LogsFault {
    /// Nothing wrong: its logs.
    #[default]
    None,
    /// A full chunk of each log every 250 ms, never the last, its main loop
    /// held for 10 seconds; the killswitch is triggered after the second
    /// chunk, at [`LOG_FAULT_AT`].
    Slow,
    /// 10 bytes of each log every 500 ms, never the last; the first at
    /// [`LOG_FAULT_AT`].
    Trickle,
    /// Each log in one chunk of 20 bytes, announced as 10.
    Oversize,
}

/// The manager fails every write of a worker's log, as on a full disk (unit
/// tests).
pub(crate) static FAIL_LOG_WRITES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// With [`FAIL_LOG_WRITES`], the first failed log write fills the disk:
/// [`DISK_FULL`] is set (unit tests).
pub(crate) static DISK_FULL_AFTER_LOGS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Every report write fails, as on a full disk (unit tests).
pub(crate) static DISK_FULL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Every log a worker sent that the manager warned it did not ask for, by
/// name (unit tests).
pub(crate) static UNASKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// When a worker's [`LogsFault`] triggered the killswitch or sent its first
/// chunk (unit tests).
pub(crate) static LOG_FAULT_AT: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/// Send `files` as `fault` says, for 30 seconds, in the background.
pub(crate) fn send_faulty_logs(fault: LogsFault, outbox: Arc<Outbox>, files: Vec<String>) {
    use super::proto::LogChunk;
    let (size, every) = match fault {
        LogsFault::Slow => (LOG_CHUNK, Duration::from_millis(250)),
        _ => (10, Duration::from_millis(500)),
    };
    tokio::spawn(async move {
        if fault == LogsFault::Oversize {
            for file in files {
                outbox
                    .push_chunk(LogChunk {
                        file,
                        offset: 0,
                        data: vec![b'x'; 20],
                        last: true,
                        error: None,
                        size: Some(10),
                    })
                    .await;
            }
            return;
        }
        for n in 0..(30_000 / every.as_millis() as u64) {
            for file in &files {
                outbox
                    .push_chunk(LogChunk {
                        file: file.clone(),
                        offset: n * size as u64,
                        data: vec![b'x'; size],
                        last: false,
                        error: None,
                        // Larger than any it sends.
                        size: Some(u64::MAX),
                    })
                    .await;
            }
            let mark = match fault {
                LogsFault::Slow => n == 1,
                _ => n == 0,
            };
            if mark {
                *LOG_FAULT_AT.lock().unwrap() = Some(std::time::Instant::now());
                if fault == LogsFault::Slow {
                    crate::trigger_killswitch("a test's Ctrl-C while logs arrive");
                }
            }
            tokio::time::sleep(every).await;
        }
    });
}

/// What a worker does wrong with its `Prepare`, in unit tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum PrepareFault {
    /// Nothing: it prepares.
    #[default]
    None,
    /// Answer with an error.
    Refuse,
    /// Go away without an answer.
    Vanish,
    /// Never answer, and wait to be shut down.
    Stall,
}
