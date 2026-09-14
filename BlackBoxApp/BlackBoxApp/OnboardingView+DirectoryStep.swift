import SwiftUI

extension OnboardingView {
    /// Onboarding step 4: pick the recordings folder (or keep the in-container
    /// default). The choice is written back to OnboardingView, which saves it
    /// in completeOnboarding().
    struct DirectoryStep: View {
        let micDenied: Bool
        let defaultDir: URL
        @Binding var outputDir: String
        @Binding var chosenURL: URL?
        @Binding var dirChangedByUser: Bool

        var body: some View {
            VStack(spacing: 16) {
                if micDenied {
                    micDeniedReminder
                }

                Image(systemName: "folder.circle.fill")
                    .font(.system(size: 56))
                    .foregroundStyle(Color.accentColor)
                    .accessibilityHidden(true)

                Text("Choose Output Directory")
                    .font(.title2)
                    .fontWeight(.semibold)

                Text("Where should BlackBox save your recordings?")
                    .foregroundStyle(.secondary)

                pathRow

                Button("Use Default Location") {
                    chosenURL = defaultDir
                    outputDir = defaultDir.path
                    dirChangedByUser = true
                }
                .font(.caption)
                .accessibilityHint("Saves recordings to the app's default folder")

                selectionStatus

                ChangeLaterCaption()
            }
            .padding(.horizontal, 32)
        }

        // DOLL-387: this reminder previously had no recovery affordance
        // and competed with the folder task. Pair it with the same
        // "Open System Settings" action the mic step offers.
        private var micDeniedReminder: some View {
            VStack(spacing: 8) {
                Label(
                    """
                    Microphone access denied \u{2014} recording won't work until you allow access \
                    in System Settings.
                    """,
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(Color(nsColor: .systemOrange))
                .font(.caption)
                .frame(maxWidth: 360)
                Button("Open System Settings") {
                    if let url = URL(
                        string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
                    ) {
                        NSWorkspace.shared.open(url)
                    }
                }
                .font(.caption)
            }
        }

        private var pathRow: some View {
            HStack {
                Text(abbreviatePath(outputDir))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundStyle(chosenURL != nil ? .primary : .secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .clipShape(.rect(cornerRadius: 6))
                    // DOLL-385: announce the full path (truncationMode elides
                    // the middle visually); match the Settings twin's label.
                    .accessibilityLabel("Output directory: \(outputDir)")
                    .accessibilityValue(chosenURL == nil ? String(localized: "No folder selected") : "")

                Button("Choose\u{2026}") {
                    chooseDirectory()
                }
                .controlSize(.large)
                .accessibilityHint("Opens a file picker to select the output directory")
            }
            .frame(maxWidth: 360)
        }

        @ViewBuilder private var selectionStatus: some View {
            if chosenURL == nil {
                Text("Select a folder to continue. BlackBox will create it if it doesn't exist.")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemOrange))
            } else {
                Label("Folder selected", systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundStyle(Color(nsColor: .systemGreen))
            }
        }

        private func abbreviatePath(_ path: String) -> String {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            if path.hasPrefix(home) { return "~" + path.dropFirst(home.count) }
            return path
        }

        /// Open NSOpenPanel and return the chosen URL (with security scope), or nil if cancelled.
        @discardableResult
        private func chooseDirectory() -> URL? {
            let panel = NSOpenPanel()
            panel.canChooseDirectories = true
            panel.canChooseFiles = false
            panel.canCreateDirectories = true
            panel.prompt = String(localized: "Select")
            panel.message = String(localized: "Select output directory for recordings")
            panel.directoryURL = URL(fileURLWithPath: outputDir)

            if panel.runModal() == .OK, let url = panel.url {
                outputDir = url.path
                chosenURL = url
                dirChangedByUser = true
                return url
            }
            return nil
        }
    }
}
