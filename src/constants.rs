use std::fs::File;
use std::io::BufWriter;
use std::sync::{Arc, Mutex};

pub const INTERMEDIATE_BUFFER_SIZE: usize = 512;
pub const DEFAULT_CHANNELS: &str = "0";
pub const DEFAULT_DEBUG: bool = false;
pub const DEFAULT_DURATION: u64 = 30;
pub const DEFAULT_OUTPUT_MODE: &str = "wav";
pub const DEFAULT_SILENCE_THRESHOLD: f32 = 0.01;
pub const MAX_CHANNELS: usize = 64;

// Constants for continuous recording mode
pub const DEFAULT_CONTINUOUS_MODE: bool = false;
pub const DEFAULT_RECORDING_CADENCE: u64 = 300; // 5 minutes
pub const DEFAULT_OUTPUT_DIR: &str = "recordings";
pub const DEFAULT_PERFORMANCE_LOGGING: bool = false;
pub const PERFORMANCE_LOG_INTERVAL: u64 = 3600; // 1 hour in seconds

// Type definitions to make complex types more readable
pub type WavWriterType = hound::WavWriter<BufWriter<File>>;
pub type MultiChannelWriters = Arc<Mutex<Vec<Option<WavWriterType>>>>;

// Environment variable names
pub const ENV_OUTPUT_DIR: &str = "BLACKBOX_OUTPUT_DIR";
pub const ENV_CHANNELS: &str = "BLACKBOX_CHANNELS";
pub const ENV_DURATION: &str = "BLACKBOX_DURATION";
pub const ENV_OUTPUT_MODE: &str = "BLACKBOX_OUTPUT_MODE";
pub const ENV_DEBUG: &str = "BLACKBOX_DEBUG";
pub const ENV_SILENCE_THRESHOLD: &str = "BLACKBOX_SILENCE_THRESHOLD";
pub const ENV_CONTINUOUS_MODE: &str = "BLACKBOX_CONTINUOUS_MODE";
pub const ENV_RECORDING_CADENCE: &str = "BLACKBOX_RECORDING_CADENCE";
pub const ENV_PERFORMANCE_LOGGING: &str = "BLACKBOX_PERFORMANCE_LOGGING";
