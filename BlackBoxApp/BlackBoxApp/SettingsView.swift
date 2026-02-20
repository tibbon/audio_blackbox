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
                    Label("General", systemImage: "gearshape")
                }
        }
        .frame(minWidth: 480, maxWidth: 480, minHeight: 450)
        .background(SettingsWindowConfigurator())
    }
}

/// Disables minimize and zoom buttons on the Settings window per Apple HIG.
private struct SettingsWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            guard let window = nsView.window else { return }
            window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
            window.standardWindowButton(.zoomButton)?.isEnabled = false
        }
    }
}

// MARK: - Recording Tab

struct RecordingSettingsTab: View {
    @ObservedObject var recorder: RecordingState
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "0"
    @AppStorage(SettingsKeys.silenceEnabled) private var silenceEnabled: Bool = true
    @AppStorage(SettingsKeys.silenceThreshold) private var silenceThreshold: Double = 0.01
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24

    private var channelSpecError: String? {
        validateChannelSpec(channelSpec)
    }

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

                Button("Refresh Devices") {
                    recorder.refreshDevices()
                }
                .font(.caption)
                .accessibilityHint("Scan for newly connected audio devices")
            }

            Section("Channels") {
                TextField("e.g. 0, 0-3, 0,2-4,7", text: $channelSpec)
                    .onSubmit {
                        if channelSpecError == nil { applyConfig() }
                    }
                    .foregroundColor(channelSpecError != nil ? .red : .primary)
                    .accessibilityLabel("Channel specification")
                    .accessibilityHint("Enter channel numbers or ranges separated by commas")
                if let error = channelSpecError {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundColor(.red)
                } else {
                    Text("Supports individual channels and ranges")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }

            Section("Bit Depth") {
                Picker("Bit Depth", selection: $bitDepth) {
                    Text("16-bit").tag(16)
                    Text("24-bit (Recommended)").tag(24)
                    Text("32-bit").tag(32)
                }
                .labelsHidden()
                .pickerStyle(.radioGroup)
                .onChange(of: bitDepth) { _ in applyConfig() }
                .accessibilityLabel("Bit depth")
                .accessibilityHint("Select the bit depth for WAV recordings")
                Text("24-bit is the professional standard. 16-bit saves space. 32-bit provides maximum headroom.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Section("Silence Detection") {
                Toggle("Enable silence detection", isOn: $silenceEnabled)
                    .onChange(of: silenceEnabled) { _ in applyConfig() }
                    .accessibilityHint("When enabled, silent recordings are automatically deleted")

                if silenceEnabled {
                    VStack(alignment: .leading, spacing: 4) {
                        Slider(value: $silenceThreshold, in: 0.001...0.1) {
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

            Section("Monitoring") {
                Button("Open Level Meter\u{2026}") {
                    NSApp.activate(ignoringOtherApps: true)
                    openWindow(id: "meter")
                }
                .accessibilityHint("Opens a window showing real-time audio levels per channel")
                Text("View real-time audio input levels per channel during recording.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private var thresholdDescription: String {
        let value = String(format: "%.3f", silenceThreshold)
        if silenceThreshold < 0.005 {
            return "Very sensitive, \(value)"
        } else if silenceThreshold < 0.02 {
            return "Sensitive, \(value)"
        } else if silenceThreshold < 0.05 {
            return "Moderate, \(value)"
        } else {
            return "Aggressive, \(value)"
        }
    }

    private func applyConfig() {
        guard channelSpecError == nil else { return }
        var config: [String: Any] = [
            "audio_channels": channelSpec,
            "silence_threshold": silenceEnabled ? silenceThreshold : 0.0,
            "bits_per_sample": bitDepth,
        ]
        if !selectedDevice.isEmpty {
            config["input_device"] = selectedDevice
        }
        recorder.bridge.setConfig(config)
    }
}

/// Validate a channel spec string (e.g. "0", "0-3", "0,2-4,7").
/// Returns nil if valid, or an error message if invalid.
private func validateChannelSpec(_ spec: String) -> String? {
    let trimmed = spec.trimmingCharacters(in: .whitespaces)
    if trimmed.isEmpty {
        return "Channel specification cannot be empty"
    }
    let parts = trimmed.split(separator: ",")
    for part in parts {
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if bounds.count != 2 {
                return "Invalid range: \(token)"
            }
            guard let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
                  let end = Int(bounds[1].trimmingCharacters(in: .whitespaces)),
                  start >= 0, end >= 0, start <= end
            else {
                return "Invalid range: \(token)"
            }
        } else {
            guard let num = Int(token), num >= 0 else {
                return "Invalid channel number: \(token)"
            }
            if num > 63 {
                return "Channel \(num) exceeds maximum (63)"
            }
        }
    }
    return nil
}

// MARK: - Output Tab

struct OutputSettingsTab: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.outputMode) private var outputMode: String = "split"
    @AppStorage(SettingsKeys.continuousMode) private var continuousMode: Bool = false
    @AppStorage(SettingsKeys.recordingCadence) private var recordingCadence: Int = 300
    @AppStorage(SettingsKeys.minDiskSpaceMB) private var minDiskSpaceMB: Int = 500
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
                Button {
                    recorder.openOutputDir()
                } label: {
                    Label("Open in Finder", systemImage: "folder")
                        .font(.caption)
                }
                .buttonStyle(.borderless)
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
                Text("Single combines all channels into one file. Split creates a separate WAV file for each channel.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Section("Continuous Recording") {
                Toggle("Enable continuous recording", isOn: $continuousMode)
                    .onChange(of: continuousMode) { _ in applyConfig() }
                    .accessibilityHint("When enabled, files are automatically rotated at the specified interval")
                Text("Automatically saves and starts a new file at regular intervals, so no audio is lost if the app closes unexpectedly.")
                    .font(.caption)
                    .foregroundColor(.secondary)

                if continuousMode {
                    HStack {
                        Text("Rotate every:")
                        TextField("seconds", value: $recordingCadence, format: .number)
                            .frame(width: 80)
                            .onSubmit {
                                if recordingCadence < 1 { recordingCadence = 1 }
                                applyConfig()
                            }
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

            Section("Disk Space") {
                HStack {
                    Text("Minimum free space:")
                    TextField("MB", value: $minDiskSpaceMB, format: .number)
                        .frame(width: 80)
                        .onSubmit {
                            if minDiskSpaceMB < 0 { minDiskSpaceMB = 0 }
                            applyConfig()
                        }
                        .accessibilityLabel("Minimum free disk space")
                        .accessibilityValue("\(minDiskSpaceMB) megabytes")
                    Text("MB")
                }
                Text("Recording stops automatically when free disk space drops below this threshold. Set to 0 to disable.")
                    .font(.caption)
                    .foregroundColor(.secondary)
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
            "min_disk_space_mb": minDiskSpaceMB,
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
    @AppStorage("debugLogging") private var debugLogging = false

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

            Section("Diagnostics") {
                Toggle("Enable debug logging", isOn: $debugLogging)
                    .accessibilityHint("Log detailed status information to macOS Console")
                Text("Logs are visible in Console.app. Filter by \"com.dollhousemediatech.blackbox\".")
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
    static let minDiskSpaceMB = "minDiskSpaceMB"
    static let hasCompletedOnboarding = "hasCompletedOnboarding"
    static let bitDepth = "bitDepth"
}
