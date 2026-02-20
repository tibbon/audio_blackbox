import SwiftUI

struct MeterView: View {
    @ObservedObject var recorder: RecordingState

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if recorder.isRecording && !recorder.peakLevels.isEmpty {
                ForEach(Array(recorder.peakLevels.enumerated()), id: \.offset) { index, peak in
                    MeterBar(channel: index + 1, peak: peak)
                }
            } else {
                Spacer()
                HStack {
                    Spacer()
                    VStack(spacing: 8) {
                        Image(systemName: "waveform")
                            .font(.system(size: 32))
                            .foregroundColor(.secondary)
                        Text("Not recording")
                            .foregroundColor(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("Level meter: Not recording")
                    Spacer()
                }
                Spacer()
            }
        }
        .padding(12)
        .frame(minWidth: 300, minHeight: 100)
        .background(MeterWindowConfigurator())
        .onAppear { recorder.isMeterWindowOpen = true }
        .onDisappear { recorder.isMeterWindowOpen = false }
    }
}

// MARK: - Single Channel Bar

private struct MeterBar: View {
    let channel: Int
    let peak: Double

    private var dBFS: Double {
        guard peak > 0 else { return -60 }
        return max(20 * log10(peak), -60)
    }

    /// Normalized 0.0–1.0 for the bar width (maps -60dB..0dB)
    private var barFraction: Double {
        (dBFS + 60) / 60
    }

    private var barColor: Color {
        if dBFS > -3 {
            return .red
        } else if dBFS > -12 {
            return .yellow
        } else {
            return .green
        }
    }

    private var dBLabel: String {
        if dBFS <= -60 {
            return "-inf"
        }
        return String(format: "%.0f dB", dBFS)
    }

    var body: some View {
        HStack(spacing: 8) {
            Text("Ch \(channel)")
                .font(.caption)
                .monospacedDigit()
                .frame(width: 36, alignment: .trailing)

            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Rectangle()
                        .fill(Color.primary.opacity(0.08))
                        .frame(height: 14)
                        .cornerRadius(3)

                    Rectangle()
                        .fill(barColor)
                        .frame(width: max(0, geo.size.width * barFraction), height: 14)
                        .cornerRadius(3)
                        .animation(.linear(duration: 0.1), value: barFraction)
                }
            }
            .frame(height: 14)

            Text(dBLabel)
                .font(.caption)
                .monospacedDigit()
                .frame(width: 44, alignment: .trailing)
                .foregroundColor(barColor == .red ? .red : .secondary)
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Channel \(channel)")
        .accessibilityValue("\(dBLabel)\(dBFS > -3 ? ", clipping" : dBFS > -12 ? ", caution" : "")")
    }
}

/// Disables minimize and zoom buttons on the Level Meter window per Apple HIG.
private struct MeterWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            guard let window = nsView.window else { return }
            window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
            window.standardWindowButton(.zoomButton)?.isEnabled = false
        }
    }
}
