use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tempfile::tempdir;

use crate::constants::CacheAlignedPeak;
use crate::tests::default_test_env;
use crate::writer_thread::{GateState, WriterThreadState};

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

/// All files (including .recording.wav) in a directory.
fn all_wav_like_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.to_str().unwrap_or_default().contains(".wav"))
        .collect()
}

fn test_env_no_silence() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));
    env
}

fn make_gate_state(
    dir: &str,
    gate_enabled: bool,
    gate_timeout_secs: u64,
    silence_threshold: f32,
) -> WriterThreadState {
    let peak_levels = Arc::new(vec![CacheAlignedPeak::new(0)]);
    let mut state = WriterThreadState::new(
        dir,
        48000,
        &[0],
        "single",
        silence_threshold,
        Arc::new(AtomicU64::new(0)),
        0,
        Arc::new(AtomicBool::new(false)),
        16,
        peak_levels,
        gate_enabled,
        gate_timeout_secs,
    )
    .unwrap();
    state.total_device_channels = 1;
    state
}

// ===========================================================================
// Test 1: Gate idle doesn't create files when fed silence
// ===========================================================================

#[test]
fn test_gate_idle_no_files_on_silence() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.01);

        assert_eq!(state.gate_state, GateState::Idle);
        assert!(state.gate_idle.load(Ordering::Relaxed));

        // Feed 1 second of silence (48000 samples at 1 channel)
        let silence = vec![0.0_f32; 48000];
        state.write_samples(&silence);

        // Still idle, no files created
        assert_eq!(state.gate_state, GateState::Idle);
        let files = all_wav_like_files(temp_dir.path());
        assert!(
            files.is_empty(),
            "No files should be created while gate is idle, found: {:?}",
            files
        );
    });
}

// ===========================================================================
// Test 2: Gate opens writers when signal exceeds threshold
// ===========================================================================

#[test]
fn test_gate_opens_on_signal() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.01);
        assert_eq!(state.gate_state, GateState::Idle);

        // Feed audio above threshold
        let signal: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        state.write_samples(&signal);
        state.process_gate_open(); // simulate main loop processing

        // Should transition to Recording
        assert_eq!(state.gate_state, GateState::Recording);
        assert!(!state.gate_idle.load(Ordering::Relaxed));

        // Should have created a .recording.wav file
        let files = all_wav_like_files(temp_dir.path());
        assert!(!files.is_empty(), "Files should be created when gate opens");
    });
}

// ===========================================================================
// Test 3: Gate closes after timeout of silence
// ===========================================================================

#[test]
fn test_gate_closes_after_timeout() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        // 1 second timeout at 48000 Hz; silence_threshold=0.0 disables post-hoc deletion
        let mut state = make_gate_state(dir, true, 1, 0.0);
        // Use a gate-specific signal threshold (the gate checks silence_threshold for transitions)
        // Override: set a low threshold so the gate sees signal
        state.silence_threshold = 0.01;

        // Feed signal to open the gate
        let signal: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        state.write_samples(&signal);
        state.process_gate_open(); // simulate main loop processing
        assert_eq!(state.gate_state, GateState::Recording);

        // Write more signal so the file has non-trivial content
        state.write_samples(&signal);

        // Feed 2 seconds of silence (exceeds 1 second timeout)
        let silence = vec![0.0_f32; 96000];
        state.write_samples(&silence);

        // Should be back to Idle
        assert_eq!(state.gate_state, GateState::Idle);
        assert!(state.gate_idle.load(Ordering::Relaxed));

        // Files should be finalized (no .recording.wav, only .wav)
        let has_recording_files = all_wav_like_files(temp_dir.path())
            .into_iter()
            .any(|p| p.to_str().unwrap_or_default().contains(".recording.wav"));
        assert!(
            !has_recording_files,
            "All .recording.wav should be renamed after gate close"
        );

        let final_files = wav_files_in(temp_dir.path());
        assert!(!final_files.is_empty(), "Finalized .wav files should exist");
    });
}

// ===========================================================================
// Test 4: Gate reopens on new signal after close (produces 2 separate files)
// ===========================================================================

#[test]
fn test_gate_reopens_produces_separate_files() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        // 1 second timeout; disable silence deletion
        let mut state = make_gate_state(dir, true, 1, 0.0);
        state.silence_threshold = 0.01; // gate uses this for signal detection

        // First signal burst — opens the gate
        let signal: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        state.write_samples(&signal);
        state.process_gate_open(); // simulate main loop processing
        assert_eq!(state.gate_state, GateState::Recording);

        // Write more signal so the file has content
        state.write_samples(&signal);

        // Silence to close gate
        let silence = vec![0.0_f32; 96000];
        state.write_samples(&silence);
        assert_eq!(state.gate_state, GateState::Idle);

        let first_files = wav_files_in(temp_dir.path());
        let first_count = first_files.len();
        assert!(first_count > 0, "First gate cycle should produce files");

        // Need distinct timestamp for new file
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Second signal burst — reopens the gate
        state.write_samples(&signal);
        state.process_gate_open(); // simulate main loop processing
        assert_eq!(state.gate_state, GateState::Recording);

        // Write more signal
        state.write_samples(&signal);

        // Finalize to close second file
        // Temporarily disable silence detection for finalize
        let saved = state.silence_threshold;
        state.silence_threshold = 0.0;
        state.finalize_all().unwrap();
        state.silence_threshold = saved;

        let all_files = wav_files_in(temp_dir.path());
        assert!(
            all_files.len() > first_count,
            "Second signal should produce additional files: first={}, total={}",
            first_count,
            all_files.len()
        );
    });
}

// ===========================================================================
// Test 5: Gate disabled = normal behavior (files created immediately)
// ===========================================================================

#[test]
fn test_gate_disabled_normal_behavior() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let state = make_gate_state(dir, false, 5, 0.01);

        // Gate disabled means GateState::Recording from the start
        assert_eq!(state.gate_state, GateState::Recording);
        assert!(!state.gate_idle.load(Ordering::Relaxed));

        // Files should already exist (.recording.wav)
        let files = all_wav_like_files(temp_dir.path());
        assert!(
            !files.is_empty(),
            "Files should be created immediately when gate is disabled"
        );
    });
}

// ===========================================================================
// Test 6: Rotation is no-op when gate idle
// ===========================================================================

#[test]
fn test_rotation_noop_when_gate_idle() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.01);
        assert_eq!(state.gate_state, GateState::Idle);

        // Rotation should be a no-op
        state.rotate_files();

        // Still idle, no files
        assert_eq!(state.gate_state, GateState::Idle);
        let files = all_wav_like_files(temp_dir.path());
        assert!(
            files.is_empty(),
            "Rotation should not create files when gate is idle"
        );
    });
}

// ===========================================================================
// Test 7: Peaks are tracked even in gate idle state
// ===========================================================================

#[test]
fn test_peaks_tracked_while_gate_idle() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        // Use a high threshold so signal stays below it (gate stays idle)
        let mut state = make_gate_state(dir, true, 5, 0.9);
        assert_eq!(state.gate_state, GateState::Idle);

        // Feed audio below the high threshold
        let data: Vec<f32> = vec![0.5; 48000];
        state.write_samples(&data);

        // Gate should still be idle (0.5 < 0.9 threshold)
        assert_eq!(state.gate_state, GateState::Idle);

        // But peaks should be tracked
        let peak_bits = state.peak_levels[0].value.load(Ordering::Relaxed);
        let peak = f32::from_bits(peak_bits);
        assert!(
            peak > 0.4,
            "Peak should be tracked even in idle state, got {}",
            peak
        );
    });
}
