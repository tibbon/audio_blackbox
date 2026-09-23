import CoreAudio

import struct os.Logger

/// Calls `onChange` on the main actor whenever CoreAudio's device list or
/// default input device changes (a USB interface plugged in or out, the
/// default input switched in System Settings). Without it the app only
/// enumerated devices at launch and on a manual Refresh, so a replugged
/// device never appeared in the menu or Settings.
///
/// `nonisolated`: the listener block runs on a CoreAudio thread (queue nil)
/// and only hops to the main actor; `deinit` removes the listeners.
nonisolated final class AudioDeviceListObserver {
    private static let log = Logger(subsystem: "com.dollhousemediatech.blackbox", category: "AudioDevices")

    private static let addresses = [
        AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        ),
        AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        ),
    ]

    private let listener: AudioObjectPropertyListenerBlock

    init(onChange: @escaping @MainActor () -> Void) {
        listener = { _, _ in
            Task { @MainActor in onChange() }
        }
        for address in Self.addresses {
            var address = address
            let status = AudioObjectAddPropertyListenerBlock(
                AudioObjectID(kAudioObjectSystemObject),
                &address,
                nil,
                listener
            )
            if status != noErr {
                Self.log.error("Could not observe audio devices: OSStatus \(status, privacy: .public)")
            }
        }
    }

    deinit {
        for address in Self.addresses {
            var address = address
            AudioObjectRemovePropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &address, nil, listener)
        }
    }
}

/// The input device a session uses, for display: the chosen device when it
/// is connected, otherwise the system default (the engine falls back to the
/// default when the chosen device is missing). `nil` when the default is
/// unknown too.
nonisolated func resolvedInputDeviceName(selected: String, available: [String], systemDefault: String?) -> String? {
    if !selected.isEmpty, available.contains(selected) { return selected }
    return systemDefault
}
