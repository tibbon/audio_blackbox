import ServiceManagement
import SwiftUI

struct SettingsView: View {
    @ObservedObject var recorder: RecordingState

    var body: some View {
        TabView {
            RecordingSettingsTab(recorder: recorder)
                .tabItem {
                    Label("Recording", systemImage: "mic")
                }

            OutputSettingsTab(recorder: recorder)
                .tabItem {
                    Label("Output", systemImage: "folder")
                }

            GeneralSettingsTab()
                .tabItem {
                    Label("General", systemImage: "gear")
                }
        }
        .frame(minWidth: 480, maxWidth: 480, minHeight: 240)
    }
}

// MARK: - Recording Tab

struct RecordingSettingsTab: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "0"
    @AppStorage(SettingsKeys.silenceEnabled) private var silenceEnabled: Bool = true
    @AppStorage(SettingsKeys.silenceThreshold) private var silenceThreshold: Double = 0.01

    var body: some View {
        Form {
            Section("Input Device") {
                Picker("Input Device", selection: $selectedDevice) {
                    Text("System Default").tag("")
                    ForEach(recorder.availableDevices, id: \.self) { device in
                        Text(device).tag(device)
                    }
                }
                .labelsHidden()
                .onChange(of: selectedDevice) { _ in applyConfig() }
                .accessibilityLabel("Input device")
                .accessibilityHint("Select the audio input device for recording")

                Button {
                    recorder.refreshDevices()
                } label: {
                    Label("Refresh Devices", systemImage: "arrow.clockwise")
                        .font(.caption)
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Refresh device list")
                .accessibilityHint("Scan for newly connected audio devices")
            }

            Section("Channels") {
                TextField("e.g. 0, 0-3, 0,2-4,7", text: $channelSpec)
                    .onSubmit { applyConfig() }
                    .accessibilityLabel("Channel specification")
                    .accessibilityHint("Enter channel numbers or ranges separated by commas")
                Text("Supports individual channels and ranges")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Section("Silence Detection") {
                Toggle("Enable silence detection", isOn: $silenceEnabled)
                    .onChange(of: silenceEnabled) { _ in applyConfig() }
                    .accessibilityHint("When enabled, silent recordings are automatically deleted")

                if silenceEnabled {
                    VStack(alignment: .leading, spacing: 4) {
                        Slider(value: $silenceThreshold, in: 0.001...1.0) {
                            Text("Threshold")
                        }
                        .onChange(of: silenceThreshold) { _ in applyConfig() }
                        .accessibilityLabel("Silence threshold")
                        .accessibilityValue(thresholdDescription)

                        HStack {
                            Text("Sensitive")
                                .font(.caption)
                                .foregroundColor(.secondary)
                            Spacer()
                            Text(String(format: "%.3f", silenceThreshold))
                                .font(.caption)
                                .monospacedDigit()
                            Spacer()
                            Text("Aggressive")
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    private var thresholdDescription: String {
        if silenceThreshold < 0.005 {
            return "Very sensitive, value \(String(format: "%.3f", silenceThreshold))"
        } else if silenceThreshold < 0.05 {
            return "Sensitive, value \(String(format: "%.3f", silenceThreshold))"
        } else if silenceThreshold < 0.2 {
            return "Moderate, value \(String(format: "%.3f", silenceThreshold))"
        } else {
            return "Aggressive, value \(String(format: "%.3f", silenceThreshold))"
        }
    }

    private func applyConfig() {
        var config: [String: Any] = [
            "audio_channels": channelSpec,
            "silence_threshold": silenceEnabled ? silenceThreshold : 0.0,
        ]
        if !selectedDevice.isEmpty {
            config["input_device"] = selectedDevice
        }
        recorder.bridge.setConfig(config)
    }
}

// MARK: - Output Tab

struct OutputSettingsTab: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.outputMode) private var outputMode: String = "split"
    @AppStorage(SettingsKeys.continuousMode) private var continuousMode: Bool = false
    @AppStorage(SettingsKeys.recordingCadence) private var recordingCadence: Int = 300
    @State private var outputDir: String = "recordings"

    var body: some View {
        Form {
            Section("Output Directory") {
                HStack {
                    Text(outputDir)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundColor(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                        .accessibilityLabel("Output directory: \(outputDir)")
                    Button("Choose\u{2026}") {
                        chooseDirectory()
                    }
                    .accessibilityHint("Opens a file picker to select the output directory")
                }
                Button("Open in Finder") {
                    recorder.openOutputDir()
                }
                .font(.caption)
                .accessibilityHint("Opens the output directory in Finder")
            }

            Section("Output Mode") {
                Picker("Output Mode", selection: $outputMode) {
                    Text("Single File").tag("single")
                    Text("Split (one file per channel)").tag("split")
                }
                .labelsHidden()
                .pickerStyle(.radioGroup)
                .onChange(of: outputMode) { _ in applyConfig() }
                .accessibilityLabel("Output mode")
                .accessibilityHint("Choose whether to record to a single file or one file per channel")
            }

            Section("Continuous Recording") {
                Toggle("Enable continuous recording", isOn: $continuousMode)
                    .onChange(of: continuousMode) { _ in applyConfig() }
                    .accessibilityHint("When enabled, files are automatically rotated at the specified interval")

                if continuousMode {
                    HStack {
                        Text("Rotate every:")
                        TextField("seconds", value: $recordingCadence, format: .number)
                            .frame(width: 80)
                            .onSubmit { applyConfig() }
                            .accessibilityLabel("Rotation interval")
                            .accessibilityValue("\(recordingCadence) seconds, \(cadenceDescription)")
                        Text("seconds")
                        if recordingCadence > 0 {
                            Text("(\(cadenceDescription))")
                                .foregroundColor(.secondary)
                                .font(.caption)
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .onAppear(perform: loadOutputDir)
    }

    private var cadenceDescription: String {
        let hours = recordingCadence / 3600
        let minutes = (recordingCadence % 3600) / 60
        let seconds = recordingCadence % 60
        if hours > 0 && minutes == 0 && seconds == 0 {
            return hours == 1 ? "1 hour" : "\(hours) hours"
        } else if hours > 0 {
            return "\(hours)h \(minutes)m"
        } else if minutes > 0 && seconds == 0 {
            return minutes == 1 ? "1 minute" : "\(minutes) minutes"
        } else if minutes > 0 {
            return "\(minutes)m \(seconds)s"
        } else {
            return "\(seconds)s"
        }
    }

    private func loadOutputDir() {
        if let config = recorder.bridge.getConfig() {
            outputDir = config["output_dir"] as? String ?? "recordings"
        }
    }

    private func applyConfig() {
        let config: [String: Any] = [
            "output_mode": outputMode,
            "continuous_mode": continuousMode,
            "recording_cadence": recordingCadence,
        ]
        recorder.bridge.setConfig(config)
    }

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Choose"
        panel.message = "Select output directory for recordings"

        if panel.runModal() == .OK, let url = panel.url {
            outputDir = url.path
            recorder.saveOutputDirBookmark(for: url)
        }
    }
}

// MARK: - General Tab

struct GeneralSettingsTab: View {
    @AppStorage(SettingsKeys.launchAtLogin) private var launchAtLogin = false
    @AppStorage(SettingsKeys.autoRecord) private var autoRecord = false

    var body: some View {
        Form {
            Section("Startup") {
                Toggle("Launch at login", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _ in
                        updateLoginItem()
                    }
                    .accessibilityHint("Automatically start BlackBox when you log in to your Mac")
                Toggle("Start recording on launch", isOn: $autoRecord)
                    .accessibilityHint("Begin recording immediately when BlackBox starts")
                Text("When auto-record is enabled, recording begins with your saved settings.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Section("About") {
                HStack {
                    Text("BlackBox Audio Recorder")
                        .font(.headline)
                    Spacer()
                    Text("v1.0")
                        .foregroundColor(.secondary)
                }
                Text("Always-on audio recording for your Mac.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear {
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
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

// MARK: - Settings Keys

/// Centralized UserDefaults keys for all persisted settings.
enum SettingsKeys {
    static let inputDevice = "inputDevice"
    static let audioChannels = "audioChannels"
    static let outputMode = "outputMode"
    static let silenceEnabled = "silenceEnabled"
    static let silenceThreshold = "silenceThreshold"
    static let continuousMode = "continuousMode"
    static let recordingCadence = "recordingCadence"
    static let launchAtLogin = "launchAtLogin"
    static let autoRecord = "autoRecord"
}
