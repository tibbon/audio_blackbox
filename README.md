# BlackBox Audio Recorder

A macOS audio recording application with menu bar integration.

## Overview

BlackBox Audio Recorder is a Rust application that provides audio recording capabilities with a macOS menu bar interface. The application can record audio in continuous mode or for a specified duration, and provides status updates through the menu bar.

## Features

- Audio recording with configurable duration
- macOS menu bar integration
- Continuous recording mode
- Performance monitoring
- Output directory selection

## Current Status

The application is currently in development. The core audio recording functionality works, but the macOS menu bar integration has some issues with thread safety in the Objective-C bindings.

### Known Issues

1. **Thread Safety Issues**: The macOS menu bar implementation uses Objective-C objects that cannot be sent between threads safely. This causes compilation errors when trying to use the full menu bar implementation.

2. **CFRunLoop Method Calls**: There are issues with calling methods on `CFRunLoop` objects, specifically the `run_in_mode` method.

3. **Cargo-Clippy Warnings**: The code generates numerous warnings related to unexpected `cfg` condition values for `cargo-clippy`.

### Current Workaround

Due to the issues with the native macOS menu bar implementation, a simplified command-line based control system is implemented:

1. The application runs with a status display in the terminal
2. Control is achieved through touch files:
   - Start recording: `touch /tmp/blackbox_start`
   - Stop recording: `touch /tmp/blackbox_stop`
   - Quit app: `touch /tmp/blackbox_quit`
   - Check status: `cat /tmp/blackbox_status`

## Configuration

The application is configured through the `blackbox.toml` file. Important settings include:

```toml
# Output mode: "single" (one file), "split" (one file per channel)
# IMPORTANT: For multi-channel recording, only use "single" or "split"
output_mode = "single"

# Audio channels to record (comma-separated list or ranges like 0-2)
audio_channels = "0"

# Recording duration in seconds (0 for unlimited)
duration = 5

# Output directory for recordings
output_dir = "./recordings"
```

## Building and Running

### Prerequisites

- Rust (nightly toolchain)
- macOS
- ImageMagick (optional, for creating placeholder icons)

### Building

```bash
cargo build
```

### Running

To run the application with the simplified menu bar implementation:

```bash
./run_menubar.sh
```

Or manually:

```bash
cargo run -- --menu-bar
```

## Future Work

1. Fix the thread safety issues in the macOS menu bar implementation
2. Implement proper error handling for the menu bar integration
3. Add more configuration options for audio recording
4. Improve the user interface with better icons and menu options

## License

This project is licensed under the MIT License - see the LICENSE file for details.