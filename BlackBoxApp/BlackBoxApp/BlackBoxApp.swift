import SwiftUI

@main
struct BlackBoxApp: App {
    @StateObject private var recorder = RecordingState()
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""

    var body: some Scene {
        MenuBarExtra("BlackBox", systemImage: menuBarIcon) {
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
            .keyboardShortcut("r")

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

        Window("BlackBox Settings", id: "settings") {
            SettingsView(recorder: recorder)
        }
        .defaultSize(width: 480, height: 500)
        .windowResizability(.contentSize)
    }

    private var menuBarIcon: String {
        if recorder.errorMessage != nil {
            return "exclamationmark.circle"
        }
        return recorder.isRecording ? "record.circle.fill" : "record.circle"
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
