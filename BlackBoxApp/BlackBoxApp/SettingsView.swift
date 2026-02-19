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
    @State private var selectedDevice: String = ""
    @State private var channelSpec: String = "0"
    @State private var outputMode: String = "single"
    @State private var silenceEnabled: Bool = true
    @State private var silenceThreshold: Double = 0.01

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
        .onAppear(perform: loadConfig)
    }

    private func loadConfig() {
        guard let config = recorder.bridge.getConfig() else { return }
        selectedDevice = config["input_device"] as? String ?? ""
        channelSpec = config["audio_channels"] as? String ?? "0"
        outputMode = config["output_mode"] as? String ?? "single"
        let threshold = config["silence_threshold"] as? Double ?? 0.01
        silenceEnabled = threshold > 0
        silenceThreshold = threshold > 0 ? threshold : 0.01
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
    @State private var outputDir: String = "recordings"
    @State private var continuousMode: Bool = false
    @State private var recordingCadence: Int = 300

    var body: some View {
        Form {
            Section("Output Directory") {
                HStack {
                    TextField("Directory", text: $outputDir)
                        .onSubmit { applyConfig() }
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
        .onAppear(perform: loadConfig)
    }

    private func loadConfig() {
        guard let config = recorder.bridge.getConfig() else { return }
        outputDir = config["output_dir"] as? String ?? "recordings"
        continuousMode = config["continuous_mode"] as? Bool ?? false
        recordingCadence = (config["recording_cadence"] as? Int) ?? 300
    }

    private func applyConfig() {
        let config: [String: Any] = [
            "output_dir": outputDir,
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
    @AppStorage("launchAtLogin") private var launchAtLogin = false
    @AppStorage("autoRecord") private var autoRecord = false

    var body: some View {
        Form {
            Section("Startup") {
                Toggle("Launch at login", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _ in
                        updateLoginItem()
                    }
                Toggle("Start recording automatically on launch", isOn: $autoRecord)
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
            // Sync toggle state with actual system registration
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
            // Revert toggle on failure
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
    }
}
