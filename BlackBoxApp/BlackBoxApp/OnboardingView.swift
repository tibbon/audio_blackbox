import AVFoundation
import SwiftUI

struct OnboardingView: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @Environment(\.dismiss) private var dismiss

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var step = 0
    @State private var micGranted = false
    @State private var micDenied = false
    @State private var continuousMode = true
    @State private var silenceGateEnabled = true
    @State private var outputDir: String = ""
    @State private var chosenURL: URL?

    private let defaultDir: URL = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Music")
        .appendingPathComponent("BlackBox Recordings")

    var body: some View {
        VStack(spacing: 0) {
            // Step indicator
            HStack(spacing: 8) {
                ForEach(0..<4) { i in
                    Circle()
                        .fill(i <= step ? Color.accentColor : Color.secondary.opacity(0.3))
                        .frame(width: 8, height: 8)
                        .contentShape(Circle())
                        .onTapGesture {
                            if i < step { animateStep { step = i } }
                        }
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Step \(step + 1) of 4")
            .padding(.top, 20)

            Spacer()

            // Step content
            Group {
                switch step {
                case 0:
                    welcomeStep
                case 1:
                    microphoneStep
                case 2:
                    recordingModeStep
                default:
                    directoryStep
                }
            }
            .transition(.opacity)
            .frame(maxWidth: .infinity)

            Spacer()

            // Navigation buttons
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
                    .foregroundColor(.secondary)
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
                default:
                    Button("Start Using BlackBox") {
                        completeOnboarding()
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(chosenURL == nil)
                }
            }
            .padding(.horizontal, 32)
            .padding(.bottom, 24)
        }
        .frame(minWidth: 460, maxWidth: 460, minHeight: 380)
        .background(OnboardingWindowConfigurator())
        .onAppear {
            outputDir = defaultDir.path
            chosenURL = defaultDir
            checkMicStatus()
        }
        .onChange(of: step) {
            if step == 1 { checkMicStatus() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            if step == 1 { checkMicStatus() }
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
                .foregroundColor(.secondary)
                .frame(maxWidth: 360)
        }
        .padding(.horizontal, 32)
    }

    private var microphoneStep: some View {
        VStack(spacing: 16) {
            Image(systemName: "mic.circle.fill")
                .font(.system(size: 56))
                .foregroundColor(.accentColor)
                .accessibilityHidden(true)

            Text("Microphone Access")
                .font(.title2)
                .fontWeight(.semibold)

            Text("BlackBox needs access to your microphone to record audio. Your recordings stay on your Mac and are never sent anywhere.")
                .multilineTextAlignment(.center)
                .foregroundColor(.secondary)
                .frame(maxWidth: 360)

            if micGranted {
                Label("Microphone access granted", systemImage: "checkmark.circle.fill")
                    .foregroundColor(Color(nsColor: .systemGreen))
            } else if micDenied {
                VStack(spacing: 8) {
                    Label("Microphone access denied", systemImage: "xmark.circle.fill")
                        .foregroundColor(Color(nsColor: .systemRed))
                    Button("Open System Settings") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .font(.caption)
                }
            } else {
                Button("Continue") {
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
                Label("Microphone access denied \u{2014} recording won't work until you allow access in System Settings.",
                      systemImage: "exclamationmark.triangle.fill")
                    .foregroundColor(Color(nsColor: .systemOrange))
                    .font(.caption)
                    .frame(maxWidth: 360)
            }

            Image(systemName: "folder.circle.fill")
                .font(.system(size: 56))
                .foregroundColor(.accentColor)
                .accessibilityHidden(true)

            Text("Choose Output Directory")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Where should BlackBox save your recordings?")
                .foregroundColor(.secondary)

            HStack {
                Text(abbreviatePath(outputDir))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundColor(chosenURL != nil ? .primary : .secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .cornerRadius(6)

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
            }
            .font(.caption)
            .accessibilityHint("Saves recordings to ~/Music/BlackBox Recordings")

            if chosenURL == nil {
                Text("Select a folder to continue. BlackBox will create it if it doesn't exist.")
                    .font(.caption)
                    .foregroundColor(Color(nsColor: .systemOrange))
            } else {
                Label("Folder selected", systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundColor(Color(nsColor: .systemGreen))
            }
        }
        .padding(.horizontal, 32)
    }

    private var recordingModeStep: some View {
        VStack(spacing: 16) {
            Image(systemName: "recordingtape.circle.fill")
                .font(.system(size: 56))
                .foregroundColor(.accentColor)
                .accessibilityHidden(true)

            Text("Continuous Recording")
                .font(.title2)
                .fontWeight(.semibold)

            Text("How should BlackBox protect your recordings?")
                .foregroundColor(.secondary)

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

            Divider()
                .frame(maxWidth: 380)

            VStack(alignment: .leading, spacing: 4) {
                Toggle("Pause recording during silence", isOn: $silenceGateEnabled)
                Text("When enabled, BlackBox waits for audio before creating files. Saves disk space when no one is speaking.")
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: 380, alignment: .leading)
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
                    .foregroundColor(isSelected ? .accentColor : .secondary)
                    .frame(width: 24)

                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .fontWeight(.medium)
                        .foregroundColor(.primary)
                    Text(description)
                        .font(.caption)
                        .foregroundColor(.secondary)
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
        AVCaptureDevice.requestAccess(for: .audio) { granted in
            Task { @MainActor in
                micGranted = granted
                micDenied = !granted
            }
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
            return url
        }
        return nil
    }

    /// Skip onboarding with default settings (experienced users).
    private func skipOnboarding() {
        let url = defaultDir
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        recorder.saveOutputDirBookmark(for: url)

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
            recorder.errorMessage = "Microphone access denied. Open System Settings to allow access."
            recorder.statusText = "Error"
        }

        hasCompletedOnboarding = true
        dismiss()
    }

    private func completeOnboarding() {
        // chosenURL is guaranteed non-nil — button is disabled until user picks a folder.
        guard let url = chosenURL else { return }

        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        recorder.saveOutputDirBookmark(for: url)

        // Save recording mode choice
        let defaults = UserDefaults.standard
        defaults.set(continuousMode, forKey: SettingsKeys.continuousMode)
        if continuousMode {
            defaults.set(3600, forKey: SettingsKeys.recordingCadence) // 1 hour
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
private struct OnboardingWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            guard let window = nsView.window else { return }
            window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
            window.standardWindowButton(.zoomButton)?.isEnabled = false
        }
    }
}
