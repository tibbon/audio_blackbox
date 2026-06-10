use crate::error::BlackboxError;
use std::error::Error;

/// Source chain on `WavSource` must be reachable so callers can downcast or
/// log the underlying `hound::Error` cause.
#[test]
fn test_wav_source_chain_preserved() {
    let inner = hound::Error::FormatError("not a wav");
    let err = BlackboxError::WavSource {
        context: "Failed to open WAV file at /tmp/foo.wav".to_string(),
        source: Box::new(inner),
    };

    let src = err.source().expect("WavSource must expose its source");
    assert!(
        src.downcast_ref::<hound::Error>().is_some(),
        "source should downcast to hound::Error, got {src:?}"
    );

    // Display still includes the context (path + reason) for human-readable logs.
    let displayed = format!("{err}");
    assert!(
        displayed.contains("/tmp/foo.wav"),
        "Display should include the file path: {displayed}"
    );
}

#[test]
fn test_audio_device_source_chain_preserved() {
    use std::io;
    let inner = io::Error::other("device timed out");
    let err = BlackboxError::AudioDeviceSource {
        context: "Failed to build input stream".to_string(),
        source: Box::new(inner),
    };

    let src = err
        .source()
        .expect("AudioDeviceSource must expose its source");
    assert!(
        src.downcast_ref::<io::Error>().is_some(),
        "source should downcast to io::Error, got {src:?}"
    );
}

#[test]
fn test_insufficient_disk_space_display_and_no_source() {
    let err = BlackboxError::InsufficientDiskSpace {
        available_mb: 12,
        required_mb: 500,
    };
    let displayed = format!("{err}");
    assert!(
        displayed.contains("12") && displayed.contains("500"),
        "Display should surface both numeric fields: {displayed}"
    );
    // No nested cause — this is a precondition failure, not a wrapped error.
    assert!(err.source().is_none());
}

/// Tuple-style variants without a source must still report `source() == None`
/// (regression guard against accidentally adding a stray `#[source]` to them).
#[test]
fn test_string_only_variants_have_no_source() {
    let cases: &[BlackboxError] = &[
        BlackboxError::AudioDevice("oops".into()),
        BlackboxError::ChannelParse("oops".into()),
        BlackboxError::Wav("oops".into()),
    ];
    for err in cases {
        assert!(err.source().is_none(), "{err:?} should not carry a source");
    }
}

/// `Io(#[from] std::io::Error)` must expose its underlying io::Error via
/// `source()` so log forwarders can downcast and inspect the OS error.
/// Round 3 review (DOLL-130) found that `Io` had no targeted variant test
/// — only the source-bearing struct variants were directly covered.
#[test]
fn test_io_variant_preserves_source() {
    use std::io;
    let inner = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
    let err: BlackboxError = inner.into();
    let src = err.source().expect("Io must expose its source");
    let downcast = src
        .downcast_ref::<io::Error>()
        .expect("Io::source should downcast to io::Error");
    assert_eq!(downcast.kind(), io::ErrorKind::PermissionDenied);
}

/// DOLL-457: `full_chain()` must append every `source()` level after the
/// top-level Display — `Display` on the `*Source` variants prints only the
/// context, and `blackbox_get_last_error` (built from `full_chain`) is the
/// Swift UI's only diagnostic. Without the chain, the actual cpal/hound
/// cause never reaches the user.
#[test]
fn test_full_chain_includes_root_cause() {
    use std::io;
    let root = io::Error::new(io::ErrorKind::PermissionDenied, "device is busy");
    let err = BlackboxError::AudioDeviceSource {
        context: "Failed to build input stream".to_string(),
        source: Box::new(root),
    };

    // Display alone drops the cause (this is the bug full_chain fixes)…
    assert!(!format!("{err}").contains("device is busy"));
    // …full_chain carries both levels.
    let chain = err.full_chain();
    assert!(
        chain.contains("Failed to build input stream") && chain.contains("device is busy"),
        "full_chain must include context AND root cause: {chain}"
    );
}

/// `Io`'s Display already embeds the io::Error message; full_chain must not
/// print it twice ("I/O error: denied: denied").
#[test]
fn test_full_chain_does_not_duplicate_io_message() {
    use std::io;
    let err: BlackboxError = io::Error::new(io::ErrorKind::PermissionDenied, "denied").into();
    let chain = err.full_chain();
    assert_eq!(
        chain.matches("denied").count(),
        1,
        "io message must appear exactly once: {chain}"
    );
}

/// Variants without a source: full_chain is just Display.
#[test]
fn test_full_chain_equals_display_when_no_source() {
    let err = BlackboxError::Wav("bad header".into());
    assert_eq!(err.full_chain(), err.to_string());
}
