use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, OutputMode};
use crate::raw_wav_writer::{RawWavWriter, WavSpec};
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

/// DOLL-350: after a disk-low self-stop (finalize_all has already cleared the
/// writers + pending pairs), a continuous-mode rotation must NOT recreate empty
/// `.recording.wav` temp files. Without the `disk_stopped` guard, rotate_files
/// calls create_wav_writer again and leaks zero-length temps on the full disk.
#[test]
fn rotate_files_is_a_noop_when_disk_stopped() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = single_state(dir);

        // Simulate the post-disk-stop state: finalize_all has taken the writer
        // and drained pending_files, and disk_stopped is latched.
        state.writer = None;
        state.pending_files.clear();
        state.disk_stopped = true;

        state.rotate_files();

        assert!(
            state.writer.is_none(),
            "rotate_files must not recreate a writer after a disk stop"
        );
        assert!(
            state.pending_files.is_empty(),
            "rotate_files must not leak new pending temp files after a disk stop"
        );
    });
}

/// DOLL-444: when creating the next period's file fails during rotation
/// (output directory deleted, volume unmounted, disk full), the writer must
/// latch the `write_failed` self-stop. Previously the error was only logged
/// and `self.writer` stayed `None` — and since `write_samples` skips absent
/// writers without bumping `write_errors`, every subsequent sample was
/// silently discarded while the app kept showing "recording".
#[test]
fn rotation_create_failure_latches_write_failed_and_stops() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = single_state(dir);
        let final_path = state.pending_files[0].1.clone();
        let write_failed = Arc::clone(&state.write_failed);

        // Land some audio in the current period.
        state.write_samples(&vec![0.25_f32; 4_800]);

        // Sabotage the next period: the output directory vanishes.
        state.output_dir = format!("{dir}/vanished");
        state.rotate_files();

        assert!(
            write_failed.load(Ordering::Relaxed),
            "a failed rotation create must latch write_failed"
        );
        assert!(state.disk_stopped, "self-stop must latch disk_stopped");
        assert!(state.writer.is_none());
        assert!(state.pending_files.is_empty());
        assert!(
            Path::new(&final_path).exists(),
            "the finished period's audio must still land under its final name"
        );

        // After the stop, write_samples is a no-op — no silent discard
        // mislabeled as recording, no error-counter spin.
        let errors_before = state.write_errors.load(Ordering::Relaxed);
        state.write_samples(&vec![0.25_f32; 4_800]);
        assert_eq!(state.write_errors.load(Ordering::Relaxed), errors_before);

        // And further rotations stay no-ops (DOLL-350 guard).
        state.rotate_files();
        assert!(state.writer.is_none());
        assert!(state.pending_files.is_empty());
    });
}

/// DOLL-444, Split mode (the app default): a per-channel create failure during
/// rotation latches the same self-stop, after the finished period's files have
/// been renamed to their final names.
#[test]
fn split_rotation_create_failure_latches_after_renaming_finished_files() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = split_state(dir);
        let finals: Vec<String> = state.pending_files.iter().map(|(_, f)| f.clone()).collect();
        let write_failed = Arc::clone(&state.write_failed);

        // Interleaved 2-channel audio for the current period.
        state.write_samples(&vec![0.25_f32; 9_600]);

        state.output_dir = format!("{dir}/vanished");
        state.rotate_files();

        assert!(
            write_failed.load(Ordering::Relaxed),
            "a failed channel create must latch write_failed"
        );
        assert!(state.disk_stopped);
        assert!(state.multichannel_writers.iter().all(Option::is_none));
        assert!(state.pending_files.is_empty());
        for f in &finals {
            assert!(
                Path::new(f).exists(),
                "finished channel file {f} must be renamed before the stop"
            );
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

/// DOLL-349/437 (filed as DOLL-452): once `write_sample` failures accumulate to
/// ~1s of audio (`sample_rate` samples), the writer self-stops — latches the
/// shared `write_failed` flag, sets `disk_stopped`, and finalizes so audio that
/// landed before the failure is renamed to its final name. Below the threshold
/// nothing latches, and after the stop `write_samples` is a no-op (no further
/// error accumulation on a dead disk).
#[test]
fn persistent_write_failures_latch_flag_and_self_stop() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = single_state(dir);
        let final_path = state.pending_files[0].1.clone();
        let write_failed = Arc::clone(&state.write_failed);
        state.writer = Some(RawWavWriter::new_failing_for_tests(&format!(
            "{dir}/failing.wav"
        )));

        // Just below the 48_000-sample (~1s) threshold: no stop yet.
        state.write_samples(&vec![0.1_f32; 47_000]);
        assert!(
            !write_failed.load(Ordering::Relaxed),
            "below ~1s of consecutive failures the flag must not latch"
        );
        assert!(!state.disk_stopped);
        assert_eq!(
            state.write_errors.load(Ordering::Relaxed),
            47_000,
            "each failed sample must still bump the shared write_errors counter"
        );

        // Crossing the threshold stops the writer.
        state.write_samples(&vec![0.1_f32; 2_000]);
        assert!(
            write_failed.load(Ordering::Relaxed),
            "crossing ~1s of consecutive failures must latch write_failed"
        );
        assert!(state.disk_stopped, "self-stop must latch disk_stopped");
        assert!(
            state.writer.is_none(),
            "self-stop must finalize (take) the writer"
        );
        assert!(
            Path::new(&final_path).exists(),
            "audio written before the failure must be renamed to its final name"
        );

        // After the stop, write_samples is a no-op.
        let errors_before = state.write_errors.load(Ordering::Relaxed);
        state.write_samples(&vec![0.1_f32; 5_000]);
        assert_eq!(
            state.write_errors.load(Ordering::Relaxed),
            errors_before,
            "post-stop writes must not keep bumping write_errors"
        );
    });
}

/// DOLL-349 (filed as DOLL-452): the failure streak is CONSECUTIVE — a clean
/// batch resets it. Two sub-threshold failure bursts separated by a healthy
/// batch must not add up to a stop even though their sum exceeds the
/// threshold; a burst crossing the threshold within one streak must.
#[test]
fn clean_batch_resets_write_failure_streak() {
    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let mut state = single_state(dir);
        let write_failed = Arc::clone(&state.write_failed);

        // First sub-threshold burst: 34k of the 48k threshold.
        state.writer = Some(RawWavWriter::new_failing_for_tests(&format!(
            "{dir}/failing-a.wav"
        )));
        state.write_samples(&vec![0.1_f32; 34_000]);
        assert!(!write_failed.load(Ordering::Relaxed));

        // One clean batch on a healthy writer resets the streak.
        state.writer = Some(
            RawWavWriter::create(
                &format!("{dir}/healthy.wav"),
                WavSpec {
                    channels: 1,
                    sample_rate: 48_000,
                    bits_per_sample: 24,
                },
            )
            .unwrap(),
        );
        state.write_samples(&vec![0.1_f32; 1_000]);

        // Second sub-threshold burst: 34k + 34k > 48k cumulative, but the
        // streak was reset, so nothing may latch.
        state.writer = Some(RawWavWriter::new_failing_for_tests(&format!(
            "{dir}/failing-b.wav"
        )));
        state.write_samples(&vec![0.1_f32; 34_000]);
        assert!(
            !write_failed.load(Ordering::Relaxed),
            "a clean batch must reset the consecutive-failure streak"
        );
        assert!(!state.disk_stopped);

        // Crossing the threshold within a single streak still latches.
        state.write_samples(&vec![0.1_f32; 15_000]);
        assert!(
            write_failed.load(Ordering::Relaxed),
            "a full ~1s streak after the reset must still stop"
        );
        assert!(state.disk_stopped);
    });
}

/// DOLL-373: 16-bit output gets TPDF dither, so a constant-0 input is perturbed
/// off zero (within ~1 LSB) instead of writing a dead-silent quantized stream;
/// 24-bit output is left undithered (exact). Deterministic given the fixed
/// xorshift seed, so this is not flaky.
#[test]
fn dither_perturbs_16bit_output_but_not_24bit() {
    temp_env::with_vars(test_env_no_silence(), || {
        // 16-bit: constant 0.0 → dithered samples in {-1, 0, +1}, not all zero.
        let td16 = tempdir().unwrap();
        let dir16 = td16.path().to_str().unwrap();
        let mut s16 = WriterThreadState::new(
            dir16,
            48_000,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::new(AtomicU64::new(0)),
            0,
            Arc::new(AtomicBool::new(false)),
            16,
            Arc::new(vec![CacheAlignedPeak::new(0)]),
            false,
            0,
        )
        .unwrap();
        s16.total_device_channels = 1;
        let final16 = s16.pending_files[0].1.clone();
        s16.write_samples(&vec![0.0_f32; 400]);
        s16.finalize_all().unwrap();
        let samples16: Vec<i32> = hound::WavReader::open(&final16)
            .unwrap()
            .into_samples::<i32>()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples16.len(), 400);
        assert!(
            samples16.iter().any(|&x| x != 0),
            "16-bit dither must perturb a constant-0 input off zero"
        );
        assert!(
            samples16.iter().all(|&x| x.abs() <= 1),
            "TPDF dither on silence must stay within ~1 LSB"
        );

        // 24-bit: constant 0.0 → exact zeros (undithered).
        let td24 = tempdir().unwrap();
        let mut s24 = single_state(td24.path().to_str().unwrap());
        let final24 = s24.pending_files[0].1.clone();
        s24.write_samples(&vec![0.0_f32; 400]);
        s24.finalize_all().unwrap();
        let samples24: Vec<i32> = hound::WavReader::open(&final24)
            .unwrap()
            .into_samples::<i32>()
            .map(Result::unwrap)
            .collect();
        assert!(
            samples24.iter().all(|&x| x == 0),
            "24-bit output must be undithered (exact zeros)"
        );
    });
}
