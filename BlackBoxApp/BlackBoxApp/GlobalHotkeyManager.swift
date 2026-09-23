import AppKit
import Carbon
import Foundation
import os.log

/// Manages a single system-wide keyboard shortcut using the Carbon Events API.
/// Works in sandboxed apps. Nothing is registered until the user picks a
/// shortcut, in Settings or in onboarding's opt-in shortcut step.
///
/// Main-actor-isolated: registration, unregistration, and action dispatch
/// all run on the main thread. The Carbon Events API delivers hotkey events
/// on the main run loop after `InstallEventHandler` is called from main, so
/// `MainActor.assumeIsolated` at the C callback boundary is safe.
@MainActor
final class GlobalHotkeyManager {
    static let shared = GlobalHotkeyManager()

    private static let log = Logger(subsystem: "com.dollhousemediatech.blackbox", category: "GlobalHotkeyManager")

    /// Persisted shortcut representation.
    struct Shortcut: Codable, Equatable {
        let keyCode: UInt32
        let carbonModifiers: UInt32

        /// Human-readable label, e.g. "⌃⌥R"
        var displayString: String {
            var parts: [String] = []
            if carbonModifiers & UInt32(controlKey) != 0 { parts.append("⌃") }
            if carbonModifiers & UInt32(optionKey) != 0 { parts.append("⌥") }
            if carbonModifiers & UInt32(shiftKey) != 0 { parts.append("⇧") }
            if carbonModifiers & UInt32(cmdKey) != 0 { parts.append("⌘") }
            parts.append(keyCodeToString(keyCode))
            return parts.joined()
        }
    }

    /// The combination onboarding offers. Never registered on its own: the
    /// user has to click "Use ⌘⇧R", because it is also the browsers'
    /// hard-reload shortcut and a global hotkey takes it over everywhere.
    static let suggestedShortcut = Shortcut(
        keyCode: UInt32(kVK_ANSI_R),
        carbonModifiers: UInt32(cmdKey | shiftKey)
    )

    private var hotkeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private(set) var currentShortcut: Shortcut?

    /// The action invoked on the main thread when the hotkey fires.
    /// Must be set before calling `register()`.
    var action: (@MainActor () -> Void)?

    private init() {
        // Singleton: the hotkey and handler are installed lazily by register(_:).
    }

    // MARK: - Public

    /// Register (or re-register) a global hotkey. Returns `true` on success.
    ///
    /// On failure the previously registered shortcut, if any, is registered
    /// again: callers only save the new shortcut on success, so UserDefaults
    /// and Settings still show the old one, and it must keep working rather
    /// than leave the app with no hotkey until the next launch.
    @discardableResult
    func register(_ shortcut: Shortcut) -> Bool {
        let previous = currentShortcut
        guard tryRegister(shortcut) else {
            if let previous, previous != shortcut, !tryRegister(previous) {
                Self.log.error("Could not restore the previous hotkey \(previous.displayString, privacy: .public)")
            }
            return false
        }
        return true
    }

    /// Register `shortcut` and, only if that worked, save it so launch
    /// restore picks it up. The one path both Settings' recorder and
    /// onboarding use when the user chooses a shortcut. A combination that
    /// can't be registered is not saved: it would never fire.
    @discardableResult
    func registerAndSave(_ shortcut: Shortcut) -> Bool {
        guard register(shortcut) else { return false }
        save(shortcut)
        return true
    }

    /// Replace whatever is registered with `shortcut`. On failure, leaves no
    /// partial state behind (`currentShortcut` nil) so a later call starts clean.
    private func tryRegister(_ shortcut: Shortcut) -> Bool {
        unregister()
        currentShortcut = shortcut
        guard installHotkeyHandler(), registerHotkey(shortcut) else {
            currentShortcut = nil
            return false
        }
        return true
    }

    /// Install the application-target Carbon handler that runs `action` when
    /// the hotkey fires. On failure, logs and leaves `handlerRef` nil.
    private func installHotkeyHandler() -> Bool {
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )

        // SAFETY: `shared` is the only instance (private init) and lives for the
        // whole process, so the unretained pointer can never dangle; `unregister()`
        // removes the handler before any re-registration, so Carbon never holds a
        // pointer past the handler's lifetime.
        // swiftlint:disable:next passunretained_stored - self is the process-lifetime singleton and unregister() removes the handler
        let selfPtr = Unmanaged.passUnretained(self).toOpaque()
        let installStatus = InstallEventHandler(
            GetApplicationEventTarget(),
            { _, _, userData -> OSStatus in
                guard let userData else { return OSStatus(eventNotHandledErr) }
                // SAFETY (DOLL-161): Carbon dispatches application-target hotkey
                // events on the main run loop, on the thread that installed the
                // handler (main — this class is @MainActor), so the callback is
                // already main-actor code that the C signature can't express.
                // swiftlint:disable:next assume_isolated - Carbon delivers application-target events on the main run loop (DOLL-161)
                MainActor.assumeIsolated {
                    let manager = Unmanaged<GlobalHotkeyManager>.fromOpaque(userData)
                        .takeUnretainedValue()
                    manager.action?()
                }
                return noErr
            },
            1,
            &eventType,
            selfPtr,
            &handlerRef
        )
        if installStatus != noErr {
            Self.log.error("InstallEventHandler failed: OSStatus \(installStatus, privacy: .public)")
            handlerRef = nil
            return false
        }
        return true
    }

    /// Register the key combination with Carbon. On failure, logs, removes the
    /// handler `installHotkeyHandler()` installed, and leaves both refs nil.
    private func registerHotkey(_ shortcut: Shortcut) -> Bool {
        let hotkeyID = EventHotKeyID(
            signature: OSType(0x424C_4B58),  // "BLKX"
            id: 1
        )
        let registerStatus = RegisterEventHotKey(
            shortcut.keyCode,
            shortcut.carbonModifiers,
            hotkeyID,
            GetApplicationEventTarget(),
            0,
            &hotkeyRef
        )
        if registerStatus != noErr {
            Self.log.error(
                "RegisterEventHotKey failed for \(shortcut.displayString, privacy: .public): OSStatus \(registerStatus, privacy: .public)"
            )
            // -9878 (eventHotKeyExistsErr) means another app or system shortcut owns this combo.
            if let ref = handlerRef {
                RemoveEventHandler(ref)
                handlerRef = nil
            }
            hotkeyRef = nil
            return false
        }
        return true
    }

    /// Unregister the current global hotkey.
    func unregister() {
        if let ref = hotkeyRef {
            UnregisterEventHotKey(ref)
            hotkeyRef = nil
        }
        if let ref = handlerRef {
            RemoveEventHandler(ref)
            handlerRef = nil
        }
        currentShortcut = nil
    }

    // MARK: - Persistence

    // DOLL-378: source the key from the central registry (value unchanged, so
    // no stored-value migration needed) so a rename can't silently orphan it.
    private static let defaultsKey = SettingsKeys.globalShortcut

    func save(_ shortcut: Shortcut?) {
        if let shortcut, let data = try? JSONEncoder().encode(shortcut) {
            UserDefaults.standard.set(data, forKey: Self.defaultsKey)
        } else {
            UserDefaults.standard.removeObject(forKey: Self.defaultsKey)
        }
    }

    func loadSaved() -> Shortcut? {
        guard let data = UserDefaults.standard.data(forKey: Self.defaultsKey) else {
            return nil
        }
        return try? JSONDecoder().decode(Shortcut.self, from: data)
    }

    // MARK: - NSEvent → Carbon Conversion

    /// Convert NSEvent modifier flags to Carbon modifier flags.
    static func carbonModifiers(from flags: UInt) -> UInt32 {
        var carbon: UInt32 = 0
        let ns = flags
        if ns & UInt(NSEvent.ModifierFlags.command.rawValue) != 0 { carbon |= UInt32(cmdKey) }
        if ns & UInt(NSEvent.ModifierFlags.shift.rawValue) != 0 { carbon |= UInt32(shiftKey) }
        if ns & UInt(NSEvent.ModifierFlags.option.rawValue) != 0 { carbon |= UInt32(optionKey) }
        if ns & UInt(NSEvent.ModifierFlags.control.rawValue) != 0 { carbon |= UInt32(controlKey) }
        return carbon
    }
}

// MARK: - Key Code to String

/// Map a virtual key code to a display string. Covers common keys.
private func keyCodeToString(_ keyCode: UInt32) -> String {
    keyCodeNames[Int(keyCode)] ?? "Key\(keyCode)"
}

/// Display names for the virtual key codes `keyCodeToString` covers.
private let keyCodeNames: [Int: String] = [
    kVK_ANSI_A: "A",
    kVK_ANSI_B: "B",
    kVK_ANSI_C: "C",
    kVK_ANSI_D: "D",
    kVK_ANSI_E: "E",
    kVK_ANSI_F: "F",
    kVK_ANSI_G: "G",
    kVK_ANSI_H: "H",
    kVK_ANSI_I: "I",
    kVK_ANSI_J: "J",
    kVK_ANSI_K: "K",
    kVK_ANSI_L: "L",
    kVK_ANSI_M: "M",
    kVK_ANSI_N: "N",
    kVK_ANSI_O: "O",
    kVK_ANSI_P: "P",
    kVK_ANSI_Q: "Q",
    kVK_ANSI_R: "R",
    kVK_ANSI_S: "S",
    kVK_ANSI_T: "T",
    kVK_ANSI_U: "U",
    kVK_ANSI_V: "V",
    kVK_ANSI_W: "W",
    kVK_ANSI_X: "X",
    kVK_ANSI_Y: "Y",
    kVK_ANSI_Z: "Z",
    kVK_ANSI_0: "0",
    kVK_ANSI_1: "1",
    kVK_ANSI_2: "2",
    kVK_ANSI_3: "3",
    kVK_ANSI_4: "4",
    kVK_ANSI_5: "5",
    kVK_ANSI_6: "6",
    kVK_ANSI_7: "7",
    kVK_ANSI_8: "8",
    kVK_ANSI_9: "9",
    kVK_F1: "F1",
    kVK_F2: "F2",
    kVK_F3: "F3",
    kVK_F4: "F4",
    kVK_F5: "F5",
    kVK_F6: "F6",
    kVK_F7: "F7",
    kVK_F8: "F8",
    kVK_F9: "F9",
    kVK_F10: "F10",
    kVK_F11: "F11",
    kVK_F12: "F12",
    kVK_Space: "Space",
    kVK_Return: "Return",
    kVK_Tab: "Tab",
    kVK_Delete: "Delete",
    kVK_Escape: "Esc",
]
