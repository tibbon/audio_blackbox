import Carbon
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
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"
    @AppStorage(SettingsKeys.silenceEnabled) private var silenceEnabled: Bool = true
    @AppStorage(SettingsKeys.silenceThreshold) private var silenceThreshold: Double = 0.01
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24
    @State private var deviceChannelCount: Int = 0
    @State private var selectedChannels: Set<Int> = [1]

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
                .onChange(of: selectedDevice) { _ in
                    refreshChannelCount()
                    applyConfig()
                }
                .accessibilityLabel("Input device")
                .accessibilityHint("Select the audio input device for recording")

                Button("Refresh Devices") {
                    recorder.refreshDevices()
                    refreshChannelCount()
                }
                .font(.caption)
                .accessibilityHint("Scan for newly connected audio devices")
            }

            Section("Channels") {
                if deviceChannelCount > 0 {
                    channelCheckboxes
                } else {
                    Text("Select an input device to see available channels.")
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
                Text("24-bit is the professional standard. 16-bit saves space. 32-bit float offers maximum precision with larger files.")
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
                            Text(thresholdPresetLabel)
                                .font(.caption)
                                .fontWeight(.medium)
                            Spacer()
                            Text("Aggressive")
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }

                    if abs(silenceThreshold - 0.01) > 0.001 {
                        Button("Reset to Default") {
                            silenceThreshold = 0.01
                            applyConfig()
                        }
                        .font(.caption)
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
        .onAppear {
            refreshChannelCount()
            syncCheckboxesFromChannelSpec()
        }
    }

    @ViewBuilder
    private var channelCheckboxes: some View {
        let columns = deviceChannelCount <= 8 ? 1 : 2
        let gridItems = Array(repeating: GridItem(.flexible(), alignment: .leading), count: columns)

        ScrollView {
            LazyVGrid(columns: gridItems, alignment: .leading, spacing: 4) {
                ForEach(1...deviceChannelCount, id: \.self) { ch in
                    Toggle(isOn: Binding(
                        get: { selectedChannels.contains(ch) },
                        set: { isOn in
                            if isOn {
                                selectedChannels.insert(ch)
                            } else if selectedChannels.count > 1 {
                                // Prevent deselecting the last channel
                                selectedChannels.remove(ch)
                            }
                            syncChannelSpecFromCheckboxes()
                        }
                    )) {
                        Text("Channel \(ch)")
                            .font(.body)
                    }
                    .toggleStyle(.checkbox)
                    .accessibilityLabel("Channel \(ch)")
                    .accessibilityHint("Include this channel in recordings")
                }
            }
        }
        .frame(maxHeight: deviceChannelCount > 8 ? 160 : CGFloat(deviceChannelCount * 24 + 8))

        HStack {
            Button("All") {
                selectedChannels = Set(1...deviceChannelCount)
                syncChannelSpecFromCheckboxes()
            }
            .font(.caption)
            Button("Reset") {
                selectedChannels = [1]  // Keep at least channel 1
                syncChannelSpecFromCheckboxes()
            }
            .font(.caption)
            Spacer()
            Text("\(selectedChannels.count) of \(deviceChannelCount) selected")
                .font(.caption)
                .foregroundColor(.secondary)
        }
    }

    /// Parse the channel spec string into the checkbox state.
    private func syncCheckboxesFromChannelSpec() {
        var channels = Set<Int>()
        for part in channelSpec.split(separator: ",") {
            let token = part.trimmingCharacters(in: .whitespaces)
            if token.contains("-") {
                let bounds = token.split(separator: "-")
                if bounds.count == 2,
                   let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
                   let end = Int(bounds[1].trimmingCharacters(in: .whitespaces)),
                   start >= 1, end >= start {
                    for ch in start...end { channels.insert(ch) }
                }
            } else if let num = Int(token), num >= 1 {
                channels.insert(num)
            }
        }
        if channels.isEmpty { channels = [1] }
        selectedChannels = channels
    }

    /// Write the checkbox state back to the channel spec string.
    private func syncChannelSpecFromCheckboxes() {
        let sorted = selectedChannels.sorted()
        channelSpec = sorted.map { String($0) }.joined(separator: ",")
        applyConfig()
    }

    /// Query the device for its channel count and refresh checkboxes.
    private func refreshChannelCount() {
        deviceChannelCount = RustBridge.getDeviceChannelCount(deviceName: selectedDevice) ?? 0
        // Clamp selected channels to what the device supports
        if deviceChannelCount > 0 {
            selectedChannels = selectedChannels.filter { $0 <= deviceChannelCount }
            if selectedChannels.isEmpty { selectedChannels = [1] }
            syncChannelSpecFromCheckboxes()
        }
    }

    private var thresholdPresetLabel: String {
        if silenceThreshold < 0.005 {
            return "Studio Quiet"
        } else if silenceThreshold < 0.02 {
            return "Home Office"
        } else if silenceThreshold < 0.05 {
            return "Moderate"
        } else {
            return "Noisy Environment"
        }
    }

    private var thresholdDescription: String {
        let value = String(format: "%.3f", silenceThreshold)
        return "\(thresholdPresetLabel), \(value)"
    }

    private func applyConfig() {
        var config: [String: Any] = [
            "audio_channels": channelSpecToZeroBased(channelSpec),
            "silence_threshold": silenceEnabled ? silenceThreshold : 0.0,
            "bits_per_sample": bitDepth,
        ]
        if !selectedDevice.isEmpty {
            config["input_device"] = selectedDevice
        }
        recorder.bridge.setConfig(config)
    }
}

/// Count the number of unique channels in a 1-based spec string (e.g. "1,3-5,8" → 5).
func countChannels(_ spec: String) -> Int {
    var channels = Set<Int>()
    for part in spec.split(separator: ",") {
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if bounds.count == 2,
               let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
               let end = Int(bounds[1].trimmingCharacters(in: .whitespaces)),
               start >= 1, end >= start {
                for ch in start...end { channels.insert(ch) }
            }
        } else if let num = Int(token), num >= 1 {
            channels.insert(num)
        }
    }
    return channels.count
}

/// Convert a 1-based channel spec string to 0-based for the Rust engine.
/// e.g. "1,3-5,8" → "0,2-4,7"
func channelSpecToZeroBased(_ spec: String) -> String {
    spec.split(separator: ",").map { part in
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if bounds.count == 2,
               let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
               let end = Int(bounds[1].trimmingCharacters(in: .whitespaces)) {
                return "\(start - 1)-\(end - 1)"
            }
            return token
        } else if let num = Int(token) {
            return "\(num - 1)"
        }
        return token
    }.joined(separator: ",")
}

/// Convert a 0-based channel spec string to 1-based for the UI.
/// e.g. "0,2-4,7" → "1,3-5,8"
func channelSpecToOneBased(_ spec: String) -> String {
    spec.split(separator: ",").map { part in
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if bounds.count == 2,
               let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
               let end = Int(bounds[1].trimmingCharacters(in: .whitespaces)) {
                return "\(start + 1)-\(end + 1)"
            }
            return token
        } else if let num = Int(token) {
            return "\(num + 1)"
        }
        return token
    }.joined(separator: ",")
}

/// Check if a channel spec uses legacy 0-based numbering (contains a "0" channel).
func isLegacyZeroBasedSpec(_ spec: String) -> Bool {
    for part in spec.split(separator: ",") {
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if let start = Int(bounds.first?.trimmingCharacters(in: .whitespaces) ?? ""),
               start == 0 {
                return true
            }
        } else if let num = Int(token), num == 0 {
            return true
        }
    }
    return false
}

// MARK: - Output Tab

struct OutputSettingsTab: View {
    @ObservedObject var recorder: RecordingState
    @AppStorage(SettingsKeys.outputMode) private var outputMode: String = "split"
    @AppStorage(SettingsKeys.continuousMode) private var continuousMode: Bool = false
    @AppStorage(SettingsKeys.recordingCadence) private var recordingCadence: Int = 300
    @AppStorage(SettingsKeys.minDiskSpaceMB) private var minDiskSpaceMB: Int = 500
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24
    @State private var outputDir: String = "recordings"
    @State private var cadenceSelection: Int = 300

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
                    Text("Split (one file per channel)").tag("split")
                    Text("Multichannel (single file)").tag("single")
                }
                .labelsHidden()
                .pickerStyle(.radioGroup)
                .onChange(of: outputMode) { _ in applyConfig() }
                .accessibilityLabel("Output mode")
                .accessibilityHint("Choose whether to record one file per channel or a combined multichannel file")
                if outputMode == "single" {
                    Text("Creates a single multichannel WAV file. Some DAWs may not import files with more than 2 channels correctly.")
                        .font(.caption)
                        .foregroundColor(.orange)
                } else {
                    Text("Creates a separate WAV file for each channel. Compatible with all DAWs.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }

            Section("Continuous Recording") {
                Toggle("Enable continuous recording", isOn: $continuousMode)
                    .onChange(of: continuousMode) { _ in applyConfig() }
                    .accessibilityHint("When enabled, files are automatically rotated at the specified interval")
                Text("Automatically saves and starts a new file at regular intervals, so no audio is lost if the app closes unexpectedly.")
                    .font(.caption)
                    .foregroundColor(.secondary)

                if continuousMode {
                    Picker("Rotate every:", selection: $cadenceSelection) {
                        Text("5 minutes").tag(300)
                        Text("15 minutes").tag(900)
                        Text("30 minutes").tag(1800)
                        Text("1 hour").tag(3600)
                        Text("2 hours").tag(7200)
                        Text("Custom").tag(-1)
                    }
                    .onChange(of: cadenceSelection) { newValue in
                        if newValue > 0 {
                            recordingCadence = newValue
                            applyConfig()
                        }
                    }
                    .accessibilityLabel("Rotation interval")

                    if cadenceSelection == -1 {
                        HStack {
                            TextField("", value: $recordingCadence, format: .number)
                                .textFieldStyle(.roundedBorder)
                                .frame(width: 80)
                                .onChange(of: recordingCadence) { newValue in
                                    if newValue < 1 { recordingCadence = 1 }
                                    applyConfig()
                                }
                                .accessibilityLabel("Custom rotation interval")
                                .accessibilityValue("\(recordingCadence) seconds")
                            Text("seconds")
                            if recordingCadence > 0 {
                                Text("(\(cadenceDescription))")
                                    .foregroundColor(.secondary)
                                    .font(.caption)
                            }
                        }
                    }

                    if let estimate = fileSizeEstimate {
                        Text("Estimated file size per chunk: \(estimate)")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }
            }

            Section("Disk Space") {
                Picker("Minimum free space:", selection: $minDiskSpaceMB) {
                    Text("Disabled").tag(0)
                    Text("500 MB").tag(500)
                    Text("1 GB").tag(1000)
                    Text("2 GB").tag(2000)
                    Text("5 GB").tag(5000)
                    Text("10 GB").tag(10000)
                }
                .onChange(of: minDiskSpaceMB) { _ in applyConfig() }
                .accessibilityLabel("Minimum free disk space")
                Text("Recording stops automatically when free disk space drops below this threshold.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadOutputDir()
            syncCadenceSelection()
        }
    }

    private var channelCount: Int {
        countChannels(channelSpec)
    }

    /// Estimated file size per rotation chunk. Uses actual sample rate from the
    /// audio device when available, falls back to 48 kHz.
    private var fileSizeEstimate: String? {
        let channels = channelCount
        guard channels > 0, recordingCadence > 0 else { return nil }
        let sampleRate = recorder.sampleRate > 0 ? recorder.sampleRate : 48000
        let bytesPerSample = bitDepth / 8
        let fileCount = outputMode == "split" ? channels : 1
        let channelsPerFile = outputMode == "split" ? 1 : channels
        let bytesPerFile = channelsPerFile * bytesPerSample * sampleRate * recordingCadence
        let totalBytes = bytesPerFile * fileCount
        let rateNote = recorder.sampleRate > 0 ? "" : " (assuming 48 kHz)"
        return formatBytes(bytesPerFile) + (fileCount > 1 ? " per file (\(formatBytes(totalBytes)) total across \(fileCount) files)" : "") + rateNote
    }

    private func formatBytes(_ bytes: Int) -> String {
        if bytes >= 1_073_741_824 {
            return String(format: "%.1f GB", Double(bytes) / 1_073_741_824)
        } else {
            return String(format: "%.0f MB", Double(bytes) / 1_048_576)
        }
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

    private static let cadencePresets: Set<Int> = [300, 900, 1800, 3600, 7200]

    private func syncCadenceSelection() {
        cadenceSelection = Self.cadencePresets.contains(recordingCadence) ? recordingCadence : -1
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
        panel.prompt = "Select"
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
    @State private var shortcutLabel: String = "None"
    @State private var isRecordingShortcut = false
    @State private var shortcutError: String?

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

            Section("Global Shortcut") {
                HStack {
                    Text("Toggle Recording:")
                    Spacer()
                    ShortcutRecorderButton(
                        shortcutLabel: $shortcutLabel,
                        isRecording: $isRecordingShortcut,
                        error: $shortcutError
                    )
                    if shortcutLabel != "None" {
                        Button("Clear") {
                            clearShortcut()
                        }
                        .font(.caption)
                    }
                }
                .accessibilityLabel("Global keyboard shortcut for toggling recording")

                if let shortcutError {
                    Text(shortcutError)
                        .font(.caption)
                        .foregroundColor(.red)
                } else {
                    Text("Works from any app. Click the button and press your desired key combination.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
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
            if let shortcut = GlobalHotkeyManager.shared.loadSaved() {
                shortcutLabel = shortcut.displayString
            }
        }
    }

    private func clearShortcut() {
        GlobalHotkeyManager.shared.unregister()
        GlobalHotkeyManager.shared.save(nil)
        shortcutLabel = "None"
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

// MARK: - Shortcut Recorder

/// A button that captures a keyboard shortcut when clicked.
struct ShortcutRecorderButton: NSViewRepresentable {
    @Binding var shortcutLabel: String
    @Binding var isRecording: Bool
    @Binding var error: String?

    func makeNSView(context: Context) -> ShortcutRecorderNSButton {
        let button = ShortcutRecorderNSButton()
        button.coordinator = context.coordinator
        button.title = shortcutLabel
        button.bezelStyle = .rounded
        button.setContentHuggingPriority(.required, for: .horizontal)
        return button
    }

    func updateNSView(_ nsView: ShortcutRecorderNSButton, context: Context) {
        nsView.title = isRecording ? "Press shortcut\u{2026}" : shortcutLabel
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    class Coordinator {
        let parent: ShortcutRecorderButton
        var localMonitor: Any?

        init(parent: ShortcutRecorderButton) {
            self.parent = parent
        }

        func startRecording() {
            parent.isRecording = true
            parent.error = nil
            localMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                self?.handleKeyEvent(event)
                return nil // Consume the event
            }
        }

        func stopRecording() {
            parent.isRecording = false
            if let monitor = localMonitor {
                NSEvent.removeMonitor(monitor)
                localMonitor = nil
            }
        }

        private static let reservedShortcuts: Set<String> = [
            "⌘Q", "⌘W", "⌘H", "⌘M", "⌘,", "⌘`",
            "⌘Z", "⌘X", "⌘C", "⌘V", "⌘A", "⌘S",
            "⌘Tab", "⌘Space",
        ]

        private func handleKeyEvent(_ event: NSEvent) {
            // Escape cancels recording
            if event.keyCode == UInt16(kVK_Escape) {
                stopRecording()
                return
            }

            // Require at least one modifier (Cmd, Ctrl, Opt)
            let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            let hasModifier = mods.contains(.command) || mods.contains(.control) || mods.contains(.option)
            guard hasModifier else { return }

            let carbonMods = GlobalHotkeyManager.carbonModifiers(from: UInt(mods.rawValue))
            let shortcut = GlobalHotkeyManager.Shortcut(
                keyCode: UInt32(event.keyCode),
                carbonModifiers: carbonMods
            )

            // Reject reserved system shortcuts
            if Self.reservedShortcuts.contains(shortcut.displayString) {
                parent.error = "\(shortcut.displayString) is reserved by macOS"
                stopRecording()
                return
            }

            // Register and save
            parent.error = nil
            let manager = GlobalHotkeyManager.shared
            manager.register(shortcut)
            manager.save(shortcut)

            parent.shortcutLabel = shortcut.displayString
            stopRecording()
        }
    }
}

/// Custom NSButton that becomes first responder to capture key events.
class ShortcutRecorderNSButton: NSButton {
    weak var coordinator: ShortcutRecorderButton.Coordinator?

    override var acceptsFirstResponder: Bool { true }

    override func mouseDown(with event: NSEvent) {
        if coordinator?.parent.isRecording == true {
            coordinator?.stopRecording()
        } else {
            coordinator?.startRecording()
            window?.makeFirstResponder(self)
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
