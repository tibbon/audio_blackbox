use std::fs::File;
use std::io::BufWriter;

pub const DEFAULT_CHANNELS: &str = "0";
pub const DEFAULT_DEBUG: bool = false;
pub const DEFAULT_DURATION: u64 = 30;
pub const DEFAULT_OUTPUT_MODE: &str = "single";
pub const DEFAULT_SILENCE_THRESHOLD: f32 = 0.01;
pub const MAX_CHANNELS: usize = 64;

// Constants for continuous recording mode
pub const DEFAULT_CONTINUOUS_MODE: bool = false;
pub const DEFAULT_RECORDING_CADENCE: u64 = 300; // 5 minutes
pub const DEFAULT_OUTPUT_DIR: &str = "recordings";
pub const DEFAULT_PERFORMANCE_LOGGING: bool = false;
// Disk space monitoring
pub const DEFAULT_MIN_DISK_SPACE_MB: u64 = 500;
/// How often the writer thread checks available disk space.
pub const DISK_CHECK_INTERVAL_SECS: u64 = 10;

// Ring buffer constants
/// How many seconds of audio the ring buffer can hold (at device sample rate * channels).
pub const RING_BUFFER_SECONDS: usize = 2;
/// How many f32 samples the writer thread reads per iteration.
pub const WRITER_THREAD_READ_CHUNK: usize = 4096;

// Type definitions to make complex types more readable
pub type WavWriterType = hound::WavWriter<BufWriter<File>>;
