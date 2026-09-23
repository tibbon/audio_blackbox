import ServiceManagement
import SwiftUI

// MARK: - General Tab

struct GeneralSettingsTab: View {
    var recorder: RecordingState
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.launchAtLogin) private var launchAtLogin = false
    @AppStorage(SettingsKeys.autoRecord) private var autoRecord = false
    @AppStorage(SettingsKeys.sleepBehavior) private var sleepBehavior: String = "resume"
    @AppStorage(SettingsKeys.preventSleep) private var preventSleep: Bool = true
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @AppStorage(SettingsKeys.debugLogging) private var debugLogging = false
    @State private var shortcutLabel = String(localized: "None")
    @State private var isRecordingShortcut = false
    @State private var shortcutError: String?

    var body: some View {
        Form {
            startupSection
            sleepBehaviorSection
            globalShortcutSection
            diagnosticsSection
            setupSection
        }
        .formStyle(.grouped)
        .onAppear {
            launchAtLogin = SMAppService.mainApp.status == .enabled
            if let shortcut = GlobalHotkeyManager.shared.loadSaved() {
                shortcutLabel = shortcut.displayString
            }
            // DOLL-197: migrate stale sleepBehavior to a supported tag.
            if sleepBehavior != "resume" && sleepBehavior != "stop" {
                sleepBehavior = "resume"
            }
        }
    }

    private var startupSection: some View {
        Section("Startup") {
            Toggle("Launch at login", isOn: $launchAtLogin)
                .onChange(of: launchAtLogin) {
                    updateLoginItem()
                }
                .accessibilityHint("Start BlackBox when you log in")
            Toggle("Start recording on launch", isOn: $autoRecord)
                .accessibilityHint("Begin recording immediately when BlackBox starts")
            Text("When auto-record is enabled, recording begins with your saved settings.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var sleepBehaviorSection: some View {
        Section("Sleep Behavior") {
            Toggle("Prevent idle sleep while recording", isOn: $preventSleep)
                .accessibilityHint("Keep your Mac awake during recording")
            Text(
                """
                When enabled, your Mac won't sleep from inactivity while recording. \
                Lid close and manual sleep are unaffected.
                """
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            Picker("When Mac sleeps during recording:", selection: $sleepBehavior) {
                Text("Pause and resume on wake").tag("resume")
                Text("Stop recording").tag("stop")
            }
            .pickerStyle(.radioGroup)
            .accessibilityLabel("Sleep behavior")

            Text("Controls what happens if your Mac is forced to sleep (lid close, low battery, etc.).")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var globalShortcutSection: some View {
        Section("Global Shortcut") {
            HStack {
                shortcutRecorderRow

                if shortcutLabel != String(localized: "None") {
                    Button("Clear") {
                        clearShortcut()
                    }
                    .font(.caption)
                }
            }

            shortcutCaption
        }
    }

    // Combine just the label + recorder into one a11y element
    // so VoiceOver gets one announcement with label, value,
    // and hint. Clear stays as a sibling so VO can still
    // focus and activate it.
    private var shortcutRecorderRow: some View {
        HStack {
            Text("Toggle Recording:")
            Spacer()
            ShortcutRecorderButton(
                shortcutLabel: $shortcutLabel,
                isRecording: $isRecordingShortcut,
                error: $shortcutError
            )
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Global keyboard shortcut for toggling recording")
        .accessibilityValue(
            shortcutLabel == String(localized: "None")
                ? String(localized: "No shortcut set") : shortcutLabel
        )
        .accessibilityHint(
            isRecordingShortcut
                ? String(localized: "Press a key combination, or Escape to cancel")
                : String(localized: "Click to record a new shortcut")
        )
    }

    @ViewBuilder private var shortcutCaption: some View {
        if let shortcutError {
            Label {
                Text(shortcutError)
            } icon: {
                Image(systemName: "xmark.circle.fill")
                    .accessibilityHidden(true)
            }
            .font(.caption)
            .foregroundStyle(Color(nsColor: .systemRed))
        } else {
            Text("Works from any app. Click the button and press your desired key combination.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var diagnosticsSection: some View {
        Section("Diagnostics") {
            Toggle("Enable debug logging", isOn: $debugLogging)
                // Also fires for Reset All Settings, which turns it off.
                .onChange(of: debugLogging) { recorder.reloadDebugLogging() }
                .accessibilityHint("Log detailed info to macOS Console")
            Text("Logs are visible in Console.app. Filter by \"com.dollhousemediatech.blackbox\".")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var setupSection: some View {
        Section("Setup") {
            Button("Run Setup Again\u{2026}") {
                hasCompletedOnboarding = false
                NSApp.activate()
                openWindow(id: "onboarding")
            }
            .accessibilityHint("Re-run the initial setup wizard")
            // While setup is incomplete the menu bar menu shows only the
            // setup items, which would hide the Stop control of a live
            // recording; and setup changes the folder and mode that
            // recording is using.
            .disabled(recorder.isRecording)
            Group {
                if recorder.isRecording {
                    Text("Stop recording to run the setup wizard again.")
                } else {
                    Text("Re-run the setup wizard to change your output directory or recording mode.")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)

            Button("Reset All Settings\u{2026}") {
                confirmResetAllSettings()
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityHint("Restore all settings to their defaults")
        }
    }

    private func clearShortcut() {
        GlobalHotkeyManager.shared.unregister()
        GlobalHotkeyManager.shared.save(nil)
        shortcutLabel = String(localized: "None")
    }

    private func confirmResetAllSettings() {
        let alert = NSAlert()
        alert.messageText = String(localized: "Reset All Settings?")
        let recording = recorder.isRecording
        alert.informativeText =
            String(localized: "This will restore all settings to their defaults. Your recordings will not be affected.")
            + (recording ? String(localized: " The current recording will be stopped.") : "")
        alert.alertStyle = .warning
        // Affirmative-first to match every other dialog in the app (confirmSettingsChange,
        // quitApp). Reset is destructive; keep Cancel as the default (Return) button so an
        // accidental keypress never wipes settings.
        alert.addButton(withTitle: String(localized: "Reset"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.buttons.first?.keyEquivalent = ""
        alert.buttons.last?.keyEquivalent = "\r"
        NSApp.activate()
        if alert.runModal() == .alertFirstButtonReturn {
            resetAllSettings()
        }
    }

    private func resetAllSettings() {
        // Stop recording first — we're about to change the engine config
        if recorder.isRecording {
            recorder.stop()
        }

        let defaults = UserDefaults.standard
        // Reset all settings keys except onboarding completion and output dir bookmark
        let keysToReset = [
            SettingsKeys.inputDevice, SettingsKeys.audioChannels, SettingsKeys.outputMode,
            SettingsKeys.silenceEnabled, SettingsKeys.silenceThreshold,
            SettingsKeys.continuousMode, SettingsKeys.recordingCadence,
            SettingsKeys.launchAtLogin, SettingsKeys.autoRecord,
            SettingsKeys.minDiskSpaceMB, SettingsKeys.bitDepth,
            SettingsKeys.silenceGateEnabled, SettingsKeys.silenceGateTimeout,
            SettingsKeys.sleepBehavior, SettingsKeys.preventSleep,
            SettingsKeys.debugLogging,
        ]
        for key in keysToReset {
            defaults.removeObject(forKey: key)
        }
        // Clear global shortcut
        clearShortcut()
        // Update launch-at-login to match (now off)
        try? SMAppService.mainApp.unregister()
        // Refresh local state
        launchAtLogin = false
        autoRecord = false
        sleepBehavior = "resume"
        preventSleep = true
        debugLogging = false

        // Push default config to Rust engine so it takes effect immediately
        recorder.bridge.setConfig([
            "input_device": "",
            "audio_channels": "0",
            "output_mode": "split",
            "silence_threshold": 0.01,
            "continuous_mode": false,
            "recording_cadence": 300,
            "min_disk_space_mb": 500,
            "bits_per_sample": 24,
            "silence_gate_enabled": true,
            "silence_gate_timeout_secs": 300,
        ])
        // Keep selectDevice's record of the applied device in step with the
        // engine, or a later pick of the old device would be ignored.
        recorder.appliedInputDevice = ""
    }

    private func updateLoginItem() {
        do {
            if launchAtLogin {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
    }
}
