import AppKit
import Carbon
import Foundation

/// Manages a single system-wide keyboard shortcut using the Carbon Events API.
/// Works in sandboxed apps. No default shortcut — user configures in Settings.
final class GlobalHotkeyManager {
    static let shared = GlobalHotkeyManager()

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

    private var hotkeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private(set) var currentShortcut: Shortcut?

    /// The action invoked on the main thread when the hotkey fires.
    /// Must be set before calling `register()`.
    var action: (() -> Void)?

    private init() {}

    // MARK: - Public

    /// Register (or re-register) a global hotkey.
    func register(_ shortcut: Shortcut) {
        unregister()
        currentShortcut = shortcut

        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )

        let selfPtr = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(
            GetApplicationEventTarget(),
            { _, event, userData -> OSStatus in
                guard let userData else { return OSStatus(eventNotHandledErr) }
                let manager = Unmanaged<GlobalHotkeyManager>.fromOpaque(userData)
                    .takeUnretainedValue()
                DispatchQueue.main.async { manager.action?() }
                return noErr
            },
            1,
            &eventType,
            selfPtr,
            &handlerRef
        )

        var hotkeyID = EventHotKeyID(
            signature: OSType(0x424C_4B58), // "BLKX"
            id: 1
        )
        RegisterEventHotKey(
            shortcut.keyCode,
            shortcut.carbonModifiers,
            hotkeyID,
            GetApplicationEventTarget(),
            0,
            &hotkeyRef
        )
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

    private static let defaultsKey = "globalShortcut"

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
    switch Int(keyCode) {
    case kVK_ANSI_A: return "A"
    case kVK_ANSI_B: return "B"
    case kVK_ANSI_C: return "C"
    case kVK_ANSI_D: return "D"
    case kVK_ANSI_E: return "E"
    case kVK_ANSI_F: return "F"
    case kVK_ANSI_G: return "G"
    case kVK_ANSI_H: return "H"
    case kVK_ANSI_I: return "I"
    case kVK_ANSI_J: return "J"
    case kVK_ANSI_K: return "K"
    case kVK_ANSI_L: return "L"
    case kVK_ANSI_M: return "M"
    case kVK_ANSI_N: return "N"
    case kVK_ANSI_O: return "O"
    case kVK_ANSI_P: return "P"
    case kVK_ANSI_Q: return "Q"
    case kVK_ANSI_R: return "R"
    case kVK_ANSI_S: return "S"
    case kVK_ANSI_T: return "T"
    case kVK_ANSI_U: return "U"
    case kVK_ANSI_V: return "V"
    case kVK_ANSI_W: return "W"
    case kVK_ANSI_X: return "X"
    case kVK_ANSI_Y: return "Y"
    case kVK_ANSI_Z: return "Z"
    case kVK_ANSI_0: return "0"
    case kVK_ANSI_1: return "1"
    case kVK_ANSI_2: return "2"
    case kVK_ANSI_3: return "3"
    case kVK_ANSI_4: return "4"
    case kVK_ANSI_5: return "5"
    case kVK_ANSI_6: return "6"
    case kVK_ANSI_7: return "7"
    case kVK_ANSI_8: return "8"
    case kVK_ANSI_9: return "9"
    case kVK_F1: return "F1"
    case kVK_F2: return "F2"
    case kVK_F3: return "F3"
    case kVK_F4: return "F4"
    case kVK_F5: return "F5"
    case kVK_F6: return "F6"
    case kVK_F7: return "F7"
    case kVK_F8: return "F8"
    case kVK_F9: return "F9"
    case kVK_F10: return "F10"
    case kVK_F11: return "F11"
    case kVK_F12: return "F12"
    case kVK_Space: return "Space"
    case kVK_Return: return "Return"
    case kVK_Tab: return "Tab"
    case kVK_Delete: return "Delete"
    case kVK_Escape: return "Esc"
    default: return "Key\(keyCode)"
    }
}
