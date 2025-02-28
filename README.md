# Audio Recorder

A robust audio recording application built with Rust that supports various recording modes, silence detection, and performance monitoring.

## Features

- **Flexible configuration** via TOML file or environment variables
- **Multiple recording modes**: single file, split channels, or continuous recording
- **Smart channel selection**: automatically adapts to available hardware
- **Silence detection**: optionally discard silent recordings
- **Performance monitoring**: track resource usage during recording

## Configuration

The application can be configured in three ways (in order of precedence):
1. Environment variables (highest priority)
2. Configuration file (`blackbox.toml`)
3. Default values (lowest priority)

On first run, a default configuration file is created in the current directory.

### Key Settings

- `audio_channels`: Channels to record (e.g., "0,1" or "0-2")
- `debug`: Enable debug output
- `duration`: Recording duration in seconds
- `output_mode`: "single" or "split"
- `silence_threshold`: Threshold for silence detection (0 disables)
- `continuous_mode`: Enable continuous recording
- `recording_cadence`: Rotation interval for continuous mode
- `output_dir`: Directory for saving recordings
- `performance_logging`: Enable performance metrics

## Usage

```bash
# Simple recording with default settings
cargo run

# Using environment variables to override config
AUDIO_CHANNELS=0 DURATION=5 cargo run

# Edit blackbox.toml to change default settings
# and simply run
cargo run
```

## Output

The application creates WAV files in the configured output directory (default: `./recordings`). In continuous mode, files are rotated according to the specified cadence.

## Hardware Compatibility

The application adapts to various audio hardware setups:
- Works with mono or stereo microphones
- Supports multi-channel recording devices
- Automatically adapts to available channels

## Building

```bash
cargo build --release
```

The compiled binary will be available at `target/release/blackbox`.