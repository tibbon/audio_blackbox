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
