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
    /// The silence settings last pushed to the engine (see
    /// SilenceDetectionSection).
    @State private var appliedSilence = SilenceSettings()

    var body: some View {
        Form {
            inputDeviceSection

            Section("Channels") {
                if deviceChannelCount > 0 {
                    ChannelSelectionGrid(
                        deviceChannelCount: deviceChannelCount,
                        selectedChannels: $selectedChannels,
                        channelSpec: channelSpec,
                        onSelectionChange: syncChannelSpecFromCheckboxes,
                        onCommitSpecText: commitChannelSpecText
                    )
                } else {
                    Text("Select an input device to see available channels.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            bitDepthSection

            SilenceDetectionSection(
                silenceEnabled: $silenceEnabled,
                silenceThreshold: $silenceThreshold,
                silenceGateEnabled: $silenceGateEnabled,
                silenceGateTimeout: $silenceGateTimeout,
                applied: appliedSilence,
                applySetting: { reason in
                    applySessionSetting(recorder: recorder, reason: reason, apply: applyConfig)
                }
            )

            monitoringSection
        }
        .formStyle(.grouped)
        .onAppear {
            appliedSilence = currentSilenceSettings
            syncCheckboxesFromChannelSpec()  // Load saved spec FIRST
            refreshChannelCount()  // Then clamp to device capabilities
            migrateStalePickerValues()  // DOLL-197
            prevBitDepth = bitDepth
            prevChannelSpec = channelSpec
        }
    }

    private var inputDeviceSection: some View {
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
    }

    private var bitDepthSection: some View {
        Section("Bit Depth") {
            Picker("Bit Depth", selection: $bitDepth) {
                Text("16-bit").tag(16)
                Text("24-bit (Recommended)").tag(24)
                Text("32-bit").tag(32)
            }
            .labelsHidden()
            .pickerStyle(.radioGroup)
            .onChange(of: bitDepth) { bitDepthChanged() }
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
    }

    private var monitoringSection: some View {
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

    /// Apply a bit-depth pick: immediately when idle, or after the user
    /// confirms a restart while recording (reverting the picker on Cancel).
    private func bitDepthChanged() {
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

    /// Commit a free-form channel-spec edit: validate it, clamp to what the
    /// device supports, and save the canonical string through the same path
    /// as the checkboxes (which asks before restarting a live recording).
    /// A malformed spec, or one naming none of the device's channels, is
    /// rejected and the saved spec stays. Returns the spec now in effect.
    private func commitChannelSpecText(_ text: String) -> String {
        let onDevice = parseChannelSpec(text).filter { $0 <= deviceChannelCount }
        guard !onDevice.isEmpty else { return channelSpec }
        selectedChannels = Set(onDevice)
        syncChannelSpecFromCheckboxes()
        return channelSpec
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
        appliedSilence = currentSilenceSettings
    }

    private var currentSilenceSettings: SilenceSettings {
        SilenceSettings(
            enabled: silenceEnabled,
            threshold: silenceThreshold,
            gateEnabled: silenceGateEnabled,
            gateTimeout: silenceGateTimeout
        )
    }
}
