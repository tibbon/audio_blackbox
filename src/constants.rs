use std::fs::File;
use std::io::BufWriter;
use std::sync::{Arc, Mutex};

pub const INTERMEDIATE_BUFFER_SIZE: usize = 512;
pub const DEFAULT_CHANNELS: &str = "0,1";
pub const DEFAULT_DEBUG: &str = "false";
pub const DEFAULT_DURATION: &str = "10";
pub const DEFAULT_OUTPUT_MODE: &str = "single";
pub const DEFAULT_SILENCE_THRESHOLD: &str = "0"; // 0 means don't delete silent files
pub const MAX_CHANNELS: usize = 64;

// Constants for continuous recording mode
pub const DEFAULT_CONTINUOUS_MODE: &str = "false";
pub const DEFAULT_RECORDING_CADENCE: &str = "300"; // 5 minutes (300 seconds)
pub const DEFAULT_OUTPUT_DIR: &str = "./recordings";
pub const DEFAULT_PERFORMANCE_LOGGING: &str = "false";
pub const PERFORMANCE_LOG_INTERVAL: u64 = 3600; // 1 hour in seconds

// Type definitions to make complex types more readable
pub type WavWriterType = hound::WavWriter<BufWriter<File>>;
pub type MultiChannelWriters = Arc<Mutex<Vec<Option<WavWriterType>>>>;
