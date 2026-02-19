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

            Button("Quit") {
                if recorder.isRecording {
                    recorder.stop()
                }
                NSApplication.shared.terminate(nil)
            }
            .keyboardShortcut("q")
        }
    }

    private var menuBarIcon: String {
        if recorder.errorMessage != nil {
            return "exclamationmark.circle"
        }
        return recorder.isRecording ? "record.circle.fill" : "record.circle"
    }
}
