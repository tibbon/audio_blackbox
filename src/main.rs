use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat};
use hound;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::env;

const INTERMEDIATE_BUFFER_SIZE: usize = 512;

fn main() {
    // Read environment variables
    let channels: Vec<usize> = env::var("AUDIO_CHANNELS")
        .unwrap_or_else(|_| "1,2".to_string())
        .split(',')
        .map(|s| s.parse().expect("Invalid channel number"))
        .collect();

    let debug: bool = env::var("DEBUG")
        .unwrap_or_else(|_| "false".to_string())
        .parse()
        .expect("Invalid debug flag");

    let record_duration: u64 = env::var("RECORD_DURATION")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .expect("Invalid record duration");

    let host = cpal::default_host();
    let device = host.default_input_device().expect("No input device available");

    println!("Using audio device: {}", device.name().unwrap());

    let config = device.default_input_config().expect("Failed to get default input stream config");

    println!("Default input stream config: {:?}", config);

    let sample_rate = config.sample_rate().0;
    let total_channels = config.channels() as usize;

    for &channel in &channels {
        if channel >= total_channels {
            panic!("The audio device does not have channel {}", channel);
        }
    }

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let writer = Arc::new(Mutex::new(Some(hound::WavWriter::create("recording.wav", spec).unwrap())));
    let intermediate_buffer = Arc::new(Mutex::new(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE)));

    let err_fn = |err| eprintln!("An error occurred on the input audio stream: {}", err);

    let stream = match config.sample_format() {
        SampleFormat::F32 => {
            let writer_clone = Arc::clone(&writer);
            let buffer_clone = Arc::clone(&intermediate_buffer);
            device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if debug {
                        println!("Received data with length: {}", data.len());
                    }
                    let mut writer_lock = writer_clone.lock().unwrap();
                    let mut buffer_lock = buffer_clone.lock().unwrap();
                    if let Some(ref mut writer) = *writer_lock {
                        for frame in data.chunks(total_channels) {
                            if frame.len() >= channels.len() {
                                let sample_left = (frame[channels[0]] * std::i16::MAX as f32) as i16;
                                let sample_right = (frame[channels[1]] * std::i16::MAX as f32) as i16;
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
                                eprintln!("Buffer too small: expected at least {} channels, found {}", channels.len(), frame.len());
                            }
                        }
                    }
                },
                err_fn,
                None, // No specific latency requirement
            ).expect("Failed to build input stream")
        },
        SampleFormat::I16 => {
            let writer_clone = Arc::clone(&writer);
            let buffer_clone = Arc::clone(&intermediate_buffer);
            device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if debug {
                        println!("Received data with length: {}", data.len());
                    }
                    let mut writer_lock = writer_clone.lock().unwrap();
                    let mut buffer_lock = buffer_clone.lock().unwrap();
                    if let Some(ref mut writer) = *writer_lock {
                        for frame in data.chunks(total_channels) {
                            if frame.len() >= channels.len() {
                                let sample_left = frame[channels[0]] as i32;
                                let sample_right = frame[channels[1]] as i32;
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
                                eprintln!("Buffer too small: expected at least {} channels, found {}", channels.len(), frame.len());
                            }
                        }
                    }
                },
                err_fn,
                None, // No specific latency requirement
            ).expect("Failed to build input stream")
        },
        SampleFormat::U16 => {
            let writer_clone = Arc::clone(&writer);
            let buffer_clone = Arc::clone(&intermediate_buffer);
            device.build_input_stream(
                &config.into(),
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    if debug {
                        println!("Received data with length: {}", data.len());
                    }
                    let mut writer_lock = writer_clone.lock().unwrap();
                    let mut buffer_lock = buffer_clone.lock().unwrap();
                    if let Some(ref mut writer) = *writer_lock {
                        for frame in data.chunks(total_channels) {
                            if frame.len() >= channels.len() {
                                let sample_left = (frame[channels[0]] as i32) - 32768;
                                let sample_right = (frame[channels[1]] as i32) - 32768;
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
                                eprintln!("Buffer too small: expected at least {} channels, found {}", channels.len(), frame.len());
                            }
                        }
                    }
                },
                err_fn,
                None, // No specific latency requirement
            ).expect("Failed to build input stream")
        },
        _ => panic!("Unsupported sample format"),
    };

    stream.play().expect("Failed to play stream");

    thread::sleep(Duration::from_secs(record_duration));

    let mut writer_lock = writer.lock().unwrap();
    let buffer_lock = intermediate_buffer.lock().unwrap();
    if let Some(ref mut writer) = *writer_lock {
        for &sample in &*buffer_lock {
            writer.write_sample(sample).unwrap();
        }
    }

    if let Some(writer) = writer_lock.take() {
        writer.finalize().unwrap();
    }
}
