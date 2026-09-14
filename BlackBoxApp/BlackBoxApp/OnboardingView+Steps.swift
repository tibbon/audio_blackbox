import SwiftUI

// The informational onboarding steps and the recording-mode choice. The
// directory and keyboard-shortcut steps have their own files. State that must
// outlive a step (it is removed when the user moves on) stays on OnboardingView
// and comes in as a value or binding.
extension OnboardingView {
    struct WelcomeStep: View {
        var body: some View {
            VStack(spacing: 16) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 80, height: 80)
                    .accessibilityHidden(true)

                Text("Welcome to BlackBox")
                    .font(.title)
                    .fontWeight(.semibold)

                Text(
                    """
                    BlackBox records audio from your Mac and saves it as WAV files. \
                    It runs quietly in your menu bar, always ready to capture.
                    """
                )
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)
            }
            .padding(.horizontal, 32)
        }
    }

    struct MicrophoneStep: View {
        let micGranted: Bool
        let micDenied: Bool
        /// Triggers the macOS microphone-permission prompt.
        let requestMicAccess: @MainActor () -> Void

        var body: some View {
            VStack(spacing: 16) {
                Image(systemName: "mic.circle.fill")
                    .font(.system(size: 56))
                    .foregroundStyle(Color.accentColor)
                    .accessibilityHidden(true)

                Text("Microphone Access")
                    .font(.title2)
                    .fontWeight(.semibold)

                Text(
                    """
                    BlackBox needs access to your microphone to record audio. \
                    Your recordings stay on your Mac and are never sent anywhere.
                    """
                )
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)

                permissionStatus
            }
            .padding(.horizontal, 32)
        }

        @ViewBuilder private var permissionStatus: some View {
            if micGranted {
                Label("Microphone access granted", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(Color(nsColor: .systemGreen))
            } else if micDenied {
                VStack(spacing: 8) {
                    Label("Microphone access denied", systemImage: "xmark.circle.fill")
                        .foregroundStyle(Color(nsColor: .systemRed))
                    Button("Open System Settings") {
                        if let url = URL(
                            string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
                        ) {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .font(.caption)
                }
            } else {
                // DOLL-255: was "Continue", which collided with the bottom-nav
                // "Continue" — VoiceOver read "Continue button" twice on this
                // step. This label also describes what the button actually does
                // (triggers the macOS mic-permission prompt).
                Button("Allow Microphone Access") {
                    requestMicAccess()
                }
                .controlSize(.large)
            }
        }
    }

    struct RecordingModeStep: View {
        @Binding var continuousMode: Bool
        @Binding var silenceGateEnabled: Bool

        var body: some View {
            VStack(spacing: 16) {
                Image(systemName: "recordingtape.circle.fill")
                    .font(.system(size: 56))
                    .foregroundStyle(Color.accentColor)
                    .accessibilityHidden(true)

                Text("Continuous Recording")
                    .font(.title2)
                    .fontWeight(.semibold)

                Text("How should BlackBox protect your recordings?")
                    .foregroundStyle(.secondary)

                modeOptions

                Divider()
                    .frame(maxWidth: 380)

                silenceGateToggle

                ChangeLaterCaption()
            }
            .padding(.horizontal, 32)
        }

        private var modeOptions: some View {
            VStack(spacing: 12) {
                recordingModeOption(
                    title: String(localized: "Continuous Recording (Recommended)"),
                    description: String(
                        localized:
                            "Saves your audio every hour so nothing is lost if the app or Mac shuts down unexpectedly."
                    ),
                    isSelected: continuousMode
                ) {
                    continuousMode = true
                }

                recordingModeOption(
                    title: String(localized: "Manual Saves Only"),
                    description: String(
                        localized:
                            "Records into one file until you stop. Simpler, but unsaved audio is lost if the app quits."
                    ),
                    isSelected: !continuousMode
                ) {
                    continuousMode = false
                }
            }
            .frame(maxWidth: 380)
            // DOLL-141: replace the accessibility tree of the two visual
            // "cards" with a real single-select Picker so VoiceOver
            // announces them as one group ("Recording mode, Continuous
            // Recording, 1 of 2") rather than two unrelated buttons.
            .accessibilityRepresentation {
                Picker("Recording mode", selection: $continuousMode) {
                    Text("Continuous Recording (Recommended)").tag(true)
                    Text("Manual Saves Only").tag(false)
                }
            }
        }

        private var silenceGateToggle: some View {
            VStack(alignment: .leading, spacing: 4) {
                // DOLL-224: see SilenceDetectionSection for the rationale on this wording.
                Toggle("Auto-split on silence", isOn: $silenceGateEnabled)
                Text(
                    """
                    When enabled, BlackBox waits for audio before creating files. \
                    Saves disk space when no one is speaking.
                    """
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: 380, alignment: .leading)
        }

        private func recordingModeOption(
            title: String,
            description: String,
            isSelected: Bool,
            action: @escaping () -> Void
        ) -> some View {
            Button(action: action) {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                        .font(.title3)
                        .foregroundStyle(isSelected ? Color.accentColor : Color.secondary)
                        .frame(width: 24)

                    VStack(alignment: .leading, spacing: 2) {
                        Text(title)
                            .fontWeight(.medium)
                            .foregroundStyle(.primary)
                        Text(description)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: 8)
                        .fill(isSelected ? Color.accentColor.opacity(0.1) : Color.clear)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(isSelected ? Color.accentColor : Color.secondary.opacity(0.3), lineWidth: 1)
                )
            }
            .buttonStyle(.plain)
            .accessibilityLabel(title)
            .accessibilityHint(description)
            .accessibilityAddTraits(isSelected ? [.isSelected] : [])
        }
    }

    // DOLL-208: final onboarding step pointing the user at the menu bar.
    // Runtime telemetry on a real install showed a user who completed
    // onboarding then never recorded — the most likely cause is they
    // didn't realise the app lives in the menu bar. A post-dismiss
    // .bounce on the icon (wired in BlackBoxApp.swift) reinforces this.
    struct MenuBarDiscoveryStep: View {
        var body: some View {
            VStack(spacing: 16) {
                Image(systemName: "arrow.up")
                    .font(.system(size: 56, weight: .light))
                    .foregroundStyle(Color.accentColor)
                    .accessibilityHidden(true)

                Image(systemName: "record.circle")
                    .font(.system(size: 28))
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)

                Text("You're all set")
                    .font(.title2)
                    .fontWeight(.semibold)

                Text(
                    """
                    BlackBox lives in your menu bar at the top of your screen. \
                    Click the icon above to start recording.
                    """
                )
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)
            }
            .padding(.horizontal, 32)
        }
    }

    // DOLL-210: reassures the user that the choices they're making in
    // onboarding aren't permanent. Shown only on the decision steps
    // (Recording Mode, Directory, Keyboard Shortcut) where users feel
    // committed; redundant on Welcome / Microphone / Menu Bar Discovery.
    struct ChangeLaterCaption: View {
        var body: some View {
            Text("You can change all of this later in Settings.")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .padding(.top, 4)
        }
    }
}
