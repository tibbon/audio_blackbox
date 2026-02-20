# BlackBox Audio Recorder

[![CI](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml/badge.svg)](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml)

A cross-platform audio recording application in Rust with macOS menu bar integration. Records from configurable input channels, saves as WAV files, supports silence detection, continuous recording with automatic file rotation, and handles up to 64+ channels with real-time performance.

## Features

- **Multi-channel recording** — record 1 to 64+ channels simultaneously
- **Configurable bit depth** — 16-bit, 24-bit (default, pro standard), or 32-bit WAV
- **Two output modes** — `single` (one file, automatically multichannel for 3+ channels), `split` (one file per channel)
- **Continuous recording** — automatic file rotation at configurable intervals with crash-safe WAV writes
- **Silence detection** — automatically deletes silent recordings on rotation
- **Disk space monitoring** — automatically stops recording when free space drops below a configurable threshold
- **Lock-free RT architecture** — audio callback uses zero file I/O, zero mutex locks, zero allocations; all writes happen on a dedicated writer thread via a SPSC ring buffer
- **Per-channel peak metering** — tracked on the writer thread at zero extra cost, exposed via FFI
- **macOS menu bar app** — native SwiftUI menu bar app with onboarding, settings, and security-scoped bookmarks
- **Flexible configuration** — TOML config file, environment variables, or `BLACKBOX_*` prefixed env vars

## Building

### Prerequisites

- Rust stable toolchain (edition 2024)
- **macOS**: Xcode command line tools
- **Linux**: `libasound2-dev` and `pkg-config`

### Commands

```bash
cargo build                          # Debug build
cargo build --release                # Release build
cargo test                           # Run all tests (125 total)
cargo clippy --all-targets --no-default-features -- -D warnings  # Lint (matches CI)
cargo fmt --all -- --check           # Format check
```

## Configuration

Configure via `blackbox.toml`, environment variables, or `BLACKBOX_*` prefixed env vars. Environment variables take precedence over the config file.

```toml
# Output mode: "single" (one file) or "split" (one file per channel)
output_mode = "single"

# Audio channels to record (comma-separated or ranges)
audio_channels = "0"

# Bit depth: 16, 24 (default), or 32
bits_per_sample = 24

# Recording duration in seconds
duration = 30

# Output directory for recordings
output_dir = "recordings"

# Silence threshold (0.0 to disable)
silence_threshold = 0.01

# Continuous recording mode
continuous_mode = false

# File rotation cadence in seconds (continuous mode)
recording_cadence = 300

# Minimum free disk space in MB (0 to disable)
min_disk_space_mb = 500
```

Channel specs support individual channels and ranges: `"0,2-4,7"` records channels 0, 2, 3, 4, and 7.

## Running

```bash
cargo run                            # Run with defaults (blackbox binary)
cargo run -- --menu-bar              # Run with macOS menu bar UI
cargo run --bin bench-writer -- --channels 64 --seconds 10 --mode single  # Run benchmarks
```

## Architecture

The core recording pipeline uses a lock-free architecture to prevent audio glitches:

```
Audio Device → cpal callback (RT thread) → rtrb ring buffer → Writer thread → WAV files
```

- **RT callback**: only pushes raw f32 samples into the ring buffer and checks an `AtomicBool` for rotation timing. No file I/O, no mutexes, no allocations.
- **Writer thread**: reads from the ring buffer, converts f32 to the configured bit depth, writes WAV via hound, tracks per-channel peak levels, handles file rotation and disk space monitoring.
- **Ring buffer**: sized for 5 seconds of audio at the device's sample rate and channel count, providing ample runway for file rotation I/O even at high channel counts.

### Key modules

| Module | Purpose |
|--------|---------|
| `src/audio_processor.rs` | `AudioProcessor` trait — central abstraction |
| `src/cpal_processor.rs` | Real audio I/O implementation using cpal |
| `src/writer_thread.rs` | Writer thread, ring buffer consumer, WAV file management |
| `src/config.rs` | TOML + env var configuration with `BLACKBOX_*` prefix support |
| `src/constants.rs` | Default values, type aliases |
| `src/error.rs` | Custom error type via thiserror |
| `src/bin/macos/` | macOS menu bar UI (feature-gated) |

## Benchmarking

A standalone benchmark binary is included for profiling:

```bash
cargo build --release --bin bench-writer

# Direct write throughput (no threading overhead)
target/release/bench-writer --channels 64 --seconds 30 --mode single

# Split mode (worst case: 64 simultaneous file handles)
target/release/bench-writer --channels 64 --seconds 30 --mode split

# Full pipeline (producer → ring buffer → writer thread → WAV)
target/release/bench-writer --channels 64 --seconds 30 --mode pipeline
```

For flamegraph profiling:

```bash
cargo install samply
samply record target/release/bench-writer --channels 64 --seconds 30 --mode pipeline
```

In-tree benchmark tests (run manually, not in CI):

```bash
cargo test benchmark -- --ignored --nocapture
```

### Performance (Apple Silicon, release build, 24-bit)

Measured on an M-series Mac with NVMe storage:

| Config | Mode | Throughput | Real-time headroom |
|--------|------|-----------|-------------------|
| 2ch / 48kHz | pipeline | 205M samples/s | **2,139x** |
| 32ch / 48kHz | single | 473M samples/s | **308x** |
| 32ch / 48kHz | split (32 files) | 259M samples/s | **169x** |
| 64ch / 48kHz | split (64 files) | 243M samples/s | **79x** |
| 32ch / 192kHz | single | 538M samples/s | **87x** |
| 64ch / 192kHz | single | 551M samples/s | **45x** |
| 64ch / 192kHz | split (64 files) | 236M samples/s | **19x** |

Even the worst case (64 channels, 192kHz, 24-bit, split mode with 64 simultaneous WAV files) runs at 19x real-time. The writer thread uses ~5% of its available capacity, leaving substantial headroom for disk I/O variability.

**Memory usage** scales with channel count and sample rate (ring buffer sizing):

| Config | Ring buffer | Total app |
|--------|-----------|-----------|
| 2ch / 48kHz | ~1.8 MB | ~20 MB |
| 32ch / 48kHz | ~29 MB | ~50 MB |
| 64ch / 192kHz | ~234 MB | ~280 MB |

**Disk throughput** at 64ch / 192kHz / 24-bit is ~36 MB/s, well within any modern SSD.

File rotation overhead is <1ms in single mode and ~10ms in 64-channel split mode, with 4,990ms+ of ring buffer runway remaining.

## CI

CI runs on every push to `main` and on pull requests. Six parallel jobs:

| Job | What it checks |
|-----|---------------|
| **Format** | `cargo fmt --check` |
| **Clippy** | `cargo clippy --all-targets --no-default-features -- -D warnings` |
| **Test (Ubuntu)** | 113 lib tests |
| **Test (macOS)** | 113 lib + 12 macOS binary tests |
| **Security audit** | `cargo audit` against RUSTSEC advisory database |
| **Benchmark smoke test** | Builds release binary, runs 64-channel smoke tests in all modes |

Dependabot is configured for weekly dependency update PRs (both Cargo crates and GitHub Actions).

## License

This project is licensed under the [MIT License](LICENSE).
