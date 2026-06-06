import AVFoundation
import Carbon
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
    @State private var shortcutLabel: String = "None"
    @State private var isRecordingShortcut: Bool = false
    @State private var shortcutError: String?
    @State private var didOfferDefaultShortcut = false

    // DOLL-344: the default lives inside the app's sandbox container so it's
    // writable out of the box. Single source of truth on RecordingState.
    private let defaultDir: URL = RecordingState.defaultOutputDir

    var body: some View {
        VStack(spacing: 0) {
            // DOLL-141: each dot is a Button with a per-step label/hint
            // so VoiceOver users can reach completed steps via VO Right-arrow
            // instead of seeing only "Step N of 4" with no navigation. The
            // current step gets `.isSelected`; reachable past steps stay
            // actionable; future steps are flagged hidden so VO doesn't
            // try to navigate to inert dots.
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
                    .accessibilityHint(i < step ? "Go back to this step" : "")
                    .accessibilityAddTraits(i == step ? [.isSelected] : [])
                    .accessibilityHidden(i > step)
                    .disabled(i >= step)
                }
            }
            .padding(.top, 20)

            Spacer()

            Group {
                switch step {
                case 0:
                    welcomeStep
                case 1:
                    microphoneStep
                case 2:
                    recordingModeStep
                case 3:
                    directoryStep
                case 4:
                    keyboardShortcutStep
                default:
                    menuBarDiscoveryStep
                }
            }
            .transition(.opacity)
            .frame(maxWidth: .infinity)

            Spacer()

            HStack {
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
                Spacer()
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
            for await _ in NotificationCenter.default.notifications(named: NSApplication.didBecomeActiveNotification) {
                if step == 1 { checkMicStatus() }
            }
        }
    }

    // MARK: - Steps

    private var welcomeStep: some View {
        VStack(spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 80, height: 80)
                .accessibilityHidden(true)

            Text("Welcome to BlackBox")
                .font(.title)
                .fontWeight(.semibold)

            Text("BlackBox records audio from your Mac and saves it as WAV files. It runs quietly in your menu bar, always ready to capture.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)
        }
        .padding(.horizontal, 32)
    }

    private var microphoneStep: some View {
        VStack(spacing: 16) {
            Image(systemName: "mic.circle.fill")
                .font(.system(size: 56))
                .foregroundStyle(Color.accentColor)
                .accessibilityHidden(true)

            Text("Microphone Access")
                .font(.title2)
                .fontWeight(.semibold)

            Text("BlackBox needs access to your microphone to record audio. Your recordings stay on your Mac and are never sent anywhere.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)

            if micGranted {
                Label("Microphone access granted", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(Color(nsColor: .systemGreen))
            } else if micDenied {
                VStack(spacing: 8) {
                    Label("Microphone access denied", systemImage: "xmark.circle.fill")
                        .foregroundStyle(Color(nsColor: .systemRed))
                    Button("Open System Settings") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
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
        .padding(.horizontal, 32)
    }

    private var directoryStep: some View {
        VStack(spacing: 16) {
            if micDenied {
                // DOLL-387: this reminder previously had no recovery affordance
                // and competed with the folder task. Pair it with the same
                // "Open System Settings" action the mic step offers.
                VStack(spacing: 8) {
                    Label("Microphone access denied \u{2014} recording won't work until you allow access in System Settings.",
                          systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(Color(nsColor: .systemOrange))
                        .font(.caption)
                        .frame(maxWidth: 360)
                    Button("Open System Settings") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .font(.caption)
                }
            }

            Image(systemName: "folder.circle.fill")
                .font(.system(size: 56))
                .foregroundStyle(Color.accentColor)
                .accessibilityHidden(true)

            Text("Choose Output Directory")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Where should BlackBox save your recordings?")
                .foregroundStyle(.secondary)

            HStack {
                Text(abbreviatePath(outputDir))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundStyle(chosenURL != nil ? .primary : .secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .clipShape(.rect(cornerRadius: 6))
                    // DOLL-385: announce the full path (truncationMode elides
                    // the middle visually); match the Settings twin's label.
                    .accessibilityLabel("Output directory: \(outputDir)")
                    .accessibilityValue(chosenURL == nil ? "No folder selected" : "")

                Button("Choose\u{2026}") {
                    chooseDirectory()
                }
                .controlSize(.large)
                .accessibilityHint("Opens a file picker to select the output directory")
            }
            .frame(maxWidth: 360)

            Button("Use Default Location") {
                chosenURL = defaultDir
                outputDir = defaultDir.path
                dirChangedByUser = true
            }
            .font(.caption)
            .accessibilityHint("Saves recordings to the app's default folder")

            if chosenURL == nil {
                Text("Select a folder to continue. BlackBox will create it if it doesn't exist.")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemOrange))
            } else {
                Label("Folder selected", systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemGreen))
            }

            changeLaterCaption
        }
        .padding(.horizontal, 32)
    }

    private var recordingModeStep: some View {
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

            VStack(spacing: 12) {
                recordingModeOption(
                    title: "Continuous Recording (Recommended)",
                    description: "Saves your audio every hour so nothing is lost if the app or Mac shuts down unexpectedly.",
                    isSelected: continuousMode
                ) {
                    continuousMode = true
                }

                recordingModeOption(
                    title: "Manual Saves Only",
                    description: "Records into one file until you stop. Simpler, but unsaved audio is lost if the app quits.",
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

            Divider()
                .frame(maxWidth: 380)

            VStack(alignment: .leading, spacing: 4) {
                // DOLL-224: see SettingsView for the rationale on this wording.
                Toggle("Auto-split on silence", isOn: $silenceGateEnabled)
                Text("When enabled, BlackBox waits for audio before creating files. Saves disk space when no one is speaking.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: 380, alignment: .leading)

            changeLaterCaption
        }
        .padding(.horizontal, 32)
    }

    // DOLL-210: reassures the user that the choices they're making in
    // onboarding aren't permanent. Shown only on the two decision steps
    // (Recording Mode + Directory) where users feel committed; redundant
    // on Welcome / Microphone / Menu Bar Discovery.
    private var changeLaterCaption: some View {
        Text("You can change all of this later in Settings.")
            .font(.caption2)
            .foregroundStyle(.tertiary)
            .padding(.top, 4)
    }

    // DOLL-209: optional global-hotkey configuration step. Suggests
    // ⌘⇧R as a default the user can keep, change, or clear. Auto-register
    // is one-shot per onboarding session via `didOfferDefaultShortcut`,
    // so navigating Back→Continue won't silently re-bind a combo the
    // user already cleared.
    private var keyboardShortcutStep: some View {
        VStack(spacing: 16) {
            Image(systemName: "keyboard.fill")
                .font(.system(size: 56))
                .foregroundStyle(Color.accentColor)
                .accessibilityHidden(true)

            Text("Keyboard Shortcut")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Toggle recording from any app with a key combination. Optional — you can skip this and set one later.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)

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
            .accessibilityValue(shortcutLabel == "None" ? "No shortcut set" : shortcutLabel)
            .accessibilityHint(isRecordingShortcut
                ? "Press a key combination, or Escape to cancel"
                : "Click to record a new shortcut")

            if shortcutLabel != "None" {
                Button("Clear shortcut") {
                    GlobalHotkeyManager.shared.unregister()
                    GlobalHotkeyManager.shared.save(nil)
                    shortcutLabel = "None"
                    shortcutError = nil
                }
                .font(.caption)
                .accessibilityHint("Removes the current keyboard shortcut")
            }

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

            changeLaterCaption
        }
        .padding(.horizontal, 32)
        .onAppear { offerDefaultShortcutIfNeeded() }
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
            shortcutError = "\u{2318}\u{21E7}R is already in use \u{2014} click the button to choose a different combination."
        }
    }

    // DOLL-208: final onboarding step pointing the user at the menu bar.
    // Runtime telemetry on a real install showed a user who completed
    // onboarding then never recorded — the most likely cause is they
    // didn't realise the app lives in the menu bar. A post-dismiss
    // .bounce on the icon (wired in BlackBoxApp.swift) reinforces this.
    private var menuBarDiscoveryStep: some View {
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

            Text("BlackBox lives in your menu bar at the top of your screen. Click the icon above to start recording.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 360)
        }
        .padding(.horizontal, 32)
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

    private func abbreviatePath(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path.hasPrefix(home) { return "~" + path.dropFirst(home.count) }
        return path
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

    /// Open NSOpenPanel and return the chosen URL (with security scope), or nil if cancelled.
    @discardableResult
    private func chooseDirectory() -> URL? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Select"
        panel.message = "Select output directory for recordings"
        panel.directoryURL = URL(fileURLWithPath: outputDir)

        if panel.runModal() == .OK, let url = panel.url {
            outputDir = url.path
            chosenURL = url
            dirChangedByUser = true
            return url
        }
        return nil
    }

    /// Skip onboarding with default settings (experienced users).
    private func skipOnboarding() {
        // DOLL-344: the default is the in-container directory — no
        // security-scoped bookmark, and useDefaultOutputDir creates it.
        recorder.useDefaultOutputDir()

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
                recorder.useDefaultOutputDir()
            } else {
                recorder.saveOutputDirBookmark(for: url)
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
    func makeNSView(context: Context) -> NSView { OnboardingConfiguratorView() }
    func updateNSView(_ nsView: NSView, context: Context) {}
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
