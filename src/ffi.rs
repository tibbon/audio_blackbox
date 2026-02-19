//! C FFI layer for BlackBox Audio Recorder.
//!
//! Exposes an opaque handle pattern so a Swift/SwiftUI frontend (or any C-compatible
//! caller) can drive the Rust audio engine without touching Rust types directly.
//!
//! All public functions use `catch_unwind` — panics must never cross the FFI boundary.
//! Complex data is exchanged as JSON strings; the caller frees them with
//! `blackbox_free_string`.

// FFI functions inherently receive raw pointers from C callers. Every function
// performs a null check before dereferencing, and the actual dereference is inside
// `catch_unwind`. Marking each function `unsafe` would be misleading since the
// null-check + catch_unwind combo already provides the safety guarantee we need.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::catch_unwind;
use std::sync::Mutex;

use crate::audio_processor::AudioProcessor;
use crate::audio_recorder::AudioRecorder;
use crate::config::AppConfig;
use crate::cpal_processor::CpalAudioProcessor;

// ---------------------------------------------------------------------------
// BlackboxHandle — opaque type exposed as `*mut BlackboxHandle` over FFI
// ---------------------------------------------------------------------------

/// Magic number to detect use-after-free or corrupted handles.
const HANDLE_MAGIC: u64 = 0xB1AC_B015_A11D_1000;

pub struct BlackboxHandle {
    magic: u64,
    config: Mutex<AppConfig>,
    recorder: Mutex<Option<AudioRecorder<CpalAudioProcessor>>>,
    last_error: Mutex<Option<String>>,
}

impl BlackboxHandle {
    fn is_valid(&self) -> bool {
        self.magic == HANDLE_MAGIC
    }

    fn set_error(&self, msg: String) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(msg);
        }
    }

    fn clear_error(&self) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a `*const c_char` to a `&str`, returning `None` on null or invalid UTF-8.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Allocate a `CString` on the heap and return a raw pointer.
/// The caller is responsible for freeing it with `blackbox_free_string`.
fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Validate a handle pointer: non-null and magic number matches.
/// Returns `None` if invalid.
fn validate_handle(handle: *const BlackboxHandle) -> Option<&'static BlackboxHandle> {
    if handle.is_null() {
        return None;
    }
    let h = unsafe { &*handle };
    if h.is_valid() { Some(h) } else { None }
}

// ---------------------------------------------------------------------------
// FFI functions
// ---------------------------------------------------------------------------

/// Create a new `BlackboxHandle` from a JSON configuration string.
///
/// If `config_json` is null or empty, default configuration is used.
/// Returns null on failure (should not happen with defaults).
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_create(config_json: *const c_char) -> *mut BlackboxHandle {
    catch_unwind(|| {
        let config = if config_json.is_null() {
            AppConfig::default()
        } else {
            let json_str = unsafe { cstr_to_str(config_json) }.unwrap_or("");
            if json_str.is_empty() {
                AppConfig::default()
            } else {
                serde_json::from_str::<AppConfig>(json_str).unwrap_or_default()
            }
        };

        let handle = Box::new(BlackboxHandle {
            magic: HANDLE_MAGIC,
            config: Mutex::new(config),
            recorder: Mutex::new(None),
            last_error: Mutex::new(None),
        });

        Box::into_raw(handle)
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Destroy a `BlackboxHandle`, freeing all resources.
///
/// If recording is in progress it will be stopped first.
/// Passing null is a safe no-op.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_destroy(handle: *mut BlackboxHandle) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(|| {
        let h = unsafe { &*handle };
        if !h.is_valid() {
            return;
        }
        let mut handle = unsafe { Box::from_raw(handle) };
        // Invalidate magic before cleanup so concurrent calls fail fast
        handle.magic = 0;
        // Stop recording if active — AudioRecorder's Drop will finalize via the processor.
        if let Ok(mut guard) = handle.recorder.lock() {
            drop(guard.take());
        }
    });
}

/// Start recording.
///
/// Creates a `CpalAudioProcessor`, wraps it in an `AudioRecorder`, and begins
/// recording using the current configuration.
///
/// Returns 0 on success, -1 on error (retrieve with `blackbox_get_last_error`).
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_start_recording(handle: *mut BlackboxHandle) -> i32 {
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return -1;
        };
        handle.clear_error();

        let config = match handle.config.lock() {
            Ok(c) => c.clone(),
            Err(e) => {
                handle.set_error(format!("Config lock poisoned: {}", e));
                return -1;
            }
        };

        let processor = match CpalAudioProcessor::with_config(&config) {
            Ok(p) => p,
            Err(e) => {
                handle.set_error(format!("Failed to create audio processor: {}", e));
                return -1;
            }
        };

        let mut recorder = AudioRecorder::with_config(processor, config);

        if let Err(e) = recorder.start_recording() {
            handle.set_error(format!("Failed to start recording: {}", e));
            return -1;
        }

        match handle.recorder.lock() {
            Ok(mut guard) => {
                *guard = Some(recorder);
                0
            }
            Err(e) => {
                handle.set_error(format!("Recorder lock poisoned: {}", e));
                -1
            }
        }
    })
    .unwrap_or(-1)
}

/// Stop recording.
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_stop_recording(handle: *mut BlackboxHandle) -> i32 {
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return -1;
        };
        handle.clear_error();

        match handle.recorder.lock() {
            Ok(mut guard) => {
                if let Some(mut recorder) = guard.take()
                    && let Err(e) = recorder.processor_mut().stop_recording()
                {
                    handle.set_error(format!("Failed to stop recording: {}", e));
                    return -1;
                }
                0
            }
            Err(e) => {
                handle.set_error(format!("Recorder lock poisoned: {}", e));
                -1
            }
        }
    })
    .unwrap_or(-1)
}

/// Check whether recording is currently active.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_is_recording(handle: *const BlackboxHandle) -> bool {
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return false;
        };
        handle
            .recorder
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|r| r.get_processor().is_recording()))
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// Return a JSON object with the current status.
///
/// Example: `{"recording": true, "input_device": "MacBook Pro Microphone", "write_errors": 0}`
///
/// The caller must free the returned string with `blackbox_free_string`.
/// Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_status_json(handle: *const BlackboxHandle) -> *mut c_char {
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return std::ptr::null_mut();
        };

        let (is_recording, write_errors) = handle
            .recorder
            .lock()
            .ok()
            .and_then(|guard| {
                guard.as_ref().map(|r| {
                    let p = r.get_processor();
                    (p.is_recording(), p.write_error_count())
                })
            })
            .unwrap_or((false, 0));

        let input_device = handle
            .config
            .lock()
            .ok()
            .and_then(|c| c.get_input_device())
            .unwrap_or_default();

        let status = serde_json::json!({
            "recording": is_recording,
            "input_device": input_device,
            "write_errors": write_errors,
        });

        to_c_string(&status.to_string())
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Return a JSON array of available input device names.
///
/// Example: `["MacBook Pro Microphone", "External USB Mic"]`
///
/// The caller must free the returned string with `blackbox_free_string`.
/// Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_list_input_devices() -> *mut c_char {
    catch_unwind(|| {
        let devices = CpalAudioProcessor::list_input_devices().unwrap_or_default();
        let json = serde_json::to_string(&devices).unwrap_or_else(|_| "[]".to_string());
        to_c_string(&json)
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Update the configuration from a JSON string.
///
/// Only fields present (non-null) in the JSON are updated; others are left unchanged.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_set_config_json(
    handle: *mut BlackboxHandle,
    json: *const c_char,
) -> i32 {
    if json.is_null() {
        return -1;
    }
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return -1;
        };
        handle.clear_error();

        let Some(json_str) = (unsafe { cstr_to_str(json) }) else {
            handle.set_error("Invalid UTF-8 in config JSON".to_string());
            return -1;
        };

        let partial: AppConfig = match serde_json::from_str(json_str) {
            Ok(c) => c,
            Err(e) => {
                handle.set_error(format!("Invalid config JSON: {}", e));
                return -1;
            }
        };

        match handle.config.lock() {
            Ok(mut guard) => {
                guard.merge(partial);
                0
            }
            Err(e) => {
                handle.set_error(format!("Config lock poisoned: {}", e));
                -1
            }
        }
    })
    .unwrap_or(-1)
}

/// Get the last error message, or null if no error has occurred.
///
/// The caller must free the returned string with `blackbox_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_last_error(handle: *const BlackboxHandle) -> *mut c_char {
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return std::ptr::null_mut();
        };
        handle
            .last_error
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|s| to_c_string(s)))
            .unwrap_or(std::ptr::null_mut())
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Free a string previously returned by any `blackbox_*` function.
///
/// Passing null is a safe no-op.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = catch_unwind(|| {
        drop(unsafe { CString::from_raw(s) });
    });
}

/// Return the current configuration as a JSON string.
///
/// The caller must free the returned string with `blackbox_free_string`.
/// Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_config_json(handle: *const BlackboxHandle) -> *mut c_char {
    catch_unwind(|| {
        let Some(handle) = validate_handle(handle) else {
            return std::ptr::null_mut();
        };
        handle.config.lock().map_or(std::ptr::null_mut(), |guard| {
            let json = serde_json::to_string(&*guard).unwrap_or_else(|_| "{}".to_string());
            to_c_string(&json)
        })
    })
    .unwrap_or(std::ptr::null_mut())
}
