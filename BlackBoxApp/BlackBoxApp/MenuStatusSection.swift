import SwiftUI

/// The top of the menu-bar menu: the status headline, the live-recording
/// details and warnings, the post-Stop summary, and the current error.
struct MenuStatusSection: View {
    var recorder: RecordingState
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.audioChannels) private var channelSpec: String = "1"

    var body: some View {
        // Menu-flicker fix v2: the live elapsed-time `Text(_, style: .timer)`
        // still caused menu reflow because the digit count changes at the
        // minute / hour boundaries — even `.monospacedDigit()` can't hide
        // "9:59" growing to "10:00" (4 → 5 glyphs). Each width change
        // re-laid out the dropdown and reset the user's hover selection.
        // The menu now shows a stable status string only; the live timer
        // moved to the meter window header where the window class
        // doesn't have the highlight-reset problem.
        Text(recorder.statusText)
            .font(.headline)
            .monospacedDigit()

        if recorder.isRecording {
            recordingDetails
        }

        // DOLL-213: transient "last recording" summary for ~30s after
        // Stop. Shown only while idle (a new recording would have
        // already cleared the snapshot). Show in Finder dismisses the
        // banner because the user has now acted on it.
        if !recorder.isRecording, let duration = recorder.lastRecordingDurationText {
            Text("Last recording: \(duration)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .monospacedDigit()
            Button("Show in Finder") {
                recorder.openOutputDir()
                recorder.dismissLastRecordingSummary()
            }
        }

        if let error = recorder.errorMessage {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Color(nsColor: .systemRed))
                .font(.caption)
                .accessibilityLabel("Error: \(error)")
        }
    }

    @ViewBuilder private var recordingDetails: some View {
        // DOLL-215: when the user hasn't picked a specific device,
        // show the resolved system default name (e.g. "MacBook Pro
        // Microphone") instead of the literal "System Default" so the
        // user knows what's actually recording.
        // A chosen device that is no longer connected is not what's
        // recording: the engine fell back to the system default.
        let device =
            resolvedInputDeviceName(
                selected: selectedDevice,
                available: recorder.availableDevices,
                systemDefault: recorder.systemDefaultDeviceName
            ) ?? "System Default"
        let chCount = countChannels(channelSpec)
        Text("\(device) \u{00B7} \(chCount) ch")
            .font(.caption)
            .foregroundStyle(.secondary)

        // DOLL-214: rotation countdown also moved to the meter window
        // header — same digit-width-flicker issue as the elapsed
        // time. The menu no longer hosts any per-second-ticking
        // text; live recording metrics live in the meter window.

        // DOLL-223: surface the running drop count when non-zero so
        // sub-warning drops (1\u{2013}500 samples) aren't invisible.
        // Bigger counts already trigger an errorMessage and auto-stop
        // via the existing engine-side thresholds.
        if recorder.writeErrorsCount > 0 {
            // DOLL-371: the default MenuBarExtra `.menu` style flattens
            // content to NSMenuItems and strips foreground colors, so the
            // orange tint alone wouldn't read as a warning. Use a Label
            // with a warning glyph (which DOES render in `.menu`) so the
            // severity survives without relying on color. The other warning
            // rows below already pair a glyph with their text.
            Label("\(recorder.writeErrorsCount) samples dropped", systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(Color(nsColor: .systemOrange))
                .accessibilityLabel("Warning: \(recorder.writeErrorsCount) samples dropped during this recording")
        }

        // DOLL-225: warn when recording on battery below 20 %, the
        // macOS-equivalent "low battery" threshold. A notification
        // also fires once on threshold crossing in case the user
        // doesn't have the menu open.
        if recorder.isLowBatteryWarning {
            Label(
                "Battery low — plug in to avoid an unexpected stop",
                systemImage: "battery.25percent"
            )
            .font(.caption)
            .foregroundStyle(Color(nsColor: .systemOrange))
            .accessibilityLabel("Warning: battery low, plug in to avoid an unexpected stop")
        }

        // DOLL-220: pre-emptive 4 GiB cap warning. Set at recording
        // start; a notification also fires for menu-closed visibility.
        if let preflight = recorder.preflightSizeWarning {
            Label(preflight, systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(Color(nsColor: .systemOrange))
                .lineLimit(3)
                .accessibilityLabel("Warning: \(preflight)")
        }
    }
}
