import AVFoundation
import SwiftUI

struct OnboardingView: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @Environment(\.dismiss) private var dismiss

    @State private var step = 0
    @State private var micGranted = false
    @State private var micDenied = false
    @State private var continuousMode = true
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
                }
            }
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
            .frame(maxWidth: .infinity)

            Spacer()

            // Navigation buttons
            HStack {
                if step > 0 {
                    Button("Back") {
                        step -= 1
                    }
                }
                Spacer()
                switch step {
                case 0:
                    Button("Get Started") {
                        step = 1
                    }
                    .keyboardShortcut(.defaultAction)
                case 1:
                    Button("Continue") {
                        step += 1
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!micGranted && !micDenied)
                case 2:
                    Button("Continue") {
                        step += 1
                    }
                    .keyboardShortcut(.defaultAction)
                default:
                    Button("Start Using BlackBox") {
                        completeOnboarding()
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(outputDir.isEmpty)
                }
            }
            .padding(.horizontal, 32)
            .padding(.bottom, 24)
        }
        .frame(width: 460, height: 380)
        .onAppear {
            outputDir = defaultDir.path
            checkMicStatus()
        }
    }

    // MARK: - Steps

    private var welcomeStep: some View {
        VStack(spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 80, height: 80)

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

            Text("Microphone Access")
                .font(.title2)
                .fontWeight(.semibold)

            Text("BlackBox needs access to your microphone to record audio. Your recordings stay on your Mac and are never sent anywhere.")
                .multilineTextAlignment(.center)
                .foregroundColor(.secondary)
                .frame(maxWidth: 360)

            if micGranted {
                Label("Microphone access granted", systemImage: "checkmark.circle.fill")
                    .foregroundColor(.green)
            } else if micDenied {
                VStack(spacing: 8) {
                    Label("Microphone access denied", systemImage: "xmark.circle.fill")
                        .foregroundColor(.red)
                    Button("Open System Settings") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .font(.caption)
                }
            } else {
                Button("Grant Microphone Access") {
                    requestMicAccess()
                }
                .controlSize(.large)
            }
        }
        .padding(.horizontal, 32)
    }

    private var directoryStep: some View {
        VStack(spacing: 16) {
            Image(systemName: "folder.circle.fill")
                .font(.system(size: 56))
                .foregroundColor(.accentColor)

            Text("Choose Output Directory")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Where should BlackBox save your recordings?")
                .foregroundColor(.secondary)

            HStack {
                Text(outputDir)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundColor(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .cornerRadius(6)

                Button("Choose\u{2026}") {
                    chooseDirectory()
                }
            }
            .frame(maxWidth: 360)

            Text("Default: ~/Music/BlackBox Recordings/")
                .font(.caption)
                .foregroundColor(.secondary)
        }
        .padding(.horizontal, 32)
    }

    private var recordingModeStep: some View {
        VStack(spacing: 16) {
            Image(systemName: "recordingtape.circle.fill")
                .font(.system(size: 56))
                .foregroundColor(.accentColor)

            Text("Recording Mode")
                .font(.title2)
                .fontWeight(.semibold)

            Text("How should BlackBox handle long recordings?")
                .foregroundColor(.secondary)

            VStack(spacing: 12) {
                recordingModeOption(
                    title: "Continuous (Recommended)",
                    description: "Automatically saves and starts a new file every hour. No audio is lost if the app closes unexpectedly.",
                    isSelected: continuousMode
                ) {
                    continuousMode = true
                }

                recordingModeOption(
                    title: "Single File",
                    description: "Records everything into one file until you stop. Simpler, but you lose unsaved audio if the app quits.",
                    isSelected: !continuousMode
                ) {
                    continuousMode = false
                }
            }
            .frame(maxWidth: 380)
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
    }

    // MARK: - Actions

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
        panel.prompt = "Choose"
        panel.message = "Select output directory for recordings"
        panel.directoryURL = URL(fileURLWithPath: outputDir)

        if panel.runModal() == .OK, let url = panel.url {
            outputDir = url.path
            chosenURL = url
            return url
        }
        return nil
    }

    private func completeOnboarding() {
        // In a sandboxed app, we need a URL from NSOpenPanel for security scope.
        // If the user didn't explicitly choose, open the panel now.
        let url: URL
        if let chosen = chosenURL {
            url = chosen
        } else if let chosen = chooseDirectory() {
            url = chosen
        } else {
            return // User cancelled the panel — stay on this step
        }

        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        recorder.saveOutputDirBookmark(for: url)

        // Save recording mode choice
        let defaults = UserDefaults.standard
        defaults.set(continuousMode, forKey: SettingsKeys.continuousMode)
        if continuousMode {
            defaults.set(3600, forKey: SettingsKeys.recordingCadence) // 1 hour
        }
        recorder.bridge.setConfig([
            "continuous_mode": continuousMode,
            "recording_cadence": continuousMode ? 3600 : 300,
        ])

        hasCompletedOnboarding = true
        dismiss()
    }
}
