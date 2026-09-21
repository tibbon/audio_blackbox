import Carbon
import SwiftUI

extension OnboardingView {
    // DOLL-209: optional global-hotkey configuration step. Suggests
    // ⌘⇧R as a default the user can keep, change, or clear. Auto-register
    // is one-shot per onboarding session via `didOfferDefaultShortcut`,
    // so navigating Back→Continue won't silently re-bind a combo the
    // user already cleared. That flag and the shortcut state live on
    // OnboardingView because this view is removed when the user moves on.
    struct KeyboardShortcutStep: View {
        @Binding var shortcutLabel: String
        @Binding var isRecordingShortcut: Bool
        @Binding var shortcutError: String?
        @Binding var didOfferDefaultShortcut: Bool

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
                    Toggle recording from any app with a key combination. \
                    Optional — you can skip this and set one later.
                    """
                )
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)

                recorderRow

                clearButton

                errorLabel

                ChangeLaterCaption()
            }
            .padding(.horizontal, 32)
            .onAppear { offerDefaultShortcutIfNeeded() }
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

        /// Try to register ⌘⇧R as a suggested default when the user first
        /// reaches the hotkey step and no shortcut is already saved. If the
        /// combo is taken by another app, surface a hint instead of a hard
        /// error so the user picks their own.
        private func offerDefaultShortcutIfNeeded() {
            // Already saved (re-run onboarding, or user came back to this step)
            if let saved = GlobalHotkeyManager.shared.loadSaved() {
                shortcutLabel = saved.displayString
                return
            }
            // Already attempted this session — respect the user's intent if
            // they cleared it.
            guard !didOfferDefaultShortcut else { return }
            didOfferDefaultShortcut = true

            let suggested = GlobalHotkeyManager.Shortcut(
                keyCode: UInt32(kVK_ANSI_R),
                carbonModifiers: UInt32(cmdKey | shiftKey)
            )
            if GlobalHotkeyManager.shared.register(suggested) {
                GlobalHotkeyManager.shared.save(suggested)
                shortcutLabel = suggested.displayString
            } else {
                shortcutError = String(
                    localized: """
                        \u{2318}\u{21E7}R is already in use \u{2014} \
                        click the button to choose a different combination.
                        """
                )
            }
        }
    }
}
