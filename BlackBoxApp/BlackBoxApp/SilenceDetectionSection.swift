import SwiftUI

/// The Silence Detection settings as last pushed to the engine.
struct SilenceSettings {
    var enabled = true
    var threshold = 0.01
    var gateEnabled = true
    var gateTimeout = 300
}

/// The Recording tab's Silence Detection section: delete-silent-recordings
/// with its threshold slider, and the auto-split silence gate with its
/// timeout. The values are the tab's `@AppStorage` settings.
///
/// The engine reads these only when a session starts, so an edit goes
/// through `applySetting`, which asks to restart a live recording and
/// returns `false` on Cancel; the control then goes back to its `applied`
/// value. A control already at its applied value has nothing to apply,
/// which is what keeps that revert from asking again.
struct SilenceDetectionSection: View {
    @Binding var silenceEnabled: Bool
    @Binding var silenceThreshold: Double
    @Binding var silenceGateEnabled: Bool
    @Binding var silenceGateTimeout: Int
    let applied: SilenceSettings
    let applySetting: @MainActor (_ reason: String) -> Bool

    /// Trailing debounce for the silence-threshold slider (DOLL-195).
    /// Slider's `onChange` fires per drag tick; without this, every
    /// drag dispatches a full setConfig() through the FFI dozens of
    /// times per second. Settle for 150 ms after the last change before
    /// pushing to Rust.
    @State private var thresholdDebounceTask: Task<Void, Never>?

    var body: some View {
        Section("Silence Detection") {
            Toggle("Enable silence detection", isOn: $silenceEnabled)
                .onChange(of: silenceEnabled) {
                    guard silenceEnabled != applied.enabled else { return }
                    if !applySetting(String(localized: "silence detection")) {
                        silenceEnabled = applied.enabled
                    }
                }
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
                .onChange(of: silenceGateEnabled) {
                    guard silenceGateEnabled != applied.gateEnabled else { return }
                    if !applySetting(String(localized: "auto-split on silence")) {
                        silenceGateEnabled = applied.gateEnabled
                    }
                }
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
                    guard !Task.isCancelled, silenceThreshold != applied.threshold else { return }
                    if !applySetting(String(localized: "the silence threshold")) {
                        silenceThreshold = applied.threshold
                    }
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
                // Applied through the slider's onChange, like a drag.
                silenceThreshold = 0.01
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
        .onChange(of: silenceGateTimeout) {
            guard silenceGateTimeout != applied.gateTimeout else { return }
            if !applySetting(String(localized: "the silence gate timeout")) {
                silenceGateTimeout = applied.gateTimeout
            }
        }
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
