# Audio Blackbox
This program captures audio from specified channels of an audio interface on a Mac and saves the recording to a WAV file. The recording duration, audio channels, and debug mode can be configured using environment variables. The output file is named using the current date and time in the format YEAR-MONTH-DAY-HOUR-MINUTE.wav.

## Features
- Records audio from specified channels of an audio interface.
- Configurable recording duration.
- Debug mode for detailed logging.
- Automatically names the output file based on the current date and time.

## Development Requirements
- Rust
- MacOS system
- A compatible audio interface with Core Audio support
- Environment variables for configuration

## Building
Ensure you have Rust installed. If not, install it from rust-lang.org.
Clone this repository.
Navigate to the project directory.
Build the project with the following command:
```sh
cargo build --release
```

## Running
Run the program with the following command:

```sh
./audio_recorder
```


### Environment Variables
You can set environment variables to customize the recording:

AUDIO_CHANNELS: Comma-separated list of audio channel indexes to record (default: 1,2).
DEBUG: Set to true to enable debug output (default: false).
RECORD_DURATION: Recording duration in seconds (default: 10).
Example
```sh
AUDIO_CHANNELS="30,31" DEBUG=true RECORD_DURATION=20 RUST_BACKTRACE=1 ./audio_recorder
```

## Output
The output file is saved in the current directory with a name in the format YEAR-MONTH-DAY-HOUR-MINUTE.wav, based on the current date and time.