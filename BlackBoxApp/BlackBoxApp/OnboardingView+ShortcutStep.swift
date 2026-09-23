import SwiftUI

extension OnboardingView {
    /// The shortcut step's state and every action it takes on the global
    /// hotkey, kept out of the view so the tests run the same code the step
    /// runs. It lives on OnboardingView because the step's view is removed
    /// when the user moves on.
    ///
    /// DOLL-209: the step is opt-in. Appearing, Continue, Back and finishing
    /// onboarding register and save nothing. A shortcut exists only after
    /// the user clicks "Use ⌘⇧R" or records their own, because a global
    /// hotkey takes the combination over in every app and ⌘⇧R is also the
    /// browsers' hard reload.
    struct ShortcutStepModel {
        /// The button's label: the shortcut, or "None".
        var label = String(localized: "None")
        var error: String?

        var hasShortcut: Bool { label != String(localized: "None") }

        /// Appearing shows a shortcut saved earlier (a "Run Setup Again"
        /// user keeps theirs). Reads defaults only: nothing is registered.
        mutating func appear() {
            label = GlobalHotkeyManager.shared.loadSaved()?.displayString ?? String(localized: "None")
        }

        /// The "Use ⌘⇧R" action: register the suggested shortcut and save
        /// it for launch restore, through the same path as the Settings
        /// recorder. Saves nothing if another app owns it.
        mutating func useSuggested() {
            let suggested = GlobalHotkeyManager.suggestedShortcut
            if GlobalHotkeyManager.shared.registerAndSave(suggested) {
                label = suggested.displayString
                error = nil
            } else {
                error = String(
                    localized: "\(suggested.displayString) is already in use. Record a different combination instead."
                )
            }
        }

        /// The "Clear shortcut" action.
        mutating func clear() {
            GlobalHotkeyManager.shared.unregister()
            GlobalHotkeyManager.shared.save(nil)
            label = String(localized: "None")
            error = nil
        }
    }

    struct KeyboardShortcutStep: View {
        @Binding var model: ShortcutStepModel
        @Binding var isRecordingShortcut: Bool

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
            .onAppear { model.appear() }
        }

        /// One-click opt-in to the suggested ⌘⇧R, with the trade-off spelled
        /// out next to it. Hidden once any shortcut is set.
        @ViewBuilder private var suggestedShortcutButton: some View {
            if !model.hasShortcut {
                VStack(spacing: 6) {
                    Button("Use \(GlobalHotkeyManager.suggestedShortcut.displayString)") {
                        model.useSuggested()
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
                    shortcutLabel: $model.label,
                    isRecording: $isRecordingShortcut,
                    error: $model.error
                )
            }
            .frame(maxWidth: 360)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Global keyboard shortcut for toggling recording")
            .accessibilityValue(model.hasShortcut ? model.label : String(localized: "No shortcut set"))
            .accessibilityHint(
                isRecordingShortcut
                    ? String(localized: "Press a key combination, or Escape to cancel")
                    : String(localized: "Click to record a new shortcut")
            )
        }

        @ViewBuilder private var clearButton: some View {
            if model.hasShortcut {
                Button("Clear shortcut") {
                    model.clear()
                }
                .font(.caption)
                .accessibilityHint("Removes the current keyboard shortcut")
            }
        }

        @ViewBuilder private var errorLabel: some View {
            if let error = model.error {
                Label {
                    Text(error)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .accessibilityHidden(true)
                }
                .font(.caption)
                .foregroundStyle(Color(nsColor: .systemOrange))
                .frame(maxWidth: 360)
            }
        }
    }
}
