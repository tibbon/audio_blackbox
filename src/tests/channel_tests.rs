use crate::constants::MAX_CHANNELS;
use crate::error::BlackboxError;
use crate::utils::parse_channel_string;

#[test]
fn test_single_channel() {
    assert_eq!(parse_channel_string("0").unwrap(), vec![0]);
    assert_eq!(parse_channel_string("1").unwrap(), vec![1]);
    assert_eq!(parse_channel_string("63").unwrap(), vec![63]);
}

#[test]
fn test_multiple_channels() {
    assert_eq!(parse_channel_string("0,1,2").unwrap(), vec![0, 1, 2]);
    assert_eq!(parse_channel_string("1,3,5").unwrap(), vec![1, 3, 5]);
    assert_eq!(
        parse_channel_string("0,1,2,3,4").unwrap(),
        vec![0, 1, 2, 3, 4]
    );
}

#[test]
fn test_channel_ranges() {
    assert_eq!(parse_channel_string("0-2").unwrap(), vec![0, 1, 2]);
    assert_eq!(parse_channel_string("1-3").unwrap(), vec![1, 2, 3]);
    assert_eq!(parse_channel_string("0-4").unwrap(), vec![0, 1, 2, 3, 4]);
}

#[test]
fn test_mixed_ranges_and_singles() {
    assert_eq!(
        parse_channel_string("0,1-3,5").unwrap(),
        vec![0, 1, 2, 3, 5]
    );
    assert_eq!(
        parse_channel_string("1-3,5,7-9").unwrap(),
        vec![1, 2, 3, 5, 7, 8, 9]
    );
}

#[test]
fn test_duplicate_channels() {
    assert_eq!(parse_channel_string("0,0,0").unwrap(), vec![0]);
    assert_eq!(parse_channel_string("1-3,2,3").unwrap(), vec![1, 2, 3]);
}

#[test]
fn test_invalid_channels() {
    // Pattern-match the variant rather than just `is_err()` (DOLL-117) —
    // a regression that returned a different error category (e.g. Io
    // instead of ChannelParse) would have silently passed the old test.
    let cases = [
        // Channel number exceeding maximum
        MAX_CHANNELS.to_string(),
        "255".to_string(),
        // Invalid range
        "2-1".to_string(),
        "3-0".to_string(),
        // Invalid format
        String::new(),
        ",".to_string(),
        "1-".to_string(),
        "-1".to_string(),
        "1--2".to_string(),
        "abc".to_string(),
    ];
    for s in &cases {
        let err = parse_channel_string(s)
            .err()
            .unwrap_or_else(|| panic!("expected error from input {s:?}"));
        assert!(
            matches!(err, BlackboxError::ChannelParse(_)),
            "input {s:?} produced wrong variant: {err:?}"
        );
    }
}

#[test]
fn test_whitespace_handling() {
    assert_eq!(parse_channel_string(" 0 ").unwrap(), vec![0]);
    assert_eq!(parse_channel_string("0 , 1").unwrap(), vec![0, 1]);
    assert_eq!(parse_channel_string("1 - 3").unwrap(), vec![1, 2, 3]);
}

#[test]
fn test_empty_input() {
    assert!(parse_channel_string("").is_err());
    assert!(parse_channel_string(" ").is_err());
    assert!(parse_channel_string(",").is_err());
    assert!(parse_channel_string(" ").is_err());
}

#[test]
fn test_complex_combinations() {
    assert_eq!(
        parse_channel_string("0,1-3,5,7-9,11").unwrap(),
        vec![0, 1, 2, 3, 5, 7, 8, 9, 11]
    );
    assert_eq!(
        parse_channel_string("1-3,5,7-9,11,13-15").unwrap(),
        vec![1, 2, 3, 5, 7, 8, 9, 11, 13, 14, 15]
    );
}

#[test]
fn test_edge_cases() {
    // Test maximum valid channel
    assert_eq!(
        parse_channel_string(&(MAX_CHANNELS - 1).to_string()).unwrap(),
        vec![MAX_CHANNELS - 1]
    );

    // Test zero-length range
    assert_eq!(parse_channel_string("0-0").unwrap(), vec![0]);

    // Test single channel with range
    assert_eq!(parse_channel_string("0,0-0").unwrap(), vec![0]);
}
