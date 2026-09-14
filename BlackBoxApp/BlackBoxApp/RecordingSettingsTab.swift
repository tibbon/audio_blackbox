import SwiftUI

// A declaration import (its own swift-format group) because a plain `import os.log`
// can't satisfy both linters: swift-format orders imports by ASCII (lowercase last)
// while swiftlint's sorted_imports is case-insensitive.
import struct os.Logger

// MARK: - Recording Tab

struct RecordingSettingsTab: View {
    private static let logger = Logger(subsystem: "com.dollhousemediatech.blackbox", category: "Settings")

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
                    let defaultLabel: String =
                        recorder.systemDefaultDeviceName
                        .map { String(localized: "System Default (\($0))") } ?? String(localized: "System Default")
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
                    confirmSettingsChange(reason: String(localized: "bit depth")) {
                        applyConfig()
                        recorder.restartIfRecording(reason: "bit depth changed")
                    } onCancel: {
                        prevBitDepth = old
                        bitDepth = old
                    }
                }
                .accessibilityLabel("Bit depth")
                .accessibilityHint("Precision of WAV recordings")
                Text(
                    """
                    24-bit is the professional standard. 16-bit saves space. \
                    32-bit float offers maximum precision with larger files.
                    """
                )
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
                    .accessibilityHint(
                        "Finalize the current file when audio goes silent and start a new one on the next signal"
                    )

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

                    Text(
                        """
                        When enabled, BlackBox waits for audio before creating files, and finalizes them \
                        after the selected silence duration. Saves disk space during long idle periods.
                        """
                    )
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
            refreshChannelCount()  // Then clamp to device capabilities
            migrateStalePickerValues()  // DOLL-197
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
            silenceGateTimeout =
                silenceGateTimeoutPresets.min(by: {
                    abs($0 - original) < abs($1 - original)
                }) ?? 300
        }
    }

    // DOLL-221: row height scales with Dynamic Type so the grid stays
    // legible at larger text sizes (the old fixed 24pt cramped rows
    // overlapped at .accessibilityExtraLarge+).
    // DOLL-264: base bumped 22 → 28 to meet the comfortable pointer
    // hit-target guidance (≥28pt), easier for Switch Control / motor-
    // impaired users.
    @ScaledMetric(relativeTo: .body) private var channelRowHeight: CGFloat = 28

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

    @ViewBuilder private var channelCheckboxes: some View {
        let gridItems = Array(
            repeating: GridItem(.flexible(), alignment: .leading),
            count: channelGridColumns
        )

        // LazyVGrid lays out row-major (left→right, top→bottom), so VoiceOver
        // and keyboard focus order already match the visual reading order.
        ScrollView {
            LazyVGrid(columns: gridItems, alignment: .leading, spacing: 8) {
                ForEach(1...deviceChannelCount, id: \.self) { ch in
                    Toggle(
                        isOn: Binding(
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
                        )
                    ) {
                        Text("Ch \(ch)")
                            .font(.body)
                            .monospacedDigit()
                    }
                    .toggleStyle(.checkbox)
                    // DOLL-264: give each toggle a ≥28pt row and make the whole
                    // row hit-test to the control, not just the box + label.
                    .frame(minHeight: channelRowHeight, alignment: .leading)
                    .contentShape(Rectangle())
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
                .accessibilityHint(
                    "Enter channels as a comma-separated list with optional dash ranges, like 1-8, 16, 24-32"
                )
        }

        HStack {
            Button("All") {
                selectedChannels = Set(1...deviceChannelCount)
                syncChannelSpecFromCheckboxes()
                announceChannelSelection()
            }
            .font(.caption)
            Button("Reset") {
                selectedChannels = [1]
                syncChannelSpecFromCheckboxes()
                announceChannelSelection()
            }
            .font(.caption)
            // DOLL-385: the count below is a separate, non-focused element, so
            // VoiceOver gives no feedback when it changes — describe the action.
            .accessibilityHint("Selects only channel 1")
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
                    start >= 1, end >= start
                {
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
    /// DOLL-385: announce the new selection count to VoiceOver after a bulk
    /// All/Reset action (the count label itself is not a focused element).
    private func announceChannelSelection() {
        AccessibilityNotification.Announcement(
            String(localized: "\(selectedChannels.count) of \(deviceChannelCount) channels selected")
        )
        .post()
    }

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
        confirmSettingsChange(reason: String(localized: "channels")) {
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
        case .success(let count):
            deviceChannelCount = count

        case .failure(let err):
            Self.logger.error(
                "getDeviceChannelCount failed for \(selectedDevice): \(String(describing: err), privacy: .public)"
            )
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
            return String(localized: "Studio Quiet")
        }
        if silenceThreshold < 0.02 {
            return String(localized: "Home Office")
        }
        if silenceThreshold < 0.05 {
            return String(localized: "Moderate")
        }
        return String(localized: "Noisy Environment")
    }

    private var thresholdDescription: String {
        let value = String(format: "%.3f", silenceThreshold)
        return String(localized: "\(thresholdPresetLabel), \(value)")
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
