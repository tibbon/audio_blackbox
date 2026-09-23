import AVFoundation
import SwiftUI

struct OnboardingView: View {
    var recorder: RecordingState
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @Environment(\.dismiss) private var dismiss

    // DOLL-253: the onboarding window is pinned to content size and not
    // user-resizable, so a hard 460pt width truncated/awkwardly-wrapped the
    // step captions at large Dynamic Type sizes. @ScaledMetric grows the
    // pinned width with the user's text size so the content keeps room.
    @ScaledMetric(relativeTo: .body) private var contentWidth: CGFloat = 460

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var step = 0
    @State private var micGranted = false
    @State private var micDenied = false
    @State private var continuousMode = true
    @State private var silenceGateEnabled = true
    @State private var outputDir: String = ""
    @State private var chosenURL: URL?
    @State private var dirChangedByUser = false
    // DOLL-209: bindings for the keyboard-shortcut step.
    // Mirror the trio the existing ShortcutRecorderButton in SettingsView
    // takes (label / isRecording / error). `didOfferDefaultShortcut`
    // is a one-shot flag scoped to the OnboardingView lifecycle so the
    // suggested ⌘⇧R only gets auto-registered once per onboarding run
    // — if the user clears it and navigates Back→Continue, we won't
    // silently re-register the default they just rejected.
    @State private var shortcutLabel = String(localized: "None")
    @State private var isRecordingShortcut: Bool = false
    @State private var shortcutError: String?
    @State private var didOfferDefaultShortcut = false

    // DOLL-344: the default lives inside the app's sandbox container so it's
    // writable out of the box. Single source of truth on RecordingState.
    private let defaultDir: URL = RecordingState.defaultOutputDir

    var body: some View {
        VStack(spacing: 0) {
            progressDots
                .padding(.top, 20)

            Spacer()

            Group {
                currentStep
            }
            .transition(.opacity)
            .frame(maxWidth: .infinity)

            Spacer()

            HStack {
                backOrSkipButton
                Spacer()
                forwardButton
            }
            .padding(.horizontal, 32)
            .padding(.bottom, 24)
        }
        // minHeight is what enforces the size on subsequent launches —
        // SwiftUI persists the window frame in user defaults, so .defaultSize
        // on the Window only applies the first time. The view-level minHeight
        // is what windowResizability(.contentSize) honours every open.
        .frame(minWidth: contentWidth, maxWidth: contentWidth, minHeight: 540)
        .background(OnboardingWindowConfigurator())
        .onAppear {
            // On re-run, preserve the user's existing output directory.
            // On first run, treat the auto-populated default folder as a
            // deliberate selection so completeOnboarding() saves a
            // security-scoped bookmark for it (DOLL-133). Without this,
            // first-run users got dropped at "Output Directory Unavailable"
            // on the next launch because no bookmark existed.
            if let savedPath = UserDefaults.standard.string(forKey: SettingsKeys.lastOutputDirPath) {
                outputDir = savedPath
                chosenURL = URL(fileURLWithPath: savedPath)
            } else {
                outputDir = defaultDir.path
                chosenURL = defaultDir
                dirChangedByUser = true
            }
            checkMicStatus()
        }
        .onChange(of: step) {
            if step == 1 { checkMicStatus() }
        }
        .task {
            for await _ in NotificationCenter.default.notifications(named: NSApplication.didBecomeActiveNotification)
            where step == 1 {
                checkMicStatus()
            }
        }
    }

    // DOLL-141: each dot is a Button with a per-step label/hint
    // so VoiceOver users can reach completed steps via VO Right-arrow
    // instead of seeing only "Step N of 4" with no navigation. The
    // current step gets `.isSelected`; reachable past steps stay
    // actionable; future steps are flagged hidden so VO doesn't
    // try to navigate to inert dots.
    private var progressDots: some View {
        HStack(spacing: 8) {
            ForEach(0..<6) { i in
                Button {
                    if i < step { animateStep { step = i } }
                } label: {
                    Circle()
                        .fill(i <= step ? Color.accentColor : Color.secondary.opacity(0.3))
                        .frame(width: 8, height: 8)
                }
                .buttonStyle(.plain)
                .contentShape(Circle())
                .accessibilityLabel("Onboarding step \(i + 1) of 6")
                .accessibilityHint(i < step ? String(localized: "Go back to this step") : "")
                .accessibilityAddTraits(i == step ? [.isSelected] : [])
                .accessibilityHidden(i > step)
                .disabled(i >= step)
            }
        }
    }

    // MARK: - Steps

    @ViewBuilder private var currentStep: some View {
        switch step {
        case 0:
            WelcomeStep()

        case 1:
            MicrophoneStep(micGranted: micGranted, micDenied: micDenied, requestMicAccess: requestMicAccess)

        case 2:
            RecordingModeStep(continuousMode: $continuousMode, silenceGateEnabled: $silenceGateEnabled)

        case 3:
            DirectoryStep(
                micDenied: micDenied,
                defaultDir: defaultDir,
                outputDir: $outputDir,
                chosenURL: $chosenURL,
                dirChangedByUser: $dirChangedByUser
            )

        case 4:
            KeyboardShortcutStep(
                shortcutLabel: $shortcutLabel,
                isRecordingShortcut: $isRecordingShortcut,
                shortcutError: $shortcutError,
                didOfferDefaultShortcut: $didOfferDefaultShortcut
            )

        default:
            MenuBarDiscoveryStep()
        }
    }

    // MARK: - Navigation

    @ViewBuilder private var backOrSkipButton: some View {
        if step > 0 {
            Button("Back") {
                animateStep { step -= 1 }
            }
        } else {
            Button("Skip Setup") {
                skipOnboarding()
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var forwardButton: some View {
        switch step {
        case 0:
            Button("Get Started") {
                animateStep { step = 1 }
            }
            .keyboardShortcut(.defaultAction)

        case 1:
            Button("Continue") {
                animateStep { step += 1 }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(!micGranted && !micDenied)

        case 2:
            Button("Continue") {
                animateStep { step += 1 }
            }
            .keyboardShortcut(.defaultAction)

        case 3:
            Button("Continue") {
                animateStep { step += 1 }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(chosenURL == nil)

        case 4:
            Button("Continue") {
                animateStep { step += 1 }
            }
            .keyboardShortcut(.defaultAction)

        default:
            Button("Start Using BlackBox") {
                completeOnboarding()
            }
            .keyboardShortcut(.defaultAction)
        }
    }

    // MARK: - Actions

    private func animateStep(_ body: () -> Void) {
        if reduceMotion {
            body()
        } else {
            withAnimation { body() }
        }
        // DOLL-255: the step content swaps via .transition(.opacity), which
        // is silent to VoiceOver — a VO user otherwise has to explore to
        // discover the screen changed. Announce the new step's heading. This
        // is the single chokepoint for every step change (nav buttons, Back,
        // and the progress dots all route through here).
        AccessibilityNotification.Announcement(stepTitle(for: step)).post()
    }

    /// Heading of each onboarding step, matching the visible title, used for
    /// the VoiceOver step-change announcement (DOLL-255).
    private func stepTitle(for step: Int) -> String {
        // DOLL-439: localized so the VoiceOver announcement matches the
        // (localizable) visible step titles.
        switch step {
        case 0: return String(localized: "Welcome to BlackBox")
        case 1: return String(localized: "Microphone Access")
        case 2: return String(localized: "Continuous Recording")
        case 3: return String(localized: "Choose Output Directory")
        case 4: return String(localized: "Keyboard Shortcut")
        default: return String(localized: "You're all set")
        }
    }

    private func checkMicStatus() {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            micGranted = true

        case .denied, .restricted:
            micDenied = true

        default:
            break
        }
    }

    private func requestMicAccess() {
        // DOLL-267: use the modern async API (matching RecordingState) instead
        // of the legacy closure form + nested @MainActor Task hop.
        Task { @MainActor in
            let granted = await AVCaptureDevice.requestAccess(for: .audio)
            micGranted = granted
            micDenied = !granted
        }
    }

    /// Skip onboarding with default settings (experienced users).
    private func skipOnboarding() {
        // DOLL-344: the default is the in-container directory — no
        // security-scoped bookmark, and useDefaultOutputDir creates it.
        // A stored bookmark means the user already picked a folder (this is
        // "Run Setup Again"); Skip keeps it rather than silently moving
        // their recordings back to the default.
        if UserDefaults.standard.data(forKey: SettingsKeys.outputDirBookmark) == nil {
            recorder.switchOutputDir(to: nil)
        }

        let defaults = UserDefaults.standard
        defaults.set(true, forKey: SettingsKeys.continuousMode)
        defaults.set(3600, forKey: SettingsKeys.recordingCadence)
        defaults.set(true, forKey: SettingsKeys.silenceGateEnabled)
        recorder.bridge.setConfig([
            "continuous_mode": true,
            "recording_cadence": 3600,
            "silence_gate_enabled": true,
        ])

        // Warn if mic permission hasn't been granted yet
        let micStatus = AVCaptureDevice.authorizationStatus(for: .audio)
        if micStatus == .denied || micStatus == .restricted {
            recorder.errorMessage = String(localized: "Microphone access denied. Open System Settings to allow access.")
            recorder.statusText = String(localized: "Error")
        }

        hasCompletedOnboarding = true
        dismiss()
    }

    private func completeOnboarding() {
        // chosenURL is guaranteed non-nil — button is disabled until user picks a folder.
        guard let url = chosenURL else { return }

        // Only update the bookmark if the user explicitly picked a new directory.
        // Re-running onboarding without changing the dir preserves the existing bookmark.
        if dirChangedByUser {
            // DOLL-344: the in-container default needs no security-scoped
            // bookmark; only a user-picked folder (outside the container) does.
            if url.standardizedFileURL == RecordingState.defaultOutputDir.standardizedFileURL {
                recorder.switchOutputDir(to: nil)
            } else {
                recorder.switchOutputDir(to: url)
            }
        }

        // Save recording mode choice
        let defaults = UserDefaults.standard
        defaults.set(continuousMode, forKey: SettingsKeys.continuousMode)
        if continuousMode {
            defaults.set(3600, forKey: SettingsKeys.recordingCadence)
        }
        defaults.set(silenceGateEnabled, forKey: SettingsKeys.silenceGateEnabled)
        recorder.bridge.setConfig([
            "continuous_mode": continuousMode,
            "recording_cadence": continuousMode ? 3600 : 300,
            "silence_gate_enabled": silenceGateEnabled,
        ])

        hasCompletedOnboarding = true
        dismiss()
    }
}

/// Disables minimize and zoom buttons on the Onboarding window per Apple HIG.
/// Uses viewDidMoveToWindow to configure once, not on every SwiftUI render.
private struct OnboardingWindowConfigurator: NSViewRepresentable {
    func makeNSView(context _: Context) -> NSView { OnboardingConfiguratorView() }
    func updateNSView(_: NSView, context _: Context) {
        // The window buttons are configured once in viewDidMoveToWindow; nothing varies per render.
    }
}

private final class OnboardingConfiguratorView: NSView {
    private var configured = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard !configured, let window else { return }
        configured = true
        window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
        window.standardWindowButton(.zoomButton)?.isEnabled = false
    }
}
