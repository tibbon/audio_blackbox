use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, OutputMode, RING_BUFFER_SECONDS};
use crate::test_utils::{generate_silent_interleaved_f32, generate_uniform_interleaved_f32};
use crate::test_utils::default_test_env;
use crate::writer_thread::{
    WriterCommand, WriterThreadState, check_and_delete_silent_files, writer_thread_main,
};

/// Collect all `.wav` files (not `.recording.wav`) in a directory.
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

fn read_wav(path: &std::path::Path) -> (hound::WavSpec, Vec<i32>) {
    let reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    let samples: Vec<i32> = reader.into_samples::<i32>().map(|s| s.unwrap()).collect();
    (spec, samples)
}

// Test helper consolidated to `crate::test_utils` (DOLL-118).
use crate::test_utils::test_env_no_silence;

// ===========================================================================
// Ring buffer overflow test
// ===========================================================================

#[test]
fn test_ring_buffer_overflow_counted() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));

        // Create a very small ring buffer (16 samples)
        let (mut producer, consumer) = rtrb::RingBuffer::new(16);

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Fill the ring buffer past capacity by calling the SAME helper the
        // production cpal callback uses — anything else would test the test,
        // not the production overflow-counting contract.
        let overflow_data = vec![0.5_f32; 116]; // 16-slot buffer + 100 overflow
        crate::cpal_processor::push_samples_with_overflow_count(
            &mut producer,
            &overflow_data,
            &write_errors,
        );

        // Shutdown the writer thread
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();

        // The production write_errors counter must reflect the rejected
        // suffix. If the increment branch in push_samples_with_overflow_count
        // were deleted, this would fail.
        let count = write_errors.load(Ordering::Relaxed);
        assert!(count >= 100, "expected >=100 rejected samples, got {count}");
    });
}

// ===========================================================================
// Writer thread processes all samples
// ===========================================================================

#[test]
fn test_writer_thread_processes_all_samples() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));

        let ring_size = 44100 * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Push 500 samples
        let data = generate_uniform_interleaved_f32(1, 500, &[0], 0.5);
        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
        }

        // Shutdown drains the ring buffer before returning the reply, so
        // the previous 50ms sleep was redundant — drop it (DOLL-96).
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1, "Expected exactly one WAV file");

        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 1);
        assert_eq!(samples.len(), 500, "All 500 samples should be written");
    });
}

// ===========================================================================
// Writer thread rotation
// ===========================================================================

#[test]
fn test_writer_thread_rotation() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));

        let ring_size = 44100 * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        // Inject a deterministic clock so the rotation produces a distinct
        // filename without sleeping past a wall-clock second.
        let clock = crate::test_utils::MockClock::new();
        state.set_timestamp_fn(clock.as_timestamp_fn());

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let samples_counter = Arc::clone(&state.samples_consumed_total);
        let rotation_clone = Arc::clone(&rotation_needed);
        let rotation_signal = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Push first batch of samples
        let data1 = generate_uniform_interleaved_f32(1, 200, &[0], 0.3);
        if let Ok(chunk) = producer.write_chunk_uninit(data1.len()) {
            chunk.fill_from_iter(data1.iter().copied());
        }

        // Rendezvous on writer-has-drained-batch-1 (DOLL-127): replaces a
        // 50ms sleep that hoped the writer had caught up.
        crate::test_utils::wait_for_samples_consumed(
            &samples_counter,
            200,
            std::time::Duration::from_secs(2),
        );

        // Advance the mock clock and signal rotation. The newly created file
        // gets a different stamp than the closed one.
        clock.advance();
        rotation_signal.store(true, Ordering::Release);

        // Rendezvous on writer-has-acknowledged-rotation: the writer clears
        // the flag back to false after finalizing the previous file.
        crate::test_utils::wait_for_flag_cleared(
            &rotation_signal,
            std::time::Duration::from_secs(2),
        );

        // Push second batch
        let data2 = generate_uniform_interleaved_f32(1, 200, &[0], 0.6);
        if let Ok(chunk) = producer.write_chunk_uninit(data2.len()) {
            chunk.fill_from_iter(data2.iter().copied());
        }

        // Shutdown drains the ring buffer before returning the reply, so
        // no rendezvous on batch 2 is needed.
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(
            files.len(),
            2,
            "Expected 2 WAV files after rotation, found: {:?}",
            files
        );
    });
}

// ===========================================================================
// Shutdown drains all remaining samples
// ===========================================================================

#[test]
fn test_writer_thread_shutdown_drains() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));

        let ring_size = 44100 * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Push samples and immediately shutdown (without sleeping)
        let data = generate_uniform_interleaved_f32(1, 300, &[0], 0.4);
        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
        }

        // Immediately send shutdown — samples should still be drained
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1);

        let (_, samples) = read_wav(&files[0]);
        assert_eq!(
            samples.len(),
            300,
            "All samples should be drained on shutdown"
        );
    });
}

// ===========================================================================
// Silence detection on rotation
// ===========================================================================

#[test]
fn test_writer_thread_silence_on_rotation() {
    // Use a high silence threshold so the file is considered silent
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("10")));

    temp_env::with_vars(env, || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));

        let ring_size = 44100 * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            10.0, // high threshold — everything is "silent"
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let samples_counter = Arc::clone(&state.samples_consumed_total);
        let rotation_clone = Arc::clone(&rotation_needed);
        let rotation_signal = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Push silent data
        let data = generate_silent_interleaved_f32(1, 500);
        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
        }

        // Rendezvous on writer-drained-batch-1 (DOLL-127).
        crate::test_utils::wait_for_samples_consumed(
            &samples_counter,
            500,
            std::time::Duration::from_secs(2),
        );

        // Signal rotation — this triggers silence check on the rotated file
        rotation_signal.store(true, Ordering::Release);

        // Rendezvous on writer-acknowledged-rotation.
        crate::test_utils::wait_for_flag_cleared(
            &rotation_signal,
            std::time::Duration::from_secs(2),
        );

        // The first file (silent) should have been deleted during rotation
        // Push some more data for the new file and shutdown
        let data2 = generate_silent_interleaved_f32(1, 100);
        if let Ok(chunk) = producer.write_chunk_uninit(data2.len()) {
            chunk.fill_from_iter(data2.iter().copied());
        }

        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        // After handle.join(): the writer thread has returned, the state has
        // been dropped, the silence-check worker has been joined, and any
        // submitted batches have completed. No sleep needed (DOLL-97 fix).
        handle.join().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert!(
            files.is_empty(),
            "Silent files should have been deleted, found: {:?}",
            files
        );
    });
}

// ===========================================================================
// Direct unit test for check_and_delete_silent_files
// ===========================================================================

#[test]
fn test_check_and_delete_silent_files_deletes_silent() {
    temp_env::with_vars(default_test_env(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path();

        // Create a silent WAV file
        let silent_path = dir.join("silent.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&silent_path, spec).unwrap();
        for _ in 0..1000 {
            writer.write_sample(0_i16).unwrap();
        }
        writer.finalize().unwrap();
        assert!(silent_path.exists());

        // Create a non-silent WAV file
        let loud_path = dir.join("loud.wav");
        let mut writer = hound::WavWriter::create(&loud_path, spec).unwrap();
        for i in 0..1000 {
            let sample = ((i as f32 / 10.0).sin() * 16000.0) as i16;
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        assert!(loud_path.exists());

        let files = vec![
            silent_path.to_str().unwrap().to_string(),
            loud_path.to_str().unwrap().to_string(),
        ];

        // threshold must be > 0 (0 disables silence detection in is_silent)
        check_and_delete_silent_files(&files, 0.01);

        // Silent file should be deleted, loud file should remain
        assert!(!silent_path.exists(), "Silent file should be deleted");
        assert!(loud_path.exists(), "Non-silent file should be kept");
    });
}

#[test]
fn test_check_and_delete_silent_files_skips_missing() {
    temp_env::with_vars(default_test_env(), || {
        // Passing a nonexistent file path should not panic
        let files = vec!["/tmp/nonexistent_test_file_12345.wav".to_string()];
        check_and_delete_silent_files(&files, 0.01);
        // Should complete without panic — error is just logged
    });
}

// ===========================================================================
// Background silence thread doesn't block writer processing
// ===========================================================================

#[test]
fn test_rotation_silence_thread_does_not_block_writer() {
    // Verify that after rotation, the writer thread continues accepting
    // new samples immediately (the silence check happens in background).
    //
    // DOLL-127 round-3 tightening: the original test used a huge ring
    // (44100 * RING_BUFFER_SECONDS) and a tiny push (700 samples), so a
    // synchronous silence check that held the writer for seconds would
    // still pass `write_errors == 0`. The current shape uses a tiny ring
    // and a flooding producer — and asserts the writer's
    // `samples_consumed_total` advanced during the post-rotation window.
    // If `rotate_files` was reverted to perform the silence check on the
    // writer thread, the counter would stall and this test would fail.
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("10")));

    temp_env::with_vars(env, || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));

        // Small ring (~5.8ms of audio at 44100 Hz) — under flooding any
        // writer pause surfaces immediately as the producer can't keep
        // pushing without waiting for drain.
        let ring_size = 256;
        let (producer, consumer) = rtrb::RingBuffer::new(ring_size);
        let producer = Arc::new(std::sync::Mutex::new(producer));

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            10.0, // high threshold — everything is "silent"
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        // Inject a deterministic clock so rotation produces a distinct
        // filename without needing a wall-clock second to elapse.
        let clock = crate::test_utils::MockClock::new();
        state.set_timestamp_fn(clock.as_timestamp_fn());

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let samples_counter = Arc::clone(&state.samples_consumed_total);
        let rotation_clone = Arc::clone(&rotation_needed);
        let rotation_signal = Arc::clone(&rotation_needed);
        let writer_handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Producer thread: floods 64-sample chunks into the tiny ring.
        // The writer is doing real WAV writes so the producer outpaces
        // it slightly — modest baseline overflow is expected. The signal
        // we care about is the writer's `samples_consumed_total` advance,
        // not the absolute overflow count.
        let producer_should_stop = Arc::new(AtomicBool::new(false));
        let producer_should_stop_c = Arc::clone(&producer_should_stop);
        let producer_we = Arc::clone(&write_errors);
        let producer_arc = Arc::clone(&producer);
        let producer_handle = std::thread::spawn(move || {
            let chunk = generate_uniform_interleaved_f32(1, 64, &[0], 0.3);
            while !producer_should_stop_c.load(Ordering::Relaxed) {
                if let Ok(mut p) = producer_arc.lock() {
                    crate::cpal_processor::push_samples_with_overflow_count(
                        &mut p, &chunk, &producer_we,
                    );
                }
                std::thread::yield_now();
            }
        });

        // Let the system reach a steady state under flooding.
        std::thread::sleep(std::time::Duration::from_millis(30));
        let pre_rotation_consumed = samples_counter.load(Ordering::Relaxed);

        // Signal rotation while the producer is still flooding.
        clock.advance();
        rotation_signal.store(true, Ordering::Release);

        // Writer should acknowledge rotation almost immediately — if it
        // were blocked on a synchronous silence check this would time
        // out (300ms is comfortably above any transient writer activity
        // on a healthy machine, but well below the seconds a synchronous
        // silence check on a real recording would take).
        crate::test_utils::wait_for_flag_cleared(
            &rotation_signal,
            std::time::Duration::from_millis(300),
        );

        // Measure forward progress during the post-rotation window.
        std::thread::sleep(std::time::Duration::from_millis(30));
        let post_rotation_consumed = samples_counter.load(Ordering::Relaxed);

        producer_should_stop.store(true, Ordering::Relaxed);
        producer_handle.join().expect("producer thread panicked");

        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        writer_handle.join().unwrap();

        // Killer-question: if `rotate_files` was reverted to run the
        // silence check synchronously on the writer thread, the writer
        // would stop draining the ring during that window and
        // `samples_consumed_total` would not advance. Asserting it
        // advanced by a non-trivial amount catches that regression.
        let consumed_during_rotation = post_rotation_consumed - pre_rotation_consumed;
        assert!(
            consumed_during_rotation > 0,
            "writer made no progress during the post-rotation window — \
             silence check may be blocking the writer thread \
             (consumed_during_rotation={consumed_during_rotation})"
        );
    });
}

// ===========================================================================
// Disk space check at startup
// ===========================================================================

#[test]
fn test_new_fails_when_disk_space_low() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let disk_space_low = Arc::new(AtomicBool::new(false));

        // Set threshold absurdly high (999 TB) — guaranteed to exceed available space
        let result = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            999_000_000, // 999 TB in MB
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        );

        let err = result
            .err()
            .expect("Should fail when disk space is below threshold");
        assert!(
            disk_space_low.load(Ordering::Relaxed),
            "disk_space_low flag should be set"
        );

        let err_msg = err.to_string();
        assert!(
            err_msg.contains("Insufficient disk space"),
            "Error should mention disk space: {err_msg}",
        );
    });
}

#[test]
fn test_new_succeeds_when_disk_check_disabled() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let disk_space_low = Arc::new(AtomicBool::new(false));

        // min_disk_space_mb = 0 disables the check
        let result = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        );

        assert!(result.is_ok(), "Should succeed when disk check is disabled");
        assert!(
            !disk_space_low.load(Ordering::Relaxed),
            "disk_space_low should not be set"
        );
    });
}

// ===========================================================================
// Disk space check behavior
// ===========================================================================

#[test]
fn test_writer_thread_disk_space_check_sets_flag() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let disk_space_low = Arc::new(AtomicBool::new(false));

        // Create state with absurdly high threshold — will trigger on first check
        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0, // Disable in constructor — we'll set it manually after creation
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        // Override disk threshold to something absurd, and set the counter
        // so check_disk_space() actually runs on the next call.
        state.min_disk_space_mb = 999_000_000;
        state.disk_check_counter = 10_000;

        let can_write = state.check_disk_space();
        assert!(
            !can_write,
            "check_disk_space should return false when disk is low"
        );
        assert!(
            disk_space_low.load(Ordering::Relaxed),
            "disk_space_low flag should be set"
        );
    });
}

#[test]
fn test_disk_stopped_skips_writes() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let disk_space_low = Arc::new(AtomicBool::new(false));

        let mut state = WriterThreadState::new(
            dir,
            44100,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = 1;

        // List all .wav files including the in-progress `.recording.wav`
        // tmp files (which `wav_files_in` filters out).
        let all_wav = |dir: &std::path::Path| -> Vec<std::path::PathBuf> {
            std::fs::read_dir(dir)
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.to_str().unwrap_or_default().contains(".wav"))
                .collect()
        };

        // Positive control: write enough samples to be observable after
        // disk-stop renames the .recording.wav → .wav. The check_disk_space
        // call below performs a synchronous finalize_all on the pending
        // file, which flushes any buffered bytes to disk.
        let data = vec![0.5_f32; 5000];
        state.write_samples(&data);

        // Trigger disk_stopped — this finalizes the pending file (rename
        // .recording.wav → .wav, flushing buffered bytes) and flips
        // disk_stopped = true.
        state.min_disk_space_mb = 999_000_000;
        state.disk_check_counter = 10_000;
        state.check_disk_space();
        assert!(state.disk_stopped, "disk_stopped should be set after check");

        let post_stop_files = all_wav(temp_dir.path());
        assert_eq!(
            post_stop_files.len(),
            1,
            "expected one finalized WAV after disk-stop, got {post_stop_files:?}"
        );
        let healthy_size = std::fs::metadata(&post_stop_files[0])
            .expect("file metadata")
            .len();
        assert!(
            healthy_size > 0,
            "expected non-empty WAV after disk-stop finalize, got {healthy_size} bytes — \
             positive control failed; means bytes never reached disk"
        );

        // Now write_samples should be a no-op. The killer-question check:
        // if we removed the disk_stopped guard from write_samples, those
        // 200 samples would change the byte total below.
        let pre_skip_total: u64 = all_wav(temp_dir.path())
            .iter()
            .map(|p| std::fs::metadata(p).map_or(0, |m| m.len()))
            .sum();

        let more_data = vec![0.5_f32; 200];
        state.write_samples(&more_data);

        let post_skip_total: u64 = all_wav(temp_dir.path())
            .iter()
            .map(|p| std::fs::metadata(p).map_or(0, |m| m.len()))
            .sum();

        assert_eq!(
            pre_skip_total, post_skip_total,
            "write_samples after disk_stopped must not change on-disk byte total"
        );
        assert_eq!(write_errors.load(Ordering::Relaxed), 0);
    });
}
