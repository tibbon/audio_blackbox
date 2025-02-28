# Audio Recorder

An audio recording application built with Rust that supports various recording modes, silence detection, and performance monitoring.

## Features

- **Recording with configurable channel selection** - Record from specific audio channels with automatic adaptation to available hardware
- **Multiple output formats** - Save to a single multichannel file or individual channel files
- **Silence detection** - Automatically discard silent recordings
- **Debug mode** - Track audio processing details
- **Continuous recording mode with configurable file rotation**
- **Performance monitoring and benchmarking**

## Requirements

The application uses environment variables for configuration:

- `AUDIO_CHANNELS` - Zero-indexed channels to record, e.g., "0,1" for first and second channels (default: "0,1"). The application will automatically adapt if requested channels aren't available.
- `DEBUG` - Enable debug output (default: "false")
- `DURATION` - Recording duration in seconds (default: "10")
- `OUTPUT_MODE` - Output mode: "single" or "split" (default: "single")
- `SILENCE_THRESHOLD` - Threshold for silence detection (default: "0" - disabled)
- `CONTINUOUS_MODE` - Enable continuous recording (default: "false")
- `RECORDING_CADENCE` - How often to rotate files in continuous mode (seconds, default: "60")
- `OUTPUT_DIR` - Directory for saving audio files (default: current directory)
- `PERFORMANCE_LOGGING` - Enable performance metrics collection (default: "false")

## Usage

```bash
# Record using default settings (channels 0,1, 10 seconds, single file output)
cargo run

# Record specific channels (zero-indexed)
AUDIO_CHANNELS=0 cargo run        # Record only first channel
AUDIO_CHANNELS=0,1,2 cargo run    # Record first three channels
AUDIO_CHANNELS=0-2 cargo run      # Record channels 0, 1, and 2 (range format)
AUDIO_CHANNELS=0,2-4 cargo run    # Record channels 0, 2, 3, and 4 (mixed format)

# Record with silence detection
SILENCE_THRESHOLD=0.01 cargo run

# Record for a specific duration
DURATION=5 cargo run

# Record with split output (one file per channel)
OUTPUT_MODE=split cargo run

# Enable debug output
DEBUG=true cargo run

# Enable continuous recording
CONTINUOUS_MODE=true RECORDING_CADENCE=30 cargo run

# With performance logging
PERFORMANCE_LOGGING=true cargo run
```

## Output

The application creates WAV files in your current directory (or the directory specified by `OUTPUT_DIR`). 

In standard mode, the recording will stop after the specified duration and save the WAV file.

In continuous mode, files will be created every `RECORDING_CADENCE` seconds. The application will continue recording until interrupted.

## Hardware Compatibility

The application is designed to work with various audio hardware setups:

- **Single Channel Microphones**: Will automatically detect and work with mono microphones
- **Multi-Channel Microphones**: Can record from any available channels
- **Limited Channel Hardware**: Automatically adapts if requested channels aren't available

## Performance Monitoring

When enabled, performance metrics include:
- CPU usage
- Memory consumption
- Disk write speed
- Audio processing latency

These metrics are logged at regular intervals and can be used for benchmarking and optimization.

## Building

```bash
cargo build --release
```

The compiled binary will be available at `target/release/blackbox`.