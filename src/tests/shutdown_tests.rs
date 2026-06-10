//! Production shutdown-path tests (DOLL-455).
//!
//! The previous version of this file drove `MockAudioProcessor` bookkeeping:
//! the mock defined `is_recording()` as `audio_processed && !finalized` and the
//! tests asserted those very fields after calling the mock methods that set
//! them — circular, with zero production code under test (its error-path case
//! also duplicated `test_recorder_finalize_error_propagation`). These tests
//! instead spawn the real `writer_thread_main` and exercise the real shutdown
//! sequence: the `Shutdown` command, drain, `finalize_all`, the reply channel
//! Swift waits on, and thread join. (Drain-of-pending-samples itself is
//! covered by `ring_buffer_tests::test_writer_thread_shutdown_drains`.)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, OutputMode, RING_BUFFER_SECONDS};
use crate::test_utils::test_env_no_silence;
use crate::writer_thread::{WriterCommand, WriterThreadState, writer_thread_main};

/// Collect all finalized `.wav` files (not `.recording.wav` temps) in a directory.
fn wav_files_in(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "wav")
                && !p.to_str().unwrap_or_default().contains(".recording.wav")
        })
        .collect()
}

/// Build a single-channel 44.1 kHz writer state writing into `dir`.
fn writer_state(dir: &str, gate_enabled: bool) -> WriterThreadState {
    let mut state = WriterThreadState::new(
        dir,
        44_100,
        &[0],
        OutputMode::Single,
        0.0, // silence_threshold: 0 → no silence worker
        Arc::new(AtomicU64::new(0)),
        0, // min_disk_space_mb: disabled
        Arc::new(AtomicBool::new(false)),
        16,
        Arc::new(vec![CacheAlignedPeak::new(0)]),
        gate_enabled,
        1,
    )
    .expect("construct writer state");
    state.total_device_channels = 1;
    state
}

/// Spawn `writer_thread_main` exactly as `process_audio_impl` does, returning
/// the producer, the command sender, and the join handle.
fn spawn_writer(
    state: WriterThreadState,
) -> (
    rtrb::Producer<f32>,
    std::sync::mpsc::SyncSender<WriterCommand>,
    std::thread::JoinHandle<()>,
) {
    let ring_size = 44_100 * RING_BUFFER_SECONDS;
    let (producer, consumer) = rtrb::RingBuffer::new(ring_size);
    let rotation_needed = Arc::new(AtomicBool::new(false));
    let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);
    let handle = std::thread::spawn(move || {
        writer_thread_main(consumer, rotation_needed, command_rx, state);
    });
    (producer, command_tx, handle)
}

/// Send `Shutdown` and return the writer thread's finalize result after
/// joining it — the exact rendezvous the FFI stop path performs.
fn shutdown(
    command_tx: &std::sync::mpsc::SyncSender<WriterCommand>,
    handle: std::thread::JoinHandle<()>,
) -> Result<(), crate::error::BlackboxError> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    command_tx
        .send(WriterCommand::Shutdown(reply_tx))
        .expect("writer thread should still be alive to receive Shutdown");
    let result = reply_rx.recv().expect("writer thread must send a reply");
    handle
        .join()
        .expect("writer thread must exit after Shutdown");
    result
}

/// Stop immediately after start (no audio ever pushed): the shutdown sequence
/// must reply Ok, finalize a valid zero-sample WAV under its final name, and
/// leave no `.recording.wav` temp behind.
#[test]
fn shutdown_with_no_samples_finalizes_valid_empty_wav() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let (_producer, command_tx, handle) = spawn_writer(writer_state(dir, false));

        shutdown(&command_tx, handle).expect("clean shutdown must reply Ok");

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1, "exactly one finalized file");
        let reader = hound::WavReader::open(&files[0])
            .expect("finalized file must be a valid WAV even with zero samples");
        assert_eq!(reader.len(), 0, "no samples were pushed");

        let temps: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.path()
                    .to_str()
                    .unwrap_or_default()
                    .contains(".recording.wav")
            })
            .collect();
        assert!(temps.is_empty(), "no .recording.wav temp may remain");
    });
}

/// With the silence gate enabled and only silence flowing, the gate stays
/// idle (no writers ever open) and shutdown must reply Ok leaving NO files —
/// not even an empty finalized WAV.
#[test]
fn shutdown_while_gate_idle_leaves_no_files() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let (mut producer, command_tx, handle) = spawn_writer(writer_state(dir, true));

        // Pure silence: peaks never exceed the threshold, gate stays Idle.
        let silence = vec![0.0_f32; 4_410];
        if let Ok(chunk) = producer.write_chunk_uninit(silence.len()) {
            chunk.fill_from_iter(silence.iter().copied());
        }

        shutdown(&command_tx, handle).expect("gate-idle shutdown must reply Ok");

        assert!(
            wav_files_in(temp_dir.path()).is_empty(),
            "gate never opened, so no files may exist"
        );
    });
}

/// When finalize fails during shutdown (here: the final rename target's
/// parent directory does not exist), the error must travel back through the
/// Shutdown reply channel — this is what the FFI stop path surfaces to Swift.
/// The thread must still exit and join.
#[test]
fn shutdown_reply_surfaces_finalize_error() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = writer_state(dir, false);
        // Sabotage the rename destination (ENOENT) so finalize_all errors.
        state.pending_files[0].1 = format!("{dir}/does_not_exist/out.wav");

        let (mut producer, command_tx, handle) = spawn_writer(state);

        let data = vec![0.25_f32; 1_000];
        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
        }

        let result = shutdown(&command_tx, handle);
        assert!(
            result.is_err(),
            "finalize failure must surface through the Shutdown reply, got {result:?}"
        );
    });
}
