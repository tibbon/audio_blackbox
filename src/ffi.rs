//! C FFI layer for BlackBox Audio Recorder.
//!
//! Exposes an opaque handle pattern so a Swift/SwiftUI frontend (or any C-compatible
//! caller) can drive the Rust audio engine without touching Rust types directly.
//! Complex data is exchanged as JSON strings; the caller frees them with
//! `blackbox_free_string`.
//!
//! ## Panic policy
//!
//! The release profile sets `panic = "abort"` (`Cargo.toml`), so any panic
//! inside an `extern "C"` body terminates the process before unwinding. This
//! is intentional — an audio recorder has no meaningful recovery path for an
//! arbitrary panic, and aborting is preferable to undefined behavior from
//! unwinding across the FFI boundary. Crash reporting on the Swift side
//! captures the abort just like any other unexpected termination.
//!
//! Concretely: callers should treat any FFI call as panic-free under the
//! abort profile. Code in this module is still written defensively (null
//! checks, lock-poison handling, validated handles) so the panic-on-bug
//! surface stays small.
//!
//! Note: the `dev` profile still unwinds. Non-release builds of this crate
//! must not be linked into FFI consumers — only `--release` artifacts are
//! safe to expose across `extern "C"`.

// FFI functions inherently receive raw pointers from C callers. Every function
// performs a null check before dereferencing, so marking each function `unsafe`
// would just push the unsafety annotation outward without adding clarity.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::audio_processor::AudioProcessor;
use crate::audio_recorder::AudioRecorder;
use crate::config::AppConfig;
use crate::cpal_processor::{CpalAudioProcessor, ProcessorStatus};
use crate::error::BlackboxError;

// ── FFI error codes (mirrored as #defines in blackbox_ffi.h) ─────────────
pub const BLACKBOX_OK: i32 = 0;
pub const BLACKBOX_ERR_INVALID_HANDLE: i32 = -1;
pub const BLACKBOX_ERR_AUDIO_DEVICE: i32 = -2;
pub const BLACKBOX_ERR_CONFIG: i32 = -3;
pub const BLACKBOX_ERR_IO: i32 = -4;
pub const BLACKBOX_ERR_LOCK_POISONED: i32 = -5;
pub const BLACKBOX_ERR_INTERNAL: i32 = -6;
pub const BLACKBOX_ERR_DISK_SPACE_LOW: i32 = -7;
/// Caller passed a null or otherwise invalid argument that isn't the handle
/// itself (e.g. a null OUT pointer, a null JSON string).
pub const BLACKBOX_ERR_INVALID_ARG: i32 = -8;

/// Lightweight C struct for status polling — no JSON, no string allocation.
/// Fields match what the Swift `updateDuration()` loop actually reads.
#[repr(C)]
pub struct StatusFlags {
    pub write_errors: u64,
    pub sample_rate: u32,
    pub is_recording: bool,
    pub gate_idle: bool,
    pub disk_space_low: bool,
    pub stream_error: bool,
    pub sample_rate_changed: bool,
}

// Compile-time check that Rust and C agree on StatusFlags layout.
// If this fails, update the C header (blackbox_ffi.h) to match.
const _: () = assert!(std::mem::size_of::<StatusFlags>() == 24);

// ---------------------------------------------------------------------------
// BlackboxHandle — opaque type exposed as `*mut BlackboxHandle` over FFI
// ---------------------------------------------------------------------------

/// Magic number to detect use-after-free or corrupted handles.
const HANDLE_MAGIC: u64 = 0xB1AC_B015_A11D_1000;

pub struct BlackboxHandle {
    magic: AtomicU64,
    config: Mutex<AppConfig>,
    recorder: Mutex<Option<AudioRecorder<CpalAudioProcessor>>>,
    last_error: Mutex<Option<String>>,
    /// Per-channel peak levels — shared with the writer thread.
    /// Stored here so the 30 Hz meter poll can read atomics without
    /// locking the recorder mutex.
    peak_levels: Mutex<Arc<Vec<crate::constants::CacheAlignedPeak>>>,
    /// Bundle of `Arc<Atomic*>` status flags from the active processor.
    ///
    /// The mutex is held only briefly during start/stop to swap in the
    /// processor's atomics. Status-poll callers lock briefly to clone the
    /// bundle (cheap `Arc` clones), drop the lock, then perform lock-free
    /// atomic loads. This keeps the 1 Hz polling loop from blocking on the
    /// multi-second device probe that runs under `recorder.lock()`.
    status: Mutex<ProcessorStatus>,
}

impl BlackboxHandle {
    fn is_valid(&self) -> bool {
        self.magic.load(Ordering::Acquire) == HANDLE_MAGIC
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

    /// Store an error message and return the typed error code for a `BlackboxError`.
    fn set_error_from(&self, msg: String, err: &BlackboxError) -> i32 {
        self.set_error(msg);
        match err {
            BlackboxError::AudioDevice(_) | BlackboxError::AudioDeviceSource { .. } => {
                BLACKBOX_ERR_AUDIO_DEVICE
            }
            BlackboxError::Config(_) | BlackboxError::ChannelParse(_) => BLACKBOX_ERR_CONFIG,
            BlackboxError::Io(_) | BlackboxError::Wav(_) | BlackboxError::WavSource { .. } => {
                BLACKBOX_ERR_IO
            }
            BlackboxError::InsufficientDiskSpace { .. } => BLACKBOX_ERR_DISK_SPACE_LOW,
        }
    }

    /// Store a lock-poisoned error and return the appropriate code.
    fn lock_poisoned(&self, msg: String) -> i32 {
        self.set_error(msg);
        BLACKBOX_ERR_LOCK_POISONED
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a `*const c_char` to a `&str`, returning `None` on null or invalid UTF-8.
///
/// # Safety
///
/// Caller must ensure that:
/// - `ptr` is either null OR points to a NUL-terminated C string.
/// - The pointed-to bytes remain valid and unmutated for the entire lifetime
///   `'a` chosen by the caller.
///
/// The lifetime `'a` is unbound at the function signature; the caller picks it.
/// In practice every call site in this module consumes the returned `&str`
/// inside the same `extern "C"` body, so the pointer is provably valid for
/// that scope. Storing the returned `&str` past the call is unsound.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller guarantees `ptr` is NUL-terminated and valid for `'a`
    // (see function-level Safety doc above).
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Allocate a `CString` on the heap and return a raw pointer.
/// The caller is responsible for freeing it with `blackbox_free_string`.
fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Validate a handle pointer: non-null and magic number matches.
/// Returns `None` if invalid.
///
/// # FFI contract (caller must uphold)
///
/// - `handle` is either null OR a pointer that originated from a successful
///   call to `blackbox_create` (which `Box::leak`-s a `BlackboxHandle`).
/// - The Swift side does not call `blackbox_destroy(h)` concurrently with any
///   other `blackbox_*` call against the same `h`. Concurrent destroy + read
///   is a data race the magic check cannot detect (a freed allocation could
///   be reused with the magic word still in place).
///
/// The returned `'static` lifetime is a polite fiction — the underlying
/// allocation is freed by `blackbox_destroy`. It is sound for the FFI call's
/// duration only because Swift owns the destroy decision and the contract
/// above forbids it happening concurrently with any other call against the
/// same handle.
fn validate_handle(handle: *const BlackboxHandle) -> Option<&'static BlackboxHandle> {
    if handle.is_null() {
        return None;
    }
    // SAFETY: per the FFI contract documented above, `handle` originated from
    // `blackbox_create` (Box::leak) and is not concurrently freed. The magic
    // word check is a UAF mitigation, not a soundness argument.
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
        magic: AtomicU64::new(HANDLE_MAGIC),
        config: Mutex::new(config),
        recorder: Mutex::new(None),
        last_error: Mutex::new(None),
        peak_levels: Mutex::new(Arc::new(Vec::new())),
        status: Mutex::new(ProcessorStatus::idle()),
    });

    Box::into_raw(handle)
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
    let h = unsafe { &*handle };
    // Atomically claim the right to destroy — only one caller can succeed.
    if h.magic
        .compare_exchange(HANDLE_MAGIC, 0, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return; // Already destroyed or invalid
    }
    let handle = unsafe { Box::from_raw(handle) };
    // Stop recording if active — AudioRecorder's Drop will finalize via the processor.
    if let Ok(mut guard) = handle.recorder.lock() {
        drop(guard.take());
    }
}

/// Start recording.
///
/// Creates a `CpalAudioProcessor`, wraps it in an `AudioRecorder`, and begins
/// recording using the current configuration.
///
/// Returns `BLACKBOX_OK` on success, or a negative error code.
/// Retrieve the human-readable message with `blackbox_get_last_error`.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_start_recording(handle: *mut BlackboxHandle) -> i32 {
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    handle.clear_error();

    let config = match handle.config.lock() {
        Ok(c) => c.clone(),
        Err(e) => return handle.lock_poisoned(format!("Config lock poisoned: {e}")),
    };

    let processor = match CpalAudioProcessor::with_config(&config) {
        Ok(p) => p,
        Err(e) => {
            return handle.set_error_from(format!("Failed to create audio processor: {e}"), &e);
        }
    };

    let mut recorder = AudioRecorder::with_config(processor, config);

    if let Err(e) = recorder.start_recording() {
        return handle.set_error_from(format!("Failed to start recording: {e}"), &e);
    }

    // Install recorder + cache its status atomics under the same lock
    // sequence so a racing stop_recording cannot see "live" status flags
    // pointing at a recorder that hasn't been installed yet.
    match handle.recorder.lock() {
        Ok(mut guard) => {
            if let Ok(mut pl) = handle.peak_levels.lock() {
                *pl = recorder.get_processor().peak_levels_arc();
            }
            if let Ok(mut s) = handle.status.lock() {
                *s = recorder.get_processor().status_arcs();
            }
            *guard = Some(recorder);
            BLACKBOX_OK
        }
        Err(e) => handle.lock_poisoned(format!("Recorder lock poisoned: {e}")),
    }
}

/// Stop recording.
///
/// Returns `BLACKBOX_OK` on success, or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_stop_recording(handle: *mut BlackboxHandle) -> i32 {
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    handle.clear_error();

    match handle.recorder.lock() {
        Ok(mut guard) => {
            // Clear cached status while holding the recorder lock so the
            // FFI status path always sees a state consistent with the
            // installed recorder.
            if let Ok(mut pl) = handle.peak_levels.lock() {
                *pl = Arc::new(Vec::new());
            }
            if let Ok(mut s) = handle.status.lock() {
                *s = ProcessorStatus::idle();
            }
            if let Some(mut recorder) = guard.take()
                && let Err(e) = recorder.processor_mut().stop_recording()
            {
                return handle.set_error_from(format!("Failed to stop recording: {e}"), &e);
            }
            BLACKBOX_OK
        }
        Err(e) => handle.lock_poisoned(format!("Recorder lock poisoned: {e}")),
    }
}

/// Check whether recording is currently active.
///
/// Lock-free with respect to `blackbox_start_recording` / `blackbox_stop_recording`:
/// reads the lifted `Arc<AtomicBool>` after a microsecond-scale `status` mutex
/// acquire, so a racing start/stop holding the recorder mutex does not stall
/// the UI poll.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_is_recording(handle: *const BlackboxHandle) -> bool {
    let Some(handle) = validate_handle(handle) else {
        return false;
    };
    let Ok(status) = handle.status.lock() else {
        return false;
    };
    // Clone the Arc out from under the lock so the load itself is lock-free.
    let flag = Arc::clone(&status.recording_active);
    drop(status);
    flag.load(Ordering::Relaxed)
}

/// Fill a `StatusFlags` struct with current engine status.
///
/// Zero-allocation, no JSON, no recorder mutex — designed for the 1 Hz polling
/// loop. The `status` mutex is held only long enough to clone an `Arc` bundle;
/// the actual flag loads are lock-free atomic reads. A racing start/stop
/// blocks the poll for microseconds at most, never the multi-second device
/// probe.
///
/// Returns `BLACKBOX_OK` on success, or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_status_flags(
    handle: *const BlackboxHandle,
    out: *mut StatusFlags,
) -> i32 {
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    if out.is_null() {
        return BLACKBOX_ERR_INVALID_ARG;
    }

    // Clone the bundle out from under the lock, then drop the lock before
    // doing any atomic loads.
    let status = match handle.status.lock() {
        Ok(s) => (*s).clone(),
        Err(_) => return handle.lock_poisoned("Status lock poisoned".to_string()),
    };

    let flags = StatusFlags {
        write_errors: status.write_errors.load(Ordering::Relaxed),
        sample_rate: status.sample_rate.load(Ordering::Relaxed),
        is_recording: status.recording_active.load(Ordering::Relaxed),
        gate_idle: status.gate_idle.load(Ordering::Relaxed),
        disk_space_low: status.disk_space_low.load(Ordering::Relaxed),
        stream_error: status.stream_error.load(Ordering::Relaxed),
        sample_rate_changed: status.sample_rate_changed.load(Ordering::Relaxed),
    };

    // Safety: out is non-null, and we write a POD struct.
    unsafe { out.write(flags) };
    BLACKBOX_OK
}

/// Return a JSON array of available input device names.
///
/// Example: `["MacBook Pro Microphone", "External USB Mic"]`
///
/// The caller must free the returned string with `blackbox_free_string`.
/// Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_list_input_devices() -> *mut c_char {
    let devices = CpalAudioProcessor::list_input_devices().unwrap_or_default();
    let json = serde_json::to_string(&devices).unwrap_or_else(|_| "[]".to_string());
    to_c_string(&json)
}

/// Get the input channel count for a device by name.
///
/// Pass an empty string or null for the system default device.
/// Returns the channel count (>= 1), or `BLACKBOX_ERR_AUDIO_DEVICE` on error.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_device_channel_count(device_name: *const c_char) -> i32 {
    let name = if device_name.is_null() {
        ""
    } else {
        unsafe { cstr_to_str(device_name) }.unwrap_or("")
    };
    CpalAudioProcessor::get_device_channel_count(name)
        .map_or(BLACKBOX_ERR_AUDIO_DEVICE, |ch| i32::from(ch))
}

/// Update the configuration from a JSON string.
///
/// Only fields present (non-null) in the JSON are updated; others are left unchanged.
/// Returns `BLACKBOX_OK` on success, or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_set_config_json(
    handle: *mut BlackboxHandle,
    json: *const c_char,
) -> i32 {
    // Validate the handle first — a null/invalid handle is a more fundamental
    // failure than a missing arg.
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    if json.is_null() {
        return BLACKBOX_ERR_INVALID_ARG;
    }
    handle.clear_error();

    let Some(json_str) = (unsafe { cstr_to_str(json) }) else {
        handle.set_error("Invalid UTF-8 in config JSON".to_string());
        return BLACKBOX_ERR_CONFIG;
    };

    let partial: AppConfig = match serde_json::from_str(json_str) {
        Ok(c) => c,
        Err(e) => {
            handle.set_error(format!("Invalid config JSON: {e}"));
            return BLACKBOX_ERR_CONFIG;
        }
    };

    match handle.config.lock() {
        Ok(mut guard) => {
            guard.merge(partial);
            BLACKBOX_OK
        }
        Err(e) => handle.lock_poisoned(format!("Config lock poisoned: {e}")),
    }
}

/// Get the last error message, or null if no error has occurred.
///
/// The caller must free the returned string with `blackbox_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_last_error(handle: *const BlackboxHandle) -> *mut c_char {
    let Some(handle) = validate_handle(handle) else {
        return std::ptr::null_mut();
    };
    handle
        .last_error
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|s| to_c_string(s)))
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
    drop(unsafe { CString::from_raw(s) });
}

/// Write current peak levels into a caller-provided buffer.
///
/// `out` must point to a float array of at least `max_channels` elements.
/// Returns the number of channels actually written (>= 0), or one of these
/// negative error codes on failure:
///
/// * `BLACKBOX_ERR_INVALID_HANDLE` — handle is null or freed.
/// * `BLACKBOX_ERR_INVALID_ARG` — `out` is null or `max_channels` <= 0.
/// * `BLACKBOX_ERR_LOCK_POISONED` — internal lock was poisoned by a prior panic.
///
/// This is a lightweight alternative to `blackbox_get_status_json` for meter UIs —
/// no JSON serialization, no string allocation, just atomic reads into the buffer.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_peak_levels(
    handle: *const BlackboxHandle,
    out: *mut f32,
    max_channels: i32,
) -> i32 {
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    if out.is_null() || max_channels <= 0 {
        return BLACKBOX_ERR_INVALID_ARG;
    }

    let buf = unsafe { std::slice::from_raw_parts_mut(out, max_channels as usize) };

    // Read from the cached Arc — no recorder mutex needed.
    let peaks = match handle.peak_levels.lock() {
        Ok(pl) => Arc::clone(&pl),
        Err(_) => return handle.lock_poisoned("peak_levels lock poisoned".to_string()),
    };
    let count = peaks.len().min(buf.len());
    for (dst, src) in buf[..count].iter_mut().zip(peaks.iter()) {
        *dst = f32::from_bits(src.value.load(std::sync::atomic::Ordering::Relaxed));
    }
    count as i32
}

/// Start audio monitoring (peak levels without recording to disk).
///
/// Creates a `CpalAudioProcessor`, wraps it in an `AudioRecorder`, and begins
/// monitoring using the current configuration.
///
/// Returns `BLACKBOX_OK` on success, or a negative error code.
/// Retrieve the human-readable message with `blackbox_get_last_error`.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_start_monitoring(handle: *mut BlackboxHandle) -> i32 {
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    handle.clear_error();

    let config = match handle.config.lock() {
        Ok(c) => c.clone(),
        Err(e) => return handle.lock_poisoned(format!("Config lock poisoned: {e}")),
    };

    // Create a processor if we don't already have a recorder
    let mut guard = match handle.recorder.lock() {
        Ok(g) => g,
        Err(e) => return handle.lock_poisoned(format!("Recorder lock poisoned: {e}")),
    };

    if guard.is_none() {
        let processor = match CpalAudioProcessor::with_config(&config) {
            Ok(p) => p,
            Err(e) => {
                return handle
                    .set_error_from(format!("Failed to create audio processor: {e}"), &e);
            }
        };
        *guard = Some(AudioRecorder::with_config(processor, config));
    }

    if let Some(recorder) = guard.as_mut() {
        if let Err(e) = recorder.start_monitoring() {
            return handle.set_error_from(format!("Failed to start monitoring: {e}"), &e);
        }
        // Cache peak_levels Arc and status atomics so the FFI poll path
        // skips the recorder mutex.
        if let Ok(mut pl) = handle.peak_levels.lock() {
            *pl = recorder.get_processor().peak_levels_arc();
        }
        if let Ok(mut s) = handle.status.lock() {
            *s = recorder.get_processor().status_arcs();
        }
    }

    BLACKBOX_OK
}

/// Stop audio monitoring.
///
/// Returns `BLACKBOX_OK` on success, or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_stop_monitoring(handle: *mut BlackboxHandle) -> i32 {
    let Some(handle) = validate_handle(handle) else {
        return BLACKBOX_ERR_INVALID_HANDLE;
    };
    handle.clear_error();

    match handle.recorder.lock() {
        Ok(mut guard) => {
            // Clear cached peak / status under the recorder lock so the
            // FFI poll path stays consistent with the installed recorder.
            if let Ok(mut pl) = handle.peak_levels.lock() {
                *pl = Arc::new(Vec::new());
            }
            if let Ok(mut s) = handle.status.lock() {
                *s = ProcessorStatus::idle();
            }
            if let Some(recorder) = guard.as_mut() {
                if let Err(e) = recorder.processor_mut().stop_monitoring() {
                    return handle
                        .set_error_from(format!("Failed to stop monitoring: {e}"), &e);
                }
                // If not recording, drop the recorder to release resources
                if !recorder.get_processor().is_recording() {
                    drop(guard.take());
                }
            }
            BLACKBOX_OK
        }
        Err(e) => handle.lock_poisoned(format!("Recorder lock poisoned: {e}")),
    }
}

/// Check whether audio monitoring is currently active.
///
/// Lock-free with respect to start/stop — see `blackbox_is_recording`.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_is_monitoring(handle: *const BlackboxHandle) -> bool {
    let Some(handle) = validate_handle(handle) else {
        return false;
    };
    let Ok(status) = handle.status.lock() else {
        return false;
    };
    let flag = Arc::clone(&status.monitoring_active);
    drop(status);
    flag.load(Ordering::Relaxed)
}

/// Return the current configuration as a JSON string.
///
/// The caller must free the returned string with `blackbox_free_string`.
/// Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn blackbox_get_config_json(handle: *const BlackboxHandle) -> *mut c_char {
    let Some(handle) = validate_handle(handle) else {
        return std::ptr::null_mut();
    };
    handle.config.lock().map_or(std::ptr::null_mut(), |guard| {
        let json = serde_json::to_string(&*guard).unwrap_or_else(|_| "{}".to_string());
        to_c_string(&json)
    })
}
