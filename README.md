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

The application is currently in development. The core audio recording functionality works well, and we have established a solid thread-safe architecture for the macOS menu bar integration.

### Implementation Details

1. **Thread-Safe Architecture**:
   - Successfully implemented a message-passing architecture for thread safety
   - Separated UI code into a dedicated thread to avoid Objective-C/Rust threading issues
   - Created proper communication channels between components
   - Eliminated issues with Objective-C objects being sent between threads

2. **Current Limitations**:
   - The menu bar UI is currently running in a simplified mode due to Objective-C exception handling issues
   - Full native menu bar integration is in progress but requires additional work to handle Cocoa exceptions safely

### Known Issues

1. **CFRunLoop Method Calls**: There are some issues with calling methods on `CFRunLoop` objects, specifically the `run_in_mode` method.

2. **Cargo-Clippy Warnings**: The code generates numerous warnings related to unexpected `cfg` condition values for `cargo-clippy`.

### Recent Improvements

1. **Thread-Safe Menu Bar Architecture**:
   - Created a robust foundation for safe communication between UI and audio processing
   - Implemented architecture that prevents thread safety violations with Cocoa objects
   - Simplified the overall design for better maintainability

2. **Output Mode Validation**:
   - Added validation for audio output modes
   - Improved error messages for invalid configurations
   - Changed default mode to match code expectations

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

To run the application with the menu bar implementation:

```bash
./run_menubar.sh
```

Or manually:

```bash
cargo run -- --menu-bar
```

## Menu Bar Integration Roadmap

Our plan for completing the menu bar implementation:

1. **Short-term**:
   - Resolve Objective-C exception handling issues
   - Implement a safe wrapper around NSApplication and NSMenu
   - Create proper event handling for menu items

2. **Medium-term**:
   - Add custom icons and improved visual design
   - Implement configuration dialogs
   - Add keyboard shortcuts

3. **Long-term**:
   - Create detailed audio visualization
   - Implement drag-and-drop for files and configurations
   - Add support for more advanced recording options

## License

This project is licensed under the MIT License - see the LICENSE file for details.