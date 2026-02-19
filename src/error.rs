/// Custom error type for the blackbox audio recorder.
#[derive(Debug, thiserror::Error)]
pub enum BlackboxError {
    #[error("Audio device error: {0}")]
    AudioDevice(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Channel parse error: {0}")]
    ChannelParse(String),

    #[error("WAV error: {0}")]
    Wav(String),
}
