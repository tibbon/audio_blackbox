#!/bin/bash
# Script to run the BlackBox Audio Recorder in macOS menu bar mode with enhanced logging

# Create logs directory if it doesn't exist
mkdir -p logs

# Define log file
LOG_FILE="logs/menubar_$(date +%Y%m%d_%H%M%S).log"
echo "BlackBox Audio Recorder startup log - $(date)" > "$LOG_FILE"

# Log system information
echo "===== System Information =====" >> "$LOG_FILE"
echo "macOS Version: $(sw_vers -productVersion)" >> "$LOG_FILE"
echo "Architecture: $(uname -m)" >> "$LOG_FILE"
echo "Rust Version: $(rustc --version)" >> "$LOG_FILE"
echo "Working Directory: $(pwd)" >> "$LOG_FILE"
echo "==========================" >> "$LOG_FILE"

# Create images directory if it doesn't exist
echo "Creating images directory if needed..." | tee -a "$LOG_FILE"
mkdir -p images

# Check if we have icon files
if [ ! -f "images/idle_icon.png" ]; then
    echo "Creating placeholder idle icon..." | tee -a "$LOG_FILE"
    convert -size 16x16 xc:black images/idle_icon.png 2>> "$LOG_FILE" || {
        echo "ERROR: Failed to create idle icon. Is ImageMagick installed?" | tee -a "$LOG_FILE"
        echo "You can install it with: brew install imagemagick" | tee -a "$LOG_FILE"
    }
fi

if [ ! -f "images/recording_icon.png" ]; then
    echo "Creating placeholder recording icon..." | tee -a "$LOG_FILE"
    convert -size 16x16 xc:red images/recording_icon.png 2>> "$LOG_FILE" || {
        echo "ERROR: Failed to create recording icon. Is ImageMagick installed?" | tee -a "$LOG_FILE"
    }
fi

# Create recordings directory if it doesn't exist
echo "Creating recordings directory if needed..." | tee -a "$LOG_FILE"
mkdir -p recordings

# Check if required files exist
echo "Checking for required files..." | tee -a "$LOG_FILE"
[ -f "Cargo.toml" ] && echo "✓ Cargo.toml found" | tee -a "$LOG_FILE" || echo "✗ Cargo.toml not found!" | tee -a "$LOG_FILE"
[ -d "src" ] && echo "✓ src directory found" | tee -a "$LOG_FILE" || echo "✗ src directory not found!" | tee -a "$LOG_FILE"
[ -f "src/main.rs" ] && echo "✓ src/main.rs found" | tee -a "$LOG_FILE" || echo "✗ src/main.rs not found!" | tee -a "$LOG_FILE"
[ -f "src/macos/mod.rs" ] && echo "✓ src/macos/mod.rs found" | tee -a "$LOG_FILE" || echo "✗ src/macos/mod.rs not found!" | tee -a "$LOG_FILE"

# Check for microphone permissions
echo "Checking microphone permissions (approximate)..." | tee -a "$LOG_FILE"
if ! system_profiler SPBluetoothDataType | grep -q "Microphone:"; then
    echo "⚠️  You might need to grant microphone permissions to Terminal/Cursor" | tee -a "$LOG_FILE"
    echo "    Go to System Preferences > Security & Privacy > Microphone" | tee -a "$LOG_FILE"
fi

# Run the application in menu bar mode with output to log file
echo "Starting BlackBox Audio Recorder in menu bar mode..." | tee -a "$LOG_FILE"
echo "Log file: $LOG_FILE" | tee -a "$LOG_FILE"
echo "==========================" | tee -a "$LOG_FILE"
echo "Application output:" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"

# Run with logging - using the simplified implementation without the menu-bar feature
RUST_BACKTRACE=1 cargo run -- --menu-bar 2>&1 | tee -a "$LOG_FILE"

# Check exit status
EXIT_CODE=$?
echo "" | tee -a "$LOG_FILE"
echo "Application exited with code: $EXIT_CODE" | tee -a "$LOG_FILE"

if [ $EXIT_CODE -ne 0 ]; then
    echo "Error: Application failed to start properly!" | tee -a "$LOG_FILE"
    echo "Check the log file for details: $LOG_FILE"
    
    # Look for common errors in the log
    if grep -q "Permission denied" "$LOG_FILE"; then
        echo "Possible permission issue detected. Check file permissions." | tee -a "$LOG_FILE"
    fi
    
    if grep -q "ThreadCreationError" "$LOG_FILE"; then
        echo "Thread creation error detected. System might be low on resources." | tee -a "$LOG_FILE"
    fi
    
    if grep -q "not found" "$LOG_FILE"; then
        echo "Missing dependencies detected. Make sure all required libraries are installed." | tee -a "$LOG_FILE"
    fi
fi

echo "Logs saved to: $LOG_FILE" 