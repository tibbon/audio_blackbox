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

// A trait to abstract the audio processing logic
pub trait AudioProcessor {
    fn process_audio(&mut self, channels: &[usize], is_mono_input: bool, debug: bool);
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
        let channels: Vec<usize> = env::var("AUDIO_CHANNELS")
            .unwrap_or_else(|_| DEFAULT_CHANNELS.to_string())
            .split(',')
            .map(|s| s.parse().expect("Invalid channel number"))
            .collect();

        let debug: bool = env::var("DEBUG")
            .unwrap_or_else(|_| DEFAULT_DEBUG.to_string())
            .parse()
            .expect("Invalid debug flag");

        let record_duration: u64 = env::var("RECORD_DURATION")
            .unwrap_or_else(|_| DEFAULT_DURATION.to_string())
            .parse()
            .expect("Invalid record duration");

        // Check if we're recording from a mono source but want stereo output
        let is_mono_input = channels.len() == 1;

        // Print recording information
        println!("Starting recording:");
        println!("  Channels: {:?}", channels);
        println!("  Debug: {}", debug);
        println!("  Duration: {} seconds", record_duration);
        println!("  Mono input: {}", is_mono_input);

        // Process audio based on channels and config
        self.processor.process_audio(&channels, is_mono_input, debug);

        // Return a success message
        Ok("Recording in progress. Press Ctrl+C to stop.".to_string())
    }
}

// Real implementation of the AudioProcessor for CPAL
pub struct CpalAudioProcessor {
    file_name: String,
    writer: Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>,
    intermediate_buffer: Arc<Mutex<Vec<i32>>>,
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
            channels: 2,  // Always output stereo WAV
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = Arc::new(Mutex::new(Some(
            hound::WavWriter::create(&file_name, spec)
                .map_err(|e| format!("Failed to create WAV file: {}", e))?
        )));
        
        let intermediate_buffer = Arc::new(Mutex::new(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE)));

        Ok(CpalAudioProcessor {
            file_name,
            writer,
            intermediate_buffer,
            sample_rate,
            stream: None,
        })
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(&mut self, channels: &[usize], is_mono_input: bool, debug: bool) {
        // Get CPAL host and device
        let host = cpal::default_host();
        let device = host.default_input_device()
            .expect("No input device available");

        println!("Using audio device: {}", device.name().unwrap());
        
        let config = device.default_input_config()
            .expect("Failed to get default input stream config");
        
        println!("Default input stream config: {:?}", config);
        
        let total_channels = config.channels() as usize;
        
        // Validate channels
        for &channel in channels {
            if channel >= total_channels {
                panic!("The audio device does not have channel {}", channel);
            }
        }

        // Clone channels to own them in the closure
        let channels_owned: Vec<usize> = channels.to_vec();
        let is_mono = is_mono_input;
        
        let writer_clone = Arc::clone(&self.writer);
        let buffer_clone = Arc::clone(&self.intermediate_buffer);
        
        let err_fn = |err| eprintln!("An error occurred on the input audio stream: {}", err);
        
        // Create different streams based on the sample format
        let stream: Box<dyn StreamTrait> = match config.sample_format() {
            SampleFormat::F32 => {
                let writer_for_callback = Arc::clone(&writer_clone);
                let buffer_for_callback = Arc::clone(&buffer_clone);
                Box::new(device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if debug {
                            println!("Received data with length: {}", data.len());
                        }
                        let mut writer_lock = writer_for_callback.lock().unwrap();
                        let mut buffer_lock = buffer_for_callback.lock().unwrap();
                        if let Some(ref mut writer) = *writer_lock {
                            for frame in data.chunks(total_channels) {
                                if frame.len() >= channels_owned.len() {
                                    let sample_left = (frame[channels_owned[0]] * std::i16::MAX as f32) as i16;
                                    let sample_right = if is_mono {
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
                    },
                    err_fn,
                    None,
                ).expect("Failed to build input stream"))
            },
            SampleFormat::I16 => {
                let writer_for_callback = Arc::clone(&writer_clone);
                let buffer_for_callback = Arc::clone(&buffer_clone);
                let channels_owned = channels_owned.clone();
                Box::new(device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if debug {
                            println!("Received data with length: {}", data.len());
                        }
                        let mut writer_lock = writer_for_callback.lock().unwrap();
                        let mut buffer_lock = buffer_for_callback.lock().unwrap();
                        if let Some(ref mut writer) = *writer_lock {
                            for frame in data.chunks(total_channels) {
                                if frame.len() >= channels_owned.len() {
                                    let sample_left = frame[channels_owned[0]] as i32;
                                    let sample_right = if is_mono {
                                        // For mono input, duplicate the channel
                                        sample_left
                                    } else {
                                        // For stereo input, use the second channel
                                        frame[channels_owned[1]] as i32
                                    };
                                    buffer_lock.push(sample_left);
                                    buffer_lock.push(sample_right);
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
                    },
                    err_fn,
                    None,
                ).expect("Failed to build input stream"))
            },
            SampleFormat::U16 => {
                let writer_for_callback = Arc::clone(&writer_clone);
                let buffer_for_callback = Arc::clone(&buffer_clone);
                let channels_owned = channels_owned.clone();
                Box::new(device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        if debug {
                            println!("Received data with length: {}", data.len());
                        }
                        let mut writer_lock = writer_for_callback.lock().unwrap();
                        let mut buffer_lock = buffer_for_callback.lock().unwrap();
                        if let Some(ref mut writer) = *writer_lock {
                            for frame in data.chunks(total_channels) {
                                if frame.len() >= channels_owned.len() {
                                    let sample_left = (frame[channels_owned[0]] as i32) - 32768;
                                    let sample_right = if is_mono {
                                        // For mono input, duplicate the channel
                                        sample_left
                                    } else {
                                        // For stereo input, use the second channel
                                        (frame[channels_owned[1]] as i32) - 32768
                                    };
                                    buffer_lock.push(sample_left);
                                    buffer_lock.push(sample_right);
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
                    },
                    err_fn,
                    None,
                ).expect("Failed to build input stream"))
            },
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
        
        let mut writer_lock = self.writer.lock().unwrap();
        let buffer_lock = self.intermediate_buffer.lock().unwrap();
        
        if let Some(ref mut writer) = *writer_lock {
            // Write any remaining samples
            for &sample in &*buffer_lock {
                let _ = writer.write_sample(sample);
            }
        }

        // Finalize the WAV file
        if let Some(writer) = writer_lock.take() {
            let _ = writer.finalize();
        }

        println!("Recording saved to {}", self.file_name);
    }
}

// Mock implementation for testing
#[cfg(test)]
pub mod test_utils {
    use super::*;
    
    pub struct MockAudioProcessor {
        pub channels: Vec<usize>,
        pub is_mono_input: bool,
        pub debug: bool,
        pub audio_processed: bool,
        pub finalized: bool,
        pub file_name: String,
    }

    impl MockAudioProcessor {
        pub fn new(file_name: &str) -> Self {
            MockAudioProcessor {
                channels: Vec::new(),
                is_mono_input: false,
                debug: false,
                audio_processed: false,
                finalized: false,
                file_name: file_name.to_string(),
            }
        }
    }

    impl AudioProcessor for MockAudioProcessor {
        fn process_audio(&mut self, channels: &[usize], is_mono_input: bool, debug: bool) {
            self.channels = channels.to_vec();
            self.is_mono_input = is_mono_input;
            self.debug = debug;
            self.audio_processed = true;
            
            // Create an empty WAV file for testing
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate: 44100,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            
            let file_path = self.file_name.clone();
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

    #[test]
    fn test_environment_variable_handling() {
        // Reset environment variables first
        env::remove_var("AUDIO_CHANNELS");
        env::remove_var("DEBUG");
        env::remove_var("RECORD_DURATION");
        
        // Set new values
        env::set_var("AUDIO_CHANNELS", "0,1");
        env::set_var("DEBUG", "true");
        env::set_var("RECORD_DURATION", "20");

        println!("Environment variables set:");
        println!("  AUDIO_CHANNELS: {}", env::var("AUDIO_CHANNELS").unwrap_or_default());
        println!("  DEBUG: {}", env::var("DEBUG").unwrap_or_default());
        println!("  RECORD_DURATION: {}", env::var("RECORD_DURATION").unwrap_or_default());

        let channels: Vec<usize> = env::var("AUDIO_CHANNELS")
            .unwrap_or_else(|_| DEFAULT_CHANNELS.to_string())
            .split(',')
            .map(|s| s.parse().expect("Invalid channel number"))
            .collect();

        let debug_str = env::var("DEBUG").unwrap_or_else(|_| DEFAULT_DEBUG.to_string());
        println!("Debug string: '{}'", debug_str);
        let debug: bool = debug_str.parse().expect("Invalid debug flag");
        println!("Parsed debug value: {}", debug);

        let record_duration: u64 = env::var("RECORD_DURATION")
            .unwrap_or_else(|_| DEFAULT_DURATION.to_string())
            .parse()
            .expect("Invalid record duration");

        assert_eq!(channels, vec![0, 1]);
        assert_eq!(debug, true);
        assert_eq!(record_duration, 20);
    }

    #[test]
    fn test_mono_recording() {
        // Reset environment variables first
        env::remove_var("AUDIO_CHANNELS");
        env::remove_var("DEBUG");
        env::remove_var("RECORD_DURATION");
        
        // Set up a temporary directory for the test
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        println!("Temp directory: {}", temp_path);
        env::set_current_dir(&temp_dir).unwrap();
        
        // Set up test environment variables
        env::set_var("AUDIO_CHANNELS", "0");
        env::set_var("DEBUG", "false");
        env::set_var("RECORD_DURATION", "1");

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
        
        // Verify the processor received the right parameters
        assert_eq!(processor.channels, vec![0]);
        assert_eq!(processor.is_mono_input, true);
        assert_eq!(processor.debug, false);
        assert!(processor.audio_processed, "Audio should have been processed");
        assert!(processor.finalized, "Recording should have been finalized");
        
        // List files in the temp directory
        println!("Files in temp directory:");
        if let Ok(entries) = std::fs::read_dir(temp_path) {
            for entry in entries {
                if let Ok(entry) = entry {
                    println!("  {}", entry.path().display());
                }
            }
        }
        
        // Verify the file was created using the full path
        let wav_path = Path::new(&file_name);
        println!("Checking if file exists: {}", wav_path.display());
        assert!(wav_path.exists(), "WAV file was not created");
        
        // Verify file has content
        let metadata = fs::metadata(wav_path).unwrap();
        assert!(metadata.len() > 0, "WAV file is empty");
        
        // Verify file content
        let mut reader = hound::WavReader::open(wav_path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2); // Should be stereo output
        
        // Count samples
        let samples: Vec<i32> = reader.samples().collect::<Result<Vec<i32>, _>>().unwrap();
        assert!(!samples.is_empty(), "No samples in the WAV file");
    }

    #[test]
    fn test_stereo_recording() {
        // Reset environment variables first
        env::remove_var("AUDIO_CHANNELS");
        env::remove_var("DEBUG");
        env::remove_var("RECORD_DURATION");
        
        // Set up a temporary directory for the test
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        println!("Temp directory: {}", temp_path);
        env::set_current_dir(&temp_dir).unwrap();
        
        // Set up test environment variables
        env::set_var("AUDIO_CHANNELS", "0,1");
        env::set_var("DEBUG", "true");
        env::set_var("RECORD_DURATION", "2");
        
        println!("Environment variables:");
        println!("  AUDIO_CHANNELS: {}", env::var("AUDIO_CHANNELS").unwrap_or_default());
        println!("  DEBUG: {}", env::var("DEBUG").unwrap_or_default());
        println!("  RECORD_DURATION: {}", env::var("RECORD_DURATION").unwrap_or_default());

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
        println!("  Is mono input: {}", processor.is_mono_input);
        println!("  Debug: {}", processor.debug);
        
        assert_eq!(processor.channels, vec![0, 1]);
        assert_eq!(processor.is_mono_input, false); // Stereo
        assert_eq!(processor.debug, true);
        assert!(processor.audio_processed, "Audio should have been processed");
        assert!(processor.finalized, "Recording should have been finalized");
        
        // List files in the temp directory
        println!("Files in temp directory:");
        if let Ok(entries) = std::fs::read_dir(temp_path) {
            for entry in entries {
                if let Ok(entry) = entry {
                    println!("  {}", entry.path().display());
                }
            }
        }
        
        // Verify the file was created using the full path
        let wav_path = Path::new(&file_name);
        println!("Checking if file exists: {}", wav_path.display());
        assert!(wav_path.exists(), "WAV file was not created");
        
        // Verify file has content
        let metadata = fs::metadata(wav_path).unwrap();
        assert!(metadata.len() > 0, "WAV file is empty");
    }
} 