import AVFoundation
import SwiftUI

struct OnboardingView: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @Environment(\.dismiss) private var dismiss

    @State private var step = 0
    @State private var micGranted = false
    @State private var micDenied = false
    @State private var outputDir: String = ""

    private let defaultDir: URL = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Music")
        .appendingPathComponent("BlackBox Recordings")

    var body: some View {
        VStack(spacing: 0) {
            // Step indicator
            HStack(spacing: 8) {
                ForEach(0..<3) { i in
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
                        step = 2
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

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Choose"
        panel.message = "Select output directory for recordings"
        panel.directoryURL = URL(fileURLWithPath: outputDir)

        if panel.runModal() == .OK, let url = panel.url {
            outputDir = url.path
        }
    }

    private func completeOnboarding() {
        // Save the chosen output directory with security-scoped bookmark
        let url = URL(fileURLWithPath: outputDir)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        recorder.saveOutputDirBookmark(for: url)

        hasCompletedOnboarding = true
        dismiss()
    }
}
