use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use std::fs;
use std::path::Path;

#[cfg(test)]
use crate::constants::DEFAULT_OUTPUT_MODE;

/// MockAudioProcessor simulates audio processing for testing purposes
/// without requiring actual audio hardware.
pub struct MockAudioProcessor {
    pub channels: Vec<usize>,
    pub output_mode: String,
    pub debug: bool,
    pub audio_processed: bool,
    pub finalized: bool,
    pub file_name: String,
    pub created_files: Vec<String>,
    /// When true, creates files with very low amplitude samples that will be
    /// detected as silent by the silence detection algorithm. Used for testing
    /// the automatic deletion of silent recordings.
    pub create_silent_file: bool,
}

impl MockAudioProcessor {
    #[cfg(test)]
    pub fn new(file_name: &str) -> Self {
        MockAudioProcessor {
            channels: Vec::new(),
            output_mode: DEFAULT_OUTPUT_MODE.to_string(),
            debug: false,
            audio_processed: false,
            finalized: false,
            file_name: file_name.to_string(),
            created_files: Vec::new(),
            create_silent_file: false,
        }
    }
}

impl AudioProcessor for MockAudioProcessor {
    fn process_audio(&mut self, channels: &[usize], output_mode: &str, debug: bool) {
        self.channels = channels.to_vec();
        self.output_mode = output_mode.to_string();
        self.debug = debug;
        self.audio_processed = true;
        self.created_files.clear();

        // Choose amplitude based on silence flag
        // Very low amplitude (0) for silent files, higher amplitude (50) for normal files
        let amplitude = if self.create_silent_file { 0 } else { 50 };

        // Make sure the output directory exists
        if let Some(dir) = Path::new(&self.file_name).parent() {
            if !dir.exists() {
                if let Err(e) = fs::create_dir_all(dir) {
                    eprintln!("Error creating directory: {}", e);
                    return;
                }
            }
        }

        match output_mode {
            "split" => {
                // Create an empty WAV file for each channel
                for &channel in channels {
                    let file_name = format!("{}-ch{}.wav", self.file_name, channel);
                    self.created_files.push(file_name.clone());

                    let spec = hound::WavSpec {
                        channels: 1, // Mono WAV
                        sample_rate: 44100,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    };

                    match hound::WavWriter::create(&file_name, spec) {
                        Ok(mut writer) => {
                            // Add some test samples
                            for i in 0..1000 {
                                let sample = (i % 100) * amplitude;
                                let _ = writer.write_sample(sample);
                            }
                            let _ = writer.finalize();
                        }
                        Err(e) => {
                            eprintln!("Error creating test WAV file: {}", e);
                        }
                    }
                }
                println!(
                    "Created {} individual mock channel WAV files",
                    channels.len()
                );
            }
            "single" if channels.len() > 2 => {
                // Create a multichannel WAV file
                let file_name = format!("{}-multichannel.wav", self.file_name);
                self.created_files.push(file_name.clone());

                let spec = hound::WavSpec {
                    channels: channels.len() as u16,
                    sample_rate: 44100,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };

                match hound::WavWriter::create(&file_name, spec) {
                    Ok(mut writer) => {
                        // Add some test samples
                        for i in 0..1000 {
                            for _ in 0..channels.len() {
                                let sample = (i % 100) * amplitude;
                                let _ = writer.write_sample(sample);
                            }
                        }
                        let _ = writer.finalize();
                    }
                    Err(e) => {
                        eprintln!("Error creating test multichannel WAV file: {}", e);
                    }
                }
                println!("Created mock multichannel WAV file");
            }
            _ => {
                // Create a stereo WAV file
                let file_path = self.file_name.clone();
                self.created_files.push(file_path.clone());

                let spec = hound::WavSpec {
                    channels: 2, // Stereo WAV
                    sample_rate: 44100,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };

                match hound::WavWriter::create(&file_path, spec) {
                    Ok(mut writer) => {
                        // Add some test samples
                        for i in 0..1000 {
                            let sample = (i % 100) * amplitude;
                            let _ = writer.write_sample(sample);
                            let _ = writer.write_sample(sample);
                        }
                        let _ = writer.finalize();
                    }
                    Err(e) => {
                        eprintln!("Error creating test WAV file: {}", e);
                    }
                }
                println!(
                    "Created mock {} WAV file",
                    if self.create_silent_file {
                        "silent"
                    } else {
                        "normal"
                    }
                );
            }
        }
    }

    fn finalize(&mut self) {
        self.finalized = true;

        // Check if we should apply the silence threshold using AppConfig
        let config = AppConfig::load();
        let silence_threshold = config.get_silence_threshold();

        if silence_threshold > 0 && self.create_silent_file {
            // If we're creating silent files and threshold is set, delete the files
            // since they should be below the threshold. This allows testing the
            // silence detection and deletion functionality.
            for file_path in &self.created_files {
                if let Err(e) = fs::remove_file(file_path) {
                    eprintln!("Failed to delete silent file in test: {}", e);
                } else {
                    println!("Deleted silent test file: {}", file_path);
                }
            }
        }
    }
}
