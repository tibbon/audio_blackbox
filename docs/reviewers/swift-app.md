# Swift app reviewer

Runs when: the branch touches `BlackBoxApp/BlackBoxApp/` (Swift sources, `Info.plist`,
`PrivacyInfo.xcprivacy`, `Localizable.xcstrings`), `BlackBoxApp/BlackBox.entitlements`,
`BlackBoxApp/project.yml`, or `BlackBoxApp/Guardrails.xcconfig`. Routing lives in
`.claude/workflows/ship-ticket.js`.

## In your lane
- Isolation (§3.1): the module defaults to `MainActor` (`Guardrails.xcconfig`), so anything that
  runs off main says `nonisolated` explicitly and states why. `@unchecked Sendable`,
  `nonisolated(unsafe)`, `Task.detached`, `MainActor.assumeIsolated`, and `unsafeBitCast` each need
  a same-file SAFETY comment that is true. Several added at once to quiet the compiler is high.
- Hops and reentrancy (§3.1): disk and CPU work off the main actor, UI mutation on it; state read
  before an `await` may be stale after it (look for `await` between check and act, especially
  around `start()`, mic permission, and bookmark restore); long-lived tasks check cancellation and
  actually stop; `Task {}` only when an owner stores and cancels it; `.task {}` in views.
- No GCD, `Timer.scheduledTimer`, `NSLock`, or `ObservableObject` in new code (§3.1, §3.3).
  Locks are `Mutex` (Synchronization) or `OSAllocatedUnfairLock` and never held across `await`.
- Lifetime (§3.2): closures registered with `NotificationCenter`, `NSWorkspace`, or event monitors
  capture `[weak self]` and their tokens are stored and removed; a class owning a handle,
  observer, or file descriptor has a reachable `deinit`.
- SwiftUI and AppKit (§3.3): macOS idioms (`MenuBarExtra`, `Settings`, `commands {}`,
  `.keyboardShortcut`); `body` does no decoding, formatting, sorting, or Rust calls; `ForEach`
  uses stable identity; no `AnyView`; `@Observable` models with `@State` for view-local values
  and `@Bindable` to pass them down; two-parameter or zero-parameter `onChange`,
  `.foregroundStyle`.
- Settings: keys live in `SettingsKeys.swift`; `@AppStorage` and `UserDefaults` hold small values
  only, never secrets or blobs.
- Localization and accessibility (§3.3): every user-facing string goes through the String Catalog;
  new controls, images, and the meter have accessibility labels and values; keyboard navigation
  still works; number, byte, and rate formatting stays locale-aware.
- Sandbox (§3.6): file access outside the container goes through the picker and security-scoped
  bookmarks; `startAccessingSecurityScopedResource()` has its result checked and is balanced by a
  stop (`ARCHITECTURE.md` "Security-scoped bookmark lifecycle"); a stale bookmark prompts a re-pick.
  Entitlements stay the minimum set (sandbox, audio input, user-selected read-write, app-scope
  bookmarks); each addition is justified in the PR. TCC denial is handled without a crash.
- Errors and logging (§3.5): `try?` that hides a failure the user should see is a bug; user-facing
  errors are `BlackBoxError` with localized descriptions; `os.Logger` per category; interpolated
  user data stays private.
- Polling (§3.7): the meter polls `fillPeakLevels(into:)` at about 30 Hz only while the meter
  window is open and visible and the engine is recording or monitoring; status reads go through
  `blackbox_get_status_flags`, never a path that takes the recorder mutex.
- Sleep and wake: changes to `SleepWakePolicy` or its handlers keep the matrix in `ARCHITECTURE.md`
  ("Sleep / wake matrix"), including which stop reasons clear `wasSleepInterrupted` (DOLL-182,
  DOLL-442).
- `project.yml` edits keep language, concurrency, and warning keys in `Guardrails.xcconfig`; the
  `settings` block overrides the xcconfig, so a key added there silently wins.

## Not your lane
- Pointer handling and handle ownership in `RustBridge.swift` and the Carbon callback:
  `ffi-unsafe`. XCTest quality: `tests`. Fastlane, metadata, versions: `build-release`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: a data race or crash on a user path; recordings written outside the sandbox grant; a
  secret or user path logged publicly.
- high: auto-record, resume on wake, or stop behaving wrongly in normal use; a UI mutation off the
  main actor forced through an unsafe escape; an entitlement added without need.
- medium: an observer or task that leaks or outlives its owner; an unbalanced security-scoped
  access; a new string missing from the catalog path; a control without an accessibility label.
- low: view structure, naming, modifier order.

## Facts that prevent false positives
- `RustBridge` is a `nonisolated final class`, not `Sendable`, and frees its handle in `deinit`;
  its doc comment explains why that is sound.
- `GlobalHotkeyManager` uses `MainActor.assumeIsolated` in the Carbon callback (DOLL-161), which
  Carbon delivers on the main run loop.
- Pure helpers (`SleepWakePolicy`, `SettingsKeys`, the channel-spec functions) and XCTest classes
  are `nonisolated` on purpose so nonisolated tests can call them.
- `AppDelegate` cancels termination when the last window closes unless the user chose Quit; that
  is the menu-bar-app design, not a bug.
- The app targets macOS 15 or later, arm64 only (DOLL-463). `Mutex` from Synchronization is
  available.
