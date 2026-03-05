pub const DEFAULT_CHANNELS: &str = "0";
pub const DEFAULT_DEBUG: bool = false;
pub const DEFAULT_DURATION: u64 = 30;
pub const DEFAULT_OUTPUT_MODE: &str = "single";
pub const DEFAULT_SILENCE_THRESHOLD: f32 = 0.01;
pub const MAX_CHANNELS: usize = 255;

// Constants for continuous recording mode
pub const DEFAULT_CONTINUOUS_MODE: bool = false;
pub const DEFAULT_RECORDING_CADENCE: u64 = 300; // 5 minutes
pub const DEFAULT_OUTPUT_DIR: &str = "recordings";
pub const DEFAULT_PERFORMANCE_LOGGING: bool = false;
pub const DEFAULT_BITS_PER_SAMPLE: u16 = 24;
// Disk space monitoring
pub const DEFAULT_MIN_DISK_SPACE_MB: u64 = 500;
// Silence gate
pub const DEFAULT_SILENCE_GATE_ENABLED: bool = true;
pub const DEFAULT_SILENCE_GATE_TIMEOUT_SECS: u64 = 300;
/// How often the writer thread checks available disk space.
pub const DISK_CHECK_INTERVAL_SECS: u64 = 10;

// Ring buffer constants
/// How many seconds of audio the ring buffer can hold (at device sample rate * channels).
pub const RING_BUFFER_SECONDS: usize = 5;
/// How many f32 samples the writer thread reads per iteration.
///
/// Larger chunks reduce per-iteration overhead (fewer `read_chunk()` atomics,
/// `write_samples()` calls, and peak publish cycles). At 48 kHz / 64 ch,
/// 16 384 samples ≈ 5.3 ms — well within the 33 ms meter polling window.
pub const WRITER_THREAD_READ_CHUNK: usize = 16_384;

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

/// Output mode for recording.
///
/// Stored as a 1-byte enum instead of a heap-allocated `String` so the hot-path
/// match in `write_samples()` compiles to a jump table (single integer comparison)
/// rather than a string comparison per frame chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Single file: mono/stereo for ≤2 channels, interleaved multichannel for >2.
    Single,
    /// One WAV file per channel.
    Split,
}

impl OutputMode {
    /// Parse from a config string. Returns `None` for invalid values.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "single" => Some(Self::Single),
            "split" => Some(Self::Split),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Split => "split",
        }
    }
}
