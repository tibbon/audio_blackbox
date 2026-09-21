import SwiftUI

/// The Recording tab's Silence Detection section: delete-silent-recordings
/// with its threshold slider, and the auto-split silence gate with its
/// timeout. The values are the tab's `@AppStorage` settings; `applyConfig`
/// pushes the tab's whole config to the engine.
struct SilenceDetectionSection: View {
    @Binding var silenceEnabled: Bool
    @Binding var silenceThreshold: Double
    @Binding var silenceGateEnabled: Bool
    @Binding var silenceGateTimeout: Int
    let applyConfig: @MainActor () -> Void

    /// Trailing debounce for the silence-threshold slider (DOLL-195).
    /// Slider's `onChange` fires per drag tick; without this, every
    /// drag dispatches a full setConfig() through the FFI dozens of
    /// times per second. Settle for 150 ms after the last change before
    /// pushing to Rust.
    @State private var thresholdDebounceTask: Task<Void, Never>?

    var body: some View {
        Section("Silence Detection") {
            Toggle("Enable silence detection", isOn: $silenceEnabled)
                .onChange(of: silenceEnabled) { applyConfig() }
                .accessibilityHint("Automatically delete silent recordings")
            Text("Recordings that contain only silence are automatically deleted when complete.")
                .font(.caption)
                .foregroundStyle(.secondary)

            if silenceEnabled {
                thresholdControls
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
                gateTimeoutControls
            }
        }
    }

    @ViewBuilder private var thresholdControls: some View {
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

    @ViewBuilder private var gateTimeoutControls: some View {
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
}
