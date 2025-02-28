# Audio Recorder

A simple Rust application for recording audio from input devices to WAV files. Supports both mono and stereo inputs, and automatically converts mono inputs to stereo output files. Now with full multichannel support for up to 64 channels!

## Features

- Records from default audio input device
- Supports multiple sample formats (F32, I16, U16)
- Handles mono, stereo, and multichannel inputs (up to 64 channels)
- Configurable through environment variables
- Flexible channel selection with support for ranges and individual channels
- Multiple output modes:
  - Standard stereo WAV file (default)
  - Single multichannel WAV file with all selected channels
  - Split mode with separate mono WAV files for each channel

## Requirements

- Rust and Cargo
- An audio input device (microphone, line-in, etc.)

## Installation

Clone the repository and build with Cargo:

```bash
git clone [repository-url]
cd blackbox
cargo build --release
```

## Usage

Run the application with:

```bash
cargo run --release
```

### Environment Variables

The application can be configured with the following environment variables:

- `AUDIO_CHANNELS`: Specify which channels to record:
  - Comma-separated values for individual channels: "0,1,5,10"
  - Ranges of channels: "1-24"
  - Mixed format: "0,1,5-10,15"
  - Default: "1,2"
- `OUTPUT_MODE`: How to save the recording:
  - "single": Record all channels into a single WAV file (default)
  - "split": Record each channel to a separate mono WAV file
- `DEBUG`: Enable debug output ("true" or "false"). Default: "false"
- `RECORD_DURATION`: Recording duration in seconds. Default: "10"

Examples:

```bash
# Record channels 0 and 1 to a stereo WAV file for 30 seconds
AUDIO_CHANNELS="0,1" RECORD_DURATION="30" cargo run --release

# Record 8 channels (0-7) to a single multichannel WAV file
AUDIO_CHANNELS="0-7" OUTPUT_MODE="single" RECORD_DURATION="60" cargo run --release

# Record channels 1, 3, 5, and 7 to individual mono WAV files
AUDIO_CHANNELS="1,3,5,7" OUTPUT_MODE="split" RECORD_DURATION="120" cargo run --release
```

## Output

The application creates WAV files in the current directory with names in the format:

- Single file mode: `YYYY-MM-DD-HH-MM.wav` or `YYYY-MM-DD-HH-MM-multichannel.wav`
- Split mode: `YYYY-MM-DD-HH-MM-chX.wav` (where X is the channel number)

## Architecture

The application is structured around a simple abstraction:

- `AudioProcessor`: Trait that handles the audio processing
- `AudioRecorder`: Main struct that coordinates the recording process
- `CpalAudioProcessor`: Implementation of `AudioProcessor` that uses the CPAL library

## Testing

The project includes unit tests that use mock objects to test the recording functionality without requiring actual audio hardware:

```bash
cargo test
```

## License

[MIT](LICENSE)