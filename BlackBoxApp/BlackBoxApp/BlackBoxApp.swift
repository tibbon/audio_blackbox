import SwiftUI

/// Handles system-initiated termination (logout, restart, shutdown) by gracefully
/// finalizing any active recording before allowing the app to quit.
class AppDelegate: NSObject, NSApplicationDelegate {
    weak var recorder: RecordingState?

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let recorder, recorder.isRecording else { return .terminateNow }
        // System is shutting down — gracefully finalize files without prompting.
        // The user already confirmed at the OS level (logout/restart/shutdown).
        recorder.stop()
        recorder.releaseOutputDirAccess()
        return .terminateNow
    }
}

@main
struct BlackBoxApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    @StateObject private var recorder = RecordingState()
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @State private var didAutoOpenOnboarding = false

    var body: some Scene {
        // Wire up delegate so applicationShouldTerminate can finalize recordings
        let _ = { appDelegate.recorder = recorder }()
        // Auto-open onboarding on first launch
        let _ = autoOpenOnboardingIfNeeded()
        MenuBarExtra {
            if !hasCompletedOnboarding {
                // Onboarding-required menu
                Text("Setup Required")
                    .font(.headline)

                Text("Complete setup to start recording")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Divider()

                Button("Set Up BlackBox\u{2026}") {
                    NSApp.activate(ignoringOtherApps: true)
                    openWindow(id: "onboarding")
                }

                Divider()

                Button("Quit") {
                    NSApplication.shared.terminate(nil)
                }
                .keyboardShortcut("q")
            } else {
                // Normal menu
                normalMenu
            }
        } label: {
            Image(nsImage: menuBarNSImage)
                .accessibilityLabel(menuBarAccessibilityLabel)
        }

        Window("Welcome to BlackBox", id: "onboarding") {
            OnboardingView(recorder: recorder)
        }
        .defaultSize(width: 460, height: 380)
        .windowResizability(.contentSize)

        Window("BlackBox Settings", id: "settings") {
            SettingsView(recorder: recorder)
        }
        .defaultSize(width: 480, height: 500)
        .windowResizability(.contentSize)

        Window("BlackBox Level Meter", id: "meter") {
            MeterView(recorder: recorder)
        }
        .defaultSize(width: 340, height: 200)
        .windowResizability(.contentSize)

        Window("About BlackBox", id: "about") {
            AboutView()
        }
        .defaultSize(width: 300, height: 200)
        .windowResizability(.contentSize)
    }

    @ViewBuilder
    private var normalMenu: some View {
        // Status line
        Text(recorder.statusText)
            .font(.headline)

        if recorder.isRecording {
            let device = selectedDevice.isEmpty ? "System Default" : selectedDevice
            let chCount = countChannels(channelSpec)
            Text("\(device) \u{00B7} \(chCount) ch")
                .font(.caption)
                .foregroundColor(.secondary)
        }

        if let error = recorder.errorMessage {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .foregroundColor(Color(nsColor: .systemRed))
                .font(.caption)
                .accessibilityLabel("Error: \(error)")
        }

        Divider()

        // Primary action — show the user's configured global shortcut if set
        Button {
            recorder.toggle()
        } label: {
            let action = recorder.isRecording ? "Stop Recording" : "Start Recording"
            if let shortcut = GlobalHotkeyManager.shared.currentShortcut {
                Text("\(action)  \(shortcut.displayString)")
            } else {
                Text(action)
            }
        }

        Divider()

        // Input device submenu with checkmarks
        if !recorder.availableDevices.isEmpty {
            Menu("Input Device") {
                Toggle("System Default", isOn: Binding(
                    get: { selectedDevice.isEmpty },
                    set: { newValue in
                        if newValue { recorder.selectDevice("") }
                    }
                ))

                Divider()

                ForEach(recorder.availableDevices, id: \.self) { device in
                    Toggle(device, isOn: Binding(
                        get: { selectedDevice == device },
                        set: { newValue in
                            if newValue { recorder.selectDevice(device) }
                        }
                    ))
                }
            }

            Divider()
        } else {
            Text("No Input Devices")

            Divider()
        }

        Button("Show in Finder") {
            recorder.openOutputDir()
        }

        Button("Level Meter\u{2026}") {
            NSApp.activate(ignoringOtherApps: true)
            openWindow(id: "meter")
        }

        Divider()

        Button("About BlackBox\u{2026}") {
            NSApp.activate(ignoringOtherApps: true)
            openWindow(id: "about")
        }

        Button("Settings\u{2026}") {
            NSApp.activate(ignoringOtherApps: true)
            openWindow(id: "settings")
        }
        .keyboardShortcut(",")

        Divider()

        Menu("Help") {
            Link("BlackBox Support", destination: URL(string: "https://dollhousemediatech.com/blackbox/support")!)
            Link("Privacy Policy", destination: URL(string: "https://dollhousemediatech.com/blackbox/privacy")!)
        }

        Divider()

        Button("Quit") {
            quitApp()
        }
        .keyboardShortcut("q")
    }

    /// Auto-open the onboarding window on first launch. Called as a side effect
    /// during body evaluation — uses asyncAfter to avoid modifying state during render.
    private func autoOpenOnboardingIfNeeded() {
        guard !hasCompletedOnboarding, !didAutoOpenOnboarding else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            didAutoOpenOnboarding = true
            NSApp.activate(ignoringOtherApps: true)
            openWindow(id: "onboarding")
        }
    }

    private var menuBarNSImage: NSImage {
        let name: String
        let description: String

        if !hasCompletedOnboarding {
            name = "questionmark.circle"
            description = "Setup Required"
        } else if recorder.errorMessage != nil {
            name = "exclamationmark.circle"
            description = "Error"
        } else if recorder.isRecording {
            name = "record.circle.fill"
            description = "Recording"
        } else {
            name = "record.circle"
            description = "BlackBox"
        }

        if recorder.isRecording {
            let config = NSImage.SymbolConfiguration(paletteColors: [.systemRed])
            if let image = NSImage(systemSymbolName: name, accessibilityDescription: description)?
                .withSymbolConfiguration(config) {
                image.isTemplate = false
                return image
            }
        }

        return NSImage(systemSymbolName: name, accessibilityDescription: description) ?? NSImage()
    }

    private var menuBarAccessibilityLabel: String {
        if !hasCompletedOnboarding {
            return "BlackBox: Setup required"
        }
        if let error = recorder.errorMessage {
            return "BlackBox: Error \u{2014} \(error)"
        }
        if recorder.isRecording {
            return "BlackBox: \(recorder.statusText)"
        }
        return "BlackBox: Ready"
    }

    private func quitApp() {
        if recorder.isRecording {
            let alert = NSAlert()
            alert.messageText = "Recording in Progress"
            alert.informativeText = "BlackBox is currently recording. Do you want to stop recording and quit?"
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Cancel")
            alert.addButton(withTitle: "Stop & Quit")

            NSApp.activate(ignoringOtherApps: true)
            if alert.runModal() == .alertFirstButtonReturn {
                return
            }
            recorder.stop()
        }
        recorder.releaseOutputDirAccess()
        NSApplication.shared.terminate(nil)
    }
}

// MARK: - About View

struct AboutView: View {
    private let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0"
    private let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"

    var body: some View {
        VStack(spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
                .accessibilityHidden(true)

            Text("BlackBox Audio Recorder")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Version \(version) (\(build))")
                .font(.caption)
                .foregroundColor(.secondary)

            Text("\u{00A9} 2026 David Fisher")
                .font(.caption)
                .foregroundColor(.secondary)

            Link("dollhousemediatech.com/blackbox", destination: URL(string: "https://dollhousemediatech.com/blackbox/")!)
                .font(.caption)

            HStack(spacing: 12) {
                Link("Release Notes", destination: URL(string: "https://dollhousemediatech.com/blackbox/support")!)
                Link("Licenses", destination: URL(string: "https://dollhousemediatech.com/blackbox/licenses")!)
            }
            .font(.caption2)
            .foregroundColor(.secondary)
        }
        .padding(24)
        .frame(minWidth: 280, maxWidth: 280)
        .background(AboutWindowConfigurator())
    }
}

/// Disables minimize and zoom buttons on the About window per Apple HIG.
private struct AboutWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            guard let window = nsView.window else { return }
            window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
            window.standardWindowButton(.zoomButton)?.isEnabled = false
        }
    }
}
