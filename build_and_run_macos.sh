#!/bin/bash

# Create images directory if it doesn't exist
mkdir -p images

# Check if we have icon files
if [ ! -f "images/idle_icon.png" ]; then
    echo "Warning: images/idle_icon.png not found. Creating a placeholder."
    # Create a simple 16x16 black square as a placeholder
    convert -size 16x16 xc:black images/idle_icon.png
fi

if [ ! -f "images/recording_icon.png" ]; then
    echo "Warning: images/recording_icon.png not found. Creating a placeholder."
    # Create a simple 16x16 red square as a placeholder
    convert -size 16x16 xc:red images/recording_icon.png
fi

# Build the app
echo "Building BlackBox Audio Recorder..."
cargo build || { echo "Build failed"; exit 1; }

# Create a simple app bundle structure for development
mkdir -p target/BlackBox.app/Contents/MacOS
mkdir -p target/BlackBox.app/Contents/Resources/images

# Copy binary and resources
cp target/debug/blackbox target/BlackBox.app/Contents/MacOS/
cp -R images/* target/BlackBox.app/Contents/Resources/images/
cp Info.plist target/BlackBox.app/Contents/

echo "Running BlackBox Audio Recorder..."
target/BlackBox.app/Contents/MacOS/blackbox 