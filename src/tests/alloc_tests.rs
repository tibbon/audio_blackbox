use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tempfile::tempdir;

use crate::alloc_counter;
use crate::constants::{CacheAlignedPeak, OutputMode};
use crate::cpal_processor::push_samples_with_overflow_count;
use crate::writer_thread::WriterThreadState;

// Test helper consolidated to `crate::test_utils` (DOLL-118).
use crate::numeric::{count_to_f64, len_to_f32};
use crate::test_utils::test_env_no_silence;

// These run in the normal suite: they take about a second in a debug build.
// The allocation counter is process-wide, so they depend on `--test-threads=1`
// (every lane and scripts/check.sh pass it) — a test allocating on another
// thread during the measured loop would fail them.

/// Generate interleaved f32 test data.
fn generate_data(total_channels: usize, frames: usize) -> Vec<f32> {
    let total = total_channels * frames;
    let mut data = vec![0.0_f32; total];
    for (i, sample) in data.iter_mut().enumerate() {
        *sample = (len_to_f32(i) * 0.01).sin() * 0.5;
    }
    data
}

/// One zeroed peak slot per channel. `CacheAlignedPeak` holds an atomic and
/// is not `Clone`, so this is the `vec![x; n]` equivalent for it.
fn zero_peaks(ch_count: usize) -> Arc<[CacheAlignedPeak]> {
    std::iter::repeat_with(|| CacheAlignedPeak::new(0))
        .take(ch_count)
        .collect()
}

// ===========================================================================
// Allocation counting: monitor mode (peak tracking only, no disk I/O)
// ===========================================================================

#[test]
fn test_write_samples_zero_alloc_monitor() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;

    temp_env::with_vars(test_env_no_silence(), || {
        let channels: Vec<usize> = (0..ch_count).collect();
        let peak_levels: Arc<[CacheAlignedPeak]> = zero_peaks(ch_count);
        let mut state = WriterThreadState::new_monitor(sample_rate, &channels, peak_levels);
        state.total_device_channels = u16::try_from(ch_count).expect("channel count fits in u16");

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
            "\n  Monitor mode (2ch/48kHz): {allocs} allocations across {iterations} write_samples() calls"
        );
        println!(
            "  ({:.3} allocations per call)",
            count_to_f64(allocs) / f64::from(iterations)
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

/// Steady-state heap allocations across 1000 `write_samples` calls of 512
/// stereo frames at 48 kHz, after a warmup that lets `BufWriter` establish
/// its internal buffer.
fn recording_allocs(output_mode: OutputMode, bits_per_sample: u16) -> u64 {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();
    let write_errors = Arc::new(AtomicU64::new(0));
    let channels: Vec<usize> = (0..ch_count).collect();

    let mut state = WriterThreadState::new(
        dir,
        sample_rate,
        &channels,
        output_mode,
        0.0,
        Arc::clone(&write_errors),
        0,
        Arc::new(AtomicBool::new(false)),
        bits_per_sample,
        zero_peaks(ch_count),
        false,
        0,
    )
    .unwrap();
    state.total_device_channels = u16::try_from(ch_count).expect("channel count fits in u16");

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
        "\n  Recording mode ({output_mode}, 2ch/48kHz/{bits_per_sample}-bit): {allocs} allocations across {iterations} write_samples() calls"
    );
    println!(
        "  ({:.3} allocations per call)",
        count_to_f64(allocs) / f64::from(iterations)
    );

    state
        .finalize_all()
        .expect("recording files should finalize cleanly");
    allocs
}

#[test]
fn test_write_samples_zero_alloc_recording() {
    temp_env::with_vars(test_env_no_silence(), || {
        assert_eq!(
            recording_allocs(OutputMode::Single, 24),
            0,
            "write_samples() in recording mode should have zero heap allocations in steady state"
        );
    });
}

/// Single mode at 16 bits runs the TPDF dither (DOLL-373) on every sample.
#[test]
fn test_write_samples_zero_alloc_recording_dithered() {
    temp_env::with_vars(test_env_no_silence(), || {
        assert_eq!(
            recording_allocs(OutputMode::Single, 16),
            0,
            "16-bit (dithered) recording should have zero heap allocations in steady state"
        );
    });
}

/// Split mode writes each channel through its own writer (`write_split_frames`).
#[test]
fn test_write_samples_zero_alloc_split() {
    temp_env::with_vars(test_env_no_silence(), || {
        assert_eq!(
            recording_allocs(OutputMode::Split, 24),
            0,
            "split-mode recording should have zero heap allocations in steady state"
        );
        assert_eq!(
            recording_allocs(OutputMode::Split, 16),
            0,
            "16-bit (dithered) split-mode recording should have zero heap allocations in steady state"
        );
    });
}

// ===========================================================================
// Allocation counting: partial frame path (combined_buf + frame_remainder)
// ===========================================================================

#[test]
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
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0,
            Arc::new(AtomicBool::new(false)),
            24,
            zero_peaks(ch_count),
            false,
            0,
        )
        .unwrap();
        state.total_device_channels = u16::try_from(ch_count).expect("channel count fits in u16");

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
            "\n  Partial frame path (2ch, 1023-sample chunks): {allocs} allocations across {iterations} calls"
        );
        println!(
            "  ({:.3} allocations per call)",
            count_to_f64(allocs) / f64::from(iterations)
        );

        state
            .finalize_all()
            .expect("recording files should finalize cleanly");

        assert_eq!(
            allocs, 0,
            "write_samples() with partial frames should have zero allocations after warmup"
        );
    });
}

// ===========================================================================
// Allocation counting: the cpal callback's ring-buffer push (RT thread)
// ===========================================================================

/// The cpal input callback runs on the CoreAudio real-time thread and does
/// nothing but `push_samples_with_overflow_count`, so this is the proof that
/// the RT side of the pipeline allocates nothing — both when the ring buffer
/// has room and when it is full and the rejected samples are counted.
#[test]
fn test_rt_push_zero_alloc() {
    const CALLBACK_SAMPLES: usize = 512;
    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(2 * CALLBACK_SAMPLES);
    let write_errors = AtomicU64::new(0);
    let data = generate_data(2, CALLBACK_SAMPLES / 2);

    // One round: two callbacks fill the buffer exactly, the third overflows
    // whole, then the writer side drains everything.
    let mut round = || {
        for _ in 0..3 {
            push_samples_with_overflow_count(&mut producer, &data, &write_errors);
        }
        let queued = consumer.slots();
        consumer
            .read_chunk(queued)
            .expect("slots() reported these as readable")
            .commit_all();
    };

    // Warmup, for symmetry with the writer tests (rtrb allocates only in new()).
    for _ in 0..10 {
        round();
    }
    write_errors.store(0, Ordering::Relaxed);

    let iterations: u32 = 1000;
    let before = alloc_counter::snapshot();
    for _ in 0..iterations {
        round();
    }
    let after = alloc_counter::snapshot();
    let allocs = after - before;

    println!(
        "\n  RT push (512 samples, 1 in 3 overflowing): {allocs} allocations across {} callbacks",
        3 * iterations
    );

    // The overflow branch must actually have run, or this proves only the
    // happy path: one whole rejected callback per round.
    assert_eq!(
        write_errors.load(Ordering::Relaxed),
        u64::from(iterations) * CALLBACK_SAMPLES as u64,
        "every third push should be rejected and counted in write_errors"
    );
    assert_eq!(
        allocs, 0,
        "push_samples_with_overflow_count() runs on the RT thread and must never allocate"
    );
}

// ===========================================================================
// Struct size reporting (always runs)
// ===========================================================================

#[test]
fn test_struct_sizes() {
    let wts_size = size_of::<WriterThreadState>();
    let peak_size = size_of::<CacheAlignedPeak>();

    println!("\n  Struct sizes:");
    println!("    WriterThreadState: {wts_size} bytes");
    println!("    CacheAlignedPeak:  {peak_size} bytes");

    assert_eq!(
        peak_size, 64,
        "CacheAlignedPeak should be exactly 64 bytes (one cache line)"
    );
}
