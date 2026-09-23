//! Compile-time constants for the recording engine.
//!
//! Two categories:
//! 1. **`DEFAULT_*`** — values `AppConfig` falls back to when neither
//!    TOML nor env vars supply one. Documented inline; the canonical
//!    surface for "what do you get if you do nothing" is here.
//! 2. **Tunables** — `MAX_CHANNELS` (255, the FFI / `peakBuffer` cap),
//!    `RING_BUFFER_SECONDS` (5, RT-thread runway), `WRITER_THREAD_READ_CHUNK`
//!    (16k samples per drain), `CacheAlignedPeak` (64-byte aligned
//!    `AtomicU32` to prevent false sharing with the UI reader thread).
//! 3. **`OutputMode`** — the `single` / `split` enum threaded through
//!    the `AudioProcessor` trait. Use `OutputMode::parse` /
//!    `OutputMode::as_str` at config-load boundaries; downstream code
//!    pattern-matches on the enum.

pub(crate) const DEFAULT_CHANNELS: &str = "0";
pub(crate) const DEFAULT_DEBUG: bool = false;
pub(crate) const DEFAULT_DURATION: u64 = 30;
pub(crate) const DEFAULT_OUTPUT_MODE: &str = "single";
pub(crate) const DEFAULT_SILENCE_THRESHOLD: f32 = 0.01;
pub(crate) const MAX_CHANNELS: usize = 255;

// Constants for continuous recording mode
pub(crate) const DEFAULT_CONTINUOUS_MODE: bool = false;
pub(crate) const DEFAULT_RECORDING_CADENCE: u64 = 300; // 5 minutes
/// Longest accepted rotation cadence in seconds (about 195 days). The RT
/// rotation threshold is `sample_rate * channels * cadence` in a `u64`; this
/// is the largest cadence for which that product can't overflow for any
/// `u32` rate and up to `MAX_CHANNELS` channels. A wrapped product used to
/// give a tiny threshold and a rotation storm.
pub(crate) const MAX_RECORDING_CADENCE: u64 = u64::MAX / (u32::MAX as u64 * MAX_CHANNELS as u64);
pub(crate) const DEFAULT_OUTPUT_DIR: &str = "recordings";
pub(crate) const DEFAULT_PERFORMANCE_LOGGING: bool = false;
pub(crate) const DEFAULT_BITS_PER_SAMPLE: u16 = 24;
// Disk space monitoring
pub(crate) const DEFAULT_MIN_DISK_SPACE_MB: u64 = 500;
// Silence gate
pub(crate) const DEFAULT_SILENCE_GATE_ENABLED: bool = true;
pub(crate) const DEFAULT_SILENCE_GATE_TIMEOUT_SECS: u64 = 300;

// Ring buffer constants
/// How many seconds of audio the ring buffer can hold (at device sample rate * channels).
pub const RING_BUFFER_SECONDS: usize = 5;
/// How many f32 samples the writer thread reads per iteration.
///
/// Larger chunks reduce per-iteration overhead (fewer `read_chunk()` atomics,
/// `write_samples()` calls, and peak publish cycles). At 48 kHz / 64 ch,
/// 16 384 samples ≈ 5.3 ms — well within the 33 ms meter polling window.
pub(crate) const WRITER_THREAD_READ_CHUNK: usize = 16_384;

/// Cache-line-aligned atomic peak level.
///
/// Each channel's peak is stored in its own cache line (64 bytes) to prevent false
/// sharing between the writer thread (which updates peaks) and the Swift UI thread
/// (which reads them at ~30 Hz). Without alignment, multiple 4-byte `AtomicU32`
/// values pack into the same cache line, causing unnecessary invalidation traffic
/// at high channel counts (16+).
#[repr(C, align(64))]
#[derive(Debug)]
pub(crate) struct CacheAlignedPeak {
    pub value: std::sync::atomic::AtomicU32,
}

impl CacheAlignedPeak {
    pub(crate) fn new(val: u32) -> Self {
        Self {
            value: std::sync::atomic::AtomicU32::new(val),
        }
    }

    /// Raise the stored peak to `peak` if it is higher. The writer calls
    /// this for every batch, so a reader sees the loudest batch since its
    /// last [`take`](Self::take), not just the most recent one.
    ///
    /// Integer `fetch_max` on the bits orders non-negative floats correctly
    /// (IEEE 754 bit patterns sort like the values for sign bit 0). Peaks are
    /// `abs()` of finite samples, so never negative or NaN; a `-0.0` from a
    /// caller would sort above everything, so it is normalised first.
    pub(crate) fn raise(&self, peak: f32) {
        let bits = if peak > 0.0 { peak.to_bits() } else { 0 };
        self.value
            .fetch_max(bits, std::sync::atomic::Ordering::Relaxed);
    }

    /// Read the peak accumulated since the last `take` and reset it to 0.
    pub(crate) fn take(&self) -> f32 {
        f32::from_bits(self.value.swap(0, std::sync::atomic::Ordering::Relaxed))
    }
}

/// Output mode for recording.
///
/// Stored as a 1-byte enum instead of a heap-allocated `String` so the hot-path
/// match in `write_samples()` compiles to a jump table (single integer comparison)
/// rather than a string comparison per frame chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OutputMode {
    /// Single file: mono/stereo for ≤2 channels, interleaved multichannel for >2.
    #[default]
    Single,
    /// One WAV file per channel.
    Split,
}

impl OutputMode {
    /// Parse from a config string. Returns `None` for invalid values.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "single" => Some(Self::Single),
            "split" => Some(Self::Split),
            _ => None,
        }
    }

    /// Config-file spelling of this mode; the inverse of [`parse`](Self::parse).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Split => "split",
        }
    }
}

impl std::fmt::Display for OutputMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
