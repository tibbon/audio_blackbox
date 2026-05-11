import AppKit
import SwiftUI

/// "About BlackBox" window content. Pure display — no recorder /
/// FFI dependency. Wired up via the `Window(id: "about")` scene in
/// `BlackBoxApp.swift`.
///
/// Extracted to its own file in DOLL-203 — previously lived inline at
/// the bottom of `BlackBoxApp.swift` where filename navigation missed it.
struct AboutView: View {
    private let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0"
    private let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"
    private let copyright = Bundle.main.object(forInfoDictionaryKey: "NSHumanReadableCopyright") as? String
        ?? "\u{00A9} 2026 David Fisher"

    var body: some View {
        VStack(spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
                .accessibilityHidden(true)

            Text("BlackBox Audio Recorder")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Version \(version) (\(build))")
                .font(.caption)
                .foregroundStyle(.secondary)

            Text(copyright)
                .font(.caption)
                .foregroundStyle(.secondary)

            if let url = AppURL.website {
                Link("dollhousemediatech.com/blackbox", destination: url)
                    .font(.caption)
            }

            HStack(spacing: 12) {
                if let url = AppURL.privacy { Link("Privacy Policy", destination: url) }
                if let url = AppURL.releaseNotes { Link("Release Notes", destination: url) }
                if let url = AppURL.license { Link("License", destination: url) }
                if let url = AppURL.acknowledgments { Link("Acknowledgments", destination: url) }
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
        .padding(24)
        .frame(minWidth: 280)
        .background(AboutWindowConfigurator())
    }
}

/// Disables minimize and zoom buttons on the About window per Apple HIG.
/// Uses viewDidMoveToWindow to configure once, not on every SwiftUI render.
private struct AboutWindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { AboutConfiguratorView() }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class AboutConfiguratorView: NSView {
    private var configured = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard !configured, let window else { return }
        configured = true
        window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
        window.standardWindowButton(.zoomButton)?.isEnabled = false
    }
}
