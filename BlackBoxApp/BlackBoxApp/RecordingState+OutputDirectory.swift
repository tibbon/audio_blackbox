import AppKit
import Foundation

import struct os.Logger

extension RecordingState {
    private static let bookmarkKey = SettingsKeys.outputDirBookmark

    func openOutputDir() {
        let config = bridge.getConfig()
        let dir = config?["output_dir"] as? String ?? "recordings"

        let url: URL
        if dir.hasPrefix("/") {
            url = URL(fileURLWithPath: dir)
        } else {
            let cwd = FileManager.default.currentDirectoryPath
            url = URL(fileURLWithPath: cwd).appendingPathComponent(dir)
        }

        // DOLL-114: defer the FileManager + NSWorkspace I/O off the main
        // actor. Both calls hit disk / Launch Services and were
        // synchronously blocking the UI on this user action. `@concurrent`
        // runs the body on the global executor while keeping the caller's
        // priority (unlike Task.detached).
        Task { @concurrent in
            try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            // open(_:) reports whether Launch Services accepted the URL; there is no
            // recovery path here beyond the Finder window simply not appearing.
            await MainActor.run { _ = NSWorkspace.shared.open(url) }
        }
    }

    // MARK: - Security-Scoped Bookmarks

    /// Point recording at the in-container default directory
    /// (`Self.defaultOutputDir`). The container is always writable, so —
    /// unlike a user-selected folder — it needs no security-scoped bookmark;
    /// any previously stored bookmark is cleared so a stale one can't shadow
    /// the default on the next launch. (DOLL-344)
    func useDefaultOutputDir() {
        let url = Self.defaultOutputDir
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        } catch {
            // Non-fatal: the Rust writer also creates missing directories. Log
            // and still point config at the path.
            Self.log.error("Failed to create default output directory: \(error.localizedDescription)")
        }
        releaseOutputDirAccess()
        UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
        UserDefaults.standard.set(url.path, forKey: SettingsKeys.lastOutputDirPath)
        bridge.setConfig(["output_dir": url.path])
        Self.log.info("Using default in-container output directory: \(url.path)")
    }

    /// Save a security-scoped bookmark for a **user-selected** output directory
    /// (one chosen via `NSOpenPanel`). Creates the directory if it doesn't
    /// exist. Do NOT call this for the in-container default — use
    /// `useDefaultOutputDir()`, which needs no bookmark (DOLL-344).
    func saveOutputDirBookmark(for url: URL) {
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            try storeOutputDirBookmark(for: url)

            // Release previous access if any
            securityScopedURL?.stopAccessingSecurityScopedResource()
            securityScopedURL = url

            // Update Rust config with the chosen path
            bridge.setConfig(["output_dir": url.path])
            Self.log.info("Saved output directory bookmark: \(url.path)")
        } catch {
            let err = String(localized: "Failed to save directory bookmark: \(error.localizedDescription)")
            errorMessage = err
            Self.log.error("\(err)")
        }
    }

    /// Persist a security-scoped bookmark for `url` without touching the
    /// access currently held. Used alone to refresh a stale bookmark for the
    /// URL we are already accessing: going through `saveOutputDirBookmark`
    /// there would stop access on that very URL.
    private func storeOutputDirBookmark(for url: URL) throws {
        let bookmarkData = try url.bookmarkData(
            options: .withSecurityScope,
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        )
        UserDefaults.standard.set(bookmarkData, forKey: Self.bookmarkKey)
        UserDefaults.standard.set(url.path, forKey: SettingsKeys.lastOutputDirPath)
    }

    /// Restore the security-scoped bookmark on launch.
    ///
    /// DOLL-114: called from a deferred Task (see `init`) so the bookmark
    /// resolution + security scope acquisition + bridge.setConfig (each of
    /// which can hit disk or IPC) run after the launch path, not on it.
    func restoreOutputDirBookmark() {
        guard let data = UserDefaults.standard.data(forKey: Self.bookmarkKey) else {
            // No bookmark means either a first run or the user is on the
            // in-container default (which never stores one). Ensure that
            // default exists and points the engine at it, rather than leaving
            // Rust on its relative "recordings" fallback (unwritable under the
            // sandbox). Also recovers users migrating from the old, broken
            // ~/Music default whose bookmark save silently failed. (DOLL-344)
            Self.log.info("No saved output directory bookmark; using in-container default")
            useDefaultOutputDir()
            return
        }
        do {
            var isStale = false
            let url = try URL(
                resolvingBookmarkData: data,
                options: .withSecurityScope,
                relativeTo: nil,
                bookmarkDataIsStale: &isStale
            )
            // Access is deliberately held, not scoped with `defer`: the engine writes
            // into this directory for the rest of the session. It is released by
            // releaseOutputDirAccess() (quit / switch to the default dir) or replaced
            // by saveOutputDirBookmark(for:), which stops the previous URL first.
            // swiftlint:disable:next security_scoped_balance - held for the session; stopped in releaseOutputDirAccess() / saveOutputDirBookmark(for:)
            if url.startAccessingSecurityScopedResource() {
                securityScopedURL = url
                bridge.setConfig(["output_dir": url.path])
                UserDefaults.standard.set(url.path, forKey: SettingsKeys.lastOutputDirPath)
                Self.log.info("Restored output directory: \(url.path)\(isStale ? " (stale, refreshing)" : "")")
                // DOLL-379: only refresh a stale bookmark when access actually
                // succeeded — otherwise we'd persist a .withSecurityScope
                // bookmark for a URL whose scope was never acquired.
                // Rewrite only the stored bookmark data: saveOutputDirBookmark
                // would stop access on securityScopedURL, which is this URL,
                // leaving the engine without access for the rest of the launch.
                if isStale {
                    do {
                        try storeOutputDirBookmark(for: url)
                    } catch {
                        // The resolved bookmark still works this launch; the next
                        // launch retries the refresh.
                        Self.log.warning("Failed to refresh stale bookmark: \(error.localizedDescription)")
                    }
                }
            } else {
                // DOLL-379: access failed. Drop the unusable bookmark so it
                // doesn't re-trigger this prompt every launch, and return so we
                // can't fall through to the stale re-save above (which would
                // re-persist a bookmark for an unscoped URL and point the engine
                // at an unwritable path).
                Self.log.warning("Failed to access security-scoped resource: \(url.path)")
                UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
                promptToReselectOutputDir(failedPath: url.path)
            }
        } catch {
            Self.log.error("Failed to restore bookmark: \(error.localizedDescription)")
            UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
            let failedPath =
                UserDefaults.standard.string(forKey: SettingsKeys.lastOutputDirPath)
                ?? String(localized: "the configured directory")
            promptToReselectOutputDir(failedPath: failedPath)
        }
    }

    /// Show an alert asking the user to re-select their output directory when
    /// a security-scoped bookmark can no longer be resolved (e.g. volume unmounted).
    ///
    /// Runs synchronously inside the bookmark-restore Task, which already runs
    /// after init, so the menu bar is up. It used to spawn its own Task and
    /// return, which let `bookmarkRestoreTask` finish while the alert was
    /// still pending: auto-record then started on Rust's relative
    /// "recordings" default, unwritable in the sandbox. Every outcome now
    /// points the engine at a usable folder before auto-record proceeds.
    private func promptToReselectOutputDir(failedPath: String) {
        let alert = NSAlert()
        alert.messageText = String(localized: "Output Directory Unavailable")
        alert.informativeText = String(
            localized:
                "BlackBox can no longer access \"\(failedPath)\". Please select a new output directory, or use the default location."
        )
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "Choose Directory\u{2026}"))
        alert.addButton(withTitle: String(localized: "Use Default"))
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            let panel = NSOpenPanel()
            panel.canChooseDirectories = true
            panel.canChooseFiles = false
            panel.canCreateDirectories = true
            panel.prompt = String(localized: "Select")
            panel.message = String(localized: "Select output directory for recordings")
            if panel.runModal() == .OK, let url = panel.url {
                saveOutputDirBookmark(for: url)
                return
            }
        }
        // "Use Default", or the picker was cancelled (which used to leave the
        // engine on the unwritable relative default): the in-container
        // default, which needs no security scope. The old ~/Music default is
        // unwritable under the sandbox. (DOLL-344)
        useDefaultOutputDir()
    }

    /// Release security-scoped resource access.
    func releaseOutputDirAccess() {
        securityScopedURL?.stopAccessingSecurityScopedResource()
        securityScopedURL = nil
    }
}
