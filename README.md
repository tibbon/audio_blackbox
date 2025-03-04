# Audio Recorder

A robust audio recording application built with Rust that supports various recording modes, silence detection, and performance monitoring.

## Features

- **Flexible configuration** via TOML file or environment variables
- **Multiple recording modes**: single file, split channels, or continuous recording
- **Smart channel selection**: automatically adapts to available hardware
- **Silence detection**: optionally discard silent recordings
- **Performance monitoring**: track resource usage during recording
- **macOS menu bar integration**: system tray controls for recording (macOS only)

## Configuration

The application can be configured in three ways (in order of precedence):
1. Environment variables with `BLACKBOX_` prefix (highest priority)
2. Environment variables without prefix
3. Configuration file (`blackbox.toml`)
4. Default values (lowest priority)

On first run, a default configuration file is created in the current directory.

### Key Settings

- `audio_channels`: Channels to record (e.g., "0,1" or "0-2")
- `debug`: Enable debug output (true/false)
- `duration`: Recording duration in seconds (0 for unlimited)
- `output_mode`: "single" (one file) or "split" (one file per channel)
- `silence_threshold`: Threshold for silence detection (0.0-1.0, 0 disables)
- `continuous_mode`: Enable continuous recording (true/false)
- `recording_cadence`: Rotation interval for continuous mode (seconds)
- `output_dir`: Directory for saving recordings
- `performance_logging`: Enable performance metrics (true/false)

### Environment Variables

Each setting can be configured via environment variables:

```bash
# With prefix (recommended)
BLACKBOX_AUDIO_CHANNELS=0,1
BLACKBOX_DEBUG=true
BLACKBOX_DURATION=300

# Without prefix (legacy support)
AUDIO_CHANNELS=0,1
DEBUG=true
RECORD_DURATION=300
```

## Usage

```bash
# Simple recording with default settings
cargo run

# Run as macOS menu bar app
cargo run -- --menu-bar

# Using environment variables
BLACKBOX_AUDIO_CHANNELS=0,1 BLACKBOX_DURATION=5 cargo run

# Using make targets (macOS)
make app-bundle      # Create .app bundle
make install        # Install as service
make start         # Start recording service
make stop          # Stop recording service
```

## Output

The application creates WAV files in the configured output directory (default: `./recordings`). Files are named with timestamps and channel information:

- Single mode: `YYYY-MM-DD-HH-mm.wav`
- Split mode: `YYYY-MM-DD-HH-mm-chN.wav`
- Continuous mode: Files are rotated according to the specified cadence

## Hardware Compatibility

The application adapts to various audio hardware setups:
- Works with mono or stereo microphones
- Supports multi-channel recording devices
- Automatically adapts to available channels

## Building

### Prerequisites

- Rust toolchain (1.70.0 or later)
- For Linux: ALSA development libraries (`libasound2-dev`)
- For macOS: Xcode Command Line Tools

```bash
# Build debug version
cargo build

# Build release version
cargo build --release

# Build macOS app bundle
make app-bundle
```

## Testing

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test config_tests
cargo test silence_tests
```

## License

This program is licensed under the GNU General Public License v3.0. See the LICENSE file for details.