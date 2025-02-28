# Audio Recorder

A simple Rust application for recording audio from input devices to WAV files. Supports both mono and stereo inputs, and automatically converts mono inputs to stereo output files.

## Features

- Records from default audio input device
- Supports multiple sample formats (F32, I16, U16)
- Handles both mono and stereo input
- Configurable through environment variables
- Outputs standard stereo WAV files

## Requirements

- Rust and Cargo
- An audio input device (microphone, line-in, etc.)

## Installation

Clone the repository and build with Cargo:

```bash
git clone [repository-url]
cd audio_recorder
cargo build --release
```

## Usage

Run the application with:

```bash
cargo run --release
```

### Environment Variables

The application can be configured with the following environment variables:

- `AUDIO_CHANNELS`: Comma-separated list of input channels to record from (e.g., "0" for mono, "0,1" for stereo). Default: "1,2"
- `DEBUG`: Enable debug output ("true" or "false"). Default: "false"
- `RECORD_DURATION`: Recording duration in seconds. Default: "10"

Example:

```bash
AUDIO_CHANNELS="0" DEBUG="true" RECORD_DURATION="30" cargo run --release
```

## Output

The application creates WAV files in the current directory with names in the format:
```
YYYY-MM-DD-HH-MM.wav
```

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