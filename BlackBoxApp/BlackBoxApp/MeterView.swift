import SwiftUI

struct MeterView: View {
    var recorder: RecordingState

    // DOLL-219: header reads device + sample rate + bit depth so the
    // user can confirm what the meter is actually showing without
    // hunting through Settings. Selected device + bit depth come from
    // @AppStorage so the header refreshes when the user changes them
    // even while the meter window is open.
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""
    @AppStorage(SettingsKeys.bitDepth) private var bitDepth: Int = 24

    /// Height of one meter row: barHeight (14pt) + vertical padding (6pt)
    private static let rowHeight: CGFloat = 20
    /// Vertical space reserved for window chrome, title bar, and padding
    private static let chromeOverhead: CGFloat = 80

    private func columnLayout(for count: Int) -> (columns: Int, rowsPerColumn: Int) {
        let availableHeight = (NSScreen.main?.visibleFrame.height ?? 800) - Self.chromeOverhead
        let maxRows = max(1, Int(availableHeight / Self.rowHeight))
        if count <= maxRows {
            return (1, count)
        }
        let cols = Int(ceil(Double(count) / Double(maxRows)))
        let rowsPer = Int(ceil(Double(count) / Double(cols)))
        return (cols, rowsPer)
    }

    /// Display name for the input device. Resolves "" to the system
    /// default's actual device name when known (DOLL-215 → 219).
    private var deviceDisplayName: String {
        if selectedDevice.isEmpty {
            return recorder.systemDefaultDeviceName ?? "System Default"
        }
        return selectedDevice
    }

    /// Sample-rate string for the header — "48 kHz" when active, "—" when idle.
    private var sampleRateDisplay: String {
        let rate = recorder.sampleRate
        guard rate > 0 else { return "\u{2014}" }
        if rate % 1000 == 0 {
            return "\(rate / 1000) kHz"
        }
        let kHz = Double(rate) / 1000.0
        return String(format: "%.1f kHz", kHz)
    }

    /// VoiceOver-friendly sample rate. The visible `sampleRateDisplay` shows a
    /// U+2014 em-dash when idle, which VoiceOver reads aloud as "em dash"
    /// ("…at em dash, 24 bits per sample"). Speak "unknown sample rate" instead.
    /// (DOLL-385)
    private var sampleRateSpoken: String {
        recorder.sampleRate > 0 ? sampleRateDisplay : "unknown sample rate"
    }

    /// Window title with a state suffix so a glance at the title bar (or
    /// the Window menu) tells the user whether bars they're looking at
    /// represent a live recording, a passive monitor, or stale state.
    /// DOLL-218.
    private var windowTitle: String {
        if recorder.isRecording { return "Level Meter (Recording)" }
        if recorder.isMonitoring { return "Level Meter (Monitoring)" }
        return "Level Meter"
    }

    /// VoiceOver verb for the header. The header label previously hard-coded
    /// "Recording" even while only monitoring or idle (DOLL-252), telling VO
    /// users they were recording when they weren't.
    private var stateVerb: String {
        if recorder.isRecording { return "Recording" }
        if recorder.isMonitoring { return "Monitoring" }
        return "Input"
    }

    /// -3 dBFS as a linear peak amplitude (10^(-3/20)). Channels above this
    /// are clipping; used to fire a single VoiceOver clip announcement.
    private static let clipPeakThreshold: Float = 0.7079458

    /// Tracks the aggregate clipping edge so the announcement fires once when
    /// the signal starts clipping, not on every frame it stays clipped.
    @State private var wasClipping = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // DOLL-219: persistent header so the user always sees which
            // device the meter is reading from, at what rate, and at what
            // bit depth. Centre-truncates the device because USB / aggregate
            // device names can be very long.
            HStack(spacing: 6) {
                Text(deviceDisplayName)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text("\u{00B7}")
                    .foregroundStyle(.tertiary)
                Text(sampleRateDisplay)
                    .monospacedDigit()
                Text("\u{00B7}")
                    .foregroundStyle(.tertiary)
                Text("\(bitDepth)-bit")
                    .monospacedDigit()
                // DOLL-217 v2: estimated current-file size relocated here
                // from the menu, where its per-second updates were
                // causing menu re-renders that reset hover/highlight.
                // A window-class view doesn't have that problem.
                if let size = recorder.currentFileSizeText {
                    Text("\u{00B7}")
                        .foregroundStyle(.tertiary)
                    Text(size)
                        .monospacedDigit()
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.bottom, 4)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(stateVerb) \(deviceDisplayName) at \(sampleRateSpoken), \(bitDepth) bits per sample")

            // DOLL-214 / DOLL-217 v3: live elapsed time + rotation
            // countdown relocated from the menu (where each tick
            // re-laid out the dropdown and reset hover selection) into
            // the meter window header. Window-class views can reflow
            // freely without disrupting menu-style selection. Both
            // use Text(_, style: .timer) so SwiftUI ticks the text
            // internally without per-frame @Observable writes.
            if let start = recorder.recordingStartTime {
                HStack(spacing: 6) {
                    (Text("Elapsed ") + Text(start, style: .timer))
                        .monospacedDigit()
                    if let next = recorder.nextRotationDate {
                        Text("\u{00B7}")
                            .foregroundStyle(.tertiary)
                        (Text("Rotates in ") + Text(next, style: .timer))
                            .monospacedDigit()
                    }
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
                .padding(.bottom, 8)
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Live recording timer in meter header")
            }

            // Snapshot peak levels so the ForEach closure captures stable values.
            // Without this, peakLevels can be cleared (monitoring stopped) between
            // ForEach range creation and closure execution, causing an index-out-of-bounds crash.
            let levels = recorder.peakLevels
            if (recorder.isRecording || recorder.isMonitoring) && !levels.isEmpty {
                let layout = columnLayout(for: levels.count)
                if layout.columns <= 1 {
                    ForEach(levels.indices, id: \.self) { index in
                        MeterBar(channel: index + 1, peak: levels[index])
                    }
                } else {
                    HStack(alignment: .top, spacing: 16) {
                        ForEach(0..<layout.columns, id: \.self) { col in
                            let start = col * layout.rowsPerColumn
                            let end = min(start + layout.rowsPerColumn, levels.count)
                            VStack(alignment: .leading, spacing: 0) {
                                ForEach(start..<end, id: \.self) { index in
                                    MeterBar(channel: index + 1, peak: levels[index])
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
                            .foregroundStyle(.secondary)
                        // DOLL-218: "No audio input" read like a hardware
                        // error; in practice this state shows briefly while
                        // monitoring spins up (or persistently if no input
                        // device is configured). "Listening for signal"
                        // matches the DOLL-216 "Armed" framing and works
                        // for both cases without alarming the user.
                        Text("Listening for signal\u{2026}")
                            .foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("Level meter: Listening for signal")
                    Spacer()
                }
                Spacer()
            }
        }
        .padding(12)
        // minHeight bumped progressively as header rows were added:
        // 100 → 130 (DOLL-219, device · rate · bit-depth)
        // → 155 (DOLL-214/217 v3, elapsed + rotation countdown row).
        .frame(minWidth: 300, minHeight: 155)
        .navigationTitle(windowTitle)
        .background(MeterWindowConfigurator { occluded in
            recorder.isMeterWindowOccluded = occluded
        })
        .onAppear { recorder.isMeterWindowOpen = true }
        .onDisappear { recorder.isMeterWindowOpen = false }
        // DOLL-252: clip indicators are otherwise purely visual. Post one
        // VoiceOver announcement on the rising edge of "any channel clipping"
        // so VO users hear that their input is too hot without re-focusing
        // each bar. Rising-edge gating keeps it from repeating every frame.
        .onChange(of: recorder.peakLevels) { _, levels in
            let clipping = levels.contains { $0 > Self.clipPeakThreshold }
            if clipping && !wasClipping {
                AccessibilityNotification.Announcement("Audio clipping, reduce input gain").post()
            }
            wasClipping = clipping
        }
    }
}

// MARK: - Single Channel Bar

private struct MeterBar: View {
    let channel: Int
    let peak: Float

    // DOLL-263: continuously animated level bars are exactly the motion
    // Reduce Motion targets; snap instead of tween when it's enabled.
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

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

    /// VoiceOver value for the meter. Bucketed to a small set of fixed
    /// thresholds so the value only changes when crossing a meaningful
    /// audio boundary (DOLL-142). Previously the middle range emitted
    /// `"\(Int(dBFS)) decibels"` which changed every ~30 Hz tick — VO
    /// users got a stream of "-17 decibels", "-18 decibels"… on any
    /// non-clipping non-silent signal. The buckets line up with the
    /// existing visual gradient stops (-12, -24, -48).
    private var meterAccessibilityValue: String {
        if dBFS > -3 { return "Clipping: signal is too loud and may distort" }
        if dBFS > -12 { return "Hot: signal is high, reduce input gain" }
        if dBFS <= -60 { return "Silent" }
        if dBFS <= -48 { return "Very low, below -48 decibels" }
        if dBFS <= -24 { return "Low, between -48 and -24 decibels" }
        return "Moderate, between -24 and -12 decibels"
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
                        .clipShape(.rect(cornerRadius: 3))

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
                        .clipShape(.rect(cornerRadius: 3))
                        .animation(reduceMotion ? nil : .linear(duration: 0.05), value: barFraction)

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
                .foregroundStyle(dBFS > -3 ? Color(nsColor: .systemRed) : dBFS > -12 ? Color(nsColor: .systemYellow) : .secondary)
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Channel \(channel)")
        .accessibilityValue(meterAccessibilityValue)
        // DOLL-252: tell VoiceOver this value updates live so it re-reads the
        // focused bar as the level changes, instead of the user re-focusing.
        .accessibilityAddTraits(.updatesFrequently)
        .onChange(of: peak) {
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
/// Uses viewDidMoveToWindow to configure once, not on every SwiftUI render.
private struct MeterWindowConfigurator: NSViewRepresentable {
    /// Called with `true` when the window becomes occluded (not visible) and
    /// `false` when it becomes visible again (DOLL-348).
    var onOcclusionChange: @MainActor (Bool) -> Void

    func makeNSView(context: Context) -> NSView {
        WindowConfiguratorView(onOcclusionChange: onOcclusionChange)
    }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class WindowConfiguratorView: NSView {
    private var configured = false
    private let onOcclusionChange: @MainActor (Bool) -> Void
    private var occlusionObserver: NSObjectProtocol?

    init(onOcclusionChange: @escaping @MainActor (Bool) -> Void) {
        self.onOcclusionChange = onOcclusionChange
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let window else { return }

        if !configured {
            configured = true
            window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
            window.standardWindowButton(.zoomButton)?.isEnabled = false
        }

        // DOLL-348: pause the meter's 30 Hz poll whenever the window isn't
        // visible. `didChangeOcclusionStateNotification` fires for covering,
        // minimizing, and Space switches — none of which trigger SwiftUI's
        // onDisappear.
        if let occlusionObserver {
            NotificationCenter.default.removeObserver(occlusionObserver)
        }
        let publish = onOcclusionChange
        occlusionObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didChangeOcclusionStateNotification,
            object: window,
            queue: .main
        ) { [weak window] _ in
            guard let window else { return }
            let occluded = !window.occlusionState.contains(.visible)
            MainActor.assumeIsolated { publish(occluded) }
        }
        // Publish the current state immediately (the window may already be
        // visible when the view is installed).
        onOcclusionChange(!window.occlusionState.contains(.visible))
    }

    deinit {
        if let occlusionObserver {
            NotificationCenter.default.removeObserver(occlusionObserver)
        }
    }
}
