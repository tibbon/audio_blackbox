// DOLL-346: this suite is now linted under the same pedantic/nursery config
// as the rest of the crate. Two test-idiomatic patterns are allowed here:
//   - similar_names: the `*_ptr` / `*_str` pairs (raw pointer vs decoded
//     String) are deliberately parallel and clearer than contrived renames.
//   - float_cmp: assertions compare against exact sentinel values the FFI
//     writes/leaves (e.g. an untouched 99.0 fill), where exactness is the point.
#![allow(clippy::similar_names, clippy::float_cmp)]

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

    // A clean parse must not leave a creation error behind (DOLL-456).
    assert!(
        blackbox_get_last_error(handle).is_null(),
        "valid JSON must not set last_error"
    );

    blackbox_destroy(handle);
}

#[test]
fn test_create_with_invalid_json() {
    let json = CString::new("{not valid json}").unwrap();
    let handle = blackbox_create(json.as_ptr());
    assert!(!handle.is_null());

    // Read back the config and confirm it matches AppConfig::default().
    // Without this anchor the test would still pass even if the fallback
    // silently chose a non-default config (DOLL-109). Compare via the
    // strongly-typed AppConfig (not JSON) to avoid f32/f64 round-trip
    // precision artifacts.
    let config_ptr = blackbox_get_config_json(handle);
    let config_str = unsafe { read_and_free(config_ptr) }.expect("config readable");
    let parsed: crate::AppConfig =
        serde_json::from_str(&config_str).expect("config parseable as AppConfig");
    let defaults = crate::AppConfig::default();
    assert_eq!(
        format!("{parsed:?}"),
        format!("{defaults:?}"),
        "invalid JSON must fall back to AppConfig::default()"
    );

    // DOLL-456: the swallowed parse must now be detectable — last_error
    // carries the serde message instead of leaving the caller blind.
    let err_ptr = blackbox_get_last_error(handle);
    let err = unsafe { read_and_free(err_ptr) }.expect("last_error must be set");
    assert!(
        err.contains("Invalid config JSON"),
        "last_error should describe the discarded config: {err}"
    );

    blackbox_destroy(handle);
}

/// DOLL-456: a syntactically valid document with ONE type-mismatched field
/// makes serde reject the whole document. The handle must still be created
/// (defaults), but last_error must record that every caller setting was
/// discarded — previously this was completely silent.
#[test]
fn test_create_type_mismatch_sets_last_error_and_keeps_defaults() {
    let json = CString::new(r#"{"debug": true, "duration": "sixty"}"#).unwrap();
    let handle = blackbox_create(json.as_ptr());
    assert!(!handle.is_null(), "creation must still succeed");

    let config_ptr = blackbox_get_config_json(handle);
    let config_str = unsafe { read_and_free(config_ptr) }.expect("config readable");
    let parsed: crate::AppConfig =
        serde_json::from_str(&config_str).expect("config parseable as AppConfig");
    let defaults = crate::AppConfig::default();
    assert_eq!(
        format!("{parsed:?}"),
        format!("{defaults:?}"),
        "a type mismatch rejects the whole document → defaults (even the valid debug field)"
    );

    let err_ptr = blackbox_get_last_error(handle);
    let err = unsafe { read_and_free(err_ptr) }.expect("last_error must be set");
    assert!(
        err.contains("Invalid config JSON"),
        "last_error should describe the discarded config: {err}"
    );

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
    assert_eq!(result, BLACKBOX_OK);
    blackbox_destroy(handle);
}

#[test]
fn test_null_handle_error_returns() {
    assert_eq!(
        blackbox_start_recording(std::ptr::null_mut()),
        BLACKBOX_ERR_INVALID_HANDLE
    );
    assert_eq!(
        blackbox_stop_recording(std::ptr::null_mut()),
        BLACKBOX_ERR_INVALID_HANDLE
    );
    assert!(blackbox_get_config_json(std::ptr::null()).is_null());
    assert!(blackbox_get_last_error(std::ptr::null()).is_null());
    assert_eq!(
        blackbox_set_config_json(std::ptr::null_mut(), std::ptr::null()),
        BLACKBOX_ERR_INVALID_HANDLE
    );
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
    assert_eq!(result, BLACKBOX_OK);

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
    // Null JSON is an arg problem, not a handle problem — see DOLL-103.
    assert_eq!(result, BLACKBOX_ERR_INVALID_ARG);
    blackbox_destroy(handle);
}

#[test]
fn test_set_config_json_invalid() {
    let handle = blackbox_create(std::ptr::null());
    let bad_json = CString::new("{invalid}").unwrap();
    let result = blackbox_set_config_json(handle, bad_json.as_ptr());
    assert_eq!(result, BLACKBOX_ERR_CONFIG);

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

#[test]
fn test_get_status_flags_idle_handle() {
    let handle = blackbox_create(std::ptr::null());
    assert!(!handle.is_null());

    let mut flags = StatusFlags {
        write_errors: 9999,
        sample_rate: 9999,
        is_recording: true,
        gate_idle: true,
        disk_space_low: true,
        stream_error: true,
        sample_rate_changed: true,
        write_failed: true,
    };
    let rc = blackbox_get_status_flags(handle, &raw mut flags);
    assert_eq!(rc, BLACKBOX_OK);
    // A freshly created handle has not started recording; status must read idle.
    assert!(!flags.is_recording);
    assert!(!flags.gate_idle);
    assert!(!flags.disk_space_low);
    assert!(!flags.stream_error);
    assert!(!flags.sample_rate_changed);
    assert_eq!(flags.write_errors, 0);
    assert_eq!(flags.sample_rate, 0);

    blackbox_destroy(handle);
}

#[test]
fn test_get_status_flags_null_handle() {
    let mut flags = StatusFlags {
        write_errors: 0,
        sample_rate: 0,
        is_recording: false,
        gate_idle: false,
        disk_space_low: false,
        stream_error: false,
        sample_rate_changed: false,
        write_failed: false,
    };
    let rc = blackbox_get_status_flags(std::ptr::null(), &raw mut flags);
    assert_eq!(rc, BLACKBOX_ERR_INVALID_HANDLE);
}

#[test]
fn test_get_status_flags_null_out() {
    let handle = blackbox_create(std::ptr::null());
    let rc = blackbox_get_status_flags(handle, std::ptr::null_mut());
    // Null OUT is an arg problem, not a handle problem — see DOLL-103.
    assert_eq!(rc, BLACKBOX_ERR_INVALID_ARG);
    blackbox_destroy(handle);
}

/// Status reads must remain lock-free with respect to other handle activity:
/// Hammer the status path from 8 reader threads while a writer thread
/// flips `last_error` and config values. The lock-free claim from DOLL-84
/// is exercised here: readers never block on the recorder mutex, and they
/// must never observe a torn read (e.g. mid-update bytes in `StatusFlags`).
///
/// The handle is shared as `usize` to cross thread boundaries, since
/// Send/Sync aren't auto-derived for raw pointers — Swift does the same
/// in practice via `OpaquePointer`.
///
/// DOLL-127 round-3 tightening: the original test only had a config-mutex
/// writer, so the atomic flags read by the status path were never raced.
/// A second writer now flips `disk_space_low` directly on the cached
/// `ProcessorStatus` bundle, and readers count observations — so reverting
/// `blackbox_get_status_flags` to hardcode flag values would no longer pass.
#[test]
fn test_status_flags_concurrent_reads() {
    let handle = blackbox_create(std::ptr::null());
    assert!(!handle.is_null());
    let handle_addr = handle as usize;

    // Snapshot the status atomics bundle so a parallel writer can
    // mutate the SAME atomics that `blackbox_get_status_flags` reads.
    let bundle_w = unsafe { (*handle).test_status_bundle() };

    // Writer 1: flips config (acquires handle.config + handle.last_error
    // mutexes) while readers hammer status. If status reads were not
    // lock-free w.r.t. config writes, this would surface as readers
    // observing wedged or torn state.
    let writer_should_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer_should_stop_c = std::sync::Arc::clone(&writer_should_stop);
    let config_writer = std::thread::spawn(move || {
        let h = handle_addr as *mut BlackboxHandle;
        let mut counter: u32 = 0;
        while !writer_should_stop_c.load(std::sync::atomic::Ordering::Relaxed) {
            counter = counter.wrapping_add(1);
            let json = CString::new(format!(r#"{{"output_dir": "/tmp/race_{counter}"}}"#)).unwrap();
            let _ = blackbox_set_config_json(h, json.as_ptr());
        }
    });

    // Writer 2: flips one of the atomic flags consumed by the status
    // path. Readers track sightings of the toggled state — without this,
    // the status path could be reverted to hardcode `false` and the
    // test would still pass (DOLL-127).
    let writer_should_stop_a = std::sync::Arc::clone(&writer_should_stop);
    let atomic_writer = std::thread::spawn(move || {
        while !writer_should_stop_a.load(std::sync::atomic::Ordering::Relaxed) {
            bundle_w
                .disk_space_low
                .store(true, std::sync::atomic::Ordering::Relaxed);
            bundle_w
                .disk_space_low
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    });

    let saw_disk_low_true = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut readers = Vec::new();
    for _ in 0..8 {
        let saw_true = std::sync::Arc::clone(&saw_disk_low_true);
        readers.push(std::thread::spawn(move || {
            let h = handle_addr as *const BlackboxHandle;
            let mut flags = StatusFlags {
                write_errors: 0,
                sample_rate: 0,
                is_recording: false,
                gate_idle: false,
                disk_space_low: false,
                stream_error: false,
                sample_rate_changed: false,
                write_failed: false,
            };
            for _ in 0..10_000 {
                let rc = blackbox_get_status_flags(h, &raw mut flags);
                assert_eq!(rc, BLACKBOX_OK);
                // Idle handle → `is_recording` must always be false even
                // under concurrent config writes. Torn reads or stale
                // bytes would surface as `true` or out-of-domain values.
                assert!(!flags.is_recording);
                assert_eq!(flags.sample_rate, 0);
                if flags.disk_space_low {
                    saw_true.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let _ = blackbox_is_recording(h);
                let _ = blackbox_is_monitoring(h);
            }
        }));
    }
    for t in readers {
        t.join().expect("status reader panicked");
    }

    // Stop the writers and join.
    writer_should_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    config_writer.join().expect("config writer panicked");
    atomic_writer.join().expect("atomic writer panicked");

    // Killer-question: if the status path was reverted to hardcode
    // `disk_space_low = false` (or to skip the atomic load entirely),
    // no reader could ever observe the writer's flip.
    assert!(
        saw_disk_low_true.load(std::sync::atomic::Ordering::Relaxed),
        "no reader ever observed disk_space_low=true — status path may not be \
         reading the cached atomic"
    );

    blackbox_destroy(handle);
}

#[test]
fn test_get_peak_levels_null_handle() {
    let mut buf = [0.0_f32; 8];
    let rc = blackbox_get_peak_levels(std::ptr::null(), buf.as_mut_ptr(), 8);
    assert_eq!(rc, BLACKBOX_ERR_INVALID_HANDLE);
}

#[test]
fn test_get_peak_levels_null_out() {
    let handle = blackbox_create(std::ptr::null());
    let rc = blackbox_get_peak_levels(handle, std::ptr::null_mut(), 8);
    // Null OUT is an arg problem, not a handle problem — see DOLL-102/103.
    assert_eq!(rc, BLACKBOX_ERR_INVALID_ARG);
    blackbox_destroy(handle);
}

#[test]
fn test_get_peak_levels_negative_max() {
    let handle = blackbox_create(std::ptr::null());
    let mut buf = [0.0_f32; 8];
    let rc = blackbox_get_peak_levels(handle, buf.as_mut_ptr(), -1);
    assert_eq!(rc, BLACKBOX_ERR_INVALID_ARG);
    blackbox_destroy(handle);
}

#[test]
fn test_get_device_channel_count_invalid_utf8() {
    // A standalone 0xFF byte followed by a NUL is valid C but invalid UTF-8.
    let bad: [u8; 2] = [0xFF, 0];
    let rc = blackbox_get_device_channel_count(bad.as_ptr().cast::<std::os::raw::c_char>());
    // DOLL-104: this used to silently fall back to the system default device.
    // Now it returns INVALID_ARG so the caller can detect a corrupt buffer.
    assert_eq!(rc, BLACKBOX_ERR_INVALID_ARG);
}

#[test]
fn test_get_peak_levels_idle_returns_zero_count() {
    // Freshly created handle has no peaks — legitimate empty read returns 0
    // (NOT a negative error code).
    let handle = blackbox_create(std::ptr::null());
    let mut buf = [99.0_f32; 8];
    let rc = blackbox_get_peak_levels(handle, buf.as_mut_ptr(), 8);
    assert_eq!(rc, 0);
    // Buffer should be untouched since no channels were written.
    assert_eq!(buf, [99.0_f32; 8]);
    blackbox_destroy(handle);
}
