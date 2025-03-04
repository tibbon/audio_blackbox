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

A simplified implementation of the `MenuBarApp` has been created that doesn't use the thread-unsafe parts of the Objective-C bindings. This allows the application to compile and run, but without the full menu bar functionality.

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