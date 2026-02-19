use std::ffi::{CStr, CString};

use crate::ffi::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Safely read and free a C string returned by the FFI.
unsafe fn read_and_free(ptr: *mut std::os::raw::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(str::to_string);
    blackbox_free_string(ptr);
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_destroy_null_config() {
    let handle = blackbox_create(std::ptr::null());
    assert!(!handle.is_null(), "create with null config should succeed");
    blackbox_destroy(handle);
}

#[test]
fn test_create_destroy_empty_config() {
    let json = CString::new("").unwrap();
    let handle = blackbox_create(json.as_ptr());
    assert!(!handle.is_null());
    blackbox_destroy(handle);
}

#[test]
fn test_create_with_valid_json() {
    let json = CString::new(r#"{"output_dir": "/tmp/blackbox_ffi_test"}"#).unwrap();
    let handle = blackbox_create(json.as_ptr());
    assert!(!handle.is_null());

    // Verify config was applied by reading it back
    let config_ptr = blackbox_get_config_json(handle);
    let config_str = unsafe { read_and_free(config_ptr) }.expect("config should be readable");
    assert!(
        config_str.contains("/tmp/blackbox_ffi_test"),
        "config should contain our output_dir: {}",
        config_str
    );

    blackbox_destroy(handle);
}

#[test]
fn test_create_with_invalid_json() {
    let json = CString::new("{not valid json}").unwrap();
    let handle = blackbox_create(json.as_ptr());
    // Should fall back to defaults, not return null
    assert!(!handle.is_null());
    blackbox_destroy(handle);
}

#[test]
fn test_destroy_null_is_safe() {
    blackbox_destroy(std::ptr::null_mut());
}

#[test]
fn test_is_recording_null_handle() {
    assert!(!blackbox_is_recording(std::ptr::null()));
}

#[test]
fn test_is_recording_not_started() {
    let handle = blackbox_create(std::ptr::null());
    assert!(!blackbox_is_recording(handle));
    blackbox_destroy(handle);
}

#[test]
fn test_stop_recording_when_not_recording() {
    let handle = blackbox_create(std::ptr::null());
    // Stopping when not recording should succeed (no-op)
    let result = blackbox_stop_recording(handle);
    assert_eq!(result, 0);
    blackbox_destroy(handle);
}

#[test]
fn test_null_handle_error_returns() {
    assert_eq!(blackbox_start_recording(std::ptr::null_mut()), -1);
    assert_eq!(blackbox_stop_recording(std::ptr::null_mut()), -1);
    assert!(blackbox_get_status_json(std::ptr::null()).is_null());
    assert!(blackbox_get_config_json(std::ptr::null()).is_null());
    assert!(blackbox_get_last_error(std::ptr::null()).is_null());
    assert_eq!(
        blackbox_set_config_json(std::ptr::null_mut(), std::ptr::null()),
        -1
    );
}

#[test]
fn test_get_status_json() {
    let handle = blackbox_create(std::ptr::null());
    let status_ptr = blackbox_get_status_json(handle);
    let status = unsafe { read_and_free(status_ptr) }.expect("status should be readable");

    let parsed: serde_json::Value = serde_json::from_str(&status).expect("should be valid JSON");
    assert_eq!(parsed["recording"], false);
    assert_eq!(parsed["write_errors"], 0);

    blackbox_destroy(handle);
}

#[test]
fn test_get_config_json_roundtrip() {
    let json = CString::new(r#"{"debug": true, "duration": 60}"#).unwrap();
    let handle = blackbox_create(json.as_ptr());

    let config_ptr = blackbox_get_config_json(handle);
    let config_str = unsafe { read_and_free(config_ptr) }.expect("config should be readable");

    let parsed: serde_json::Value =
        serde_json::from_str(&config_str).expect("should be valid JSON");
    assert_eq!(parsed["debug"], true);
    assert_eq!(parsed["duration"], 60);

    blackbox_destroy(handle);
}

#[test]
fn test_set_config_json() {
    let handle = blackbox_create(std::ptr::null());

    let update = CString::new(r#"{"debug": true, "duration": 120}"#).unwrap();
    let result = blackbox_set_config_json(handle, update.as_ptr());
    assert_eq!(result, 0);

    // Verify the update
    let config_ptr = blackbox_get_config_json(handle);
    let config_str = unsafe { read_and_free(config_ptr) }.expect("config should be readable");
    let parsed: serde_json::Value =
        serde_json::from_str(&config_str).expect("should be valid JSON");
    assert_eq!(parsed["debug"], true);
    assert_eq!(parsed["duration"], 120);

    blackbox_destroy(handle);
}

#[test]
fn test_set_config_json_null_json() {
    let handle = blackbox_create(std::ptr::null());
    let result = blackbox_set_config_json(handle, std::ptr::null());
    assert_eq!(result, -1);
    blackbox_destroy(handle);
}

#[test]
fn test_set_config_json_invalid() {
    let handle = blackbox_create(std::ptr::null());
    let bad_json = CString::new("{invalid}").unwrap();
    let result = blackbox_set_config_json(handle, bad_json.as_ptr());
    assert_eq!(result, -1);

    // Should have an error message
    let err_ptr = blackbox_get_last_error(handle);
    let err = unsafe { read_and_free(err_ptr) };
    assert!(err.is_some(), "should have error message");
    assert!(
        err.unwrap().contains("Invalid config JSON"),
        "error should mention invalid JSON"
    );

    blackbox_destroy(handle);
}

#[test]
fn test_get_last_error_initially_null() {
    let handle = blackbox_create(std::ptr::null());
    let err_ptr = blackbox_get_last_error(handle);
    assert!(err_ptr.is_null(), "no error should exist initially");
    blackbox_destroy(handle);
}

#[test]
fn test_list_input_devices() {
    let devices_ptr = blackbox_list_input_devices();
    // Should always return something (at least "[]" on systems with no devices)
    let devices_str =
        unsafe { read_and_free(devices_ptr) }.expect("device list should be readable");

    let parsed: serde_json::Value =
        serde_json::from_str(&devices_str).expect("should be valid JSON");
    assert!(parsed.is_array(), "should be a JSON array");
}

#[test]
fn test_free_string_null_is_safe() {
    blackbox_free_string(std::ptr::null_mut());
}

#[test]
fn test_config_with_input_device() {
    let json = CString::new(r#"{"input_device": "Nonexistent Device"}"#).unwrap();
    let handle = blackbox_create(json.as_ptr());

    let config_ptr = blackbox_get_config_json(handle);
    let config_str = unsafe { read_and_free(config_ptr) }.expect("config should be readable");
    assert!(
        config_str.contains("Nonexistent Device"),
        "should contain our device name"
    );

    blackbox_destroy(handle);
}
