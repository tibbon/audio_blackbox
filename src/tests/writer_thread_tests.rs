use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, OutputMode};
use crate::test_utils::test_env_no_silence;
use crate::writer_thread::WriterThreadState;

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
