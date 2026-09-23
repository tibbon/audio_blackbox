import SwiftUI

extension OnboardingView {
    // DOLL-209: optional global-hotkey step. Opt-in: appearing, Continue,
    // Back and finishing onboarding register and save nothing. A shortcut
    // exists only after the user clicks "Use ⌘⇧R" or records their own,
    // because a global hotkey takes the combination over in every app and
    // ⌘⇧R is also the browsers' hard reload. The shortcut state lives on
    // OnboardingView because this view is removed when the user moves on.
    struct KeyboardShortcutStep: View {
        @Binding var shortcutLabel: String
        @Binding var isRecordingShortcut: Bool
        @Binding var shortcutError: String?

        var body: some View {
            VStack(spacing: 16) {
                Image(systemName: "keyboard.fill")
                    .font(.system(size: 56))
                    .foregroundStyle(Color.accentColor)
                    .accessibilityHidden(true)

                Text("Keyboard Shortcut")
                    .font(.title2)
                    .fontWeight(.semibold)

                Text(
                    """
                    Want a key combination that starts and stops recording? \
                    It works system-wide, so it replaces that combination in \
                    every app while BlackBox is running. Optional — skip this \
                    and set one later.
                    """
                )
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)

                suggestedShortcutButton

                recorderRow

                clearButton

                errorLabel

                ChangeLaterCaption()
            }
            .padding(.horizontal, 32)
            // Show a shortcut saved earlier (a "Run Setup Again" user keeps
            // theirs). Reading only: nothing is registered here.
            .onAppear { shortcutLabel = Self.savedShortcutLabel() }
        }

        /// One-click opt-in to the suggested ⌘⇧R, with the trade-off spelled
        /// out next to it. Hidden once any shortcut is set.
        @ViewBuilder private var suggestedShortcutButton: some View {
            if shortcutLabel == String(localized: "None") {
                VStack(spacing: 6) {
                    Button("Use \(GlobalHotkeyManager.suggestedShortcut.displayString)") {
                        useSuggestedShortcut()
                    }
                    .accessibilityHint("Sets Command-Shift-R as the shortcut for starting and stopping recording")

                    Text(
                        """
                        \(GlobalHotkeyManager.suggestedShortcut.displayString) is also your browser's \
                        hard reload. BlackBox takes it over while running.
                        """
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 360)
                }
            }
        }

        private var recorderRow: some View {
            HStack {
                Text("Toggle Recording:")
                Spacer()
                ShortcutRecorderButton(
                    shortcutLabel: $shortcutLabel,
                    isRecording: $isRecordingShortcut,
                    error: $shortcutError
                )
            }
            .frame(maxWidth: 360)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Global keyboard shortcut for toggling recording")
            .accessibilityValue(
                shortcutLabel == String(localized: "None") ? String(localized: "No shortcut set") : shortcutLabel
            )
            .accessibilityHint(
                isRecordingShortcut
                    ? String(localized: "Press a key combination, or Escape to cancel")
                    : String(localized: "Click to record a new shortcut")
            )
        }

        @ViewBuilder private var clearButton: some View {
            if shortcutLabel != String(localized: "None") {
                Button("Clear shortcut") {
                    GlobalHotkeyManager.shared.unregister()
                    GlobalHotkeyManager.shared.save(nil)
                    shortcutLabel = String(localized: "None")
                    shortcutError = nil
                }
                .font(.caption)
                .accessibilityHint("Removes the current keyboard shortcut")
            }
        }

        @ViewBuilder private var errorLabel: some View {
            if let shortcutError {
                Label {
                    Text(shortcutError)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .accessibilityHidden(true)
                }
                .font(.caption)
                .foregroundStyle(Color(nsColor: .systemOrange))
                .frame(maxWidth: 360)
            }
        }

        private func useSuggestedShortcut() {
            let suggested = GlobalHotkeyManager.suggestedShortcut
            if Self.chooseSuggestedShortcut() {
                shortcutLabel = suggested.displayString
                shortcutError = nil
            } else {
                shortcutError = String(
                    localized: "\(suggested.displayString) is already in use. Record a different combination instead."
                )
            }
        }

        /// Label for the saved shortcut, or "None". Reads defaults only, so
        /// reaching this step never registers or saves a shortcut.
        static func savedShortcutLabel() -> String {
            GlobalHotkeyManager.shared.loadSaved()?.displayString ?? String(localized: "None")
        }

        /// The "Use ⌘⇧R" action: register the suggested shortcut and save it
        /// for launch restore, through the same path as the Settings
        /// recorder. Returns `false` (and saves nothing) if another app owns it.
        static func chooseSuggestedShortcut() -> Bool {
            GlobalHotkeyManager.shared.registerAndSave(GlobalHotkeyManager.suggestedShortcut)
        }
    }
}
