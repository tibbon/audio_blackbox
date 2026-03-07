import SwiftUI

/// Handles system-initiated termination (logout, restart, shutdown) by gracefully
/// finalizing any active recording before allowing the app to quit.
///
/// Also prevents SwiftUI from terminating the app when the last Window scene closes,
/// which is a known issue with MenuBarExtra + Window combinations.
class AppDelegate: NSObject, NSApplicationDelegate {
    weak var recorder: RecordingState?

    /// Set to true before calling NSApp.terminate() from Quit menu items.
    /// Prevents SwiftUI's spurious terminate-on-last-window-close from killing the app.
    var explicitQuit = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Ensure we start as an accessory app (menu bar only, no Dock icon).
        NSApp.setActivationPolicy(.accessory)

        // System shutdown/logout fires willPowerOff before applicationShouldTerminate.
        // Mark it as explicit so we cooperate with the system instead of blocking.
        let wsnc = NSWorkspace.shared.notificationCenter

        wsnc.addObserver(
            forName: NSWorkspace.willPowerOffNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.explicitQuit = true
        }

        wsnc.addObserver(
            forName: NSWorkspace.willSleepNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.recorder?.handleWillSleep()
        }

        wsnc.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.recorder?.handleDidWake()
        }

        wsnc.addObserver(
            forName: NSWorkspace.sessionDidResignActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.recorder?.handleSessionDidResignActive()
        }

        wsnc.addObserver(
            forName: NSWorkspace.sessionDidBecomeActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.recorder?.handleSessionDidBecomeActive()
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        // SwiftUI triggers NSApp.terminate() when the last Window scene closes.
        // Block that — we're a menu bar app and should stay alive.
        guard explicitQuit else {
            NSApp.setActivationPolicy(.accessory)
            return .terminateCancel
        }

        // Explicit quit (user or system) — finalize recordings gracefully.
        if let recorder, recorder.isRecording {
            recorder.stop()
        }
        recorder?.releaseOutputDirAccess()
        return .terminateNow
    }
}

@main
struct BlackBoxApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    @State private var recorder = RecordingState()
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
                Text("Setup Required")
                    .font(.headline)

                Text("Complete setup to start recording")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Divider()

                Button("Set Up BlackBox\u{2026}") {
                    NSApp.activate()
                    openWindow(id: "onboarding")
                }

                Divider()

                Button("Quit BlackBox") {
                    appDelegate.explicitQuit = true
                    NSApplication.shared.terminate(nil)
                }
                .keyboardShortcut("q")
            } else {
                normalMenu
            }
        } label: {
            Image(nsImage: menuBarNSImage)
                .accessibilityLabel(menuBarAccessibilityLabel)
                .background(StatusItemTooltip(tooltip: menuBarTooltip))
        }

        Window("Welcome to BlackBox", id: "onboarding") {
            OnboardingView(recorder: recorder)
        }
        .defaultSize(width: 460, height: 380)
        .windowResizability(.contentSize)

        Window("Settings", id: "settings") {
            SettingsView(recorder: recorder)
        }
        .defaultSize(width: 480, height: 500)
        .windowResizability(.contentSize)

        Window("Level Meter", id: "meter") {
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
        Text(recorder.statusText)
            .font(.headline)
            .monospacedDigit()

        if recorder.isRecording {
            let device = selectedDevice.isEmpty ? "System Default" : selectedDevice
            let chCount = countChannels(channelSpec)
            Text("\(device) \u{00B7} \(chCount) ch")
                .font(.caption)
                .foregroundStyle(.secondary)
        }

        if let error = recorder.errorMessage {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Color(nsColor: .systemRed))
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

        Button("Level Meter\u{2026}") {
            NSApp.activate()
            openWindow(id: "meter")
        }

        Divider()

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
                .accessibilityLabel("No input devices available")

            Divider()
        }

        Button("Show in Finder") {
            recorder.openOutputDir()
        }

        Divider()

        Button("About BlackBox\u{2026}") {
            NSApp.activate()
            openWindow(id: "about")
        }

        Button("Settings\u{2026}") {
            NSApp.activate()
            openWindow(id: "settings")
        }
        .keyboardShortcut(",")

        Divider()

        Menu("Help") {
            if let url = AppURL.support { Link("BlackBox Support", destination: url) }
            if let url = AppURL.privacy { Link("Privacy Policy", destination: url) }
        }

        Divider()

        Button("Quit BlackBox") {
            quitApp()
        }
        .keyboardShortcut("q")
    }

    /// Auto-open the onboarding window on first launch. Called as a side effect
    /// during body evaluation — uses Task to avoid modifying state during render.
    private func autoOpenOnboardingIfNeeded() {
        guard !hasCompletedOnboarding, !didAutoOpenOnboarding else { return }
        didAutoOpenOnboarding = true  // Set immediately to prevent duplicate Tasks
        Task {
            try? await Task.sleep(for: .milliseconds(300))
            NSApp.activate()
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

    private var menuBarTooltip: String {
        if !hasCompletedOnboarding {
            return "BlackBox — Setup required"
        }
        if recorder.isRecording {
            return "BlackBox — \(recorder.statusText)"
        }
        return "BlackBox"
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

            NSApp.activate()
            if alert.runModal() == .alertFirstButtonReturn {
                return
            }
            recorder.stop()
        }
        recorder.releaseOutputDirAccess()
        appDelegate.explicitQuit = true
        NSApplication.shared.terminate(nil)
    }
}

// MARK: - URLs

private enum AppURL {
    static let support = URL(string: "https://dollhousemediatech.com/blackbox/support")
    static let privacy = URL(string: "https://dollhousemediatech.com/blackbox/privacy")
    static let website = URL(string: "https://dollhousemediatech.com/blackbox/")
    static let releaseNotes = URL(string: "https://github.com/tibbon/audio_blackbox/commits/main/")
    static let license = URL(string: "https://github.com/tibbon/audio_blackbox/blob/main/LICENSE")
    static let acknowledgments = URL(string: "https://github.com/tibbon/audio_blackbox/blob/main/ACKNOWLEDGMENTS.md")
}

// MARK: - About View

struct AboutView: View {
    private let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0"
    private let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"
    private let copyright = Bundle.main.object(forInfoDictionaryKey: "NSHumanReadableCopyright") as? String
        ?? "\u{00A9} 2026 David Fisher"

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
                .foregroundStyle(.secondary)

            Text(copyright)
                .font(.caption)
                .foregroundStyle(.secondary)

            if let url = AppURL.website {
                Link("dollhousemediatech.com/blackbox", destination: url)
                    .font(.caption)
            }

            HStack(spacing: 12) {
                if let url = AppURL.privacy { Link("Privacy Policy", destination: url) }
                if let url = AppURL.releaseNotes { Link("Release Notes", destination: url) }
                if let url = AppURL.license { Link("License", destination: url) }
                if let url = AppURL.acknowledgments { Link("Acknowledgments", destination: url) }
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
        .padding(24)
        .frame(minWidth: 280)
        .background(AboutWindowConfigurator())
    }
}

/// Sets a tooltip on the menu bar status item by walking up the view hierarchy
/// to find the NSStatusBarButton parent.
private struct StatusItemTooltip: NSViewRepresentable {
    let tooltip: String

    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ nsView: NSView, context: Context) {
        Task { @MainActor in
            var view = nsView.superview
            while let v = view {
                if let button = v as? NSStatusBarButton {
                    button.toolTip = tooltip
                    return
                }
                view = v.superview
            }
        }
    }
}

/// Disables minimize and zoom buttons on the About window per Apple HIG.
/// Uses viewDidMoveToWindow to configure once, not on every SwiftUI render.
private struct AboutWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { AboutConfiguratorView() }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class AboutConfiguratorView: NSView {
    private var configured = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard !configured, let window else { return }
        configured = true
        window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
        window.standardWindowButton(.zoomButton)?.isEnabled = false
    }
}
