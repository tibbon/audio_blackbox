import Carbon
import ServiceManagement
import StoreKit
import SwiftUI

struct SettingsView: View {
    var recorder: RecordingState
    @Environment(\.requestReview) private var requestReview

    // DOLL-253: the window is pinned to content size (.windowResizability
    // (.contentSize) in BlackBoxApp), and the user can't widen it. A hard
    // 480pt width truncated/awkwardly-wrapped the captions and picker labels
    // at large Dynamic Type sizes. @ScaledMetric grows the pinned width with
    // the user's text size (≈480 at default, wider at accessibility sizes)
    // so content keeps its proportions instead of cramming into 480pt.
    @ScaledMetric(relativeTo: .body) private var contentWidth: CGFloat = 480

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

            GeneralSettingsTab(recorder: recorder)
                .tabItem {
                    Label("General", systemImage: "slider.horizontal.3")
                }
        }
        .frame(minWidth: contentWidth, maxWidth: contentWidth, minHeight: 450)
        .background(SettingsWindowConfigurator())
        .onAppear {
            promptForReviewIfReady()
        }
    }

    private func promptForReviewIfReady() {
        let sessions = UserDefaults.standard.integer(forKey: "successfulRecordingSessions")
        guard sessions >= 3 else { return }
        guard !UserDefaults.standard.bool(forKey: "hasPromptedForReview") else { return }
        UserDefaults.standard.set(true, forKey: "hasPromptedForReview")
        Task {
            try? await Task.sleep(for: .seconds(1))
            requestReview()
        }
    }
}

/// Disables minimize and zoom buttons on the Settings window per Apple HIG.
/// Uses viewDidMoveToWindow to configure once, not on every SwiftUI render.
private struct SettingsWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { SettingsConfiguratorView() }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class SettingsConfiguratorView: NSView {
    private var configured = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard !configured, let window else { return }
        configured = true
        window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
        window.standardWindowButton(.zoomButton)?.isEnabled = false
    }
}

// MARK: - Recording Tab

struct RecordingSettingsTab: View {
    var recorder: RecordingState
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"
    @AppStorage(SettingsKeys.silenceEnabled) private var silenceEnabled: Bool = true
    @AppStorage(SettingsKeys.silenceThreshold) private var silenceThreshold: Double = 0.01
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24
    @AppStorage(SettingsKeys.silenceGateEnabled) private var silenceGateEnabled: Bool = true
    @AppStorage(SettingsKeys.silenceGateTimeout) private var silenceGateTimeout: Int = 300
    @State private var deviceChannelCount: Int = 0
    @State private var selectedChannels: Set<Int> = [1]
    @State private var prevBitDepth: Int = 24
    @State private var prevChannelSpec: String = "1"
    /// Trailing debounce for the silence-threshold slider (DOLL-195).
    /// Slider's `onChange` fires per drag tick; without this, every
    /// drag dispatches a full setConfig() through the FFI dozens of
    /// times per second. Settle for 150 ms after the last change before
    /// pushing to Rust.
    @State private var thresholdDebounceTask: Task<Void, Never>?

    var body: some View {
        Form {
            Section("Input Device") {
                Picker("Input Device", selection: $selectedDevice) {
                    // DOLL-215: append the resolved default device name so
                    // the user can see what "System Default" maps to.
                    let defaultLabel: String = recorder.systemDefaultDeviceName
                        .map { "System Default (\($0))" } ?? "System Default"
                    Text(defaultLabel).tag("")
                    ForEach(recorder.availableDevices, id: \.self) { device in
                        Text(device).tag(device)
                    }
                }
                .labelsHidden()
                .onChange(of: selectedDevice) {
                    refreshChannelCount()
                    applyConfig()
                    recorder.selectDevice(selectedDevice)
                }
                .accessibilityLabel("Input device")
                .accessibilityHint("Select the audio input device")

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
                        .foregroundStyle(.secondary)
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
                .onChange(of: bitDepth) {
                    let old = prevBitDepth
                    guard bitDepth != old else { return }
                    prevBitDepth = bitDepth
                    guard recorder.isRecording else {
                        applyConfig()
                        return
                    }
                    confirmSettingsChange(reason: "bit depth") {
                        applyConfig()
                        recorder.restartIfRecording(reason: "bit depth changed")
                    } onCancel: {
                        prevBitDepth = old
                        bitDepth = old
                    }
                }
                .accessibilityLabel("Bit depth")
                .accessibilityHint("Precision of WAV recordings")
                Text("24-bit is the professional standard. 16-bit saves space. 32-bit float offers maximum precision with larger files.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Silence Detection") {
                Toggle("Enable silence detection", isOn: $silenceEnabled)
                    .onChange(of: silenceEnabled) { applyConfig() }
                    .accessibilityHint("Automatically delete silent recordings")
                Text("Recordings that contain only silence are automatically deleted when complete.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                if silenceEnabled {
                    VStack(alignment: .leading, spacing: 4) {
                        Slider(value: $silenceThreshold, in: 0.001...0.1, step: 0.005) {
                            Text("Threshold")
                        }
                        .onChange(of: silenceThreshold) {
                            // DOLL-195: debounce 150 ms before pushing
                            // to Rust. Cancels any pending settle on
                            // the next drag tick.
                            thresholdDebounceTask?.cancel()
                            thresholdDebounceTask = Task { @MainActor in
                                try? await Task.sleep(for: .milliseconds(150))
                                guard !Task.isCancelled else { return }
                                applyConfig()
                            }
                        }
                        .accessibilityLabel("Silence threshold")
                        .accessibilityValue(thresholdDescription)

                        HStack {
                            Text("Sensitive")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(thresholdPresetLabel)
                                .font(.caption)
                                .fontWeight(.medium)
                            Spacer()
                            Text("Aggressive")
                                .font(.caption)
                                .foregroundStyle(.secondary)
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

                // DOLL-224: "Pause recording during silence" implied the
                // whole recording stops; the silence gate actually finalizes
                // the current file and opens a new one on the next signal.
                // "Auto-split on silence" describes the real behavior.
                Toggle("Auto-split on silence", isOn: $silenceGateEnabled)
                    .onChange(of: silenceGateEnabled) { applyConfig() }
                    .accessibilityHint("Finalize the current file when audio goes silent and start a new one on the next signal")

                if silenceGateEnabled {
                    Picker("Resume after:", selection: $silenceGateTimeout) {
                        Text("1 minute").tag(60)
                        Text("2 minutes").tag(120)
                        Text("5 minutes").tag(300)
                        Text("10 minutes").tag(600)
                        Text("30 minutes").tag(1800)
                    }
                    .onChange(of: silenceGateTimeout) { applyConfig() }
                    .accessibilityLabel("Silence gate timeout")

                    Text("When enabled, BlackBox waits for audio before creating files, and finalizes them after the selected silence duration. Saves disk space during long idle periods.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Section("Monitoring") {
                Button("Open Level Meter\u{2026}") {
                    NSApp.activate()
                    openWindow(id: "meter")
                }
                .accessibilityHint("Opens real-time audio level meter")
                Text("View real-time audio input levels per channel during recording.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear {
            syncCheckboxesFromChannelSpec()  // Load saved spec FIRST
            refreshChannelCount()            // Then clamp to device capabilities
            migrateStalePickerValues()       // DOLL-197
            prevBitDepth = bitDepth
            prevChannelSpec = channelSpec
        }
    }

    /// DOLL-197: migrate stale UserDefaults values for this tab's
    /// pickers. See OutputSettingsTab.migrateStalePickerValues for
    /// rationale.
    private func migrateStalePickerValues() {
        let bitDepthPresets: Set<Int> = [16, 24, 32]
        if !bitDepthPresets.contains(bitDepth) {
            bitDepth = 24
        }

        let silenceGateTimeoutPresets: Set<Int> = [60, 120, 300, 600, 1800]
        if !silenceGateTimeoutPresets.contains(silenceGateTimeout) {
            let original = silenceGateTimeout
            silenceGateTimeout = silenceGateTimeoutPresets.min(by: {
                abs($0 - original) < abs($1 - original)
            }) ?? 300
        }
    }

    // DOLL-221: row height scales with Dynamic Type so the grid stays
    // legible at larger text sizes (the old fixed 24pt cramped rows
    // overlapped at .accessibilityExtraLarge+).
    @ScaledMetric(relativeTo: .body) private var channelRowHeight: CGFloat = 22

    /// Number of grid columns to use for the current device. Stays at 1
    /// for small devices (looks like a list), and scales up for high-
    /// channel-count interfaces (32 / 64 ch aggregate devices) so all
    /// channels are reachable without scrolling forever (DOLL-221).
    private var channelGridColumns: Int {
        switch deviceChannelCount {
        case ...8: return 1
        case 9...24: return 2
        case 25...48: return 3
        default: return 4
        }
    }

    /// Vertical budget for the channel grid. Caps at ~280pt for 32+
    /// channel devices so the Settings window doesn't grow unreasonably,
    /// but lets small devices show every channel without scrolling.
    private var channelGridHeight: CGFloat {
        let rowCount = Int(ceil(Double(deviceChannelCount) / Double(channelGridColumns)))
        let exact = CGFloat(rowCount) * channelRowHeight + 8
        let cap: CGFloat = deviceChannelCount >= 32 ? 280 : 200
        return min(exact, cap)
    }

    @ViewBuilder
    private var channelCheckboxes: some View {
        let gridItems = Array(
            repeating: GridItem(.flexible(), alignment: .leading),
            count: channelGridColumns
        )

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
                        Text("Ch \(ch)")
                            .font(.body)
                            .monospacedDigit()
                    }
                    .toggleStyle(.checkbox)
                    .accessibilityLabel("Channel \(ch)")
                    .accessibilityHint("Include this channel in recordings")
                }
            }
        }
        .frame(maxHeight: channelGridHeight)

        // DOLL-221: range/list text field so users on 32+ channel
        // aggregate devices don't have to click checkboxes one at a time.
        // Parser already understood "1-8, 16, 24-32" — we just exposed
        // it. The text reflects the *current* spec for transparency.
        HStack(spacing: 8) {
            Text("Range:")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField("e.g. 1-8, 16, 24-32", text: $channelSpec)
                .textFieldStyle(.roundedBorder)
                .font(.caption)
                .monospacedDigit()
                .onSubmit { commitChannelSpecText() }
                .accessibilityLabel("Channel range")
                .accessibilityHint("Enter channels as a comma-separated list with optional dash ranges, like 1-8, 16, 24-32")
        }

        HStack {
            Button("All") {
                selectedChannels = Set(1...deviceChannelCount)
                syncChannelSpecFromCheckboxes()
            }
            .font(.caption)
            Button("Reset") {
                selectedChannels = [1]
                syncChannelSpecFromCheckboxes()
            }
            .font(.caption)
            Spacer()
            Text("\(selectedChannels.count) of \(deviceChannelCount) selected")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    /// Commit a free-form channel-spec edit: re-parse, clamp to what the
    /// device supports, and push the canonicalised string back to
    /// @AppStorage so the grid + persisted spec stay in sync.
    private func commitChannelSpecText() {
        // Re-derive the canonical Set from whatever the user typed; this
        // also drops out-of-range and malformed entries silently.
        syncCheckboxesFromChannelSpec()
        selectedChannels = selectedChannels.filter { $0 >= 1 && $0 <= deviceChannelCount }
        if selectedChannels.isEmpty { selectedChannels = [1] }
        syncChannelSpecFromCheckboxes()
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
        let newSpec = sorted.map { String($0) }.joined(separator: ",")
        let old = prevChannelSpec
        guard newSpec != old else { return }
        channelSpec = newSpec
        prevChannelSpec = newSpec
        guard recorder.isRecording else {
            applyConfig()
            recorder.restartMonitoring()
            return
        }
        confirmSettingsChange(reason: "channels") {
            applyConfig()
            recorder.restartIfRecording(reason: "channels changed")
        } onCancel: {
            prevChannelSpec = old
            channelSpec = old
            syncCheckboxesFromChannelSpec()
        }
    }

    /// Query the device for its channel count and refresh checkboxes.
    /// Clamps selected channels to what the device supports and applies config
    /// directly (no confirmation dialog — device-initiated, not user-initiated).
    private func refreshChannelCount() {
        // DOLL-125: getDeviceChannelCount returns Result so audioDevice
        // (-2) and invalidArg (-8) are distinguishable. Treat both as
        // "unknown channel count = 0" for UI purposes — same observable
        // behavior as before — but the failure mode is now logged
        // instead of silently nil'd.
        switch RustBridge.getDeviceChannelCount(deviceName: selectedDevice) {
        case .success(let n):
            deviceChannelCount = n
        case .failure(let err):
            print("getDeviceChannelCount failed for \(selectedDevice): \(err)")
            deviceChannelCount = 0
        }
        if deviceChannelCount > 0 {
            selectedChannels = selectedChannels.filter { $0 <= deviceChannelCount }
            if selectedChannels.isEmpty { selectedChannels = [1] }
            let sorted = selectedChannels.sorted()
            let newSpec = sorted.map { String($0) }.joined(separator: ",")
            channelSpec = newSpec
            prevChannelSpec = newSpec
            applyConfig()
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
            "silence_gate_enabled": silenceGateEnabled,
            "silence_gate_timeout_secs": silenceGateTimeout,
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

/// Show a confirmation dialog before changing settings during an active recording.
/// Calls `onRestart` if the user confirms, or `onCancel` if they dismiss.
private func confirmSettingsChange(
    reason: String,
    onRestart: () -> Void,
    onCancel: (() -> Void)? = nil
) {
    let alert = NSAlert()
    alert.messageText = "Restart Recording?"
    alert.informativeText = "Changing \(reason) will finalize the current file and start a new one."
    alert.alertStyle = .informational
    alert.addButton(withTitle: "Restart")
    alert.addButton(withTitle: "Cancel")
    NSApp.activate()
    if alert.runModal() == .alertFirstButtonReturn {
        onRestart()
    } else {
        onCancel?()
    }
}

// MARK: - Output Tab

struct OutputSettingsTab: View {
    var recorder: RecordingState
    @AppStorage(SettingsKeys.outputMode) private var outputMode: String = "split"
    @AppStorage(SettingsKeys.continuousMode) private var continuousMode: Bool = false
    @AppStorage(SettingsKeys.recordingCadence) private var recordingCadence: Int = 300
    @AppStorage(SettingsKeys.minDiskSpaceMB) private var minDiskSpaceMB: Int = 500
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24
    @State private var outputDir: String = "recordings"
    @State private var cadenceSelection: Int = 300
    @State private var prevOutputMode: String = "split"
    /// Tracks focus on the custom-cadence TextField so DOLL-196 can
    /// clamp + applyConfig only when the user commits (focus loss /
    /// Return), not on every keystroke. The old onChange-clamp made a
    /// user typing "60" briefly see the field jump as intermediate
    /// values were clipped to the lower bound.
    @FocusState private var customCadenceFocused: Bool

    var body: some View {
        Form {
            Section("Output Directory") {
                HStack {
                    Text(displayPath)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
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
                .onChange(of: outputMode) {
                    let old = prevOutputMode
                    guard outputMode != old else { return }
                    prevOutputMode = outputMode
                    guard recorder.isRecording else {
                        applyConfig()
                        return
                    }
                    confirmSettingsChange(reason: "output mode") {
                        applyConfig()
                        recorder.restartIfRecording(reason: "output mode changed")
                    } onCancel: {
                        prevOutputMode = old
                        outputMode = old
                    }
                }
                .accessibilityLabel("Output mode")
                .accessibilityHint("One file per channel or one multichannel file")
                if outputMode == "single" {
                    Label {
                        Text("Creates a single multichannel WAV file. Some DAWs may not import files with more than 2 channels correctly.")
                    } icon: {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .accessibilityHidden(true)
                    }
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemOrange))
                } else {
                    Text("Creates a separate WAV file for each channel. Compatible with all DAWs.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Section("Continuous Recording") {
                Toggle("Enable continuous recording", isOn: $continuousMode)
                    .onChange(of: continuousMode) { applyConfig() }
                    .accessibilityHint("Automatically rotate files at regular intervals")
                Text("Automatically saves and starts a new file at regular intervals, so no audio is lost if the app closes unexpectedly.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                if continuousMode {
                    Picker("Rotate every:", selection: $cadenceSelection) {
                        // DOLL-222: short presets at the top so the user
                        // can verify that rotation is actually happening
                        // (5 min is too long to wait for a smoke test).
                        // The duration itself signals "for testing" — no
                        // real user would pick 30 s for production audio.
                        Text("30 seconds").tag(30)
                        Text("1 minute").tag(60)
                        Text("5 minutes").tag(300)
                        Text("15 minutes").tag(900)
                        Text("30 minutes").tag(1800)
                        Text("1 hour").tag(3600)
                        Text("2 hours").tag(7200)
                        Text("Custom").tag(-1)
                    }
                    .onChange(of: cadenceSelection) {
                        if cadenceSelection > 0 {
                            recordingCadence = cadenceSelection
                            applyConfig()
                        }
                    }
                    .accessibilityLabel("Rotation interval")

                    if cadenceSelection == -1 {
                        HStack {
                            TextField("", value: $recordingCadence, format: .number)
                                .textFieldStyle(.roundedBorder)
                                .frame(width: 80)
                                .focused($customCadenceFocused)
                                // DOLL-196: clamp + applyConfig on focus
                                // loss / Return, not on every keystroke.
                                // The old onChange-clamp made the field
                                // jump to 1 mid-typing.
                                .onSubmit { commitCustomCadence() }
                                .onChange(of: customCadenceFocused) { _, focused in
                                    if !focused { commitCustomCadence() }
                                }
                                .accessibilityLabel("Custom rotation interval")
                                .accessibilityValue("\(recordingCadence) seconds")
                            Text("seconds")
                            if recordingCadence >= 86400 {
                                Text("Maximum: 24 hours")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            } else if recordingCadence > 0 {
                                Text("(\(cadenceDescription))")
                                    .foregroundStyle(.secondary)
                                    .font(.caption)
                            }
                        }
                    }

                    if let estimate = fileSizeEstimate {
                        Text("Estimated file size per chunk: \(estimate)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
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
                .onChange(of: minDiskSpaceMB) { applyConfig() }
                .accessibilityLabel("Minimum free disk space")
                Text("Recording stops automatically when free disk space drops below this threshold.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear {
            loadOutputDir()
            syncCadenceSelection()
            migrateStalePickerValues()
            prevOutputMode = outputMode
        }
    }

    /// DOLL-197: migrate stale UserDefaults values to the nearest
    /// supported preset for this tab's pickers. Without this, a Picker
    /// reading a stale value (cross-version, hand-edited, old preset
    /// removed) silently selects nothing and writes the first tag on
    /// next interaction — user's preference lost without a signal.
    /// cadenceSelection already handles this via "Custom"; the others
    /// need a one-shot migration.
    private func migrateStalePickerValues() {
        let minDiskSpacePresets: Set<Int> = [0, 500, 1000, 2000, 5000, 10000]
        if !minDiskSpacePresets.contains(minDiskSpaceMB) {
            let original = minDiskSpaceMB
            // Snap to the nearest preset.
            minDiskSpaceMB = minDiskSpacePresets.min(by: { abs($0 - original) < abs($1 - original) })
                ?? 500
        }

        let outputModePresets: Set<String> = ["single", "split"]
        if !outputModePresets.contains(outputMode) {
            outputMode = "split"
        }
    }

    /// Abbreviate home directory paths with ~ for readability.
    private var displayPath: String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if outputDir.hasPrefix(home) {
            return "~" + outputDir.dropFirst(home.count)
        }
        return outputDir
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

    /// Commit the custom-cadence TextField on focus loss / Return.
    /// DOLL-196: validation happens once here instead of on every
    /// keystroke; a user typing "60" no longer sees the field jump to
    /// 1 mid-typing.
    private func commitCustomCadence() {
        if recordingCadence < 1 {
            recordingCadence = 1
        } else if recordingCadence > 86400 {
            recordingCadence = 86400
        }
        applyConfig()
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
    var recorder: RecordingState
    @Environment(\.openWindow) private var openWindow
    @AppStorage(SettingsKeys.launchAtLogin) private var launchAtLogin = false
    @AppStorage(SettingsKeys.autoRecord) private var autoRecord = false
    @AppStorage(SettingsKeys.sleepBehavior) private var sleepBehavior: String = "resume"
    @AppStorage(SettingsKeys.preventSleep) private var preventSleep: Bool = true
    @AppStorage(SettingsKeys.hasCompletedOnboarding) private var hasCompletedOnboarding = false
    @AppStorage("debugLogging") private var debugLogging = false
    @State private var shortcutLabel: String = "None"
    @State private var isRecordingShortcut = false
    @State private var shortcutError: String?

    var body: some View {
        Form {
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

            Section("Sleep Behavior") {
                Toggle("Prevent idle sleep while recording", isOn: $preventSleep)
                    .accessibilityHint("Keep your Mac awake during recording")
                Text("When enabled, your Mac won't sleep from inactivity while recording. Lid close and manual sleep are unaffected.")
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

            Section("Global Shortcut") {
                HStack {
                    // Combine just the label + recorder into one a11y element
                    // so VoiceOver gets one announcement with label, value,
                    // and hint. Clear stays as a sibling so VO can still
                    // focus and activate it.
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
                    .accessibilityValue(shortcutLabel == "None" ? "No shortcut set" : shortcutLabel)
                    .accessibilityHint(isRecordingShortcut
                        ? "Press a key combination, or Escape to cancel"
                        : "Click to record a new shortcut")

                    if shortcutLabel != "None" {
                        Button("Clear") {
                            clearShortcut()
                        }
                        .font(.caption)
                    }
                }

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

            Section("Diagnostics") {
                Toggle("Enable debug logging", isOn: $debugLogging)
                    .accessibilityHint("Log detailed info to macOS Console")
                Text("Logs are visible in Console.app. Filter by \"com.dollhousemediatech.blackbox\".")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Setup") {
                Button("Run Setup Again\u{2026}") {
                    hasCompletedOnboarding = false
                    NSApp.activate()
                    openWindow(id: "onboarding")
                }
                .accessibilityHint("Re-run the initial setup wizard")
                Text("Re-run the setup wizard to change your output directory or recording mode.")
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

    private func clearShortcut() {
        GlobalHotkeyManager.shared.unregister()
        GlobalHotkeyManager.shared.save(nil)
        shortcutLabel = "None"
    }

    private func confirmResetAllSettings() {
        let alert = NSAlert()
        alert.messageText = "Reset All Settings?"
        let recording = recorder.isRecording
        alert.informativeText = "This will restore all settings to their defaults. Your recordings will not be affected."
            + (recording ? " The current recording will be stopped." : "")
        alert.alertStyle = .warning
        // Affirmative-first to match every other dialog in the app (confirmSettingsChange,
        // quitApp). Reset is destructive; keep Cancel as the default (Return) button so an
        // accidental keypress never wipes settings.
        alert.addButton(withTitle: "Reset")
        alert.addButton(withTitle: "Cancel")
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
            "debugLogging",
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

    @MainActor
    class Coordinator {
        let parent: ShortcutRecorderButton
        var localMonitor: Any?

        init(parent: ShortcutRecorderButton) {
            self.parent = parent
        }

        // deinit is nonisolated — clean up the event monitor directly rather
        // than calling stopRecording(), which is @MainActor (it also mutates
        // parent state we can't reach from a nonisolated deinit).
        deinit {
            if let monitor = localMonitor {
                NSEvent.removeMonitor(monitor)
            }
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

            // Register and save. If registration fails (e.g. combo already
            // claimed by macOS or another app), surface that and don't persist
            // a shortcut that won't actually fire.
            let manager = GlobalHotkeyManager.shared
            if manager.register(shortcut) {
                parent.error = nil
                manager.save(shortcut)
                parent.shortcutLabel = shortcut.displayString
            } else {
                parent.error = "\(shortcut.displayString) couldn't be registered — try a different combination"
            }
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

// SettingsKeys moved to SettingsKeys.swift (DOLL-203).
