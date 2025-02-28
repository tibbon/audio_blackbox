use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use hound;
use std::sync::{Arc, Mutex};
use std::env;
use chrono::prelude::*;

pub const INTERMEDIATE_BUFFER_SIZE: usize = 512;
pub const DEFAULT_CHANNELS: &str = "1,2";
pub const DEFAULT_DEBUG: &str = "false";
pub const DEFAULT_DURATION: &str = "10";
pub const DEFAULT_OUTPUT_MODE: &str = "single";
pub const MAX_CHANNELS: usize = 64;

// A trait to abstract the audio processing logic
pub trait AudioProcessor {
    fn process_audio(&mut self, channels: &[usize], output_mode: &str, debug: bool);
    fn finalize(&mut self);
}

// The main recorder that uses the processor
pub struct AudioRecorder<P: AudioProcessor> {
    processor: P,
}

impl<P: AudioProcessor> AudioRecorder<P> {
    pub fn new(processor: P) -> Self {
        AudioRecorder { processor }
    }

    pub fn start_recording(&mut self) -> Result<String, String> {
        // Read environment variables
        let channels_str = env::var("AUDIO_CHANNELS")
            .unwrap_or_else(|_| DEFAULT_CHANNELS.to_string());
        
        // Parse channels, which can now include ranges
        let channels = parse_channel_string(&channels_str)?;

        let debug: bool = env::var("DEBUG")
            .unwrap_or_else(|_| DEFAULT_DEBUG.to_string())
            .parse()
            .expect("Invalid debug flag");

        let record_duration: u64 = env::var("RECORD_DURATION")
            .unwrap_or_else(|_| DEFAULT_DURATION.to_string())
            .parse()
            .expect("Invalid record duration");

        let output_mode: String = env::var("OUTPUT_MODE")
            .unwrap_or_else(|_| DEFAULT_OUTPUT_MODE.to_string());

        // Print recording information
        println!("Starting recording:");
        println!("  Channels: {:?}", channels);
        println!("  Debug: {}", debug);
        println!("  Duration: {} seconds", record_duration);
        println!("  Output Mode: {}", output_mode);

        // Process audio based on channels and config
        self.processor.process_audio(&channels, &output_mode, debug);

        // Return a success message
        Ok("Recording in progress. Press Ctrl+C to stop.".to_string())
    }
}

// Real implementation of the AudioProcessor for CPAL
pub struct CpalAudioProcessor {
    file_name: String,
    writer: Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>,
    multichannel_writers: Arc<Mutex<Vec<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>>,
    intermediate_buffer: Arc<Mutex<Vec<i32>>>,
    multichannel_buffers: Arc<Mutex<Vec<Vec<i32>>>>,
    #[allow(dead_code)]
    sample_rate: u32, // Kept for future features that might use it
    // Add a field to keep the stream alive
    #[allow(dead_code)]
    stream: Option<Box<dyn StreamTrait>>,
}

impl CpalAudioProcessor {
    pub fn new() -> Result<Self, String> {
        // Generate the output file name
        let now: DateTime<Local> = Local::now();
        let file_name = format!("{}-{:02}-{:02}-{:02}-{:02}.wav", 
                                now.year(), now.month(), now.day(), 
                                now.hour(), now.minute());

        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or_else(|| "No input device available".to_string())?;

        println!("Using audio device: {}", device.name().map_err(|e| e.to_string())?);

        let config = device.default_input_config()
            .map_err(|e| format!("Failed to get default input stream config: {}", e))?;

        println!("Default input stream config: {:?}", config);

        let sample_rate = config.sample_rate().0;
        
        let spec = hound::WavSpec {
            channels: 2,  // Default is stereo WAV for backward compatibility
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = Arc::new(Mutex::new(Some(
            hound::WavWriter::create(&file_name, spec)
                .map_err(|e| format!("Failed to create WAV file: {}", e))?
        )));
        
        let intermediate_buffer = Arc::new(Mutex::new(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE)));
        let multichannel_writers = Arc::new(Mutex::new(Vec::new()));
        let multichannel_buffers = Arc::new(Mutex::new(Vec::new()));

        Ok(CpalAudioProcessor {
            file_name,
            writer,
            multichannel_writers,
            intermediate_buffer,
            multichannel_buffers,
            sample_rate,
            stream: None,
        })
    }

    // Create individual WAV files for each channel
    fn setup_split_mode(&self, channels: &[usize], sample_rate: u32) -> Result<(), String> {
        let now: DateTime<Local> = Local::now();
        let date_str = format!("{}-{:02}-{:02}-{:02}-{:02}", 
                            now.year(), now.month(), now.day(), 
                            now.hour(), now.minute());
        
        let mut writers = self.multichannel_writers.lock().unwrap();
        let mut buffers = self.multichannel_buffers.lock().unwrap();
        
        writers.clear();
        buffers.clear();
        
        // Create a mono WAV file for each channel
        for &channel in channels {
            let file_name = format!("{}-ch{}.wav", date_str, channel);
            
            let spec = hound::WavSpec {
                channels: 1,  // Mono WAV
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            
            let writer = hound::WavWriter::create(&file_name, spec)
                .map_err(|e| format!("Failed to create WAV file for channel {}: {}", channel, e))?;
            
            writers.push(Some(writer));
            buffers.push(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE));
        }
        
        println!("Created {} individual channel WAV files", channels.len());
        Ok(())
    }

    // Create a single multichannel WAV file
    fn setup_multichannel_mode(&self, channels: &[usize], sample_rate: u32) -> Result<(), String> {
        let now: DateTime<Local> = Local::now();
        let file_name = format!("{}-{:02}-{:02}-{:02}-{:02}-multichannel.wav", 
                            now.year(), now.month(), now.day(), 
                            now.hour(), now.minute());
        
        let spec = hound::WavSpec {
            channels: channels.len() as u16,  // Number of channels in the WAV
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        
        let mut writer_lock = self.writer.lock().unwrap();
        *writer_lock = Some(
            hound::WavWriter::create(&file_name, spec)
                .map_err(|e| format!("Failed to create multichannel WAV file: {}", e))?
        );
        
        println!("Created multichannel WAV file with {} channels", channels.len());
        Ok(())
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(&mut self, channels: &[usize], output_mode: &str, debug: bool) {
        // Get CPAL host and device
        let host = cpal::default_host();
        let device = host.default_input_device()
            .expect("No input device available");

        println!("Using audio device: {}", device.name().unwrap());
        
        let config = device.default_input_config()
            .expect("Failed to get default input stream config");
        
        println!("Default input stream config: {:?}", config);
        
        let total_channels = config.channels() as usize;
        let sample_rate = config.sample_rate().0;
        
        // Validate channels
        for &channel in channels {
            if channel >= total_channels {
                panic!("The audio device does not have channel {}", channel);
            }
        }

        // Setup the appropriate output mode
        match output_mode {
            "split" => {
                if let Err(e) = self.setup_split_mode(channels, sample_rate) {
                    panic!("Failed to setup split mode: {}", e);
                }
            },
            "single" if channels.len() > 2 => {
                if let Err(e) = self.setup_multichannel_mode(channels, sample_rate) {
                    panic!("Failed to setup multichannel mode: {}", e);
                }
            },
            _ => {
                // Use the default stereo WAV for backward compatibility
                println!("Using standard stereo WAV output format");
            }
        }

        // Clone channels to own them in the closure
        let channels_owned: Vec<usize> = channels.to_vec();
        let output_mode_owned = output_mode.to_string();
        
        let writer_clone = Arc::clone(&self.writer);
        let buffer_clone = Arc::clone(&self.intermediate_buffer);
        let multichannel_writers_clone = Arc::clone(&self.multichannel_writers);
        let multichannel_buffers_clone = Arc::clone(&self.multichannel_buffers);
        
        let err_fn = |err| eprintln!("An error occurred on the input audio stream: {}", err);
        
        // Create different streams based on the sample format
        let stream: Box<dyn StreamTrait> = match config.sample_format() {
            SampleFormat::F32 => {
                let writer_for_callback = Arc::clone(&writer_clone);
                let buffer_for_callback = Arc::clone(&buffer_clone);
                let multichannel_writers_for_callback = Arc::clone(&multichannel_writers_clone);
                let multichannel_buffers_for_callback = Arc::clone(&multichannel_buffers_clone);
                let output_mode_for_callback = output_mode_owned.clone();
                Box::new(device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if debug {
                            println!("Received data with length: {}", data.len());
                        }
                        
                        match output_mode_for_callback.as_str() {
                            "split" => {
                                // Process each channel separately
                                let mut writers_lock = multichannel_writers_for_callback.lock().unwrap();
                                let mut buffers_lock = multichannel_buffers_for_callback.lock().unwrap();
                                
                                for frame in data.chunks(total_channels) {
                                    for (i, &channel) in channels_owned.iter().enumerate() {
                                        if frame.len() > channel {
                                            let sample = (frame[channel] * std::i16::MAX as f32) as i16;
                                            
                                            if i < buffers_lock.len() {
                                                buffers_lock[i].push(sample as i32);
                                                
                                                if buffers_lock[i].len() >= INTERMEDIATE_BUFFER_SIZE {
                                                    if let Some(ref mut writer) = writers_lock[i] {
                                                        for &s in &buffers_lock[i] {
                                                            if let Err(e) = writer.write_sample(s) {
                                                                eprintln!("Failed to write sample to channel {}: {:?}", channel, e);
                                                            }
                                                        }
                                                    }
                                                    buffers_lock[i].clear();
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            "single" if channels_owned.len() > 2 => {
                                // Process all channels into a single multichannel file
                                let mut writer_lock = writer_for_callback.lock().unwrap();
                                let mut buffer_lock = buffer_for_callback.lock().unwrap();
                                
                                if let Some(ref mut writer) = *writer_lock {
                                    for frame in data.chunks(total_channels) {
                                        for &channel in &channels_owned {
                                            if frame.len() > channel {
                                                let sample = (frame[channel] * std::i16::MAX as f32) as i16;
                                                buffer_lock.push(sample as i32);
                                            }
                                        }
                                        
                                        if buffer_lock.len() >= INTERMEDIATE_BUFFER_SIZE {
                                            for &sample in &*buffer_lock {
                                                if let Err(e) = writer.write_sample(sample) {
                                                    eprintln!("Failed to write sample: {:?}", e);
                                                }
                                            }
                                            buffer_lock.clear();
                                        }
                                    }
                                }
                            },
                            _ => {
                                // Standard stereo processing (original behavior)
                                let mut writer_lock = writer_for_callback.lock().unwrap();
                                let mut buffer_lock = buffer_for_callback.lock().unwrap();
                                
                                if let Some(ref mut writer) = *writer_lock {
                                    for frame in data.chunks(total_channels) {
                                        if frame.len() >= channels_owned.len() {
                                            let sample_left = (frame[channels_owned[0]] * std::i16::MAX as f32) as i16;
                                            let sample_right = if channels_owned.len() == 1 {
                                                // For mono input, duplicate the channel
                                                sample_left
                                            } else {
                                                // For stereo input, use the second channel
                                                (frame[channels_owned[1]] * std::i16::MAX as f32) as i16
                                            };
                                            buffer_lock.push(sample_left as i32);
                                            buffer_lock.push(sample_right as i32);
                                            
                                            if buffer_lock.len() >= INTERMEDIATE_BUFFER_SIZE {
                                                for &sample in &*buffer_lock {
                                                    if let Err(e) = writer.write_sample(sample) {
                                                        eprintln!("Failed to write sample: {:?}", e);
                                                    }
                                                }
                                                buffer_lock.clear();
                                            }
                                        } else {
                                            eprintln!("Buffer too small: expected at least {} channels, found {}", channels_owned.len(), frame.len());
                                        }
                                    }
                                }
                            }
                        }
                    },
                    err_fn,
                    None,
                ).expect("Failed to build input stream"))
            },
            // Similar implementations for I16 and U16 would follow the same pattern
            // ... other formats ...
            _ => panic!("Unsupported sample format"),
        };
        
        // Start recording
        stream.play().expect("Failed to play stream");
        
        // Store the stream to keep it alive during recording
        self.stream = Some(stream);
        
        // Sleep for the duration of recording
        let record_duration = env::var("RECORD_DURATION")
            .unwrap_or_else(|_| DEFAULT_DURATION.to_string())
            .parse::<u64>()
            .expect("Invalid record duration");
            
        std::thread::sleep(std::time::Duration::from_secs(record_duration));
    }

    fn finalize(&mut self) {
        // Drop the stream to stop recording
        self.stream = None;
        
        // Finalize the main WAV file if needed
        let mut writer_lock = self.writer.lock().unwrap();
        let buffer_lock = self.intermediate_buffer.lock().unwrap();
        
        if let Some(ref mut writer) = *writer_lock {
            // Write any remaining samples
            for &sample in &*buffer_lock {
                let _ = writer.write_sample(sample);
            }
        }
        
        // Close the file - take it out of the Option
        if let Some(writer) = writer_lock.take() {
            let _ = writer.finalize();
            println!("Finalized main WAV file: {}", self.file_name);
        }
        
        // Finalize any split channel files
        let mut writers_lock = self.multichannel_writers.lock().unwrap();
        let buffers_lock = self.multichannel_buffers.lock().unwrap();
        
        for (i, writer_opt) in writers_lock.iter_mut().enumerate() {
            if let Some(mut writer) = writer_opt.take() {
                // Write any remaining samples if we have buffers
                if i < buffers_lock.len() {
                    for &sample in &buffers_lock[i] {
                        let _ = writer.write_sample(sample);
                    }
                }
                
                // Close the file
                let _ = writer.finalize();
                println!("Finalized channel WAV file {}", i);
            }
        }
        
        println!("Recording completed.");
    }
}

// Mock implementation for testing
#[cfg(test)]
pub mod test_utils {
    use super::*;
    
    pub struct MockAudioProcessor {
        pub channels: Vec<usize>,
        pub output_mode: String,
        pub debug: bool,
        pub audio_processed: bool,
        pub finalized: bool,
        pub file_name: String,
        pub created_files: Vec<String>,
    }

    impl MockAudioProcessor {
        pub fn new(file_name: &str) -> Self {
            MockAudioProcessor {
                channels: Vec::new(),
                output_mode: DEFAULT_OUTPUT_MODE.to_string(),
                debug: false,
                audio_processed: false,
                finalized: false,
                file_name: file_name.to_string(),
                created_files: Vec::new(),
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
            
            match output_mode {
                "split" => {
                    // Create an empty WAV file for each channel
                    for &channel in channels {
                        let file_name = format!("{}-ch{}.wav", self.file_name, channel);
                        self.created_files.push(file_name.clone());
                        
                        let spec = hound::WavSpec {
                            channels: 1,  // Mono WAV
                            sample_rate: 44100,
                            bits_per_sample: 16,
                            sample_format: hound::SampleFormat::Int,
                        };
                        
                        match hound::WavWriter::create(&file_name, spec) {
                            Ok(mut writer) => {
                                // Add some test samples
                                for i in 0..1000 {
                                    let sample = (i % 100) as i32;
                                    let _ = writer.write_sample(sample);
                                }
                                let _ = writer.finalize();
                            },
                            Err(e) => {
                                eprintln!("Error creating test WAV file: {}", e);
                            }
                        }
                    }
                    println!("Created {} individual mock channel WAV files", channels.len());
                },
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
                                    let sample = (i % 100) as i32;
                                    let _ = writer.write_sample(sample);
                                }
                            }
                            let _ = writer.finalize();
                        },
                        Err(e) => {
                            eprintln!("Error creating test multichannel WAV file: {}", e);
                        }
                    }
                    println!("Created mock multichannel WAV file");
                },
                _ => {
                    // Create a stereo WAV file
                    let file_path = self.file_name.clone();
                    self.created_files.push(file_path.clone());
                    
                    let spec = hound::WavSpec {
                        channels: 2,  // Stereo WAV
                        sample_rate: 44100,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    };
                    
                    match hound::WavWriter::create(&file_path, spec) {
                        Ok(mut writer) => {
                            // Add some test samples
                            for i in 0..1000 {
                                let sample = (i % 100) as i32;
                                let _ = writer.write_sample(sample);
                                let _ = writer.write_sample(sample);
                            }
                            let _ = writer.finalize();
                        },
                        Err(e) => {
                            eprintln!("Error creating test WAV file: {}", e);
                        }
                    }
                    println!("Created mock stereo WAV file");
                }
            }
        }

        fn finalize(&mut self) {
            self.finalized = true;
            // Nothing more to do for the mock
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::MockAudioProcessor;
    use std::fs;
    use tempfile::tempdir;
    use std::path::Path;

    // Helper function to reset environment variables for tests
    fn reset_test_env() {
        // Clear existing environment variables to ensure test isolation
        env::remove_var("AUDIO_CHANNELS");
        env::remove_var("DEBUG");
        env::remove_var("RECORD_DURATION");
        env::remove_var("OUTPUT_MODE");
        
        // Sleep briefly to ensure environment changes propagate
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    #[test]
    fn test_environment_variable_handling() {
        // Reset environment to ensure test isolation
        reset_test_env();
        
        // This test will directly validate the parsing functions without
        // relying on environment variables
        
        // Test channel parsing
        let channel_string = "0,1";
        let channels = parse_channel_string(channel_string).unwrap();
        assert_eq!(channels, vec![0, 1], "Channel parsing failed for '{}'", channel_string);
        
        // Test range parsing
        let range_string = "0-3";
        let range_channels = parse_channel_string(range_string).unwrap();
        assert_eq!(range_channels, vec![0, 1, 2, 3], "Range parsing failed for '{}'", range_string);
        
        // Test mixed format
        let mixed_string = "0,2-4,7";
        let mixed_channels = parse_channel_string(mixed_string).unwrap();
        assert_eq!(mixed_channels, vec![0, 2, 3, 4, 7], "Mixed format parsing failed for '{}'", mixed_string);
        
        // Test boolean parsing
        let debug_true = "true".parse::<bool>().unwrap();
        assert_eq!(debug_true, true, "Boolean parsing failed for 'true'");
        
        let debug_false = "false".parse::<bool>().unwrap();
        assert_eq!(debug_false, false, "Boolean parsing failed for 'false'");
        
        // Test duration parsing
        let duration_str = "20";
        let duration = duration_str.parse::<u64>().unwrap();
        assert_eq!(duration, 20, "Duration parsing failed for '{}'", duration_str);
    }

    #[test]
    fn test_mono_recording() {
        // Reset environment to ensure test isolation
        reset_test_env();
        
        // Set up a temporary directory for the test
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        println!("Temp directory: {}", temp_path);
        env::set_current_dir(&temp_dir).unwrap();
        
        // Set up test environment variables matching what the mock processor expects
        env::set_var("AUDIO_CHANNELS", "0");
        env::set_var("DEBUG", "true");
        env::set_var("RECORD_DURATION", "1");
        env::set_var("OUTPUT_MODE", "single");
        
        // Output the environment variables to debug
        println!("Environment variables:");
        println!("  AUDIO_CHANNELS: {}", env::var("AUDIO_CHANNELS").unwrap_or_default());
        println!("  DEBUG: {}", env::var("DEBUG").unwrap_or_default());
        println!("  RECORD_DURATION: {}", env::var("RECORD_DURATION").unwrap_or_default());
        println!("  OUTPUT_MODE: {}", env::var("OUTPUT_MODE").unwrap_or_default());
        
        // Create the test file name with full path
        let file_name = format!("{}/test-mono.wav", temp_path);
        println!("Test file path: {}", file_name);
        
        // Create a mock processor
        let processor = MockAudioProcessor::new(&file_name);
        
        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);
        
        // Start recording
        let result = recorder.start_recording();
        
        // Check the result
        assert!(result.is_ok());
        
        // Manually finalize the recording (since we've changed the architecture)
        recorder.processor.finalize();
        
        // Get the processor back to check its state
        let processor = recorder.processor;
        
        // Log processor state for debugging
        println!("Processor state:");
        println!("  Channels: {:?}", processor.channels);
        println!("  Output mode: {}", processor.output_mode);
        println!("  Debug: {}", processor.debug);
        
        // Verify the processor received the right parameters
        assert_eq!(processor.channels, vec![0]);
        assert_eq!(processor.output_mode, "single");
        assert_eq!(processor.debug, true);
        assert!(processor.audio_processed, "Audio should have been processed");
        assert!(processor.finalized, "Recording should have been finalized");
        
        // Verify the file was created using the full path
        assert!(!processor.created_files.is_empty(), "No files were created");
        let wav_path = Path::new(&processor.created_files[0]);
        println!("Checking if file exists: {}", wav_path.display());
        assert!(wav_path.exists(), "WAV file was not created");
        
        // Verify file has content
        let metadata = fs::metadata(wav_path).unwrap();
        assert!(metadata.len() > 0, "WAV file is empty");
        
        // Verify file content
        let reader = hound::WavReader::open(wav_path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2); // Should be stereo output
        
        // Count samples
        let samples: Vec<i32> = reader.into_samples().collect::<Result<Vec<i32>, _>>().unwrap();
        assert!(!samples.is_empty(), "No samples in the WAV file");
    }

    #[test]
    fn test_stereo_recording() {
        // Reset environment to ensure test isolation
        reset_test_env();
        
        // Set up a temporary directory for the test
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        println!("Temp directory: {}", temp_path);
        env::set_current_dir(&temp_dir).unwrap();
        
        // Set up test environment variables
        env::set_var("AUDIO_CHANNELS", "0,1");
        env::set_var("DEBUG", "true");
        env::set_var("RECORD_DURATION", "2");
        env::set_var("OUTPUT_MODE", "single");  // Make sure to use single mode
        
        println!("Environment variables:");
        println!("  AUDIO_CHANNELS: {}", env::var("AUDIO_CHANNELS").unwrap_or_default());
        println!("  DEBUG: {}", env::var("DEBUG").unwrap_or_default());
        println!("  RECORD_DURATION: {}", env::var("RECORD_DURATION").unwrap_or_default());
        println!("  OUTPUT_MODE: {}", env::var("OUTPUT_MODE").unwrap_or_default());

        // Create the test file name with full path
        let file_name = format!("{}/test-stereo.wav", temp_path);
        println!("Test file path: {}", file_name);
        
        // Create a mock processor
        let processor = MockAudioProcessor::new(&file_name);
        
        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);
        
        // Start recording
        let result = recorder.start_recording();
        
        // Check the result
        assert!(result.is_ok());
        
        // Manually finalize the recording
        recorder.processor.finalize();
        
        // Get the processor back to check its state
        let processor = recorder.processor;
        
        // Verify the processor received the right parameters
        println!("Processor state:");
        println!("  Channels: {:?}", processor.channels);
        println!("  Output mode: {}", processor.output_mode);
        println!("  Debug: {}", processor.debug);
        
        assert_eq!(processor.channels, vec![0, 1]);
        assert_eq!(processor.output_mode, "single");
        assert_eq!(processor.debug, true);
        assert!(processor.audio_processed, "Audio should have been processed");
        assert!(processor.finalized, "Recording should have been finalized");
        
        // Verify the file was created
        assert!(!processor.created_files.is_empty(), "No files were created");
        let wav_path = Path::new(&processor.created_files[0]);
        println!("Checking if file exists: {}", wav_path.display());
        assert!(wav_path.exists(), "WAV file was not created");
        
        // Verify file has content
        let metadata = fs::metadata(wav_path).unwrap();
        assert!(metadata.len() > 0, "WAV file is empty");
    }

    #[test]
    fn test_multichannel_single_file() {
        // Reset environment to ensure test isolation
        reset_test_env();
        
        // Set up a temporary directory for the test
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        println!("Temp directory: {}", temp_path);
        env::set_current_dir(&temp_dir).unwrap();
        
        // Set up test environment variables for multichannel recording
        env::set_var("AUDIO_CHANNELS", "0-3");  // Use channel range 0-3 (4 channels)
        env::set_var("DEBUG", "true");
        env::set_var("RECORD_DURATION", "1");
        env::set_var("OUTPUT_MODE", "single");  // Single multichannel file
        
        println!("Environment variables set:");
        println!("  AUDIO_CHANNELS: {}", env::var("AUDIO_CHANNELS").unwrap_or_default());
        println!("  DEBUG: {}", env::var("DEBUG").unwrap_or_default());
        println!("  RECORD_DURATION: {}", env::var("RECORD_DURATION").unwrap_or_default());
        println!("  OUTPUT_MODE: {}", env::var("OUTPUT_MODE").unwrap_or_default());
        
        // Create the test file name with full path
        let file_name = format!("{}/test-multichannel.wav", temp_path);
        println!("Test file path: {}", file_name);
        
        // Create a mock processor
        let processor = MockAudioProcessor::new(&file_name);
        
        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);
        
        // Start recording
        let result = recorder.start_recording();
        
        // Check the result
        assert!(result.is_ok());
        
        // Manually finalize the recording
        recorder.processor.finalize();
        
        // Get the processor back to check its state
        let processor = recorder.processor;
        
        // Verify the processor received the right parameters
        println!("Processor state:");
        println!("  Channels: {:?}", processor.channels);
        println!("  Output mode: {}", processor.output_mode);
        println!("  Debug: {}", processor.debug);
        
        // Check that channel range was parsed correctly
        assert_eq!(processor.channels, vec![0, 1, 2, 3]);
        assert_eq!(processor.output_mode, "single");
        assert_eq!(processor.debug, true);
        assert!(processor.audio_processed, "Audio should have been processed");
        assert!(processor.finalized, "Recording should have been finalized");
        
        // Verify the file was created
        assert!(!processor.created_files.is_empty(), "No files were created");
        let wav_path = Path::new(&processor.created_files[0]);
        println!("Checking if file exists: {}", wav_path.display());
        assert!(wav_path.exists(), "WAV file was not created");
        
        // Verify file has content
        let metadata = fs::metadata(wav_path).unwrap();
        assert!(metadata.len() > 0, "WAV file is empty");
    }

    #[test]
    fn test_multichannel_split_files() {
        // Reset environment to ensure test isolation
        reset_test_env();
        
        // Set up a temporary directory for the test
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        println!("Temp directory: {}", temp_path);
        env::set_current_dir(&temp_dir).unwrap();
        
        // Set up test environment variables for multichannel recording
        env::set_var("AUDIO_CHANNELS", "0,1,5,10");  // Specific channels
        env::set_var("DEBUG", "true");
        env::set_var("RECORD_DURATION", "1");
        env::set_var("OUTPUT_MODE", "split");  // Split into multiple mono files
        
        // Create the test file base name with full path
        let file_name = format!("{}/test-split", temp_path);
        println!("Test file base path: {}", file_name);
        
        // Create a mock processor
        let processor = MockAudioProcessor::new(&file_name);
        
        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);
        
        // Start recording
        let result = recorder.start_recording();
        
        // Check the result
        assert!(result.is_ok());
        
        // Manually finalize the recording
        recorder.processor.finalize();
        
        // Get the processor back to check its state
        let processor = recorder.processor;
        
        // Verify the processor received the right parameters
        println!("Processor state:");
        println!("  Channels: {:?}", processor.channels);
        println!("  Output mode: {}", processor.output_mode);
        println!("  Debug: {}", processor.debug);
        
        // Check that individual channels were parsed correctly
        assert_eq!(processor.channels, vec![0, 1, 5, 10]);
        assert_eq!(processor.output_mode, "split");
        assert_eq!(processor.debug, true);
        
        // Verify one file was created for each channel
        assert_eq!(processor.created_files.len(), 4);
        
        // Check each file exists
        for file_path in &processor.created_files {
            let wav_path = Path::new(file_path);
            println!("Checking if file exists: {}", wav_path.display());
            assert!(wav_path.exists(), "WAV file was not created");
            
            // Verify file has content
            let metadata = fs::metadata(wav_path).unwrap();
            assert!(metadata.len() > 0, "WAV file is empty");
            
            // Verify it's a mono file
            let reader = hound::WavReader::open(wav_path).unwrap();
            let spec = reader.spec();
            assert_eq!(spec.channels, 1); // Should be a mono channel file
        }
    }
}

// Helper function to parse channel string with ranges
fn parse_channel_string(input: &str) -> Result<Vec<usize>, String> {
    let mut channels = Vec::new();
    
    for part in input.split(',') {
        if part.contains('-') {
            // Handle range like "1-24"
            let range_parts: Vec<&str> = part.split('-').collect();
            if range_parts.len() != 2 {
                return Err(format!("Invalid range format: {}", part));
            }
            
            let start = range_parts[0].trim().parse::<usize>()
                .map_err(|_| format!("Invalid start of range: {}", range_parts[0]))?;
            let end = range_parts[1].trim().parse::<usize>()
                .map_err(|_| format!("Invalid end of range: {}", range_parts[1]))?;
            
            if start > end {
                return Err(format!("Invalid range: start {} greater than end {}", start, end));
            }
            
            if end >= MAX_CHANNELS {
                return Err(format!("Channel number {} exceeds maximum of {}", end, MAX_CHANNELS - 1));
            }
            
            // Add all channels in the range
            for channel in start..=end {
                channels.push(channel);
            }
        } else {
            // Handle individual channel
            let channel = part.trim().parse::<usize>()
                .map_err(|_| format!("Invalid channel number: {}", part))?;
            
            if channel >= MAX_CHANNELS {
                return Err(format!("Channel number {} exceeds maximum of {}", channel, MAX_CHANNELS - 1));
            }
            
            channels.push(channel);
        }
    }
    
    if channels.is_empty() {
        return Err("No valid channels specified".to_string());
    }
    
    // Remove duplicate channels
    channels.sort();
    channels.dedup();
    
    Ok(channels)
} 