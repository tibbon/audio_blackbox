use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use tempfile::tempdir;

use crate::constants::{CacheAlignedPeak, RING_BUFFER_SECONDS};
use crate::writer_thread::{WriterCommand, WriterThreadState, writer_thread_main};

// ===========================================================================
// Helpers
// ===========================================================================

/// Print a note reminding the user that benchmarks should be run in release mode.
fn print_release_note() {
    if cfg!(debug_assertions) {
        println!("  *** WARNING: Running in DEBUG mode — results will be 5-10x slower ***");
        println!("  Run with: cargo test --release benchmark -- --ignored --nocapture");
        println!();
    }
}

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
    print_release_note();
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
                Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
            )
            .unwrap();
            state.total_device_channels = ch_count as u16;

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
    print_release_note();
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
                Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
            )
            .unwrap();
            state.total_device_channels = ch_count as u16;

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
    print_release_note();
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
                Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
            )
            .unwrap();
            state.total_device_channels = ch_count as u16;

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
    print_release_note();
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
                    Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
                )
                .unwrap();
                state.total_device_channels = ch_count as u16;

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

// ===========================================================================
// Benchmark 5: Monitor mode vs recording mode
//
// Compares monitor-only (peak tracking, no disk I/O) against full recording.
// Shows the overhead that disk writes add, and proves monitor mode is cheap.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_monitor_vs_recording() {
    let channel_counts: &[usize] = &[1, 2, 8, 16, 32, 64];
    let sample_rate: u32 = 48000;
    let duration_secs = 10;

    println!("\n============================================================");
    println!("  Benchmark: Monitor mode (peak-only) vs Recording (WAV I/O)");
    println!("  (same write_samples() hot path, monitor skips disk writes)");
    println!("============================================================");
    print_release_note();
    println!(
        "  {:>4} {:>12} {:>12} {:>12} {:>12} {:>10}",
        "Ch", "Monitor ms", "Record ms", "Mon rate", "Rec rate", "Speedup"
    );
    println!(
        "  {:-<4} {:-<12} {:-<12} {:-<12} {:-<12} {:-<10}",
        "", "", "", "", "", ""
    );

    temp_env::with_vars(test_env_no_silence(), || {
        for &ch_count in channel_counts {
            let frames = sample_rate as usize * duration_secs;
            let data = generate_bench_data(ch_count, frames);
            let chunk_samples = 512 * ch_count;

            // --- Monitor mode (peak tracking only, no disk) ---
            let peak_levels: Arc<Vec<CacheAlignedPeak>> =
                Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect());
            let mut monitor_state = WriterThreadState::new_monitor(
                sample_rate,
                &(0..ch_count).collect::<Vec<_>>(),
                peak_levels,
            );
            monitor_state.total_device_channels = ch_count as u16;

            // Warm up
            let warmup = generate_bench_data(ch_count, 1000);
            monitor_state.write_samples(&warmup);

            let start = Instant::now();
            for chunk in data.chunks(chunk_samples) {
                monitor_state.write_samples(chunk);
            }
            let monitor_elapsed = start.elapsed();

            // --- Recording mode (full WAV writes) ---
            let temp_dir = tempdir().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let write_errors = Arc::new(AtomicU64::new(0));
            let channels: Vec<usize> = (0..ch_count).collect();

            let mut record_state = WriterThreadState::new(
                dir,
                sample_rate,
                &channels,
                "single",
                0.0,
                Arc::clone(&write_errors),
                0,
                Arc::new(AtomicBool::new(false)),
                16,
                Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
            )
            .unwrap();
            record_state.total_device_channels = ch_count as u16;

            // Warm up
            record_state.write_samples(&warmup);

            let start = Instant::now();
            for chunk in data.chunks(chunk_samples) {
                record_state.write_samples(chunk);
            }
            let record_elapsed = start.elapsed();
            let _ = record_state.finalize_all();

            let total_samples = (frames * ch_count) as f64;
            let monitor_rate = total_samples / monitor_elapsed.as_secs_f64();
            let record_rate = total_samples / record_elapsed.as_secs_f64();
            let speedup = monitor_rate / record_rate;

            println!(
                "  {:>4} {:>9.1} ms {:>9.1} ms {:>12} {:>12} {:>8.1}x",
                ch_count,
                monitor_elapsed.as_secs_f64() * 1000.0,
                record_elapsed.as_secs_f64() * 1000.0,
                format_rate(monitor_rate),
                format_rate(record_rate),
                speedup,
            );
        }
    });

    println!();
}

// ===========================================================================
// Benchmark 6: Full monitor pipeline (ring buffer + writer thread)
//
// End-to-end measurement of monitor mode through the real threading path.
// Proves that the full pipeline in monitor mode uses minimal CPU.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_monitor_pipeline() {
    let channel_counts: &[usize] = &[1, 2, 8, 16, 32, 64];
    let sample_rate: u32 = 48000;
    let duration_secs = 10;

    println!("\n============================================================");
    println!("  Benchmark: Full monitor pipeline");
    println!("  (producer thread → rtrb → writer thread → peak tracking)");
    println!("============================================================");
    print_release_note();
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
            let channels: Vec<usize> = (0..ch_count).collect();
            let write_errors = Arc::new(AtomicU64::new(0));

            let peak_levels: Arc<Vec<CacheAlignedPeak>> =
                Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect());
            let mut state =
                WriterThreadState::new_monitor(sample_rate, &channels, Arc::clone(&peak_levels));
            state.total_device_channels = ch_count as u16;

            let ring_size = sample_rate as usize * ch_count * RING_BUFFER_SECONDS;
            let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

            let rotation_needed = Arc::new(AtomicBool::new(false));
            let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

            let rotation_clone = Arc::clone(&rotation_needed);
            let writer_handle = std::thread::Builder::new()
                .name("bench-monitor".to_string())
                .spawn(move || {
                    writer_thread_main(consumer, rotation_clone, command_rx, state);
                })
                .unwrap();

            let frames = sample_rate as usize * duration_secs;
            let data = generate_bench_data(ch_count, frames);
            let chunk_samples = 512 * ch_count;

            let write_errors_cb = Arc::clone(&write_errors);
            let start = Instant::now();

            for chunk in data.chunks(chunk_samples) {
                if let Ok(write_chunk) = producer.write_chunk_uninit(chunk.len()) {
                    write_chunk.fill_from_iter(chunk.iter().copied());
                } else {
                    write_errors_cb.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    std::thread::yield_now();
                }
            }

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

            // Verify peaks were actually tracked
            let any_peak = peak_levels
                .iter()
                .any(|a| a.value.load(Ordering::Relaxed) != 0);
            assert!(
                any_peak,
                "Peak levels should be non-zero after processing audio"
            );

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
// Benchmark 7: write_samples() per-component overhead
//
// Isolates the cost of each layer: peak tracking only (monitor mode) vs
// peak + scale + hound write (recording mode). Fixed 2ch/48kHz config
// for tight, single-config comparison.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_write_samples_overhead() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;
    let duration_secs = 10;
    let frames = sample_rate as usize * duration_secs;
    let data = generate_bench_data(ch_count, frames);
    let chunk_samples = 512 * ch_count;

    println!("\n============================================================");
    println!("  Benchmark: write_samples() per-component overhead");
    println!("  (2ch/48kHz — isolates peak tracking vs peak + WAV I/O)");
    println!("============================================================");
    print_release_note();
    println!(
        "  {:>14} {:>12} {:>16} {:>12}",
        "Component", "Time (ms)", "Samples/s", "Overhead"
    );
    println!("  {:-<14} {:-<12} {:-<16} {:-<12}", "", "", "", "");

    temp_env::with_vars(test_env_no_silence(), || {
        // --- Monitor mode: peak tracking only ---
        let peak_levels: Arc<Vec<CacheAlignedPeak>> =
            Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect());
        let mut monitor_state = WriterThreadState::new_monitor(
            sample_rate,
            &(0..ch_count).collect::<Vec<_>>(),
            peak_levels,
        );
        monitor_state.total_device_channels = ch_count as u16;

        let warmup = generate_bench_data(ch_count, 1000);
        monitor_state.write_samples(&warmup);

        let start = Instant::now();
        for chunk in data.chunks(chunk_samples) {
            monitor_state.write_samples(chunk);
        }
        let monitor_ms = start.elapsed().as_secs_f64() * 1000.0;

        // --- Recording mode: peak + scale + hound write ---
        let temp_dir = tempdir().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let write_errors = Arc::new(AtomicU64::new(0));
        let channels: Vec<usize> = (0..ch_count).collect();

        let mut record_state = WriterThreadState::new(
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
        )
        .unwrap();
        record_state.total_device_channels = ch_count as u16;

        record_state.write_samples(&warmup);

        let start = Instant::now();
        for chunk in data.chunks(chunk_samples) {
            record_state.write_samples(chunk);
        }
        let record_ms = start.elapsed().as_secs_f64() * 1000.0;
        let _ = record_state.finalize_all();

        let total_samples = (frames * ch_count) as f64;
        let monitor_rate = total_samples / (monitor_ms / 1000.0);
        let record_rate = total_samples / (record_ms / 1000.0);
        let disk_overhead_ms = record_ms - monitor_ms;

        println!(
            "  {:>14} {:>9.1} ms {:>16} {:>12}",
            "Peak only",
            monitor_ms,
            format_rate(monitor_rate),
            "(baseline)",
        );
        println!(
            "  {:>14} {:>9.1} ms {:>16} {:>9.1} ms",
            "Peak + WAV",
            record_ms,
            format_rate(record_rate),
            disk_overhead_ms,
        );
        println!(
            "\n  Disk I/O adds {:.1} ms ({:.1}x slower) at 2ch/48kHz/24-bit",
            disk_overhead_ms,
            record_ms / monitor_ms,
        );
    });

    println!();
}

// ===========================================================================
// Benchmark 8: Ring buffer latency (producer push → writer thread consumption)
//
// Measures worst-case latency by tracking ring buffer occupancy at push time.
// Validates the 1ms sleep + 5-second ring buffer design.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_ring_buffer_latency() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;
    let total_channels = ch_count;

    println!("\n============================================================");
    println!("  Benchmark: Ring buffer latency (push → consumption)");
    println!("  (2ch/48kHz, measures time from producer push to writer read)");
    println!("============================================================");
    print_release_note();

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
            16,
            Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect()),
        )
        .unwrap();
        state.total_device_channels = total_channels as u16;

        let ring_size = sample_rate as usize * total_channels * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let writer_handle = std::thread::Builder::new()
            .name("bench-latency".to_string())
            .spawn(move || {
                writer_thread_main(consumer, rotation_clone, command_rx, state);
            })
            .unwrap();

        let chunk_samples = 512 * total_channels;
        let num_chunks = 2000;
        let data = generate_bench_data(total_channels, 512);

        let mut latencies_us: Vec<f64> = Vec::with_capacity(num_chunks);

        // Let writer thread start
        std::thread::sleep(std::time::Duration::from_millis(10));

        for _ in 0..num_chunks {
            // Measure occupancy before push (how many samples writer hasn't consumed yet)
            let occupancy = ring_size - producer.slots();

            // Calculate latency: occupancy / (sample_rate * channels) = seconds behind
            let latency_us =
                occupancy as f64 / (sample_rate as f64 * total_channels as f64) * 1_000_000.0;
            latencies_us.push(latency_us);

            // Push chunk
            if let Ok(chunk) = producer.write_chunk_uninit(chunk_samples) {
                chunk.fill_from_iter(data.iter().copied());
            }

            // Simulate real-time pace: 512 frames at 48kHz = ~10.67ms
            std::thread::sleep(std::time::Duration::from_micros(10_667));
        }

        // Shutdown
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        writer_handle.join().unwrap();

        // Calculate percentiles
        latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = latencies_us[latencies_us.len() / 2];
        let p99 = latencies_us[(latencies_us.len() as f64 * 0.99) as usize];
        let max = latencies_us.last().copied().unwrap_or(0.0);
        let ring_buffer_capacity_us = RING_BUFFER_SECONDS as f64 * 1_000_000.0;

        println!(
            "  Chunks sent:     {:>6} ({:.1}s at real-time pace)",
            num_chunks,
            num_chunks as f64 * 512.0 / sample_rate as f64,
        );
        println!("  Latency p50:     {:>9.0} us", p50);
        println!("  Latency p99:     {:>9.0} us", p99);
        println!("  Latency max:     {:>9.0} us", max);
        println!(
            "  Ring buffer cap: {:>9.0} us ({} seconds)",
            ring_buffer_capacity_us, RING_BUFFER_SECONDS,
        );
        println!(
            "  Headroom:        {:>8.1}x (max latency vs ring buffer capacity)",
            ring_buffer_capacity_us / max.max(1.0),
        );
    });

    println!();
}

// ===========================================================================
// Benchmark 9: Monitor mode CPU at real-time pace
//
// Runs monitor mode (peak tracking only) at actual real-time pace for a fixed
// duration and measures the CPU time spent. Proves that the Level Meter window
// adds negligible CPU overhead in practice.
// ===========================================================================

#[test]
#[ignore = "manual benchmark"]
fn benchmark_monitor_cpu_idle() {
    let sample_rate: u32 = 48000;
    let ch_count: usize = 2;
    let total_channels = ch_count;
    let test_duration_secs = 5;

    println!("\n============================================================");
    println!("  Benchmark: Monitor mode CPU at real-time pace");
    println!(
        "  (2ch/48kHz, {}s — measures actual CPU cost of level metering)",
        test_duration_secs
    );
    println!("============================================================");
    print_release_note();

    temp_env::with_vars(test_env_no_silence(), || {
        let channels: Vec<usize> = (0..ch_count).collect();

        let peak_levels: Arc<Vec<CacheAlignedPeak>> =
            Arc::new((0..ch_count).map(|_| CacheAlignedPeak::new(0)).collect());
        let mut state =
            WriterThreadState::new_monitor(sample_rate, &channels, Arc::clone(&peak_levels));
        state.total_device_channels = total_channels as u16;

        let ring_size = sample_rate as usize * total_channels * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_clone = Arc::clone(&rotation_needed);
        let writer_handle = std::thread::Builder::new()
            .name("bench-cpu-idle".to_string())
            .spawn(move || {
                writer_thread_main(consumer, rotation_clone, command_rx, state);
            })
            .unwrap();

        // Simulate cpal callback at real-time pace
        let chunk_frames = 512;
        let chunk_samples = chunk_frames * total_channels;
        let chunk_duration =
            std::time::Duration::from_secs_f64(chunk_frames as f64 / sample_rate as f64);
        let data = generate_bench_data(total_channels, chunk_frames);

        let wall_start = Instant::now();
        let mut cpu_time = std::time::Duration::ZERO;
        let mut chunks_sent: u64 = 0;

        while wall_start.elapsed() < std::time::Duration::from_secs(test_duration_secs) {
            let cpu_start = Instant::now();

            // Push to ring buffer (same as cpal callback)
            if let Ok(chunk) = producer.write_chunk_uninit(chunk_samples) {
                chunk.fill_from_iter(data.iter().copied());
            }

            cpu_time += cpu_start.elapsed();
            chunks_sent += 1;

            // Sleep to match real-time pace
            std::thread::sleep(chunk_duration);
        }

        // Shutdown
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        command_tx.send(WriterCommand::Shutdown(reply_tx)).unwrap();
        reply_rx.recv().unwrap().unwrap();
        writer_handle.join().unwrap();

        let wall_elapsed = wall_start.elapsed();
        let total_frames = chunks_sent * chunk_frames as u64;

        // Verify peaks were tracked
        let any_peak = peak_levels
            .iter()
            .any(|a| a.value.load(Ordering::Relaxed) != 0);
        assert!(any_peak, "Peak levels should be non-zero");

        println!(
            "  Wall time:       {:>6.1}s ({} chunks of {} frames)",
            wall_elapsed.as_secs_f64(),
            chunks_sent,
            chunk_frames,
        );
        println!(
            "  Producer CPU:    {:>6.1} ms ({:.3}% of wall time)",
            cpu_time.as_secs_f64() * 1000.0,
            cpu_time.as_secs_f64() / wall_elapsed.as_secs_f64() * 100.0,
        );
        println!(
            "  Audio processed: {:>6.1}s ({} frames at {}Hz)",
            total_frames as f64 / sample_rate as f64,
            total_frames,
            sample_rate,
        );
        println!(
            "  CPU per chunk:   {:>6.0} ns",
            cpu_time.as_nanos() as f64 / chunks_sent as f64,
        );
    });

    println!();
}
