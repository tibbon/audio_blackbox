use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use tempfile::tempdir;

use crate::alloc_counter;
use crate::constants::CacheAlignedPeak;
use crate::writer_thread::WriterThreadState;

fn test_env_no_silence() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env = crate::tests::default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));
    env
}

/// Generate interleaved f32 test data.
fn generate_data(total_channels: usize, frames: usize) -> Vec<f32> {
    let total = total_channels * frames;
    let mut data = vec![0.0_f32; total];
    for (i, sample) in data.iter_mut().enumerate() {
        *sample = ((i as f32) * 0.01).sin() * 0.5;
    }
    data
}

// ===========================================================================
// Allocation counting: monitor mode (peak tracking only, no disk I/O)
// ===========================================================================

#[test]
#[ignore = "allocation test — run with: cargo test --release alloc -- --ignored --nocapture --test-threads=1"]
fn test_write_samples_zero_alloc_monitor() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;

    temp_env::with_vars(test_env_no_silence(), || {
        let channels: Vec<usize> = (0..ch_count).collect();
        let peak_levels: Arc<Vec<CacheAlignedPeak>> =
            Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect());
        let mut state = WriterThreadState::new_monitor(sample_rate, &channels, peak_levels);
        state.total_device_channels = ch_count as u16;

        let data = generate_data(ch_count, 512);

        // Warmup: establish Vec capacities
        for _ in 0..10 {
            state.write_samples(&data);
        }

        // Measure
        let iterations = 1000;
        let before = alloc_counter::snapshot();
        for _ in 0..iterations {
            state.write_samples(&data);
        }
        let after = alloc_counter::snapshot();
        let allocs = after - before;

        println!(
            "\n  Monitor mode (2ch/48kHz): {} allocations across {} write_samples() calls",
            allocs, iterations
        );
        println!(
            "  ({:.3} allocations per call)",
            allocs as f64 / iterations as f64
        );

        assert_eq!(
            allocs, 0,
            "write_samples() in monitor mode should have zero heap allocations in steady state"
        );
    });
}

// ===========================================================================
// Allocation counting: recording mode (WAV I/O via BufWriter)
// ===========================================================================

#[test]
#[ignore = "allocation test — run with: cargo test --release alloc -- --ignored --nocapture --test-threads=1"]
fn test_write_samples_zero_alloc_recording() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;

    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let channels: Vec<usize> = (0..ch_count).collect();

        let mut state = WriterThreadState::new(
            dir,
            sample_rate,
            &channels,
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            24,
            Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = ch_count as u16;

        let data = generate_data(ch_count, 512);

        // Warmup: let BufWriter establish its internal buffer
        for _ in 0..10 {
            state.write_samples(&data);
        }

        // Measure
        let iterations = 1000;
        let before = alloc_counter::snapshot();
        for _ in 0..iterations {
            state.write_samples(&data);
        }
        let after = alloc_counter::snapshot();
        let allocs = after - before;

        println!(
            "\n  Recording mode (2ch/48kHz/24-bit): {} allocations across {} write_samples() calls",
            allocs, iterations
        );
        println!(
            "  ({:.3} allocations per call)",
            allocs as f64 / iterations as f64
        );

        let _ = state.finalize_all();

        assert_eq!(
            allocs, 0,
            "write_samples() in recording mode should have zero heap allocations in steady state"
        );
    });
}

// ===========================================================================
// Allocation counting: partial frame path (combined_buf + frame_remainder)
// ===========================================================================

#[test]
#[ignore = "allocation test — run with: cargo test --release alloc -- --ignored --nocapture --test-threads=1"]
fn test_write_samples_zero_alloc_partial_frames() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;

    temp_env::with_vars(test_env_no_silence(), || {
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let channels: Vec<usize> = (0..ch_count).collect();

        let mut state = WriterThreadState::new(
            dir,
            sample_rate,
            &channels,
            "single",
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            24,
            Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = ch_count as u16;

        // Data that doesn't divide evenly by frame_size (2 channels):
        // 1023 samples = 511 full frames + 1 leftover sample in frame_remainder
        let data = generate_data(1, 1023); // 1023 f32 values

        // Warmup: grow combined_buf and frame_remainder to their max needed capacity
        for _ in 0..20 {
            state.write_samples(&data);
        }

        // Measure: every call triggers the combined_buf join path
        let iterations = 1000;
        let before = alloc_counter::snapshot();
        for _ in 0..iterations {
            state.write_samples(&data);
        }
        let after = alloc_counter::snapshot();
        let allocs = after - before;

        println!(
            "\n  Partial frame path (2ch, 1023-sample chunks): {} allocations across {} calls",
            allocs, iterations
        );
        println!(
            "  ({:.3} allocations per call)",
            allocs as f64 / iterations as f64
        );

        let _ = state.finalize_all();

        assert_eq!(
            allocs, 0,
            "write_samples() with partial frames should have zero allocations after warmup"
        );
    });
}

// ===========================================================================
// Struct size reporting (always runs)
// ===========================================================================

#[test]
fn test_struct_sizes() {
    let wts_size = std::mem::size_of::<WriterThreadState>();
    let peak_size = std::mem::size_of::<CacheAlignedPeak>();

    println!("\n  Struct sizes:");
    println!("    WriterThreadState: {} bytes", wts_size);
    println!("    CacheAlignedPeak:  {} bytes", peak_size);

    assert_eq!(
        peak_size, 64,
        "CacheAlignedPeak should be exactly 64 bytes (one cache line)"
    );
}
