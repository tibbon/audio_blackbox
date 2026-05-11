//! Typed errors for the recording engine.
//!
//! Source-chain contract:
//! - `*Source` variants (`AudioDeviceSource`, `WavSource`) carry
//!   `#[source]` so callers recover the underlying `cpal::*Error` /
//!   `hound::Error` via `std::error::Error::source()`.
//! - `Io(#[from] std::io::Error)` likewise exposes the underlying
//!   `io::Error`.
//! - String-only variants (`AudioDevice`, `ChannelParse`, `Wav`) and
//!   `InsufficientDiskSpace` carry no source — `error.source()` returns
//!   `None`. Use them when there's no concrete underlying error to
//!   wrap (e.g. a synthesized message).
//!
//! Mapping to FFI codes lives in `src/ffi.rs` (`BLACKBOX_ERR_*`).

/// Custom error type for the blackbox audio recorder.
///
/// Some variants carry a `#[source]` chain so callers can recover the
/// underlying `cpal::*Error` / `hound::Error` via `std::error::Error::source()`.
/// The string-only variants are kept for cases where there is no underlying
/// error to wrap (e.g. config validation failures synthesized from raw input).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BlackboxError {
    /// Audio-device-layer error from a context that has no underlying
    /// error to wrap (e.g. a synthesized message about device discovery
    /// state). Prefer [`AudioDeviceSource`](Self::AudioDeviceSource)
    /// when a concrete underlying error is available.
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

    // `Config(String)` was removed in DOLL-189 — config validation is
    // forgiving (bad TOML / bad env vars log + fall back to defaults).
    // No production path constructs this; FFI now maps ChannelParse to
    // BLACKBOX_ERR_CONFIG on its own. If strict validation is wanted in
    // the future, reintroduce the variant alongside actually returning
    // it from `AppConfig::load` / `apply_env_vars`.
    /// Filesystem-layer error, automatically converted from
    /// `std::io::Error` via `?` (the `#[from]` derives `From<io::Error>`).
    /// Used for WAV-file I/O, output-directory creation, and disk-space
    /// queries; the underlying `io::Error` is recoverable via
    /// `std::error::Error::source()`.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Channel-spec parse failure (e.g. `"1,bogus,3"` or a range like
    /// `"5-2"` with end < start). The string is the offending input
    /// formatted for user display.
    #[error("Channel parse error: {0}")]
    ChannelParse(String),

    /// WAV-layer error from a context that has no underlying error to
    /// wrap (e.g. an internal validation message). Prefer
    /// [`WavSource`](Self::WavSource) when a concrete `hound::Error` or
    /// other underlying error is available.
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
