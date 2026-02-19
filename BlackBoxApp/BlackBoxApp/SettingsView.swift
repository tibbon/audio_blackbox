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
        .frame(width: 480, height: 360)
    }
}

// MARK: - Recording Tab

struct RecordingSettingsTab: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "0"
    @AppStorage(SettingsKeys.outputMode) private var outputMode: String = "single"
    @AppStorage(SettingsKeys.silenceEnabled) private var silenceEnabled: Bool = true
    @AppStorage(SettingsKeys.silenceThreshold) private var silenceThreshold: Double = 0.01

    var body: some View {
        Form {
            Section("Input Device") {
                Picker("Device", selection: $selectedDevice) {
                    Text("System Default").tag("")
                    ForEach(recorder.availableDevices, id: \.self) { device in
                        Text(device).tag(device)
                    }
                }
                .onChange(of: selectedDevice) { _ in applyConfig() }

                Button("Refresh Devices") {
                    recorder.refreshDevices()
                }
                .font(.caption)
            }

            Section("Channels") {
                TextField("Channel spec (e.g. 0, 0-3, 0,2-4,7)", text: $channelSpec)
                    .onSubmit { applyConfig() }
                Text("Supports individual channels and ranges")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Section("Output Mode") {
                Picker("Mode", selection: $outputMode) {
                    Text("Single File").tag("single")
                    Text("Split (one file per channel)").tag("split")
                }
                .pickerStyle(.radioGroup)
                .onChange(of: outputMode) { _ in applyConfig() }
            }

            Section("Silence Detection") {
                Toggle("Enable silence detection", isOn: $silenceEnabled)
                    .onChange(of: silenceEnabled) { _ in applyConfig() }

                if silenceEnabled {
                    HStack {
                        Text("Threshold:")
                        Slider(value: $silenceThreshold, in: 0.001...1.0)
                            .onChange(of: silenceThreshold) { _ in applyConfig() }
                        Text(String(format: "%.3f", silenceThreshold))
                            .monospacedDigit()
                            .frame(width: 50)
                    }
                }
            }
        }
        .padding()
    }

    private func applyConfig() {
        var config: [String: Any] = [
            "audio_channels": channelSpec,
            "output_mode": outputMode,
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
    @AppStorage(SettingsKeys.continuousMode) private var continuousMode: Bool = false
    @AppStorage(SettingsKeys.recordingCadence) private var recordingCadence: Int = 300
    @State private var outputDir: String = "recordings"

    var body: some View {
        Form {
            Section("Output Directory") {
                HStack {
                    TextField("Directory", text: $outputDir)
                        .disabled(true)
                    Button("Choose...") {
                        chooseDirectory()
                    }
                }
                Button("Open in Finder") {
                    recorder.openOutputDir()
                }
                .font(.caption)
            }

            Section("Continuous Recording") {
                Toggle("Enable continuous recording", isOn: $continuousMode)
                    .onChange(of: continuousMode) { _ in applyConfig() }

                if continuousMode {
                    HStack {
                        Text("Rotate every:")
                        TextField("", value: $recordingCadence, format: .number)
                            .frame(width: 80)
                            .onSubmit { applyConfig() }
                        Text("seconds")
                    }
                    Text("Files will be automatically rotated at this interval")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
        }
        .padding()
        .onAppear(perform: loadOutputDir)
    }

    private func loadOutputDir() {
        if let config = recorder.bridge.getConfig() {
            outputDir = config["output_dir"] as? String ?? "recordings"
        }
    }

    private func applyConfig() {
        let config: [String: Any] = [
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
                Toggle("Start recording automatically when launched", isOn: $autoRecord)
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
        .padding()
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
