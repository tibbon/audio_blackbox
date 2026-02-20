use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use tempfile::tempdir;

use crate::constants::RING_BUFFER_SECONDS;
use crate::test_utils::{generate_silent_interleaved_f32, generate_uniform_interleaved_f32};
use crate::tests::default_test_env;
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

/// Test env with silence detection disabled (threshold=0).
fn test_env_no_silence() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));
    env
}

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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Fill the ring buffer past capacity
        // First fill it up
        let data = [0.5_f32; 16];
        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
        }

        // Now try to push more — should fail since it's full
        let overflow_data = vec![0.5_f32; 100];
        let mut overflow_count: u64 = 0;
        for &sample in &overflow_data {
            if producer.push(sample).is_err() {
                overflow_count += 1;
            }
        }

        assert!(overflow_count > 0, "Should have had ring buffer overflow");

        // Shutdown the writer thread
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();
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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
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

        // Give writer thread time to process
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Shutdown and verify
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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

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

        // Wait for writer to process, then sleep past the second boundary
        // so the rotated file gets a distinct timestamp.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // Signal rotation
        rotation_signal.store(true, Ordering::Release);

        // Wait for rotation to happen
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Push second batch
        let data2 = generate_uniform_interleaved_f32(1, 200, &[0], 0.6);
        if let Ok(chunk) = producer.write_chunk_uninit(data2.len()) {
            chunk.fill_from_iter(data2.iter().copied());
        }

        // Wait and shutdown
        std::thread::sleep(std::time::Duration::from_millis(50));

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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
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
            "single",
            10.0, // high threshold — everything is "silent"
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

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

        // Wait for writer to process
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Signal rotation — this triggers silence check on the rotated file
        rotation_signal.store(true, Ordering::Release);

        // Wait for rotation
        std::thread::sleep(std::time::Duration::from_millis(50));

        // The first file (silent) should have been deleted during rotation
        // Push some more data for the new file and shutdown
        let data2 = generate_silent_interleaved_f32(1, 100);
        if let Ok(chunk) = producer.write_chunk_uninit(data2.len()) {
            chunk.fill_from_iter(data2.iter().copied());
        }

        std::thread::sleep(std::time::Duration::from_millis(50));

        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();

        // Both files were silent — both should be deleted.
        // Extra sleep to allow background silence-check thread to finish.
        std::thread::sleep(std::time::Duration::from_millis(200));

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
            "single",
            10.0, // high threshold — everything is "silent"
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
        )
        .unwrap();
        state.total_device_channels = 1;

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let rotation_signal = Arc::clone(&rotation_needed);
        let handle = std::thread::spawn(move || {
            writer_thread_main(consumer, rotation_clone, command_rx, state);
        });

        // Push first batch
        let data1 = generate_uniform_interleaved_f32(1, 200, &[0], 0.3);
        if let Ok(chunk) = producer.write_chunk_uninit(data1.len()) {
            chunk.fill_from_iter(data1.iter().copied());
        }

        // Wait for processing, then cross second boundary for distinct timestamp
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // Signal rotation
        rotation_signal.store(true, Ordering::Release);

        // Immediately push a second batch — writer thread should accept it
        // without waiting for the silence check to finish
        std::thread::sleep(std::time::Duration::from_millis(20));
        let data2 = generate_uniform_interleaved_f32(1, 500, &[0], 0.3);
        if let Ok(chunk) = producer.write_chunk_uninit(data2.len()) {
            chunk.fill_from_iter(data2.iter().copied());
        }

        // Give time for processing
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Shutdown
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        handle.join().unwrap();

        // Allow background silence thread to complete
        std::thread::sleep(std::time::Duration::from_millis(200));

        // No write errors should have occurred — the writer thread was
        // never blocked by silence detection
        assert_eq!(
            write_errors.load(Ordering::Relaxed),
            0,
            "No write errors expected — writer thread should not be blocked by silence check"
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
            "single",
            0.0,
            Arc::clone(&write_errors),
            999_000_000, // 999 TB in MB
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0, // Disable in constructor — we'll set it manually after creation
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
        )
        .unwrap();
        state.total_device_channels = 1;

        // Override disk threshold to something absurd, and reset the timer
        // so check_disk_space() actually runs on the next call.
        state.min_disk_space_mb = 999_000_000;
        state.last_disk_check = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(20))
            .unwrap();

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
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::clone(&disk_space_low),
            16,
            Arc::new(vec![AtomicU32::new(0)]),
        )
        .unwrap();
        state.total_device_channels = 1;

        // Write some data first
        let data = vec![0.5_f32; 100];
        state.write_samples(&data);

        // Simulate disk_stopped by setting min_disk_space_mb high and triggering check
        state.min_disk_space_mb = 999_000_000;
        state.last_disk_check = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(20))
            .unwrap();
        state.check_disk_space(); // This sets disk_stopped = true and finalizes files

        // Now write_samples should be a no-op
        let more_data = vec![0.5_f32; 200];
        state.write_samples(&more_data);

        // The state should still have no errors from the skipped writes
        assert_eq!(write_errors.load(Ordering::Relaxed), 0);
    });
}
