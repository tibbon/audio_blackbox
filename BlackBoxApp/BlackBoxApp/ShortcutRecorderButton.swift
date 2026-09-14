import Carbon
import SwiftUI

// MARK: - Shortcut Recorder

/// A button that captures a keyboard shortcut when clicked.
struct ShortcutRecorderButton: NSViewRepresentable {
    @Binding var shortcutLabel: String
    @Binding var isRecording: Bool
    @Binding var error: String?

    func makeNSView(context: Context) -> ShortcutRecorderNSButton {
        let button = ShortcutRecorderNSButton()
        button.coordinator = context.coordinator
        button.title = shortcutLabel
        button.bezelStyle = .rounded
        button.setContentHuggingPriority(.required, for: .horizontal)
        return button
    }

    func updateNSView(_ nsView: ShortcutRecorderNSButton, context _: Context) {
        nsView.title = isRecording ? String(localized: "Press shortcut\u{2026}") : shortcutLabel
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    /// SwiftUI's main-actor teardown hook, called before the coordinator is
    /// released. It replaces a `deinit` cleanup: the monitor token is not
    /// Sendable, so a nonisolated deinit can't touch it under Swift 6.
    static func dismantleNSView(_: ShortcutRecorderNSButton, coordinator: Coordinator) {
        coordinator.removeMonitor()
    }

    @MainActor
    class Coordinator {
        let parent: ShortcutRecorderButton
        var localMonitor: Any?

        init(parent: ShortcutRecorderButton) {
            self.parent = parent
        }

        func startRecording() {
            parent.isRecording = true
            parent.error = nil
            localMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                self?.handleKeyEvent(event)
                return nil  // Consume the event
            }
        }

        func stopRecording() {
            parent.isRecording = false
            removeMonitor()
        }

        /// Uninstall the key monitor if one is active. Also the teardown path
        /// (via `dismantleNSView`), so a window closed mid-capture can't leave
        /// a monitor swallowing every keyDown in the app.
        func removeMonitor() {
            if let monitor = localMonitor {
                NSEvent.removeMonitor(monitor)
                localMonitor = nil
            }
        }

        private static let reservedShortcuts: Set<String> = [
            "⌘Q", "⌘W", "⌘H", "⌘M", "⌘,", "⌘`",
            "⌘Z", "⌘X", "⌘C", "⌘V", "⌘A", "⌘S",
            "⌘Tab", "⌘Space",
        ]

        private func handleKeyEvent(_ event: NSEvent) {
            // Escape cancels recording
            if event.keyCode == UInt16(kVK_Escape) {
                stopRecording()
                return
            }

            // Require at least one modifier (Cmd, Ctrl, Opt)
            let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            let hasModifier = mods.contains(.command) || mods.contains(.control) || mods.contains(.option)
            guard hasModifier else { return }

            let carbonMods = GlobalHotkeyManager.carbonModifiers(from: UInt(mods.rawValue))
            let shortcut = GlobalHotkeyManager.Shortcut(
                keyCode: UInt32(event.keyCode),
                carbonModifiers: carbonMods
            )

            // Reject reserved system shortcuts
            if Self.reservedShortcuts.contains(shortcut.displayString) {
                parent.error = String(localized: "\(shortcut.displayString) is reserved by macOS")
                stopRecording()
                return
            }

            // Register and save. If registration fails (e.g. combo already
            // claimed by macOS or another app), surface that and don't persist
            // a shortcut that won't actually fire.
            let manager = GlobalHotkeyManager.shared
            if manager.register(shortcut) {
                parent.error = nil
                manager.save(shortcut)
                parent.shortcutLabel = shortcut.displayString
            } else {
                parent.error = String(
                    localized: "\(shortcut.displayString) couldn't be registered — try a different combination"
                )
            }
            stopRecording()
        }
    }
}

/// Custom NSButton that becomes first responder to capture key events.
class ShortcutRecorderNSButton: NSButton {
    weak var coordinator: ShortcutRecorderButton.Coordinator?

    override var acceptsFirstResponder: Bool { true }

    override func mouseDown(with _: NSEvent) {
        if coordinator?.parent.isRecording == true {
            coordinator?.stopRecording()
        } else {
            coordinator?.startRecording()
            window?.makeFirstResponder(self)
        }
    }
}
