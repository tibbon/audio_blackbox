//! Registers a CoreAudio property listener on
//! `kAudioDevicePropertyNominalSampleRate` for the active input device.
//! When the sample rate changes, sets an `AtomicBool` flag that the
//! Swift UI polling loop can detect and restart the recording with
//! the correct sample rate in the new WAV header.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use log::{info, warn};

type AudioObjectID = u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct PropAddr {
    selector: u32,
    scope: u32,
    element: u32,
}

// CoreAudio FourCC constants
const SYSTEM_OBJECT: AudioObjectID = 1;
const SEL_DEVICES: u32 = u32::from_be_bytes(*b"dev#");
const SEL_DEFAULT_INPUT: u32 = u32::from_be_bytes(*b"dIn ");
const SEL_NAME: u32 = u32::from_be_bytes(*b"lnam");
const SEL_NOMINAL_RATE: u32 = u32::from_be_bytes(*b"nsrt");
const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
const ELEMENT_MAIN: u32 = 0;

type ListenerProc = unsafe extern "C" fn(AudioObjectID, u32, *const PropAddr, *mut c_void) -> i32;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    unsafe fn AudioObjectGetPropertyDataSize(
        id: AudioObjectID,
        addr: *const PropAddr,
        qual_size: u32,
        qual: *const c_void,
        out_size: *mut u32,
    ) -> i32;

    unsafe fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        addr: *const PropAddr,
        qual_size: u32,
        qual: *const c_void,
        io_size: *mut u32,
        out_data: *mut c_void,
    ) -> i32;

    unsafe fn AudioObjectAddPropertyListener(
        id: AudioObjectID,
        addr: *const PropAddr,
        listener: ListenerProc,
        client_data: *mut c_void,
    ) -> i32;

    unsafe fn AudioObjectRemovePropertyListener(
        id: AudioObjectID,
        addr: *const PropAddr,
        listener: ListenerProc,
        client_data: *mut c_void,
    ) -> i32;
}

/// RAII guard: registers a CoreAudio property listener on creation, removes on drop.
pub struct SampleRateListener {
    device_id: AudioObjectID,
    /// Raw pointer to an `AtomicBool` (the inner value of an `Arc<AtomicBool>`,
    /// produced via `Arc::into_raw`). The listener owns exactly one strong
    /// reference logically; the Arc is *never* reclaimed while the listener
    /// might still receive callbacks (see Drop and the SAFETY block below).
    client_data: *mut c_void,
}

// SAFETY: this listener can be transferred between threads because:
//
// 1. The pointed-to data is `AtomicBool`. `Arc::into_raw` returned a
//    `*const AtomicBool`; we own one strong reference's worth of refcount,
//    not a pointer to an `Arc` itself. Both `Arc` and `AtomicBool` are
//    `Send + Sync`.
// 2. The CoreAudio thread that calls `on_rate_changed` only performs an
//    atomic store on the bool — no Rust-state access that requires
//    higher-level synchronization.
// 3. We DO NOT rely on CoreAudio to serialize the listener-removal
//    callback against in-flight callbacks on other threads. Apple's docs
//    don't actually guarantee that — only that no *new* callbacks start
//    after `AudioObjectRemovePropertyListener` returns. To eliminate the
//    race entirely, Drop unconditionally leaks the Arc strong reference
//    (see Drop impl below). The AtomicBool then lives until process exit,
//    safe to dereference from any straggler callback. Cost: one
//    sizeof(AtomicBool) = 1 byte per CoreAudio listener for the
//    process lifetime, which is bounded (one listener per recording).
unsafe impl Send for SampleRateListener {}

impl SampleRateListener {
    /// Register a CoreAudio listener for sample rate changes on the given device.
    /// Returns `None` if the device can't be found or registration fails.
    pub fn new(device_name: Option<&str>, flag: Arc<AtomicBool>) -> Option<Self> {
        let device_id = find_device_id(device_name)?;
        let client_data = Arc::into_raw(Arc::clone(&flag)) as *mut c_void;

        let status = unsafe {
            AudioObjectAddPropertyListener(device_id, &rate_addr(), on_rate_changed, client_data)
        };

        if status != 0 {
            // Reclaim the Arc since registration failed
            unsafe {
                drop(Arc::from_raw(client_data as *const AtomicBool));
            }
            warn!(
                "Failed to register sample rate listener (status {})",
                status
            );
            return None;
        }

        info!("Registered sample rate listener on device {}", device_id);
        Some(Self {
            device_id,
            client_data,
        })
    }
}

impl Drop for SampleRateListener {
    fn drop(&mut self) {
        let status = unsafe {
            AudioObjectRemovePropertyListener(
                self.device_id,
                &rate_addr(),
                on_rate_changed,
                self.client_data,
            )
        };
        if status != 0 {
            warn!("Failed to remove sample rate listener (status {})", status);
        }
        // LEAKS BY DESIGN — see SAFETY block on `unsafe impl Send` above.
        // Apple's docs don't guarantee that callbacks already in flight on
        // another thread have ceased by the time
        // `AudioObjectRemovePropertyListener` returns; only that no *new*
        // callbacks start. Reclaiming the Arc here would race a straggler
        // callback's atomic load against a freed `AtomicBool`. Cost: one
        // `sizeof(AtomicBool) = 1` byte per listener, bounded by the
        // recording lifecycle.
        //
        // Reconstitute and forget rather than letting the raw pointer fall
        // off the stack — the explicit `mem::forget` is grep-able and
        // makes the intent obvious to a future reader.
        let arc_back = unsafe { Arc::from_raw(self.client_data as *const AtomicBool) };
        std::mem::forget(arc_back);
    }
}

fn rate_addr() -> PropAddr {
    PropAddr {
        selector: SEL_NOMINAL_RATE,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    }
}

/// CoreAudio callback — runs on an internal CoreAudio thread.
unsafe extern "C" fn on_rate_changed(
    _id: AudioObjectID,
    _count: u32,
    _addrs: *const PropAddr,
    client_data: *mut c_void,
) -> i32 {
    if !client_data.is_null() {
        let flag = unsafe { &*(client_data as *const AtomicBool) };
        // status flag only; reader at sample_rate_changed() loads Relaxed.
        flag.store(true, Ordering::Relaxed);
    }
    0
}

// --- Device lookup helpers ---

fn find_device_id(name: Option<&str>) -> Option<AudioObjectID> {
    match name {
        Some(n) if !n.is_empty() => device_by_name(n).or_else(default_input_device),
        _ => default_input_device(),
    }
}

fn default_input_device() -> Option<AudioObjectID> {
    let addr = PropAddr {
        selector: SEL_DEFAULT_INPUT,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let mut device_id: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;

    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &raw const addr,
            0,
            std::ptr::null(),
            &raw mut size,
            (&raw mut device_id).cast::<c_void>(),
        )
    };
    (status == 0 && device_id != 0).then_some(device_id)
}

fn device_by_name(name: &str) -> Option<AudioObjectID> {
    let addr = PropAddr {
        selector: SEL_DEVICES,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };

    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            SYSTEM_OBJECT,
            &raw const addr,
            0,
            std::ptr::null(),
            &raw mut size,
        )
    };
    if status != 0 || size == 0 {
        return None;
    }

    let count = size as usize / size_of::<AudioObjectID>();
    let mut ids = vec![0u32; count];

    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &raw const addr,
            0,
            std::ptr::null(),
            &raw mut size,
            ids.as_mut_ptr().cast::<c_void>(),
        )
    };
    if status != 0 {
        return None;
    }

    ids.iter()
        .copied()
        .find(|&id| device_name(id).is_some_and(|n| n == name))
}

fn device_name(device_id: AudioObjectID) -> Option<String> {
    let addr = PropAddr {
        selector: SEL_NAME,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let mut name_ref: CFStringRef = std::ptr::null();
    let mut size = size_of::<CFStringRef>() as u32;

    let status = unsafe {
        AudioObjectGetPropertyData(
            device_id,
            &raw const addr,
            0,
            std::ptr::null(),
            &raw mut size,
            (&raw mut name_ref).cast::<c_void>(),
        )
    };
    if status != 0 || name_ref.is_null() {
        return None;
    }

    // SAFETY: `name_ref` was just verified non-null. CoreAudio's
    // `kAudioObjectPropertyName` is documented to return a +1
    // retained CFStringRef; `wrap_under_create_rule` consumes that
    // retain count without re-retaining, so the wrapper drops it
    // exactly once.
    let cf = unsafe { CFString::wrap_under_create_rule(name_ref) };
    Some(cf.to_string())
}

#[cfg(test)]
mod tests {
    //! These tests exercise the highest-risk code in the crate: the raw
    //! `unsafe extern "C"` callback and the deliberately-leaking `Drop`.
    //! `cargo +nightly miri` can't run them — the `Drop` path and the real
    //! lifecycle test call into the CoreAudio framework, which miri's FFI
    //! sandbox does not support. They are written to be ASan-clean and to
    //! verify the exact `Arc::into_raw` / `from_raw` ownership the production
    //! code relies on.
    use super::*;

    /// The CoreAudio thread invokes `on_rate_changed` with the `client_data`
    /// pointer produced by `Arc::into_raw` at registration. Exercise that
    /// exact path: the callback must deref it as an `AtomicBool` and store
    /// `true`.
    #[test]
    fn on_rate_changed_flips_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        // Mirror `new()`: hand a strong ref to the callback as `*mut c_void`.
        let raw = Arc::into_raw(Arc::clone(&flag)) as *mut c_void;

        // SAFETY: `raw` points to the live `AtomicBool` inside `flag` — the
        // same contract the callback is given at registration.
        let status = unsafe { on_rate_changed(0, 1, std::ptr::null(), raw) };

        assert_eq!(status, 0);
        assert!(flag.load(Ordering::Relaxed), "callback must set the flag");

        // Reclaim the leaked ref so the test itself doesn't leak.
        // SAFETY: `raw` came from `Arc::into_raw` above and is still live.
        unsafe { drop(Arc::from_raw(raw as *const AtomicBool)) };
    }

    /// A null `client_data` must be a no-op, never a null deref.
    #[test]
    fn on_rate_changed_null_client_data_is_noop() {
        // SAFETY: null `client_data` is explicitly handled by the callback.
        let status = unsafe { on_rate_changed(0, 1, std::ptr::null(), std::ptr::null_mut()) };
        assert_eq!(status, 0);
    }

    /// `Drop` must NOT reclaim the client `Arc` — it leaks the strong
    /// reference by design so a straggler CoreAudio callback can't deref a
    /// freed `AtomicBool`. Verify the refcount survives `Drop` (no decrement,
    /// hence no eventual double-free of the shared `AtomicBool`).
    #[test]
    fn drop_leaks_client_arc_by_design() {
        let flag = Arc::new(AtomicBool::new(false));
        let raw = Arc::into_raw(Arc::clone(&flag)) as *mut c_void;
        // `flag` + the `into_raw`'d ref = 2.
        assert_eq!(Arc::strong_count(&flag), 2);

        // Build the guard directly (same-module access to private fields) with
        // a bogus device id so the test needs no audio hardware. `Drop` calls
        // the real `AudioObjectRemovePropertyListener`, which returns an error
        // status for an unregistered listener/id (logged, harmless), then
        // leaks the Arc.
        let listener = SampleRateListener {
            device_id: 0,
            client_data: raw,
        };

        // The callback still works while the listener is alive.
        // SAFETY: `raw` is live.
        unsafe {
            on_rate_changed(
                listener.device_id,
                1,
                std::ptr::null(),
                listener.client_data,
            )
        };
        assert!(flag.load(Ordering::Relaxed));

        drop(listener);

        // Leaked by design: refcount unchanged, the `AtomicBool` is still alive.
        assert_eq!(
            Arc::strong_count(&flag),
            2,
            "Drop must not reclaim the client Arc"
        );

        // Test-only cleanup: reclaim the intentionally-leaked ref so the test
        // process doesn't actually leak (production leaks for process life).
        // SAFETY: `raw` is still valid; we drop exactly the leaked strong ref.
        unsafe { drop(Arc::from_raw(raw as *const AtomicBool)) };
        assert_eq!(Arc::strong_count(&flag), 1);
    }

    /// When a real default input device exists (dev machines), the full
    /// register → drop lifecycle must succeed without crashing or
    /// double-freeing. Skipped on headless CI runners with no input device.
    #[test]
    fn real_listener_lifecycle_when_device_present() {
        if default_input_device().is_none() {
            return; // no audio hardware (e.g. headless CI) — nothing to exercise
        }
        let flag = Arc::new(AtomicBool::new(false));
        let listener = SampleRateListener::new(None, Arc::clone(&flag));
        assert!(
            listener.is_some(),
            "registration should succeed when a default input device exists"
        );
        // Real `AudioObjectRemovePropertyListener` on a registered listener;
        // must not crash or double-free.
        drop(listener);
    }
}
