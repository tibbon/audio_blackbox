import SwiftUI

struct MeterView: View {
    @ObservedObject var recorder: RecordingState

    /// Height of one meter row: barHeight (14pt) + vertical padding (6pt)
    private static let rowHeight: CGFloat = 20
    /// Vertical space reserved for window chrome, title bar, and padding
    private static let chromeOverhead: CGFloat = 80

    private var columnLayout: (columns: Int, rowsPerColumn: Int) {
        let count = recorder.peakLevels.count
        let availableHeight = (NSScreen.main?.visibleFrame.height ?? 800) - Self.chromeOverhead
        let maxRows = max(1, Int(availableHeight / Self.rowHeight))
        if count <= maxRows {
            return (1, count)
        }
        let cols = Int(ceil(Double(count) / Double(maxRows)))
        let rowsPer = Int(ceil(Double(count) / Double(cols)))
        return (cols, rowsPer)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if (recorder.isRecording || recorder.isMonitoring) && !recorder.peakLevels.isEmpty {
                let layout = columnLayout
                if layout.columns <= 1 {
                    ForEach(0..<recorder.peakLevels.count, id: \.self) { index in
                        MeterBar(channel: index + 1, peak: recorder.peakLevels[index])
                    }
                } else {
                    HStack(alignment: .top, spacing: 16) {
                        ForEach(0..<layout.columns, id: \.self) { col in
                            let start = col * layout.rowsPerColumn
                            let end = min(start + layout.rowsPerColumn, recorder.peakLevels.count)
                            VStack(alignment: .leading, spacing: 0) {
                                ForEach(start..<end, id: \.self) { index in
                                    MeterBar(channel: index + 1, peak: recorder.peakLevels[index])
                                }
                            }
                            .frame(minWidth: 280)
                        }
                    }
                }
            } else {
                Spacer()
                HStack {
                    Spacer()
                    VStack(spacing: 8) {
                        Image(systemName: "waveform")
                            .font(.largeTitle)
                            .foregroundColor(.secondary)
                        Text("No audio input")
                            .foregroundColor(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("Level meter: No audio input")
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
    let peak: Float

    @ScaledMetric(relativeTo: .caption) private var channelLabelWidth: CGFloat = 36
    @ScaledMetric(relativeTo: .caption) private var dBLabelWidth: CGFloat = 50
    @ScaledMetric(relativeTo: .caption) private var barHeight: CGFloat = 14

    @State private var peakHold: Float = -60
    @State private var peakHoldInstant: ContinuousClock.Instant = .now

    /// Peak hold decay: hold for 2 seconds, then drop.
    private static let holdDuration: ContinuousClock.Duration = .seconds(2)

    private var dBFS: Float {
        guard peak > 0 else { return -60 }
        return max(20 * log10(peak), -60)
    }

    /// Normalized 0.0-1.0 for the bar width (maps -60dB..0dB)
    private var barFraction: CGFloat {
        CGFloat((dBFS + 60) / 60)
    }

    private var peakHoldFraction: CGFloat {
        CGFloat((peakHold + 60) / 60)
    }

    private var meterAccessibilityValue: String {
        if dBFS > -3 { return "Clipping: signal is too loud and may distort" }
        if dBFS > -12 { return "Hot: signal is high, reduce input gain" }
        if dBFS <= -60 { return "Silent" }
        return "\(Int(dBFS)) decibels"
    }

    private var dBLabel: String {
        if dBFS > -3 {
            return "CLIP"
        }
        if dBFS > -12 {
            return "HOT"
        }
        if dBFS <= -60 {
            return "-inf"
        }
        return "\(Int(dBFS)) dB"
    }

    /// dB positions for tick marks and labels on the meter background
    private static let tickPositions: [(dB: Float, label: String?)] = [
        (-48, "-48"), (-24, "-24"), (-12, "-12"), (-6, nil), (-3, "-3"), (0, "0"),
    ]

    /// Gradient stops matching the green-yellow-red meter convention
    private static let meterGradient = LinearGradient(
        stops: [
            .init(color: Color(nsColor: .systemGreen), location: 0.0),
            .init(color: Color(nsColor: .systemGreen), location: 0.7),    // -60 to -18 dB
            .init(color: Color(nsColor: .systemYellow), location: 0.8),    // -12 dB
            .init(color: Color(nsColor: .systemYellow), location: 0.92),   // -5 dB
            .init(color: Color(nsColor: .systemRed), location: 0.95),      // -3 dB
            .init(color: Color(nsColor: .systemRed), location: 1.0),
        ],
        startPoint: .leading,
        endPoint: .trailing
    )

    var body: some View {
        HStack(spacing: 8) {
            Text("Ch \(channel)")
                .font(.caption)
                .monospacedDigit()
                .frame(width: channelLabelWidth, alignment: .trailing)

            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    // Background track
                    Rectangle()
                        .fill(Color.primary.opacity(0.08))
                        .frame(height: barHeight)
                        .cornerRadius(3)

                    // dB scale tick marks
                    ForEach(MeterBar.tickPositions, id: \.dB) { tick in
                        let fraction = CGFloat((tick.dB + 60) / 60)
                        Rectangle()
                            .fill(Color.primary.opacity(0.15))
                            .frame(width: 1, height: barHeight)
                            .position(x: geo.size.width * fraction, y: barHeight / 2)
                    }

                    // Gradient-filled level bar
                    Self.meterGradient
                        .frame(width: max(0, geo.size.width * barFraction), height: barHeight)
                        .cornerRadius(3)
                        .animation(.linear(duration: 0.05), value: barFraction)

                    // Peak hold indicator
                    if peakHold > -60 {
                        Rectangle()
                            .fill(peakHold > -3 ? Color(nsColor: .systemRed) : Color.primary.opacity(0.6))
                            .frame(width: 2, height: barHeight)
                            .position(
                                x: min(geo.size.width * peakHoldFraction, geo.size.width - 1),
                                y: barHeight / 2
                            )
                    }
                }
            }
            .frame(height: barHeight)

            Text(dBLabel)
                .font(.system(.caption, design: .monospaced))
                .frame(width: dBLabelWidth, alignment: .trailing)
                .foregroundColor(dBFS > -3 ? Color(nsColor: .systemRed) : dBFS > -12 ? Color(nsColor: .systemYellow) : .secondary)
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Channel \(channel)")
        .accessibilityValue(meterAccessibilityValue)
        .onChange(of: peak) { _ in
            updatePeakHold()
        }
    }

    /// Update peak hold: set new high, or decay after hold duration
    private func updatePeakHold() {
        let now = ContinuousClock.now
        if dBFS > peakHold {
            peakHold = dBFS
            peakHoldInstant = now
        } else if now - peakHoldInstant > Self.holdDuration {
            // Decay: drop toward current level
            let decayed = peakHold - 1.5  // ~1.5 dB per frame at 30fps ≈ 45 dB/s
            peakHold = max(decayed, dBFS)
            if peakHold <= -60 {
                peakHold = -60
            }
        }
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
