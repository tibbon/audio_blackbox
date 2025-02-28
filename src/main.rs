use blackbox::{AudioRecorder, CpalAudioProcessor};
use std::process;

fn main() {
    // Create the audio processor with CPAL implementation
    let processor = match CpalAudioProcessor::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error initializing audio processor: {}", e);
            process::exit(1);
        }
    };

    // Create the recorder with our processor
    let mut recorder = AudioRecorder::new(processor);

    // Start recording
    match recorder.start_recording() {
        Ok(message) => println!("{}", message),
        Err(e) => {
            eprintln!("Error during recording: {}", e);
            process::exit(1);
        }
    }

    // The recorder will automatically stop after the specified duration
    // and all resources will be cleaned up when the program exits
}
