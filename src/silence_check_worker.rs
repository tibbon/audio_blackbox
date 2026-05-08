//! Single dedicated silence-check thread fed via a bounded channel.
//!
//! Extracted from `writer_thread.rs` in DOLL-147. Replaces the prior
//! fire-and-forget pattern of spawning a fresh thread per rotation.
//! Benefits: bounded thread count regardless of rotation churn,
//! shutdown joins the worker (no detached thread races with
//! file-system teardown), and the worker can be unit-tested in
//! isolation (see DOLL-130).

use log::error;

use crate::writer_thread::check_and_delete_silent_files;

pub struct SilenceCheckWorker {
    /// Channel sender. Wrapped in `Option` so `Drop` can take + drop it
    /// before joining, which closes the channel and lets the worker exit
    /// its `recv()` loop cleanly.
    tx: Option<std::sync::mpsc::SyncSender<Vec<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SilenceCheckWorker {
    /// Construct a new worker. Returns `None` if the underlying thread
    /// spawn fails (resource exhaustion: EAGAIN/ENOMEM/RLIMIT_NPROC).
    /// Callers store `silence_worker: None` and the writer thread keeps
    /// running — silent files just don't get auto-deleted that session
    /// (DOLL-122).
    pub fn new(threshold: f32) -> Option<Self> {
        // Bounded channel: 8 batches in flight is generous given that
        // rotation cadence is per-second at the fastest. Backpressure on
        // the rotation path (a brief block on `send`) is preferable to
        // unbounded memory growth.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<String>>(8);
        let handle = match std::thread::Builder::new()
            .name("blackbox-silence".to_string())
            .spawn(move || {
                #[cfg(target_os = "macos")]
                // SAFETY: macOS-only libc QoS call. No pointer args;
                // sets the calling thread's QoS class so the silence-
                // check work runs at lower priority than the audio
                // writer thread.
                unsafe {
                    libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
                }
                while let Ok(files) = rx.recv() {
                    check_and_delete_silent_files(&files, threshold);
                }
            }) {
            Ok(h) => h,
            Err(e) => {
                error!(
                    "Failed to spawn silence-check worker thread ({e}); \
                     silence detection will be disabled this session."
                );
                return None;
            }
        };

        Some(SilenceCheckWorker {
            tx: Some(tx),
            handle: Some(handle),
        })
    }

    /// Submit a batch of file paths for silence checking. Best-effort: if
    /// the channel is closed (worker died), the batch is silently dropped
    /// — matches the prior `spawn(...).ok()` behavior.
    pub fn submit(&self, files: Vec<String>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(files);
        }
    }
}

impl Drop for SilenceCheckWorker {
    fn drop(&mut self) {
        // Drop the sender first to close the channel; the worker's
        // `recv()` returns Err and the loop exits.
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SilenceCheckWorker;
    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::tempdir;

    /// Drop must wait for the in-flight batch to be processed before
    /// returning. If `Drop` was reverted to skip `h.join()`, the worker
    /// thread would race the test thread and the silent file would
    /// (sometimes, depending on scheduling) still exist at the assertion
    /// (DOLL-130).
    #[test]
    fn silence_check_worker_drop_joins_pending_batch() {
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

        // The Drop impl closes the channel and joins the worker, so any
        // batch already submitted MUST be drained before drop returns.
        drop(worker);

        assert!(
            !path.exists(),
            "Drop did not wait for the silence worker to process the in-flight batch \
             — the silent file should have been deleted"
        );
    }
}
