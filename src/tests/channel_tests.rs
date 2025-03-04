use crate::constants::MAX_CHANNELS;
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
    // Test channel number exceeding maximum
    assert!(parse_channel_string(&MAX_CHANNELS.to_string()).is_err());
    assert!(parse_channel_string("64").is_err());

    // Test invalid range
    assert!(parse_channel_string("2-1").is_err());
    assert!(parse_channel_string("3-0").is_err());

    // Test invalid format
    assert!(parse_channel_string("").is_err());
    assert!(parse_channel_string(",").is_err());
    assert!(parse_channel_string("1-").is_err());
    assert!(parse_channel_string("-1").is_err());
    assert!(parse_channel_string("1--2").is_err());
    assert!(parse_channel_string("abc").is_err());
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
