# BlackBox Audio Recorder

[![CI](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml/badge.svg)](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml)

A cross-platform audio recording application in Rust with macOS menu bar integration. Records from configurable input channels, saves as WAV files, supports silence detection, continuous recording with automatic file rotation, and handles up to 64+ channels with real-time performance.

## Features

- **Multi-channel recording** — record 1 to 64+ channels simultaneously (tested at 121x realtime on Apple Silicon)
- **Three output modes** — `single` (one multichannel file), `split` (one file per channel), `multichannel` (all channels in one file)
- **Continuous recording** — automatic file rotation at configurable intervals with crash-safe WAV writes
- **Silence detection** — automatically deletes silent recordings on rotation
- **Lock-free RT architecture** — audio callback uses zero file I/O, zero mutex locks, zero allocations; all writes happen on a dedicated writer thread via a SPSC ring buffer
- **macOS menu bar** — optional native menu bar UI (feature-gated)
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
cargo test                           # Run all tests (108 tests)
cargo clippy --no-default-features -- -D warnings   # Lint (matches CI)
cargo fmt --all -- --check           # Format check
```

## Configuration

Configure via `blackbox.toml`, environment variables, or `BLACKBOX_*` prefixed env vars. Environment variables take precedence over the config file.

```toml
# Output mode: "single", "split", or "multichannel"
output_mode = "single"

# Audio channels to record (comma-separated or ranges)
audio_channels = "0"

# Recording duration in seconds
duration = 30

# Output directory for recordings
output_dir = "./recordings"

# Silence threshold (0.0 to disable)
silence_threshold = 0.01

# Continuous recording mode
continuous_mode = false

# File rotation cadence in seconds (continuous mode)
recording_cadence = 300
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
- **Writer thread**: reads from the ring buffer, converts f32 to i16, writes WAV via hound, handles file rotation (finalize, rename, silence check, create new files).
- **Ring buffer**: sized for 2 seconds of audio at the device's sample rate and channel count, providing ample runway for file rotation I/O.

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

## CI

CI runs on every push to `main` and on pull requests. Six parallel jobs:

| Job | What it checks |
|-----|---------------|
| **Format** | `cargo fmt --check` |
| **Clippy** | `cargo clippy --all-targets --no-default-features -- -D warnings` |
| **Test (Ubuntu)** | All 108 tests |
| **Test (macOS)** | All 108 tests |
| **Security audit** | `cargo audit` against RUSTSEC advisory database |
| **Benchmark smoke test** | Builds release binary, runs 64-channel smoke tests in all modes |

Dependabot is configured for weekly dependency update PRs (both Cargo crates and GitHub Actions).

## License

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
