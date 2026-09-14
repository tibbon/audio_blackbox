import SwiftUI

/// The Recording tab's channel picker for a device with a known channel
/// count: a checkbox grid, a range text field, and All / Reset buttons.
/// The selection and spec state stay on `RecordingSettingsTab`, which
/// decides when a change needs a restart confirmation.
struct ChannelSelectionGrid: View {
    let deviceChannelCount: Int
    @Binding var selectedChannels: Set<Int>
    @Binding var channelSpec: String
    /// Writes the checkbox state back to the channel spec (and the engine).
    let onSelectionChange: @MainActor () -> Void
    /// Re-parses and clamps a typed channel spec.
    let onCommitSpecText: @MainActor () -> Void

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

    var body: some View {
        let gridItems = Array(
            repeating: GridItem(.flexible(), alignment: .leading),
            count: channelGridColumns
        )

        // LazyVGrid lays out row-major (left→right, top→bottom), so VoiceOver
        // and keyboard focus order already match the visual reading order.
        ScrollView {
            LazyVGrid(columns: gridItems, alignment: .leading, spacing: 8) {
                ForEach(1...deviceChannelCount, id: \.self) { ch in
                    channelToggle(ch)
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
                .onSubmit { onCommitSpecText() }
                .accessibilityLabel("Channel range")
                .accessibilityHint(
                    "Enter channels as a comma-separated list with optional dash ranges, like 1-8, 16, 24-32"
                )
        }

        HStack {
            Button("All") {
                selectedChannels = Set(1...deviceChannelCount)
                onSelectionChange()
                announceChannelSelection()
            }
            .font(.caption)
            Button("Reset") {
                selectedChannels = [1]
                onSelectionChange()
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

    private func channelToggle(_ ch: Int) -> some View {
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
                    onSelectionChange()
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

    /// DOLL-385: announce the new selection count to VoiceOver after a bulk
    /// All/Reset action (the count label itself is not a focused element).
    private func announceChannelSelection() {
        AccessibilityNotification.Announcement(
            String(localized: "\(selectedChannels.count) of \(deviceChannelCount) channels selected")
        )
        .post()
    }
}
