use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use tempfile::tempdir;

use crate::constants::RING_BUFFER_SECONDS;
use crate::writer_thread::{WriterCommand, WriterThreadState, writer_thread_main};

// ===========================================================================
// Helpers
// ===========================================================================

fn test_env_no_silence() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env = crate::tests::default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));
    env
}

/// Generate `frames` of interleaved f32 data for `total_channels` channels.
/// Uses a simple sine wave pattern — realistic enough for I/O benchmarks.
fn generate_bench_data(total_channels: usize, frames: usize) -> Vec<f32> {
    let total = total_channels * frames;
    let mut data = vec![0.0_f32; total];
    for (i, sample) in data.iter_mut().enumerate() {
        *sample = ((i as f32) * 0.01).sin() * 0.5;
    }
    data
}

/// Format a sample rate as a human-readable string.
fn format_rate(samples_per_sec: f64) -> String {
    if samples_per_sec >= 1_000_000.0 {
        format!("{:.2}M samples/s", samples_per_sec / 1_000_000.0)
    } else if samples_per_sec >= 1_000.0 {
        format!("{:.1}K samples/s", samples_per_sec / 1_000.0)
    } else {
        format!("{:.0} samples/s", samples_per_sec)
    }
}

// ===========================================================================
// Benchmark 1: Direct WriterThreadState throughput
//
// Measures raw write_samples() speed — how fast can we push data through
// hound's WAV writers without any threading overhead.
// ===========================================================================

#[test]
#[ignore = "manual benchmark — run with: cargo test benchmark -- --ignored --nocapture"]
fn benchmark_direct_write_throughput() {
    let channel_counts: &[usize] = &[1, 2, 8, 16, 32, 64];
    let sample_rate: u32 = 48000;
    // Simulate 10 seconds of audio per run
    let duration_secs = 10;

    println!("\n============================================================");
    println!("  Benchmark: Direct WriterThreadState throughput");
    println!("  (measures raw hound write_sample speed, no threading)");
    println!("============================================================");
    println!(
        "  {:>4} {:>10} {:>15} {:>12} {:>10}",
        "Ch", "Frames", "Time (ms)", "Samples/s", "Realtime"
    );
    println!(
        "  {:-<4} {:-<10} {:-<15} {:-<12} {:-<10}",
        "", "", "", "", ""
    );

    temp_env::with_vars(test_env_no_silence(), || {
        for &ch_count in channel_counts {
            let temp_dir = tempdir().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let write_errors = Arc::new(AtomicU64::new(0));

            let channels: Vec<usize> = (0..ch_count).collect();
            // Use "single" mode for >2 channels (multichannel), split for benchmarking split too
            let output_mode = "single";

            let mut state = WriterThreadState::new(
                dir,
                sample_rate,
                &channels,
                output_mode,
                0.0,
                Arc::clone(&write_errors),
                0,
                Arc::new(AtomicBool::new(false)),
                16,
                Arc::new((0..ch_count).map(|_| AtomicU32::new(0)).collect()),
            )
            .unwrap();
            state.total_device_channels = ch_count;

            let frames = sample_rate as usize * duration_secs;
            let data = generate_bench_data(ch_count, frames);

            // Warm up — write a small chunk first
            let warmup = generate_bench_data(ch_count, 1000);
            state.write_samples(&warmup);

            // Benchmark
            let start = Instant::now();
            // Feed in realistic chunk sizes (512 frames = typical cpal callback)
            let chunk_samples = 512 * ch_count;
            for chunk in data.chunks(chunk_samples) {
                state.write_samples(chunk);
            }
            let elapsed = start.elapsed();

            let total_samples = frames * ch_count;
            let samples_per_sec = total_samples as f64 / elapsed.as_secs_f64();
            // Real-time rate = sample_rate * ch_count samples/sec
            let realtime_rate = sample_rate as f64 * ch_count as f64;
            let realtime_multiple = samples_per_sec / realtime_rate;

            let errors = write_errors.load(Ordering::Relaxed);

            println!(
                "  {:>4} {:>10} {:>12.1} ms {:>12} {:>8.1}x{}",
                ch_count,
                frames,
                elapsed.as_secs_f64() * 1000.0,
                format_rate(samples_per_sec),
                realtime_multiple,
                if errors > 0 {
                    format!("  ({} errors)", errors)
                } else {
                    String::new()
                }
            );

            // Finalize to close files properly
            let _ = state.finalize_all();
        }
    });

    println!();
}

// ===========================================================================
// Benchmark 2: Split mode throughput (worst case for file I/O)
//
// Split mode writes to N separate files simultaneously — tests file handle
// fan-out overhead.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_split_mode_throughput() {
    let channel_counts: &[usize] = &[2, 8, 16, 32, 64];
    let sample_rate: u32 = 48000;
    let duration_secs = 10;

    println!("\n============================================================");
    println!("  Benchmark: Split mode throughput (1 file per channel)");
    println!("  (worst case: N simultaneous file handles)");
    println!("============================================================");
    println!(
        "  {:>4} {:>6} {:>15} {:>12} {:>10}",
        "Ch", "Files", "Time (ms)", "Samples/s", "Realtime"
    );
    println!(
        "  {:-<4} {:-<6} {:-<15} {:-<12} {:-<10}",
        "", "", "", "", ""
    );

    temp_env::with_vars(test_env_no_silence(), || {
        for &ch_count in channel_counts {
            let temp_dir = tempdir().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let write_errors = Arc::new(AtomicU64::new(0));

            let channels: Vec<usize> = (0..ch_count).collect();

            let mut state = WriterThreadState::new(
                dir,
                sample_rate,
                &channels,
                "split",
                0.0,
                Arc::clone(&write_errors),
                0,
                Arc::new(AtomicBool::new(false)),
                16,
                Arc::new((0..ch_count).map(|_| AtomicU32::new(0)).collect()),
            )
            .unwrap();
            state.total_device_channels = ch_count;

            let frames = sample_rate as usize * duration_secs;
            let data = generate_bench_data(ch_count, frames);

            let start = Instant::now();
            let chunk_samples = 512 * ch_count;
            for chunk in data.chunks(chunk_samples) {
                state.write_samples(chunk);
            }
            let elapsed = start.elapsed();

            let total_samples = frames * ch_count;
            let samples_per_sec = total_samples as f64 / elapsed.as_secs_f64();
            let realtime_rate = sample_rate as f64 * ch_count as f64;
            let realtime_multiple = samples_per_sec / realtime_rate;

            println!(
                "  {:>4} {:>6} {:>12.1} ms {:>12} {:>8.1}x",
                ch_count,
                ch_count,
                elapsed.as_secs_f64() * 1000.0,
                format_rate(samples_per_sec),
                realtime_multiple,
            );

            let _ = state.finalize_all();
        }
    });

    println!();
}

// ===========================================================================
// Benchmark 3: Full ring buffer pipeline (producer → ring buffer → writer thread)
//
// Simulates the real production path: a "callback" thread pushes data into
// the ring buffer, the writer thread reads and writes WAV.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_ring_buffer_pipeline() {
    let channel_counts: &[usize] = &[1, 2, 8, 16, 32, 64];
    let sample_rate: u32 = 48000;
    let duration_secs = 10;

    println!("\n============================================================");
    println!("  Benchmark: Full ring buffer pipeline");
    println!("  (producer thread → rtrb → writer thread → WAV)");
    println!("============================================================");
    println!(
        "  {:>4} {:>12} {:>15} {:>12} {:>10} {:>8}",
        "Ch", "Ring size", "Time (ms)", "Samples/s", "Realtime", "Drops"
    );
    println!(
        "  {:-<4} {:-<12} {:-<15} {:-<12} {:-<10} {:-<8}",
        "", "", "", "", "", ""
    );

    temp_env::with_vars(test_env_no_silence(), || {
        for &ch_count in channel_counts {
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
                16,
                Arc::new((0..ch_count).map(|_| AtomicU32::new(0)).collect()),
            )
            .unwrap();
            state.total_device_channels = ch_count;

            let ring_size = sample_rate as usize * ch_count * RING_BUFFER_SECONDS;
            let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

            let rotation_needed = Arc::new(AtomicBool::new(false));
            let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

            let rotation_clone = Arc::clone(&rotation_needed);
            let writer_handle = std::thread::Builder::new()
                .name("bench-writer".to_string())
                .spawn(move || {
                    writer_thread_main(consumer, rotation_clone, command_rx, state);
                })
                .unwrap();

            let frames = sample_rate as usize * duration_secs;
            let data = generate_bench_data(ch_count, frames);
            let chunk_samples = 512 * ch_count; // Simulate typical cpal callback size

            let write_errors_cb = Arc::clone(&write_errors);
            let start = Instant::now();

            // Simulate audio callback: push chunks at roughly real-time pace
            // (but as fast as possible — we're measuring max throughput)
            for chunk in data.chunks(chunk_samples) {
                if let Ok(write_chunk) = producer.write_chunk_uninit(chunk.len()) {
                    write_chunk.fill_from_iter(chunk.iter().copied());
                } else {
                    // Ring buffer full — count drops like the real callback does
                    write_errors_cb.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    // Brief yield to let writer thread catch up
                    std::thread::yield_now();
                }
            }

            // Shutdown writer thread (will drain remaining samples)
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
            reply_rx.recv().unwrap().unwrap();
            let elapsed = start.elapsed();
            writer_handle.join().unwrap();

            let total_samples = frames * ch_count;
            let samples_per_sec = total_samples as f64 / elapsed.as_secs_f64();
            let realtime_rate = sample_rate as f64 * ch_count as f64;
            let realtime_multiple = samples_per_sec / realtime_rate;
            let drops = write_errors.load(Ordering::Relaxed);

            println!(
                "  {:>4} {:>9.1}MB {:>12.1} ms {:>12} {:>8.1}x {:>8}",
                ch_count,
                (ring_size * 4) as f64 / (1024.0 * 1024.0),
                elapsed.as_secs_f64() * 1000.0,
                format_rate(samples_per_sec),
                realtime_multiple,
                drops,
            );
        }
    });

    println!();
}

// ===========================================================================
// Benchmark 4: File rotation overhead
//
// Measures how long rotate_files() takes at various channel counts.
// This blocks the writer thread — the ring buffer absorbs the pause.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_rotation_overhead() {
    let channel_counts: &[usize] = &[1, 2, 8, 16, 32, 64];
    let sample_rate: u32 = 48000;

    println!("\n============================================================");
    println!("  Benchmark: File rotation overhead");
    println!("  (finalize + rename + silence check + new files)");
    println!("============================================================");
    println!(
        "  {:>4} {:>8} {:>15} {:>20}",
        "Ch", "Mode", "Rotation (ms)", "Ring buffer runway"
    );
    println!("  {:-<4} {:-<8} {:-<15} {:-<20}", "", "", "", "");

    temp_env::with_vars(test_env_no_silence(), || {
        for &ch_count in channel_counts {
            // Test both single and split modes
            for mode in &["single", "split"] {
                let temp_dir = tempdir().unwrap();
                let dir = temp_dir.path().to_str().unwrap();
                let write_errors = Arc::new(AtomicU64::new(0));

                let channels: Vec<usize> = (0..ch_count).collect();

                let mut state = WriterThreadState::new(
                    dir,
                    sample_rate,
                    &channels,
                    mode,
                    0.0, // No silence detection — measure just I/O
                    Arc::clone(&write_errors),
                    0,
                    Arc::new(AtomicBool::new(false)),
                    16,
                    Arc::new((0..ch_count).map(|_| AtomicU32::new(0)).collect()),
                )
                .unwrap();
                state.total_device_channels = ch_count;

                // Write some data first so the files have content
                let data = generate_bench_data(ch_count, sample_rate as usize * 5);
                state.write_samples(&data);

                // Need to sleep 1s so the new file gets a different timestamp
                std::thread::sleep(std::time::Duration::from_millis(1100));

                // Benchmark rotation
                let start = Instant::now();
                state.rotate_files();
                let elapsed = start.elapsed();

                let ring_buffer_ms =
                    (RING_BUFFER_SECONDS as f64).mul_add(1000.0, -elapsed.as_secs_f64() * 1000.0);

                println!(
                    "  {:>4} {:>8} {:>12.2} ms {:>14.0} ms left",
                    ch_count,
                    mode,
                    elapsed.as_secs_f64() * 1000.0,
                    ring_buffer_ms,
                );

                let _ = state.finalize_all();
            }
        }
    });

    println!();
}
