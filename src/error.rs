/// Custom error type for the blackbox audio recorder.
///
/// Some variants carry a `#[source]` chain so callers can recover the
/// underlying `cpal::*Error` / `hound::Error` via `std::error::Error::source()`.
/// The string-only variants are kept for cases where there is no underlying
/// error to wrap (e.g. config validation failures synthesized from raw input).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BlackboxError {
    #[error("Audio device error: {0}")]
    AudioDevice(String),

    /// Audio-device-layer error that wraps a concrete underlying error
    /// (cpal device-config, build-stream, play-stream, etc.).
    #[error("Audio device error: {context}")]
    AudioDeviceSource {
        context: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Channel parse error: {0}")]
    ChannelParse(String),

    #[error("WAV error: {0}")]
    Wav(String),

    /// WAV-layer error that wraps the underlying `hound::Error` (or other
    /// concrete WAV-related error).
    #[error("WAV error: {context}")]
    WavSource {
        context: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// Reported from the writer thread when the configured
    /// `min_disk_space_mb` precondition fails. Distinct from `Io` so the UI
    /// can surface a "free up space" message rather than a generic IO error.
    #[error("Insufficient disk space: {available_mb} MB available, {required_mb} MB required")]
    InsufficientDiskSpace { available_mb: u64, required_mb: u64 },
}
