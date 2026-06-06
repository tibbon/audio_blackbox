use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, OutputMode};
use crate::test_utils::test_env_no_silence;
use crate::writer_thread::WriterThreadState;

/// Build a single-file writer state (1 channel, 48 kHz, 24-bit, gate disabled)
/// with its writer + pending pair already created.
fn single_state(dir: &str) -> WriterThreadState {
    let channels: Vec<usize> = vec![0];
    let mut state = WriterThreadState::new(
        dir,
        48_000,
        &channels,
        OutputMode::Single,
        0.0,
        Arc::new(AtomicU64::new(0)),
        0,
        Arc::new(AtomicBool::new(false)),
        24,
        Arc::new(vec![CacheAlignedPeak::new(0)]),
        false,
        0,
    )
    .expect("construct single writer state");
    state.total_device_channels = 1;
    state
}

/// Read the WAV data-chunk size field (little-endian u32 at byte offset 40).
/// 0 means the header has not been rewritten since `create` (which writes 0).
fn read_wav_data_size(path: &str) -> u32 {
    let mut f = File::open(path).expect("open wav");
    f.seek(SeekFrom::Start(40)).expect("seek to data size");
    let mut b = [0u8; 4];
    f.read_exact(&mut b).expect("read data size");
    u32::from_le_bytes(b)
}

/// Build a 2-channel Split-mode writer state with its initial writers + pending
/// pairs already created (gate disabled). Returns the state plus the temp dir
/// guard (kept alive by the caller).
fn split_state(dir: &str) -> WriterThreadState {
    let channels: Vec<usize> = vec![0, 1];
    let mut state = WriterThreadState::new(
        dir,
        48_000,
        &channels,
        OutputMode::Split,
        0.0, // silence_threshold: 0 → no silence worker
        Arc::new(AtomicU64::new(0)),
        0, // min_disk_space_mb: disabled
        Arc::new(AtomicBool::new(false)),
        24,
        Arc::new((0..2).map(|_| CacheAlignedPeak::new(0)).collect()),
        false, // gate_enabled: false → writers created immediately
        0,
    )
    .expect("construct split writer state");
    state.total_device_channels = 2;
    state
}

/// DOLL-345: a rename failure on one channel must not strand the others. The
/// old `?`-on-first-error `finalize_all` returned immediately, leaving the
/// remaining channels' audio under `.recording.wav` temp names in the app's
/// default Split mode. The fix attempts every writer/rename and returns the
/// first error only after all attempts.
#[test]
fn finalize_all_continues_past_a_failed_rename() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = split_state(dir);
        assert_eq!(
            state.pending_files.len(),
            2,
            "split mode should have one pending pair per channel"
        );

        // Sabotage channel 0's destination so its rename fails (ENOENT: the
        // parent directory does not exist), leaving channel 1 intact.
        let good_final = state.pending_files[1].1.clone();
        state.pending_files[0].1 = format!("{}/does_not_exist/ch0.wav", dir);

        let result = state.finalize_all();

        assert!(
            result.is_err(),
            "finalize_all must surface the failed rename"
        );
        assert!(
            Path::new(&good_final).exists(),
            "channel 1 must still be finalized despite channel 0's failure"
        );
    });
}

/// Happy path: with no induced failure, every channel is finalized and the
/// call succeeds.
#[test]
fn finalize_all_renames_every_channel() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = split_state(dir);
        let finals: Vec<String> = state.pending_files.iter().map(|(_, f)| f.clone()).collect();

        let result = state.finalize_all();

        assert!(result.is_ok(), "finalize_all should succeed: {result:?}");
        for f in &finals {
            assert!(Path::new(f).exists(), "expected finalized file {f}");
        }
    });
}

/// DOLL-347: the crash-recovery flush path (`flush_writers` → `RawWavWriter::flush`)
/// backs the SIGKILL-safety guarantee — after a flush the in-progress
/// `.recording.wav` must be a valid, playable WAV with the correct data size,
/// without ever calling `finalize()`. Previously untested, so a regression to
/// the mid-recording header rewrite would have shipped silently (the E2E tests
/// only read files after `finalize`, which always rewrites the header).
#[test]
fn flush_writers_makes_unfinalized_recording_readable() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = single_state(dir);
        let tmp = state.pending_files[0].0.clone();

        let n = 1_000;
        let data: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        state.write_samples(&data);

        // Cross the sample_rate*10 frame threshold so the flush actually fires.
        state.flush_writers((state.sample_rate as usize) * 10);

        // The still-unfinalized .recording.wav must parse and report n samples.
        let reader = hound::WavReader::open(&tmp)
            .expect("flushed .recording.wav should be a valid WAV before finalize");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.bits_per_sample, 24);
        assert_eq!(
            reader.len(),
            n as u32,
            "flushed header must report every written sample"
        );
    });
}

/// DOLL-347: the flush is gated on accumulating `sample_rate * 10` frames, so a
/// below-threshold call must NOT rewrite the on-disk header, and a call that
/// crosses it must. Drives the gating via an observable disk effect.
#[test]
fn flush_writers_respects_frame_threshold() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = single_state(dir);
        let tmp = state.pending_files[0].0.clone();
        let byte_width = 3; // 24-bit mono

        // First flush (threshold crossed) establishes a real on-disk header.
        let first: Vec<f32> = vec![0.1; 1_000];
        state.write_samples(&first);
        state.flush_writers((state.sample_rate as usize) * 10);
        assert_eq!(
            read_wav_data_size(&tmp),
            1_000 * byte_width,
            "crossing the threshold should rewrite the header"
        );

        // Write more, then flush BELOW the threshold: header must be unchanged.
        let more: Vec<f32> = vec![0.2; 1_000];
        state.write_samples(&more);
        state.flush_writers(1_000); // 1000 frames << sample_rate*10
        assert_eq!(
            read_wav_data_size(&tmp),
            1_000 * byte_width,
            "a below-threshold flush must not rewrite the header"
        );

        // Cross the threshold again: header now reflects all 2000 samples.
        state.flush_writers((state.sample_rate as usize) * 10);
        assert_eq!(
            read_wav_data_size(&tmp),
            2_000 * byte_width,
            "crossing the threshold again should rewrite the header"
        );
    });
}
