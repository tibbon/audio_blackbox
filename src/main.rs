use audio_recorder::{AudioRecorder, CpalAudioProcessor};
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
}
