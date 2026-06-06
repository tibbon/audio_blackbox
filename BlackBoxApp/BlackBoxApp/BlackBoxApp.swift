import SwiftUI

/// Handles system-initiated termination (logout, restart, shutdown) by gracefully
/// finalizing any active recording before allowing the app to quit.
///
/// Also prevents SwiftUI from terminating the app when the last Window scene closes,
/// which is a known issue with MenuBarExtra + Window combinations.
///
/// Main-actor-isolated: NSWorkspace observers below pass `queue: .main`, so the
/// closures already deliver on the main thread. The annotation makes the
/// isolation explicit for Swift 6 strict concurrency.
@MainActor
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
            // DOLL-183: drain the recording directly here, not in the later
            // applicationShouldTerminate dispatch. macOS gives ~5s for
            // shutdown; if SwiftUI is slow to deliver
            // applicationShouldTerminate (other apps holding the run loop,
            // scene teardown), the recording can be killed before finalize
            // and the WAV header is left without correct RIFF/data sizes.
            // stop() is fast and the subsequent applicationShouldTerminate
            // will no-op on the already-stopped recorder.
            self?.explicitQuit = true
            if let recorder = self?.recorder, recorder.isRecording {
                recorder.stop()
            }
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

        // DOLL-185: re-check notification authorization when the app
        // becomes active. NSApplication.didBecomeActiveNotification
        // fires when the user clicks back into the app after granting
        // permission in System Settings, so the recorder picks up the
        // new state without a relaunch.
        NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.recorder?.refreshNotificationAuthorization()
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
    @Environment(\.openSettings) private var openSettings
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    // DOLL-211: surface the output directory in the dropdown so users
    // don't have to open Finder to see where recordings are going.
    @AppStorage(SettingsKeys.lastOutputDirPath) private var lastOutputDirPath: String = ""
    // DOLL-212: the bit depth feeds the pre-flight summary below the
    // Start Recording button so the user can verify the format before
    // committing.
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24
    @State private var didAutoOpenOnboarding = false
    // DOLL-208: one-shot nudge on the menu-bar icon after onboarding finishes.
    // Defaults to false on every launch, so it only fires on the false→true
    // transition that happens during a live onboarding completion — never on
    // cold-launch of an already-onboarded user.
    @State private var shouldShowDiscoveryNudge = false

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
                    bringOnboardingForward()
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
            menuBarLabel
                .accessibilityLabel(menuBarAccessibilityLabel)
                .background(StatusItemTooltip(tooltip: menuBarTooltip))
                .onChange(of: hasCompletedOnboarding) { oldValue, newValue in
                    guard !oldValue, newValue else { return }
                    shouldShowDiscoveryNudge = true
                    Task { @MainActor in
                        try? await Task.sleep(for: .seconds(5))
                        shouldShowDiscoveryNudge = false
                    }
                }
        }

        Window("Welcome to BlackBox", id: "onboarding") {
            OnboardingView(recorder: recorder)
        }
        // Sized for the tallest step (recordingModeStep): icon + title + subtitle
        // + two recording-mode cards + divider + silence toggle + button row.
        .defaultSize(width: 460, height: 540)
        .windowResizability(.contentSize)

        // DOLL-148: SwiftUI Settings scene rather than a generic Window.
        // This gives us the standard macOS Settings affordance — `⌘,`
        // reopen, system-managed close-on-`⌘W` semantics, and a standard
        // app-menu position — instead of a custom Window that only the
        // menu-bar Settings… button knew how to surface.
        Settings {
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
        // Menu-flicker fix v2: the live elapsed-time `Text(_, style: .timer)`
        // still caused menu reflow because the digit count changes at the
        // minute / hour boundaries — even `.monospacedDigit()` can't hide
        // "9:59" growing to "10:00" (4 → 5 glyphs). Each width change
        // re-laid out the dropdown and reset the user's hover selection.
        // The menu now shows a stable status string only; the live timer
        // moved to the meter window header where the window class
        // doesn't have the highlight-reset problem.
        Text(recorder.statusText)
            .font(.headline)
            .monospacedDigit()

        if recorder.isRecording {
            // DOLL-215: when the user hasn't picked a specific device,
            // show the resolved system default name (e.g. "MacBook Pro
            // Microphone") instead of the literal "System Default" so the
            // user knows what's actually recording.
            let device = selectedDevice.isEmpty
                ? (recorder.systemDefaultDeviceName ?? "System Default")
                : selectedDevice
            let chCount = countChannels(channelSpec)
            Text("\(device) \u{00B7} \(chCount) ch")
                .font(.caption)
                .foregroundStyle(.secondary)

            // DOLL-214: rotation countdown also moved to the meter window
            // header — same digit-width-flicker issue as the elapsed
            // time. The menu no longer hosts any per-second-ticking
            // text; live recording metrics live in the meter window.

            // DOLL-223: surface the running drop count when non-zero so
            // sub-warning drops (1\u{2013}500 samples) aren't invisible.
            // Bigger counts already trigger an errorMessage and auto-stop
            // via the existing engine-side thresholds.
            if recorder.writeErrorsCount > 0 {
                // DOLL-371: the default MenuBarExtra `.menu` style flattens
                // content to NSMenuItems and strips foreground colors, so the
                // orange tint alone wouldn't read as a warning. Use a Label
                // with a warning glyph (which DOES render in `.menu`) so the
                // severity survives without relying on color. The other warning
                // rows below already pair a glyph with their text.
                Label("\(recorder.writeErrorsCount) samples dropped", systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemOrange))
                    .accessibilityLabel("Warning: \(recorder.writeErrorsCount) samples dropped during this recording")
            }

            // DOLL-225: warn when recording on battery below 20 %, the
            // macOS-equivalent "low battery" threshold. A notification
            // also fires once on threshold crossing in case the user
            // doesn't have the menu open.
            if recorder.isLowBatteryWarning {
                Label("Battery low — plug in to avoid an unexpected stop",
                      systemImage: "battery.25percent")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemOrange))
                    .accessibilityLabel("Warning: battery low, plug in to avoid an unexpected stop")
            }

            // DOLL-220: pre-emptive 4 GiB cap warning. Set at recording
            // start; a notification also fires for menu-closed visibility.
            if let preflight = recorder.preflightSizeWarning {
                Label(preflight, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemOrange))
                    .lineLimit(3)
                    .accessibilityLabel("Warning: \(preflight)")
            }
        }

        // DOLL-213: transient "last recording" summary for ~30s after
        // Stop. Shown only while idle (a new recording would have
        // already cleared the snapshot). Show in Finder dismisses the
        // banner because the user has now acted on it.
        if !recorder.isRecording, let duration = recorder.lastRecordingDurationText {
            Text("Last recording: \(duration)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .monospacedDigit()
            Button("Show in Finder") {
                recorder.openOutputDir()
                recorder.dismissLastRecordingSummary()
            }
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
                    // DOLL-385: without this VoiceOver speaks the raw glyphs
                    // ("Start Recording command shift R"). Keep the label clean
                    // and expose the shortcut as a hint instead.
                    .accessibilityLabel(action)
                    .accessibilityHint("Keyboard shortcut \(shortcut.displayString)")
            } else {
                Text(action)
            }
        }

        // DOLL-212: pre-flight summary so the user can verify what's
        // about to be recorded before pressing Start. Hidden mid-record
        // because the active-recording caption above already covers it
        // (and changing settings while recording isn't a flow we want
        // to encourage here).
        if !recorder.isRecording {
            let device = selectedDevice.isEmpty
                ? (recorder.systemDefaultDeviceName ?? "System Default")
                : selectedDevice
            let chCount = countChannels(channelSpec)
            let chLabel = chCount == 1 ? "1 channel" : "\(chCount) channels"

            Text("Device: \(device)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Text("Format: \(bitDepth)-bit \u{00B7} \(chLabel)")
                .font(.caption)
                .foregroundStyle(.secondary)
            if !lastOutputDirPath.isEmpty {
                Text("Location: \(abbreviateHomePath(lastOutputDirPath))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }

        Button("Level Meter\u{2026}") {
            bringWindowForward(id: "meter", titleHint: "Level Meter")
        }

        Divider()

        if !recorder.availableDevices.isEmpty {
            Menu("Input Device") {
                // DOLL-215: surface the resolved device name so users know
                // what "System Default" maps to right now.
                let defaultLabel: String = recorder.systemDefaultDeviceName
                    .map { "System Default (\($0))" } ?? "System Default"
                Toggle(defaultLabel, isOn: Binding(
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

            // DOLL-387: the menu is the primary surface where the user notices
            // no devices; offer inline recovery after they plug hardware in,
            // rather than making them open Settings just to re-scan.
            Button("Refresh Devices") {
                recorder.refreshDevices()
            }

            Divider()
        }

        Button("Show in Finder") {
            recorder.openOutputDir()
        }
        // DOLL-211: caption under the action shows the abbreviated
        // destination path (the in-container default folder, or a user-chosen
        // one). Center-truncates because the menu is narrow and arbitrary
        // paths can be long.
        if !lastOutputDirPath.isEmpty {
            Text(abbreviateHomePath(lastOutputDirPath))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .accessibilityLabel("Recordings save to \(abbreviateHomePath(lastOutputDirPath))")
        }

        Divider()

        Button("About BlackBox\u{2026}") {
            bringWindowForward(id: "about", titleHint: "About BlackBox")
        }

        Button("Settings\u{2026}") {
            NSApp.activate(ignoringOtherApps: true)
            openSettings()
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
            bringOnboardingForward()
        }
    }

    /// Open the onboarding window, or bring it to the foreground if it
    /// is already open.
    @MainActor
    private func bringOnboardingForward() {
        bringWindowForward(id: "onboarding", titleHint: "Welcome to BlackBox")
    }

    /// Open the SwiftUI `Window(id:)` matching `id`, or — if the user
    /// previously opened it and Cmd-Tabbed away — bring the existing
    /// NSWindow to the front. SwiftUI's `openWindow(id:)` is a no-op
    /// when the window already exists, so naively calling it leaves the
    /// menu item looking dead.
    @MainActor
    private func bringWindowForward(id: String, titleHint: String) {
        // `activate(ignoringOtherApps:)` is deprecated on macOS 14+, but the
        // no-arg `activate()` does not reliably foreground a background
        // accessory app from a user-initiated menu click. Keep the
        // load-bearing form.
        NSApp.activate(ignoringOtherApps: true)

        // SwiftUI assigns the exact `id` string as the NSWindow identifier's
        // rawValue, so an `==` match is correct (and tighter than substring,
        // which would collide if any future window id were a substring of
        // another). titleHint is a fallback for environments where the
        // identifier isn't set yet.
        if let window = NSApp.windows.first(where: { window in
            window.identifier?.rawValue == id
                || window.title == titleHint
        }) {
            window.makeKeyAndOrderFront(nil)
        } else {
            openWindow(id: id)
        }
    }

    /// Whether the user has asked the system to reduce motion in
    /// `System Settings → Accessibility → Display`. Honoured for the
    /// recording-state pulse so motion-sensitive users see a static red icon.
    private var prefersReducedMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    /// Replace the user's home directory with "~" so paths display tightly
    /// in the menu. DOLL-211. Mirrors the abbreviation used in OnboardingView.
    private func abbreviateHomePath(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path.hasPrefix(home) { return "~" + path.dropFirst(home.count) }
        return path
    }

    @ViewBuilder
    private var menuBarLabel: some View {
        if !hasCompletedOnboarding {
            Image(systemName: "questionmark.circle")
        } else if recorder.errorMessage != nil {
            Image(systemName: "exclamationmark.circle")
        } else if recorder.isRecording {
            Image(systemName: "record.circle.fill")
                .foregroundStyle(.red)
                .symbolEffect(.pulse, options: .repeating, isActive: !prefersReducedMotion)
        } else {
            // DOLL-208: bounce the idle icon for ~3 hops when onboarding
            // completes, so the user notices where the app lives.
            Image(systemName: "record.circle")
                .symbolEffect(
                    .bounce,
                    options: .repeat(3),
                    isActive: shouldShowDiscoveryNudge && !prefersReducedMotion
                )
        }
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
            // Stop & Quit discards the in-progress recording — mark it
            // destructive (DOLL-254). Cancel stays first, so it remains the
            // default (Return) button.
            alert.buttons.last?.hasDestructiveAction = true

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

// AppURL is internal (no `private`) so AboutView.swift can use it after
// the DOLL-203 extraction. Anyone else needing app URLs imports it here.
enum AppURL {
    static let support = URL(string: "https://dollhousemediatech.com/blackbox/support")
    static let privacy = URL(string: "https://dollhousemediatech.com/blackbox/privacy")
    static let website = URL(string: "https://dollhousemediatech.com/blackbox/")
    static let releaseNotes = URL(string: "https://github.com/tibbon/audio_blackbox/commits/main/")
    static let license = URL(string: "https://github.com/tibbon/audio_blackbox/blob/main/LICENSE")
    static let acknowledgments = URL(string: "https://github.com/tibbon/audio_blackbox/blob/main/ACKNOWLEDGMENTS.md")
}

// AboutView moved to AboutView.swift (DOLL-203).

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

// AboutWindowConfigurator moved to AboutView.swift (DOLL-203).
