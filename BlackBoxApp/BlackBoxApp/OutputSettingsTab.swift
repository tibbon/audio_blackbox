import SwiftUI

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

    var body: some View {
        Form {
            outputDirectorySection
            outputModeSection
            continuousRecordingSection
            diskSpaceSection
        }
        .formStyle(.grouped)
        .onAppear {
            loadOutputDir()
            syncCadenceSelection()
            migrateStalePickerValues()
            prevOutputMode = outputMode
        }
    }

    private var outputDirectorySection: some View {
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
    }

    private var outputModeSection: some View {
        Section("Output Mode") {
            Picker("Output Mode", selection: $outputMode) {
                Text("Split (one file per channel)").tag("split")
                Text("Multichannel (single file)").tag("single")
            }
            .labelsHidden()
            .pickerStyle(.radioGroup)
            .onChange(of: outputMode) { outputModeChanged() }
            .accessibilityLabel("Output mode")
            .accessibilityHint("One file per channel or one multichannel file")
            outputModeCaption
        }
    }

    @ViewBuilder private var outputModeCaption: some View {
        if outputMode == "single" {
            Label {
                Text(
                    """
                    Creates a single multichannel WAV file. \
                    Some DAWs may not import files with more than 2 channels correctly.
                    """
                )
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

    private var continuousRecordingSection: some View {
        Section("Continuous Recording") {
            Toggle("Enable continuous recording", isOn: $continuousMode)
                .onChange(of: continuousMode) { applyConfig() }
                .accessibilityHint("Automatically rotate files at regular intervals")
            Text(
                """
                Automatically saves and starts a new file at regular intervals, \
                so no audio is lost if the app closes unexpectedly.
                """
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if continuousMode {
                rotationControls
            }
        }
    }

    @ViewBuilder private var rotationControls: some View {
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
            CustomCadenceField(recordingCadence: $recordingCadence, onCommit: commitCustomCadence)
        }

        if let estimate = fileSizeEstimate {
            Text("Estimated file size per chunk: \(estimate)")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var diskSpaceSection: some View {
        Section("Disk Space") {
            Picker("Minimum free space:", selection: $minDiskSpaceMB) {
                Text("Disabled").tag(0)
                Text("500 MB").tag(500)
                Text("1 GB").tag(1000)
                Text("2 GB").tag(2000)
                Text("5 GB").tag(5000)
                Text("10 GB").tag(10_000)
            }
            .onChange(of: minDiskSpaceMB) { applyConfig() }
            .accessibilityLabel("Minimum free disk space")
            Text("Recording stops automatically when free disk space drops below this threshold.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    /// Apply an output-mode pick: immediately when idle, or after the user
    /// confirms a restart while recording (reverting the picker on Cancel).
    private func outputModeChanged() {
        let old = prevOutputMode
        guard outputMode != old else { return }
        prevOutputMode = outputMode
        guard recorder.isRecording else {
            applyConfig()
            return
        }
        confirmSettingsChange(reason: String(localized: "output mode")) {
            applyConfig()
            recorder.restartIfRecording(reason: "output mode changed")
        } onCancel: {
            prevOutputMode = old
            outputMode = old
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
        let minDiskSpacePresets: Set<Int> = [0, 500, 1000, 2000, 5000, 10_000]
        if !minDiskSpacePresets.contains(minDiskSpaceMB) {
            let original = minDiskSpaceMB
            // Snap to the nearest preset.
            minDiskSpaceMB =
                minDiskSpacePresets.min(by: { abs($0 - original) < abs($1 - original) })
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
        let sampleRate = recorder.sampleRate > 0 ? recorder.sampleRate : 48_000
        let bytesPerSample = bitDepth / 8
        let fileCount = outputMode == "split" ? channels : 1
        let channelsPerFile = outputMode == "split" ? 1 : channels
        let bytesPerFile = channelsPerFile * bytesPerSample * sampleRate * recordingCadence
        let totalBytes = bytesPerFile * fileCount
        // DOLL-460: single localized format strings instead of fragment
        // concatenation, so translators see the whole sentence structure.
        let perFile = formatBytes(bytesPerFile)
        let rateNote = recorder.sampleRate > 0 ? "" : String(localized: " (assuming 48 kHz)")
        guard fileCount > 1 else { return perFile + rateNote }
        let total = formatBytes(totalBytes)
        return String(localized: "\(perFile) per file (\(total) total across \(fileCount) files)\(rateNote)")
    }

    private func formatBytes(_ bytes: Int) -> String {
        // DOLL-377: locale-aware binary byte formatting (honors the user's
        // decimal separator / unit labels) instead of a hardcoded "%.1f GB".
        Int64(bytes).formatted(.byteCount(style: .binary))
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
        } else if recordingCadence > 86_400 {
            recordingCadence = 86_400
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
        panel.prompt = String(localized: "Select")
        panel.message = String(localized: "Select output directory for recordings")

        guard panel.runModal() == .OK, let url = panel.url else { return }
        guard recorder.isRecording else {
            outputDir = url.path
            recorder.switchOutputDir(to: url)
            return
        }
        // The live session keeps writing to the old folder until it restarts;
        // switchOutputDir finalizes it before releasing that folder's access.
        confirmSettingsChange(reason: String(localized: "the output folder")) {
            outputDir = url.path
            recorder.switchOutputDir(to: url)
        }
    }
}
