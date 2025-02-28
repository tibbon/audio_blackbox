#!/bin/bash

# Build the project
echo "Building project..."
cargo build

# Test 1: Basic recording with default settings
echo -e "\n=== Test 1: Basic Recording (Default Settings) ==="
echo "Recording for 3 seconds with channels 0,1..."
DURATION=3 cargo run
echo "Test 1 complete."

# Test 2: Single channel recording
echo -e "\n=== Test 2: Single Channel Recording ==="
echo "Recording for 3 seconds with channel 0 only..."
DURATION=3 AUDIO_CHANNELS=0 cargo run
echo "Test 2 complete."

# Test 3: Continuous mode (short test)
echo -e "\n=== Test 3: Continuous Recording Mode ==="
echo "Recording in continuous mode for 7 seconds with 2-second cadence..."
# Run the continuous mode test in the background and capture its PID
CONTINUOUS_MODE=true RECORDING_CADENCE=2 cargo run &
CONTINUOUS_PID=$!

# Wait for 7 seconds
echo "Waiting 7 seconds..."
sleep 7

# Send signal to stop the recording (equivalent to Ctrl+C)
echo "Stopping continuous recording..."
kill -SIGINT $CONTINUOUS_PID

# Wait for the process to finish cleanly
wait $CONTINUOUS_PID
echo "Test 3 complete."

echo -e "\nAll tests completed. Check the output files in the current directory." 

# Clean up test files (optional - uncomment to enable automatic cleanup)
echo -e "\n=== Cleaning up test files ==="
echo "Removing WAV files in current directory..."
find . -maxdepth 1 -name "*.wav" -delete
echo "Removing recordings directory..."
rm -rf ./recordings

echo "Cleanup complete." 