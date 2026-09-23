import SwiftUI

/// The Output tab's custom rotation-interval row (shown when the rotation
/// picker is on Custom): a seconds field plus a readable duration. Edits are
/// committed through `onCommit` on Return or focus loss, not per keystroke.
struct CustomCadenceField: View {
    @Binding var recordingCadence: Int
    /// Clamps the typed interval and applies the tab's config (DOLL-196).
    let onCommit: @MainActor () -> Void

    /// Tracks focus on the custom-cadence TextField so DOLL-196 can
    /// clamp + applyConfig only when the user commits (focus loss /
    /// Return), not on every keystroke. The old onChange-clamp made a
    /// user typing "60" briefly see the field jump as intermediate
    /// values were clipped to the lower bound.
    @FocusState private var customCadenceFocused: Bool

    var body: some View {
        HStack {
            TextField("", value: $recordingCadence, format: .number)
                .textFieldStyle(.roundedBorder)
                .frame(width: 80)
                .focused($customCadenceFocused)
                // DOLL-196: clamp + applyConfig on focus
                // loss / Return, not on every keystroke.
                // The old onChange-clamp made the field
                // jump to 1 mid-typing.
                .onSubmit { onCommit() }
                .onChange(of: customCadenceFocused) { _, focused in
                    if !focused { onCommit() }
                }
                .accessibilityLabel("Custom rotation interval")
                .accessibilityValue("\(recordingCadence) seconds")
            Text("seconds")
            if recordingCadence >= 86_400 {
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

    private var cadenceDescription: String { Self.cadenceDescription(recordingCadence) }

    /// A readable duration for a rotation interval in seconds. Whole hours
    /// and minutes use the catalog's plural variations ("1 hour", "2 hours")
    /// rather than an English-only singular/plural branch.
    static func cadenceDescription(_ recordingCadence: Int) -> String {
        let hours = recordingCadence / 3600
        let minutes = (recordingCadence % 3600) / 60
        let seconds = recordingCadence % 60
        if hours > 0 && minutes == 0 && seconds == 0 {
            return String(localized: "\(hours) hours", comment: "A whole number of hours; plural variations")
        }
        if hours > 0 {
            return String(localized: "\(hours)h \(minutes)m")
        }
        if minutes > 0 && seconds == 0 {
            return String(localized: "\(minutes) minutes", comment: "A whole number of minutes; plural variations")
        }
        if minutes > 0 {
            return String(localized: "\(minutes)m \(seconds)s")
        }
        return String(localized: "\(seconds)s")
    }
}
