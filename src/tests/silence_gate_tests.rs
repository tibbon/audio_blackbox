use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, OutputMode};
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

// Test helper consolidated to `crate::test_utils` (DOLL-118).
use crate::test_utils::test_env_no_silence;

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
        OutputMode::Single,
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
        state.process_gate_close(); // simulate main loop processing

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

        // Inject a deterministic clock so the second open of the gate
        // produces a distinct filename without sleeping a real second.
        let clock = crate::test_utils::MockClock::new();
        state.set_timestamp_fn(clock.as_timestamp_fn());

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
        state.process_gate_close(); // simulate main loop processing
        assert_eq!(state.gate_state, GateState::Idle);

        let first_files = wav_files_in(temp_dir.path());
        let first_count = first_files.len();
        assert!(first_count > 0, "First gate cycle should produce files");

        // Advance the mock clock so the next opened file gets a distinct name.
        clock.advance();

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

        // Positive control first: rotation while Recording must produce a
        // new file. Without this anchor, the Idle assertion below could
        // pass even if rotate_files() became a no-op for ALL states.
        {
            let mut state = make_gate_state(dir, false, 5, 0.0);
            assert_eq!(state.gate_state, GateState::Recording);
            let before = all_wav_like_files(temp_dir.path()).len();
            // Advance the clock so the rotated file gets a distinct stamp.
            let clock = crate::test_utils::MockClock::new();
            state.set_timestamp_fn(clock.as_timestamp_fn());
            clock.advance();
            state.rotate_files();
            let after = all_wav_like_files(temp_dir.path()).len();
            assert!(
                after > before,
                "rotation while Recording must produce new files: before={before}, after={after}"
            );
            // Drop state to release file handles before the negative case.
        }

        // Negative case: rotation while Idle should be a no-op.
        let temp_dir2 = tempdir().unwrap();
        let dir2 = temp_dir2.path().to_str().unwrap();
        let mut state = make_gate_state(dir2, true, 5, 0.01);
        assert_eq!(state.gate_state, GateState::Idle);

        state.rotate_files();

        assert_eq!(state.gate_state, GateState::Idle);
        let files = all_wav_like_files(temp_dir2.path());
        assert!(
            files.is_empty(),
            "Rotation should not create files when gate is idle, found: {files:?}"
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

// ===========================================================================
// NaN guards (DOLL-81)
// ===========================================================================

#[test]
#[allow(clippy::float_cmp)] // exact 0.0 expected when all input samples are filtered
fn test_nan_sample_does_not_poison_peak_meter() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.9);

        // Buffer of NaN samples — peak atomic should remain 0.0, not NaN.
        let data: Vec<f32> = vec![f32::NAN; 4800];
        state.write_samples(&data);

        let peak_bits = state.peak_levels[0].value.load(Ordering::Relaxed);
        let peak = f32::from_bits(peak_bits);
        assert!(
            peak.is_finite(),
            "Peak meter must never publish NaN; got bits {:#x}",
            peak_bits
        );
        assert_eq!(peak, 0.0, "All-NaN input should leave peak at 0.0");
    });
}

#[test]
fn test_nan_does_not_block_silence_gate_open() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        // Threshold 0.1 — clean 0.5 should open the gate.
        let mut state = make_gate_state(dir, true, 5, 0.1);
        assert_eq!(state.gate_state, GateState::Idle);

        // Mix one NaN sample into otherwise-clean audio. NaN must not poison
        // max_peak so that the > comparison stays usable.
        let mut data: Vec<f32> = vec![0.5; 4800];
        data[100] = f32::NAN;
        state.write_samples(&data);

        // Gate should have flagged for opening (the main loop transitions
        // Idle→Recording on the next iteration via gate_pending_open).
        assert!(
            state.gate_pending_open,
            "Silence gate must request open when finite signal exceeds threshold, even with NaN samples in the buffer"
        );
    });
}

#[test]
#[allow(clippy::float_cmp)] // exact 0.0 expected when all input samples are filtered
fn test_inf_sample_clamps_peak_meter_to_one() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.9);

        // Buffer with ±Inf — also filtered by is_finite(); peak stays at 0.
        let mut data: Vec<f32> = vec![0.0; 4800];
        data[10] = f32::INFINITY;
        data[20] = f32::NEG_INFINITY;
        state.write_samples(&data);

        let peak_bits = state.peak_levels[0].value.load(Ordering::Relaxed);
        let peak = f32::from_bits(peak_bits);
        assert!(peak.is_finite(), "Peak must be finite even with Inf input");
        assert_eq!(peak, 0.0, "Inf samples must not contribute to peak");
    });
}

// ===========================================================================
// Gate-open recording writes the actual post-open signal (content, not just
// file existence)
// ===========================================================================

/// DOLL-355 (updated by DOLL-465): the gate tests asserted file existence /
/// state transitions but never opened the WAV to verify content. Originally
/// this asserted the triggering batch was NOT written (the deferred-open
/// design discarded it); DOLL-465 added a pre-roll that replays the
/// triggering batch on gate open, so the file must now contain the trigger
/// batch followed by the post-open batch — and nothing else.
#[test]
fn test_gate_recording_writes_onset_and_post_open_signal() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.01);

        // Burst above threshold trips the gate; processed in the peaks-only
        // branch while idle, retained as pre-roll, replayed on open.
        let trigger: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        state.write_samples(&trigger);
        state.process_gate_open();
        assert_eq!(state.gate_state, GateState::Recording);

        // Live audio after the open.
        let n = 2_000;
        let post = vec![0.5_f32; n];
        state.write_samples(&post);
        state.finalize_all().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1, "expected exactly one finalized WAV");

        let reader = hound::WavReader::open(&files[0]).expect("finalized gate WAV must be valid");
        let samples: Vec<i32> = reader.into_samples::<i32>().map(Result::unwrap).collect();
        assert_eq!(
            samples.len(),
            trigger.len() + n,
            "file must contain the replayed onset batch plus the post-open batch"
        );
        // The onset must be the trigger sine, not leaked zeros: spot-check an
        // early sample. sin(0.1)*0.5 → ~1636 at 16-bit, ±1 LSB TPDF dither
        // (DOLL-373).
        let expected_onset = ((0.1_f32).sin() * 0.5 * 32767.0).round() as i32;
        assert!(
            (samples[1] - expected_onset).abs() <= 1,
            "onset sample must match the trigger batch, got {} want ~{expected_onset}",
            samples[1]
        );
        // The tail is the uniform post-open batch: 0.5 → ~16384, ±1 LSB.
        let expected_post = (0.5_f32 * 32767.0).round() as i32;
        assert!(
            samples[trigger.len()..]
                .iter()
                .all(|&s| (s - expected_post).abs() <= 1),
            "post-open samples must follow the replayed onset"
        );
    });
}

/// DOLL-465: only the LAST idle batch is retained as pre-roll — earlier
/// idle silence must not leak into the file, and the pre-roll must not be
/// duplicated by the live writes that follow.
#[test]
fn test_gate_preroll_keeps_only_last_idle_batch() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = make_gate_state(dir, true, 5, 0.01);

        // A long stretch of idle silence (must NOT land in the file)…
        state.write_samples(&vec![0.0_f32; 30_000]);
        assert_eq!(state.gate_state, GateState::Idle);

        // …then the triggering batch (must land exactly once).
        let trigger = vec![0.4_f32; 5_000];
        state.write_samples(&trigger);
        state.process_gate_open();
        assert_eq!(state.gate_state, GateState::Recording);

        state.finalize_all().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1, "expected exactly one finalized WAV");
        let reader = hound::WavReader::open(&files[0]).expect("valid WAV");
        let samples: Vec<i32> = reader.into_samples::<i32>().map(Result::unwrap).collect();
        assert_eq!(
            samples.len(),
            trigger.len(),
            "file must contain exactly the triggering batch — no prior silence, no duplication"
        );
        let expected = (0.4_f32 * 32767.0).round() as i32;
        assert!(
            samples.iter().all(|&s| (s - expected).abs() <= 1),
            "retained onset must be the trigger batch's content"
        );
    });
}
