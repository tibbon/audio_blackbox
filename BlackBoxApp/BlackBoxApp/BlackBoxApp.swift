import SwiftUI

@main
struct BlackBoxApp: App {
    @StateObject private var recorder = RecordingState()

    var body: some Scene {
        MenuBarExtra("BlackBox", systemImage: menuBarIcon) {
            // Status line
            Text(recorder.statusText)
                .font(.headline)

            if let error = recorder.errorMessage {
                Text(error)
                    .foregroundColor(.red)
                    .font(.caption)
            }

            Divider()

            // Primary action
            Button(recorder.isRecording ? "Stop Recording" : "Start Recording") {
                recorder.toggle()
            }
            .keyboardShortcut("r")

            Divider()

            // Input device submenu
            if !recorder.availableDevices.isEmpty {
                Menu("Input Device") {
                    ForEach(recorder.availableDevices, id: \.self) { device in
                        Button(device) {
                            recorder.selectDevice(device)
                        }
                    }
                }

                Divider()
            }

            Button("Show Recordings in Finder") {
                recorder.openOutputDir()
            }

            Divider()

            preferencesButton

            Button("Quit") {
                if recorder.isRecording {
                    recorder.stop()
                }
                recorder.releaseOutputDirAccess()
                NSApplication.shared.terminate(nil)
            }
            .keyboardShortcut("q")
        }

        Settings {
            SettingsView(recorder: recorder)
        }
    }

    @ViewBuilder
    private var preferencesButton: some View {
        // SettingsLink doesn't work reliably inside MenuBarExtra menus,
        // so we use the sendAction approach on all macOS versions.
        Button("Preferences...") {
            // Activate the app first so the settings window comes to front
            NSApp.activate(ignoringOtherApps: true)
            // On macOS 14+, the selector is showSettingsWindow:
            // On macOS 13, it's showPreferencesWindow:
            if #available(macOS 14.0, *) {
                NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
            } else {
                NSApp.sendAction(Selector(("showPreferencesWindow:")), to: nil, from: nil)
            }
        }
        .keyboardShortcut(",")
    }

    private var menuBarIcon: String {
        if recorder.errorMessage != nil {
            return "exclamationmark.circle"
        }
        return recorder.isRecording ? "record.circle.fill" : "record.circle"
    }
}
