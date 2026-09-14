//! Standalone benchmark binary for profiling the writer thread hot path.
//!
//! **Caveat (DOLL-192):** the `single` and `split` direct-write modes use
//! the `hound` crate's `WavWriter`, NOT the production `RawWavWriter`.
//! Numbers from those modes are useful as relative measurements (e.g.,
//! comparing channel counts or sample rates) but should NOT be quoted as
//! production write throughput — `RawWavWriter` is what ships and has
//! different buffering / header behaviour. The `pipeline` mode IS
//! representative: as of DOLL-251 it delegates to
//! `blackbox::bench_real_pipeline`, driving the shipped writer thread
//! (`WriterThreadState`, `writer_thread_main`, `RawWavWriter`) through an
//! rtrb ring buffer and the adaptive-sleep drain loop — the exact production
//! write path. The CI throughput floor asserts on `pipeline`, so it guards
//! shipped code.
//!
//! Usage:
//!   cargo build --release --bin bench-writer --features benchmarking
//!   samply record target/release/bench-writer \[OPTIONS\]
//!
//! Options:
//!   --channels N     Number of channels (default: 64)
//!   --seconds N      Duration of simulated audio (default: 30)
//!   --mode MODE      "single", "split", or "pipeline" (default: single)
//!   --sample-rate N  Sample rate in Hz (default: 48000)

// Benchmark binary: stderr is its UI and exit codes are its contract with
// scripts/bench-assert.sh, so the GUI-oriented process rules do not apply.
#![expect(
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "standalone bench binary: stderr is the UI, a bad argument or a failed setup aborts with a message, and the exit code is the contract with scripts/bench-assert.sh"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Inline copy of `writer_thread::f32_to_wav_sample` so this benchmark
/// binary doesn't widen the lib's public API (DOLL-129). The lib helper
/// is `pub(crate)` for the same reason. If the conversion math changes
/// in the lib, this copy needs to track it.
fn f32_to_wav_sample(sample: f32, bits_per_sample: u16) -> i32 {
    let scale = match bits_per_sample {
        16 => 32_767.0,
        24 => 8_388_607.0,
        // 2^31: `i32::MAX` has no exact f32 and rounds here; `as i32` saturates.
        _ => 32_768.0_f32 * 65_536.0,
    };
    (sample.clamp(-1.0, 1.0) * scale).round() as i32
}

/// A sample count as `f64` for the throughput report.
/// Exact below 2^53; this binary's counts are at most a few hundred million.
#[expect(
    clippy::cast_precision_loss,
    reason = "exact below 2^53, far above any count this benchmark produces"
)]
const fn count_to_f64(count: usize) -> f64 {
    count as f64
}

/// A sample index as the phase of the synthetic waveform. Exact below 2^24,
/// which one 512-frame chunk only passes above 32,768 channels; past that the
/// rounding just nudges the generated samples.
#[expect(
    clippy::cast_precision_loss,
    reason = "exact below 2^24; beyond it the rounding only nudges synthetic samples"
)]
const fn index_to_f32(index: usize) -> f32 {
    index as f32
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut channels: usize = 64;
    let mut seconds: usize = 30;
    let mut mode = "single".to_owned();
    let mut sample_rate: u32 = 48000;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--channels" => {
                channels = args[i + 1].parse().expect("invalid --channels");
                i += 2;
            }
            "--seconds" => {
                seconds = args[i + 1].parse().expect("invalid --seconds");
                i += 2;
            }
            "--mode" => {
                mode.clone_from(&args[i + 1]);
                i += 2;
            }
            "--sample-rate" => {
                sample_rate = args[i + 1].parse().expect("invalid --sample-rate");
                i += 2;
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: bench-writer [--channels N] [--seconds N] [--mode single|split|pipeline] [--sample-rate N]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(1);
            }
        }
    }

    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let dir = temp_dir.path().to_str().unwrap();

    let frames = sample_rate as usize * seconds;
    let total_samples = frames * channels;

    eprintln!(
        "Benchmark: {mode} mode, {channels} channels, {seconds} seconds ({frames} frames, {total_samples} total samples)"
    );
    eprintln!("Output dir: {dir}");
    eprintln!("Sample rate: {sample_rate} Hz");
    eprintln!();

    // Generate test data — interleaved f32, 512-frame chunks (typical cpal callback)
    let chunk_frames = 512;
    let chunk_samples = chunk_frames * channels;
    let mut chunk_data = vec![0.0_f32; chunk_samples];
    for (idx, sample) in chunk_data.iter_mut().enumerate() {
        *sample = (index_to_f32(idx) * 0.01).sin() * 0.5;
    }

    match mode.as_str() {
        "single" | "split" => {
            run_direct(dir, sample_rate, channels, &mode, frames, &chunk_data);
        }
        "pipeline" => {
            run_pipeline(dir, sample_rate, channels, frames, &chunk_data);
        }
        other => {
            eprintln!("Unknown mode: {other}. Use single, split, or pipeline.");
            std::process::exit(1);
        }
    }
}

/// Direct write benchmark — exercises the exact hot path of `write_samples()`.
fn run_direct(
    dir: &str,
    sample_rate: u32,
    num_channels: usize,
    mode: &str,
    total_frames: usize,
    chunk_data: &[f32],
) {
    use std::io::BufWriter;

    let chunk_frames = chunk_data.len() / num_channels;
    let channel_indices: Vec<usize> = (0..num_channels).collect();
    let write_errors = Arc::new(AtomicU64::new(0));

    if mode == "split" {
        let mut writers: Vec<Option<hound::WavWriter<BufWriter<std::fs::File>>>> = Vec::new();
        for ch in 0..num_channels {
            let path = format!("{dir}/bench-ch{ch}.recording.wav");
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            };
            writers.push(Some(hound::WavWriter::create(&path, spec).unwrap()));
        }

        eprintln!("Writing {total_frames} frames in split mode ({num_channels} files)...");
        let start = Instant::now();

        let mut frames_written = 0;
        while frames_written < total_frames {
            let frames_this_chunk = chunk_frames.min(total_frames - frames_written);
            let samples_this_chunk = frames_this_chunk * num_channels;
            let data = &chunk_data[..samples_this_chunk];

            for frame in data.chunks(num_channels) {
                for (idx, &channel) in channel_indices.iter().enumerate() {
                    if channel < frame.len()
                        && let Some(w) = &mut writers[idx]
                    {
                        // 24-bit; same clamp+round contract as the production hot path (DOLL-110).
                        let sample = f32_to_wav_sample(frame[channel], 24);
                        if w.write_sample(sample).is_err() {
                            write_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }

            frames_written += frames_this_chunk;
        }

        let elapsed = start.elapsed();
        report_results(
            num_channels,
            total_frames,
            elapsed,
            write_errors.load(Ordering::Relaxed),
            sample_rate,
        );

        for w in &mut writers {
            if let Some(writer) = w.take() {
                writer.finalize().expect("finalize split wav");
            }
        }
    } else {
        let path = format!("{dir}/bench.recording.wav");
        let spec = hound::WavSpec {
            channels: num_channels as u16,
            sample_rate,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();

        eprintln!("Writing {total_frames} frames in single/multichannel mode...");
        let start = Instant::now();

        let mut frames_written = 0;
        while frames_written < total_frames {
            let frames_this_chunk = chunk_frames.min(total_frames - frames_written);
            let samples_this_chunk = frames_this_chunk * num_channels;
            let data = &chunk_data[..samples_this_chunk];

            for frame in data.chunks(num_channels) {
                for &channel in &channel_indices {
                    if channel < frame.len() {
                        // 24-bit; same clamp+round contract as the production hot path (DOLL-110).
                        let sample = f32_to_wav_sample(frame[channel], 24);
                        if writer.write_sample(sample).is_err() {
                            write_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }

            frames_written += frames_this_chunk;
        }

        let elapsed = start.elapsed();
        report_results(
            num_channels,
            total_frames,
            elapsed,
            write_errors.load(Ordering::Relaxed),
            sample_rate,
        );

        writer.finalize().expect("finalize wav");
    }
}

/// Full pipeline benchmark — drives the REAL production writer (DOLL-251).
///
/// Delegates to `blackbox::bench_real_pipeline`, which routes samples through
/// the shipped `WriterThreadState` + `writer_thread_main` + `RawWavWriter`
/// and the adaptive-sleep drain loop. This is the mode the CI throughput
/// floor asserts on, so a regression in the real write path is now caught
/// (it previously used a hand-rolled hound loop — see DOLL-192).
fn run_pipeline(
    dir: &str,
    sample_rate: u32,
    num_channels: usize,
    total_frames: usize,
    chunk_data: &[f32],
) {
    eprintln!(
        "Running real pipeline: producer → ring buffer → production writer thread (RawWavWriter)..."
    );

    let (elapsed, errors) =
        blackbox::bench_real_pipeline(dir, sample_rate, num_channels, total_frames, chunk_data);

    report_results(num_channels, total_frames, elapsed, errors, sample_rate);
}

fn report_results(
    channels: usize,
    frames: usize,
    elapsed: std::time::Duration,
    errors: u64,
    sample_rate: u32,
) {
    let total_samples = frames * channels;
    let samples_per_sec = count_to_f64(total_samples) / elapsed.as_secs_f64();
    let realtime_rate = f64::from(sample_rate) * count_to_f64(channels);
    let realtime_multiple = samples_per_sec / realtime_rate;

    eprintln!();
    eprintln!("  Elapsed:     {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    eprintln!(
        "  Throughput:  {:.2}M samples/s",
        samples_per_sec / 1_000_000.0
    );
    eprintln!("  Real-time:   {realtime_multiple:.1}x (need >1.0x)");
    eprintln!("  Errors:      {errors}");
    eprintln!();
}
