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
pub const DEFAULT_BITS_PER_SAMPLE: u16 = 24;
// Disk space monitoring
pub const DEFAULT_MIN_DISK_SPACE_MB: u64 = 500;
/// How often the writer thread checks available disk space.
pub const DISK_CHECK_INTERVAL_SECS: u64 = 10;

// Ring buffer constants
/// How many seconds of audio the ring buffer can hold (at device sample rate * channels).
pub const RING_BUFFER_SECONDS: usize = 5;
/// How many f32 samples the writer thread reads per iteration.
pub const WRITER_THREAD_READ_CHUNK: usize = 4096;

/// Cache-line-aligned atomic peak level.
///
/// Each channel's peak is stored in its own cache line (64 bytes) to prevent false
/// sharing between the writer thread (which updates peaks) and the Swift UI thread
/// (which reads them at ~30 Hz). Without alignment, multiple 4-byte `AtomicU32`
/// values pack into the same cache line, causing unnecessary invalidation traffic
/// at high channel counts (16+).
#[repr(C, align(64))]
pub struct CacheAlignedPeak {
    pub value: std::sync::atomic::AtomicU32,
}

impl CacheAlignedPeak {
    pub fn new(val: u32) -> Self {
        Self {
            value: std::sync::atomic::AtomicU32::new(val),
        }
    }
}

// Type definitions to make complex types more readable
pub type WavWriterType = hound::WavWriter<BufWriter<File>>;
