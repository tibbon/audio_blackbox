# BlackBox Audio Recorder

[![CI](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml/badge.svg)](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml)

**Set-and-forget continuous audio recording for macOS.** Captures from any input device — built-in mic, USB interface, multichannel audio interface — and never loses a take when the app, the Mac, or the power decides otherwise. Designed for audio engineers, podcasters, and field recorders who want a quiet menu-bar utility that just runs.

> Built around a lock-free real-time audio pipeline so the recording thread never blocks on disk I/O, mutex locks, or allocations — captures are clean even on heavy-load systems.

## Install

**Mac App Store:** see the [App Store listing](https://apps.apple.com/app/blackbox-audio-recorder/id6502949317) (or search "BlackBox Audio Recorder"). The shipped product is the SwiftUI menu-bar app — settings, level meter, onboarding, and security-scoped output-folder picker included.

**Build from source** (CLI binary, or for development):

```bash
git clone https://github.com/tibbon/audio_blackbox.git
cd audio_blackbox
make app           # SwiftUI app — opens in Xcode build
# or
cargo build --release   # CLI binary at target/release/blackbox
```

Prerequisites: Rust stable toolchain (edition 2024). macOS users also need Xcode command-line tools. Linux users (CLI only — there is no Linux GUI) additionally need `libasound2-dev` and `pkg-config`.

## Features

- **Multi-channel recording** — 1 to 64+ channels simultaneously.
- **Configurable bit depth** — 16-bit, 24-bit (default, pro standard), or 32-bit WAV.
- **Two output modes** — `single` (one file, automatically multichannel for 3+ channels) or `split` (one file per channel).
- **Continuous recording** — automatic file rotation at configurable intervals with crash-safe WAV writes.
- **Silence gate** — pauses recording during silence, resumes when audio is detected (configurable threshold and timeout).
- **Disk-space monitoring** — automatically stops recording when free space drops below a configurable threshold.
- **Lock-free RT architecture** — audio callback uses zero file I/O, zero mutex locks, zero allocations; all writes happen on a dedicated writer thread via a SPSC ring buffer.
- **Per-channel peak metering** — tracked on the writer thread at zero extra cost, exposed via FFI to the SwiftUI app.
- **Privacy-respecting** — no network access, all recordings stay local.

## Quick start (CLI)

```bash
cargo run                            # Run with defaults
cargo test                           # Run all tests (121 lib tests, 14 ignored benchmarks)
cargo test --features ffi            # 147 lib tests (adds the FFI suite)
cargo clippy --all-targets --no-default-features -- -D warnings  # Lint (matches CI)
cargo fmt --all -- --check           # Format check
make verify                          # Kitchen-sink local check (fmt + clippy + tests + ASC metadata lint + Swift tests)
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

# Silence gate — pauses recording during silence
silence_gate_enabled = true
silence_gate_timeout_secs = 300

# Continuous recording mode
continuous_mode = false

# File rotation cadence in seconds (continuous mode)
recording_cadence = 300

# Minimum free disk space in MB (0 to disable)
min_disk_space_mb = 500

# Input device name (leave unset for system default)
# input_device = "MacBook Pro Microphone"
```

Channel specs support individual channels and ranges: `"0,2-4,7"` records channels 0, 2, 3, 4, and 7.

## Architecture

The recording pipeline is lock-free at the audio-thread boundary so dropouts can't be introduced by I/O latency:

```
Audio device → cpal callback (RT thread) → rtrb ring buffer → Writer thread → WAV files
```

- **RT callback** only pushes raw f32 samples into the ring buffer and checks an `AtomicBool` for rotation timing. No file I/O, no mutexes, no allocations.
- **Writer thread** reads from the ring buffer, converts f32 to the configured bit depth, writes WAV directly (custom `RawWavWriter`, no third-party WAV library in the hot path), tracks per-channel peak levels, and handles file rotation and disk-space monitoring.
- **Ring buffer** is sized for 5 seconds of audio at the device's sample rate and channel count, providing ample runway for file rotation I/O even at high channel counts.
- **Silence-check worker** is a single dedicated thread fed via a bounded channel; the writer thread submits rotated files to it without blocking.

### Key modules

| Module | Purpose |
|--------|---------|
| `src/audio_processor.rs` | `AudioProcessor` trait — central abstraction |
| `src/cpal_processor.rs` | Real audio I/O implementation using cpal |
| `src/writer_thread.rs` | Writer thread, ring buffer consumer, WAV file management |
| `src/config.rs` | TOML + env var configuration with `BLACKBOX_*` prefix support |
| `src/ffi.rs` | C FFI layer consumed by the SwiftUI app |
| `src/raw_wav_writer.rs` | Hand-rolled WAV writer for the hot path (zero hound) |
| `src/error.rs` | Typed error enum with `thiserror` |
| `BlackBoxApp/` | SwiftUI menu-bar app (Xcode project, calls Rust via FFI) |

## Benchmarking

A standalone benchmark binary is included for profiling. The `benchmarking` feature pulls in `sysinfo` and is off by default so the shipped binary stays small — pass `--features benchmarking` when building or running it.

```bash
cargo build --release --bin bench-writer --features benchmarking

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
cargo test benchmark --features benchmarking -- --ignored --nocapture
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

CI runs on every push to `main` and on pull requests:

| Job | What it checks |
|-----|---------------|
| **Format** | `cargo fmt --all -- --check` |
| **Clippy** | `cargo clippy --all-targets --no-default-features -- -D warnings` |
| **MSRV (1.95)** | `cargo check --all-targets --no-default-features` on the pinned MSRV toolchain |
| **Test (macOS)** | 121 lib tests (14 benchmarks ignored) |
| **Security audit** | `cargo audit` against RUSTSEC advisory database |
| **Benchmark smoke test** | Builds release binary, asserts ≥10× real-time throughput in all modes |
| **Swift app** | Builds Rust static library with FFI and SwiftUI app via xcodebuild |
| **CodeQL** | Static analysis on Rust + Swift + GitHub Actions YAML |

A separate **Ignored tests** workflow runs the long `#[ignore]`-marked benchmark / perf tests weekly (Mondays 08:00 UTC) and on manual `workflow_dispatch`. The **Release** workflow runs on `v*` tag pushes and via `workflow_dispatch`, gated on a fresh test run.

Dependabot is configured for weekly dependency update PRs (Cargo crates, GitHub Actions, and Bundler).

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes per version.

## License

This project is licensed under the [Business Source License 1.1](LICENSE). Personal, non-commercial use is permitted. Commercial use requires a license from the author. On 2030-03-01, the code converts to Apache License 2.0.
