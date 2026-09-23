//! Single dedicated silence-check thread fed via a bounded channel.
//!
//! Extracted from `writer_thread.rs` in DOLL-147. Replaces the prior
//! fire-and-forget pattern of spawning a fresh thread per rotation.
//! Benefits: one thread per recording session regardless of rotation
//! churn, and the worker can be unit-tested in isolation (see DOLL-130).
//!
//! Dropping the worker closes its channel but does not join the thread:
//! the thread scans whatever is still queued and then exits on its own.
//! Scanning a long silent file decodes it to the end, which can take
//! minutes, and the drop happens while stopping a recording — on the
//! Swift main thread via `blackbox_stop_recording`. Stop must not wait on
//! that. If the process exits first, the unscanned files are simply kept:
//! the check only ever deletes, so cutting it short never loses audio.
//! [`wait_for_silence_checks`] lets a caller that can afford to block
//! (the CLI on exit, tests) wait for the queue to empty.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use log::error;

use crate::writer_thread::check_and_delete_silent_files;

/// Count of batches submitted to any silence-check worker and not yet
/// scanned, process-wide, with a condvar signalled when it drops.
struct PendingBatches {
    count: Mutex<usize>,
    changed: Condvar,
}

impl PendingBatches {
    const fn new() -> Self {
        Self {
            count: Mutex::new(0),
            changed: Condvar::new(),
        }
    }

    fn add(&self) {
        *self.count.lock().unwrap_or_else(PoisonError::into_inner) += 1;
    }

    fn finish(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_sub(1);
        drop(count);
        self.changed.notify_all();
    }

    fn wait_idle(&self, timeout: Duration) -> bool {
        let (count, _) = self
            .changed
            .wait_timeout_while(
                self.count.lock().unwrap_or_else(PoisonError::into_inner),
                timeout,
                |n| *n > 0,
            )
            .unwrap_or_else(PoisonError::into_inner);
        *count == 0
    }
}

static PENDING: PendingBatches = PendingBatches::new();

/// Block until every file submitted for a silence check so far has been
/// checked (and deleted if silent), or `timeout` elapses.
///
/// Returns `true` if the queue emptied. Stopping a recording no longer waits
/// for these checks, so a process that wants silent files gone before it
/// exits calls this after stopping. Never call it from a UI thread.
pub fn wait_for_silence_checks(timeout: Duration) -> bool {
    PENDING.wait_idle(timeout)
}

pub(crate) struct SilenceCheckWorker {
    /// Channel sender. Wrapped in `Option` so `Drop` can take + drop it,
    /// which closes the channel and lets the worker exit its `recv()` loop
    /// once the queue is empty.
    tx: Option<std::sync::mpsc::SyncSender<Vec<String>>>,
}

impl SilenceCheckWorker {
    /// Construct a new worker. Returns `None` if the underlying thread
    /// spawn fails (resource exhaustion: `EAGAIN/ENOMEM/RLIMIT_NPROC`).
    /// Callers store `silence_worker: None` and the writer thread keeps
    /// running — silent files just don't get auto-deleted that session
    /// (DOLL-122).
    pub(crate) fn new(threshold: f32) -> Option<Self> {
        // Bounded channel: 8 batches in flight is generous given that
        // rotation cadence is per-second at the fastest. Backpressure on
        // the rotation path (a brief block on `send`) is preferable to
        // unbounded memory growth.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<String>>(8);
        let spawned = std::thread::Builder::new()
            .name("blackbox-silence".to_owned())
            .spawn(move || {
                #[cfg(target_os = "macos")]
                // SAFETY: macOS-only libc QoS call. No pointer args;
                // sets the calling thread's QoS class so the silence-
                // check work runs at lower priority than the audio
                // writer thread.
                unsafe {
                    libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
                }
                // Ends when the owning worker is dropped (channel closed)
                // and the queue is drained.
                while let Ok(files) = rx.recv() {
                    check_and_delete_silent_files(&files, threshold);
                    PENDING.finish();
                }
            });
        match spawned {
            // Detached on purpose: closing the channel is the shutdown path,
            // and nothing waits on the thread (see the module doc).
            Ok(handle) => drop(handle),
            Err(e) => {
                error!(
                    "Failed to spawn silence-check worker thread ({e}); \
                     silence detection will be disabled this session."
                );
                return None;
            }
        }

        Some(Self { tx: Some(tx) })
    }

    /// Submit a batch of file paths for silence checking, blocking while the
    /// queue is full. Best-effort: if the channel is closed (worker died),
    /// the batch is dropped and its files are kept.
    pub(crate) fn submit(&self, files: Vec<String>) {
        if let Some(tx) = &self.tx {
            let batch = files.len();
            PENDING.add();
            if tx.send(files).is_err() {
                PENDING.finish();
                log::debug!("silence-check worker already stopped; dropping {batch} files");
            }
        }
    }

    /// Like [`submit`](Self::submit) but never blocks: with the queue full,
    /// the files are kept unchecked. Used when closing files on the way to a
    /// stop, which must not wait for scans already queued.
    pub(crate) fn try_submit(&self, files: Vec<String>) {
        if let Some(tx) = &self.tx {
            PENDING.add();
            match tx.try_send(files) {
                Ok(()) => {}
                Err(std::sync::mpsc::TrySendError::Full(files)) => {
                    PENDING.finish();
                    log::warn!(
                        "Silence-check queue is full; keeping {} file(s) without a check",
                        files.len()
                    );
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(files)) => {
                    PENDING.finish();
                    log::debug!(
                        "silence-check worker already stopped; dropping {} files",
                        files.len()
                    );
                }
            }
        }
    }
}

impl Drop for SilenceCheckWorker {
    fn drop(&mut self) {
        // Close the channel. The thread finishes the queued batches and then
        // its `recv()` returns Err and it exits; nothing joins it.
        self.tx.take();
    }
}

#[cfg(test)]
mod tests {
    use super::{SilenceCheckWorker, wait_for_silence_checks};
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::time::Duration;
    use tempfile::tempdir;

    /// A batch submitted before the drop is still processed afterwards: the
    /// thread drains its queue before exiting (DOLL-130).
    #[test]
    fn silence_check_worker_processes_batch_after_drop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("silent.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(&path, spec).unwrap();
        for _ in 0..1000 {
            w.write_sample(0_i16).unwrap();
        }
        w.finalize().unwrap();
        assert!(path.exists(), "silent test file should exist before submit");

        let worker = SilenceCheckWorker::new(0.01).expect("worker thread should spawn");
        worker.submit(vec![path.to_string_lossy().into_owned()]);
        drop(worker);

        assert!(
            wait_for_silence_checks(Duration::from_secs(10)),
            "the queued batch was never processed"
        );
        assert!(
            !path.exists(),
            "the silent file submitted before the drop should have been deleted"
        );
    }

    /// Dropping the worker must not wait for a scan in progress: that drop
    /// runs while stopping a recording, on the app's main thread. The scan
    /// here is stuck opening a FIFO that has no writer yet; with the old
    /// join-on-drop, the drop would block until the FIFO is opened.
    #[test]
    fn drop_does_not_wait_for_a_scan_in_progress() {
        let dir = tempdir().unwrap();
        let fifo = dir.path().join("stuck.wav");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed");

        let worker = SilenceCheckWorker::new(0.01).expect("worker thread should spawn");
        worker.submit(vec![fifo.to_string_lossy().into_owned()]);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(worker);
            done_tx.send(()).unwrap();
        });
        let returned = done_rx.recv_timeout(Duration::from_secs(5)).is_ok();

        // Unblock the scan either way: opening the write end lets the
        // worker's open return, and closing it gives it EOF (not a WAV).
        drop(std::fs::OpenOptions::new().write(true).open(&fifo).unwrap());
        dropper.join().unwrap();
        assert!(returned, "dropping the worker waited for the scan");
        assert!(wait_for_silence_checks(Duration::from_secs(10)));
    }
}
