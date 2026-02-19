//! Standalone benchmark binary for profiling the writer thread hot path.
//!
//! Usage:
//!   cargo build --release --bin bench-writer
//!   samply record target/release/bench-writer [OPTIONS]
//!
//! Options:
//!   --channels N     Number of channels (default: 64)
//!   --seconds N      Duration of simulated audio (default: 30)
//!   --mode MODE      "single", "split", or "pipeline" (default: single)
//!   --sample-rate N  Sample rate in Hz (default: 48000)

// Benchmark binary — suppress pedantic lints that don't matter here.
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::tuple_array_conversions)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use blackbox::RING_BUFFER_SECONDS;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut channels: usize = 64;
    let mut seconds: usize = 30;
    let mut mode = "single".to_string();
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
        "Benchmark: {} mode, {} channels, {} seconds ({} frames, {} total samples)",
        mode, channels, seconds, frames, total_samples
    );
    eprintln!("Output dir: {}", dir);
    eprintln!("Sample rate: {} Hz", sample_rate);
    eprintln!();

    // Generate test data — interleaved f32, 512-frame chunks (typical cpal callback)
    let chunk_frames = 512;
    let chunk_samples = chunk_frames * channels;
    let mut chunk_data = vec![0.0_f32; chunk_samples];
    for (i, sample) in chunk_data.iter_mut().enumerate() {
        *sample = ((i as f32) * 0.01).sin() * 0.5;
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
            let path = format!("{}/bench-ch{}.recording.wav", dir, ch);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            writers.push(Some(hound::WavWriter::create(&path, spec).unwrap()));
        }

        eprintln!(
            "Writing {} frames in split mode ({} files)...",
            total_frames, num_channels
        );
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
                        let sample = (frame[channel] * 32767.0) as i32;
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
            &write_errors,
            sample_rate,
        );

        for w in &mut writers {
            if let Some(writer) = w.take() {
                let _ = writer.finalize();
            }
        }
    } else {
        let path = format!("{}/bench.recording.wav", dir);
        let spec = hound::WavSpec {
            channels: num_channels as u16,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();

        eprintln!(
            "Writing {} frames in single/multichannel mode...",
            total_frames
        );
        let start = Instant::now();

        let mut frames_written = 0;
        while frames_written < total_frames {
            let frames_this_chunk = chunk_frames.min(total_frames - frames_written);
            let samples_this_chunk = frames_this_chunk * num_channels;
            let data = &chunk_data[..samples_this_chunk];

            for frame in data.chunks(num_channels) {
                for &channel in &channel_indices {
                    if channel < frame.len() {
                        let sample = (frame[channel] * 32767.0) as i32;
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
            &write_errors,
            sample_rate,
        );

        let _ = writer.finalize();
    }
}

/// Full pipeline benchmark — producer pushes into rtrb, writer thread reads and writes WAV.
fn run_pipeline(
    dir: &str,
    sample_rate: u32,
    num_channels: usize,
    total_frames: usize,
    chunk_data: &[f32],
) {
    let chunk_frames = chunk_data.len() / num_channels;
    let write_errors = Arc::new(AtomicU64::new(0));

    let channel_indices: Vec<usize> = (0..num_channels).collect();
    let ring_size = sample_rate as usize * num_channels * RING_BUFFER_SECONDS;

    let (mut producer, mut consumer) = rtrb::RingBuffer::new(ring_size);

    let spec = hound::WavSpec {
        channels: num_channels as u16,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let path = format!("{}/bench-pipeline.recording.wav", dir);
    let writer = hound::WavWriter::create(&path, spec).unwrap();

    let write_errors_writer = Arc::clone(&write_errors);
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_writer = Arc::clone(&shutdown);

    let writer_handle = std::thread::Builder::new()
        .name("bench-writer".to_string())
        .spawn(move || {
            let mut writer = writer;
            let read_chunk_size = 4096_usize;

            loop {
                let available = consumer.slots();
                if available > 0 {
                    let to_read = available.min(read_chunk_size);
                    if let Ok(chunk) = consumer.read_chunk(to_read) {
                        let (first, second) = chunk.as_slices();

                        for slice in [first, second] {
                            for frame in slice.chunks(num_channels) {
                                for &channel in &channel_indices {
                                    if channel < frame.len() {
                                        let sample = (frame[channel] * 32767.0) as i32;
                                        if writer.write_sample(sample).is_err() {
                                            write_errors_writer.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                }
                            }
                        }
                        chunk.commit_all();
                    }
                } else if shutdown_writer.load(Ordering::Acquire) {
                    break;
                } else {
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            }

            let _ = writer.finalize();
        })
        .unwrap();

    eprintln!(
        "Running pipeline: producer → ring buffer ({:.1}MB) → writer thread...",
        (ring_size * 4) as f64 / (1024.0 * 1024.0)
    );

    let start = Instant::now();
    let mut frames_written = 0;
    let mut drops: u64 = 0;

    while frames_written < total_frames {
        let frames_this_chunk = chunk_frames.min(total_frames - frames_written);
        let samples_this_chunk = frames_this_chunk * num_channels;
        let data = &chunk_data[..samples_this_chunk];

        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
        } else {
            drops += data.len() as u64;
            std::thread::yield_now();
            continue;
        }

        frames_written += frames_this_chunk;
    }

    shutdown.store(true, Ordering::Release);
    writer_handle.join().unwrap();

    let elapsed = start.elapsed();
    report_results(
        num_channels,
        total_frames,
        elapsed,
        &write_errors,
        sample_rate,
    );
    if drops > 0 {
        eprintln!("  Ring buffer back-pressure retries: {drops} samples");
    }
}

fn report_results(
    channels: usize,
    frames: usize,
    elapsed: std::time::Duration,
    write_errors: &Arc<AtomicU64>,
    sample_rate: u32,
) {
    let total_samples = frames * channels;
    let samples_per_sec = total_samples as f64 / elapsed.as_secs_f64();
    let realtime_rate = f64::from(sample_rate) * channels as f64;
    let realtime_multiple = samples_per_sec / realtime_rate;
    let errors = write_errors.load(Ordering::Relaxed);

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
