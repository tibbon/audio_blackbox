import SwiftUI

@main
struct BlackBoxApp: App {
    @StateObject private var recorder = RecordingState()
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false

    var body: some Scene {
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

        if let error = recorder.errorMessage {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .foregroundColor(.red)
                .font(.caption)
        }

        Divider()

        // Primary action
        Button(recorder.isRecording ? "Stop Recording" : "Start Recording") {
            recorder.toggle()
        }
        .keyboardShortcut("r", modifiers: [.command, .shift])

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
        }

        Button("Show Recordings in Finder") {
            recorder.openOutputDir()
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

        Button("Quit") {
            quitApp()
        }
        .keyboardShortcut("q")
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

    private func quitApp() {
        if recorder.isRecording {
            let alert = NSAlert()
            alert.messageText = "Recording in Progress"
            alert.informativeText = "BlackBox is currently recording. Do you want to stop recording and quit?"
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Stop & Quit")
            alert.addButton(withTitle: "Cancel")

            NSApp.activate(ignoringOtherApps: true)
            if alert.runModal() != .alertFirstButtonReturn {
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

            Text("BlackBox Audio Recorder")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Version \(version) (\(build))")
                .font(.caption)
                .foregroundColor(.secondary)

            Text("\u{00A9} 2026 David Fisher")
                .font(.caption)
                .foregroundColor(.secondary)

            Link("dollhousemediatech.com", destination: URL(string: "https://dollhousemediatech.com")!)
                .font(.caption)
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
