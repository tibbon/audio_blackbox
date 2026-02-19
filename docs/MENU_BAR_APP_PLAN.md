# BlackBox Menu Bar App — Implementation Plan

> Synthesized from research by UX/UI, App Store, Architecture, and Security teams.

## Executive Summary

Ship BlackBox Audio Recorder as a **$9.99 paid upfront** macOS menu bar app on the Apple App Store, using a **Swift/SwiftUI frontend + Rust library via C FFI** architecture. The existing CLI mode stays untouched. The menu bar app provides a native, professional UI for the same proven Rust audio engine (112 tests, lock-free ring buffer, 121x realtime on Apple Silicon).

**Key differentiators at the $10 price point:**
- Multi-channel recording (1–64+ channels) — no competitor offers this
- Continuous recording with automatic file rotation
- Split recording (one WAV per channel)
- Silence detection ("Smart Recording")

**Tagline:** *"Always-on audio recording for your Mac."*

---

## Architecture Decision

### SwiftUI + Rust FFI (Option B from architecture research)

The current Cocoa/objc Rust bindings in `src/bin/macos/` are fundamentally broken:
- No action dispatch (menu clicks go nowhere)
- Wrong threading model (UI spawned on background thread)
- Memory leaks (`Arc::into_raw` without corresponding `Arc::from_raw`)
- Based on unmaintained crates (`cocoa` last significant release: 2022)

**SwiftUI gives us:**
- Native dark mode, SF Symbols, accessibility for free
- `MenuBarExtra` for a proper menu bar app in ~30 lines of Swift
- Standard Preferences window via SwiftUI `Settings` scene
- Xcode project with proper code signing, entitlements, sandboxing
- App Store review compatibility

**The Rust library is untouched** — only a thin C FFI layer is added (~8 functions).

### Binary Structure
- **CLI**: `cargo build` → `blackbox` binary (unchanged, no Swift dependency)
- **App**: Xcode builds `BlackBox Audio Recorder.app` with `libblackbox.a` statically linked
- **Both share the same Rust library** — identical audio engine, different frontends

---

## Phase 0: Foundation (Pre-UI)

### 0.1 Create C FFI layer in Rust

**File: `src/ffi.rs`** (new)

Expose a minimal C API using opaque handle pattern:

```
blackbox_create(config_json: *const c_char) -> *mut BlackboxHandle
blackbox_start_recording(handle) -> i32
blackbox_stop_recording(handle) -> i32
blackbox_is_recording(handle) -> bool
blackbox_get_status_json(handle) -> *const c_char
blackbox_list_input_devices() -> *const c_char  (returns JSON array)
blackbox_set_config_json(handle, json: *const c_char) -> i32
blackbox_destroy(handle)
```

`BlackboxHandle` wraps `AudioRecorder<CpalAudioProcessor>` + `AppConfig`. All FFI functions wrapped in `catch_unwind` — panics must never cross the FFI boundary.

**File: `src/lib.rs`** — Add `pub mod ffi;` behind `#[cfg(feature = "ffi")]` feature flag.

**File: `Cargo.toml`** — Add `crate-type = ["lib", "staticlib"]` and `ffi = []` feature.

**File: `include/blackbox_ffi.h`** (new) — C header for Swift to import.

### 0.2 Add `input_device` to AppConfig

**File: `src/config.rs`** — Add `input_device: Option<String>` field. TOML key: `input_device`. Env var: `BLACKBOX_INPUT_DEVICE`. When `None`, use system default (current behavior).

**File: `src/cpal_processor.rs`** — Use `input_device` config to select the cpal device by name instead of always using default.

### 0.3 Tests for FFI layer

**File: `src/tests/ffi_tests.rs`** (new) — Test create/destroy lifecycle, config round-trip, device listing, start/stop recording, error handling, JSON serialization.

### Verification
```bash
cargo test
cargo clippy --all-targets --no-default-features -- -D warnings
cargo build --release --features ffi  # produces libblackbox.a
```

---

## Phase 1: Minimal Viable Menu Bar App

Goal: A working menu bar app that can start/stop recording. No preferences window yet. Distribute outside App Store (no sandbox) for rapid iteration.

### 1.1 Create Xcode project

```
BlackBoxApp/
├── BlackBoxApp.xcodeproj/
├── BlackBoxApp/
│   ├── BlackBoxApp.swift          # @main, MenuBarExtra
│   ├── RecordingState.swift       # ObservableObject wrapping FFI
│   ├── RustBridge.swift           # Swift wrapper around C functions
│   ├── Assets.xcassets/           # App icon, SF Symbols
│   └── Info.plist
├── bridge/
│   ├── blackbox_ffi.h             # C header (copied from include/)
│   └── module.modulemap
```

### 1.2 SwiftUI Menu Bar App

**BlackBoxApp.swift:**
```swift
@main
struct BlackBoxApp: App {
    @StateObject var recorder = RecordingState()

    var body: some Scene {
        MenuBarExtra("BlackBox",
            systemImage: recorder.isRecording ? "record.circle.fill" : "record.circle") {
            // Status line
            Text(recorder.statusText)
            Divider()
            // Primary action
            Button(recorder.isRecording ? "Stop Recording" : "Start Recording") {
                recorder.toggle()
            }
            .keyboardShortcut("r")
            Divider()
            Button("Show Recordings in Finder") { recorder.openOutputDir() }
            Divider()
            Button("Quit") { NSApplication.shared.terminate(nil) }
        }
    }
}
```

### 1.3 Build integration

**Makefile additions:**
- `make rust-lib` — `cargo build --release --features ffi` (produces `libblackbox.a`)
- `make swift-app` — `xcodebuild` linking `libblackbox.a`
- `make app` — both combined
- `make run-app` — build + open the .app

### 1.4 Menu bar icon states

| State | SF Symbol | Color |
|-------|-----------|-------|
| Idle | `circle` | Default (template) |
| Recording | `record.circle.fill` | Red tint |
| Error | `exclamationmark.circle` | Yellow |

### Verification
- App appears in menu bar
- Click Start Recording → icon changes, WAV file created
- Click Stop Recording → icon reverts, WAV finalized
- Recordings appear in default output directory
- CLI mode still works: `cargo run`

---

## Phase 2: Preferences Window

### 2.1 Three-tab Settings window

**Tab 1: Recording** (microphone icon)
- Input device dropdown (populated from `blackbox_list_input_devices()`)
- Channel selection: checkbox grid for ≤8 channels, text field for manual entry
- Output mode: Single File / Split radio buttons
- Silence detection: toggle + threshold slider

**Tab 2: Output** (folder icon)
- Output directory with "Choose..." button (NSOpenPanel)
- Continuous recording toggle + rotation cadence
- File naming pattern with live preview

**Tab 3: General** (gear icon)
- Menu bar icon style (circle vs microphone)
- Show duration in menu bar toggle
- Launch at login toggle (`SMAppService`)
- Auto-record on launch toggle
- Debug/performance logging toggles

### 2.2 Settings persistence

- SwiftUI `@AppStorage` for UI preferences (icon style, launch at login)
- Rust-side `AppConfig` for audio settings, synchronized via FFI JSON
- Settings saved to app container when sandboxed, or `~/.config/blackbox/` when not

### 2.3 Device selection

Add a "device changed" callback from Swift to Rust FFI. Changing device while recording shows a confirmation alert: "Changing input device will stop the current recording. Continue?"

### 2.4 Recent recordings submenu

Track last 5 recordings in `RecordingState`. Show in menu dropdown with "Open in Finder" and "Play" options.

### Verification
- All preferences persist across app restarts
- Changing settings while recording shows appropriate warnings
- Channel selection works for both checkbox grid and manual text entry
- Output directory picker works and persists

---

## Phase 3: App Store Preparation

### 3.1 Entitlements file

**File: `BlackBoxApp/BlackBox.entitlements`**
```xml
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.device.audio-input</key>
    <true/>
    <key>com.apple.security.files.user-selected.read-write</key>
    <true/>
    <key>com.apple.security.files.bookmarks.app-scope</key>
    <true/>
</dict>
```

### 3.2 Security-scoped bookmarks

When user selects output directory via NSOpenPanel:
1. Create bookmark data from the URL
2. Store in UserDefaults
3. On next launch, resolve bookmark → `startAccessingSecurityScopedResource()`
4. On quit → `stopAccessingSecurityScopedResource()`

This gives persistent write access to the user-chosen directory across app launches.

### 3.3 Microphone permission handling

**Critical (cpal gotcha):** Three things ALL required:
1. `NSMicrophoneUsageDescription` in Info.plist
2. `com.apple.security.device.audio-input` in entitlements
3. Entitlements embedded in code signature via `codesign --entitlements`

Missing any one = silent failure, no permission prompt.

Add permission state checking in Swift:
- `.notDetermined` → first recording attempt triggers system prompt
- `.authorized` → proceed normally
- `.denied` → show alert with "Open System Settings" button
- `.restricted` → show "restricted by administrator" message

### 3.4 Privacy manifest

**File: `BlackBoxApp/PrivacyInfo.xcprivacy`**
```xml
<dict>
    <key>NSPrivacyTracking</key>
    <false/>
    <key>NSPrivacyTrackingDomains</key>
    <array/>
    <key>NSPrivacyCollectedDataTypes</key>
    <array/>
    <key>NSPrivacyAccessedAPITypes</key>
    <array/>
</dict>
```

App Store privacy label: **"Data Not Collected"** (all recordings stay on device, no network access).

### 3.5 Universal binary

```bash
cargo build --release --features ffi --target=aarch64-apple-darwin
cargo build --release --features ffi --target=x86_64-apple-darwin
lipo -create \
  target/aarch64-apple-darwin/release/libblackbox.a \
  target/x86_64-apple-darwin/release/libblackbox.a \
  -output target/universal/libblackbox.a
```

### 3.6 Minimum macOS version

**Target: macOS 13 (Ventura)** — required for SwiftUI `MenuBarExtra`. This is reasonable for a new app shipping in 2026.

### 3.7 App icon

Need a proper `.icns` file with all required sizes (16x16 through 1024x1024). Design: a circle/record button motif matching the menu bar icon aesthetic.

### 3.8 Hardened Runtime

Rust compiles to native machine code — no JIT, no unsigned executable memory. Hardened Runtime is compatible out of the box. No exception entitlements needed.

### 3.9 Code signing & submission

```bash
# Sign with Apple Distribution certificate
codesign -s "Apple Distribution: David Fisher (TEAMID)" \
  -f --timestamp -o runtime \
  --entitlements BlackBox.entitlements \
  "BlackBox Audio Recorder.app"

# Package and upload
productbuild --component "BlackBox Audio Recorder.app" /Applications \
  --sign "3rd Party Mac Developer Installer: David Fisher (TEAMID)" \
  BlackBox.pkg
xcrun altool --upload-app -f BlackBox.pkg -t macos -u "your@email.com" -p @keychain:AC_PASSWORD
```

### Verification
- App works correctly in sandbox (test on clean macOS install)
- Microphone permission prompt appears on first launch
- Output directory persists across launches via bookmarks
- `codesign -dv --verbose=4` shows runtime flag and correct entitlements
- All 112+ existing tests still pass
- CLI mode completely unaffected

---

## Phase 4: Polish & Ship

### 4.1 App Store listing

- **Price**: $9.99 (Small Business Program → 15% commission → $8.49 net)
- **Category**: Primary: Music, Secondary: Utilities
- **Age rating**: 4+
- **Screenshots**: 2880x1800, showing menu bar dropdown and preferences window
- **Description**: Highlight multi-channel, continuous recording, silence detection
- **Privacy policy**: Simple page stating no data collection (host on GitHub Pages)

### 4.2 Dual distribution

Ship both:
- **Mac App Store** — sandboxed, discovery, trusted
- **Direct download** (notarized DMG) — no sandbox restrictions, for power users

### 4.3 Launch at login

Use `SMAppService.mainApp` (macOS 13+) — the modern way to register as a login item. No LaunchAgent daemon (that would be rejected by App Store review).

### 4.4 First launch experience

No onboarding wizard. App is ready to record with sensible defaults:
- Default device: system default input
- Channels: 0 (mono)
- Output: app container or ~/Downloads
- Mode: single file
- First-time notification: "BlackBox is ready. Click the menu bar icon or press ⌘R to start recording."

### 4.5 Error handling UX

- Device disconnected while recording → notification + stop recording gracefully
- Disk full → notification + stop recording, don't lose what was written
- Permission denied → clear guidance to System Settings

---

## Files Changed Summary

| File | Action | Phase | Description |
|------|--------|-------|-------------|
| `src/ffi.rs` | **Create** | 0 | C FFI exports (~8 functions) |
| `src/lib.rs` | Modify | 0 | Add `pub mod ffi` behind feature flag |
| `src/config.rs` | Modify | 0 | Add `input_device` field |
| `src/cpal_processor.rs` | Modify | 0 | Device selection by name |
| `src/tests/ffi_tests.rs` | **Create** | 0 | FFI integration tests |
| `include/blackbox_ffi.h` | **Create** | 0 | C header for Swift bridge |
| `Cargo.toml` | Modify | 0 | Add `staticlib` crate-type, `ffi` feature |
| `BlackBoxApp/` | **Create** | 1 | Entire Xcode project directory |
| `BlackBoxApp/BlackBoxApp.swift` | **Create** | 1 | @main App entry, MenuBarExtra |
| `BlackBoxApp/RecordingState.swift` | **Create** | 1 | ObservableObject FFI bridge |
| `BlackBoxApp/RustBridge.swift` | **Create** | 1 | Swift wrapper for C functions |
| `BlackBoxApp/SettingsView.swift` | **Create** | 2 | 3-tab Preferences window |
| `BlackBoxApp/BlackBox.entitlements` | **Create** | 3 | Sandbox + mic + file entitlements |
| `BlackBoxApp/PrivacyInfo.xcprivacy` | **Create** | 3 | Privacy manifest |
| `BlackBoxApp/Info.plist` | **Create** | 1 | App metadata, mic description |
| `Makefile` | Modify | 1 | Add `rust-lib`, `swift-app`, `app` targets |
| `.github/workflows/rust.yml` | Modify | 1 | Add macOS Xcode build job |

### Files to Remove (Phase 1+, after Swift UI works)
- `src/bin/macos/mod.rs` — replaced by Swift UI
- `src/bin/macos/safe_cocoa.rs` — replaced by Swift UI
- Eventually: `cocoa`, `objc`, `core-foundation`, `core-graphics`, `libc` dependencies

### Files NOT Changed
- `src/bin/main.rs` — CLI mode untouched
- `src/audio_processor.rs` — trait unchanged
- `src/audio_recorder.rs` — unchanged
- `src/writer_thread.rs` — unchanged
- `src/bin/bench_writer.rs` — unchanged
- All existing tests — unchanged

---

## Risk Mitigation

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| FFI boundary bugs | Medium | High | `catch_unwind` on all FFI functions, extensive tests, JSON for complex data |
| Two build systems (Cargo + Xcode) | Low | Medium | Makefile orchestrates both; CI uses macOS runner |
| Thread safety across FFI | Medium | High | Rust owns all audio threads; Swift only calls FFI from main actor |
| App Store rejection | Low | Medium | Architecture and entitlements follow proven patterns; Rust apps pass review |
| Sandbox file access issues | Medium | Medium | Security-scoped bookmarks; test early on clean install |
| cpal silent mic failure | High (if signing wrong) | Critical | Triple-check: Info.plist + entitlements + embedded in signature |

---

## Competitive Positioning

| Feature | Voice Memos (Free) | Recordia ($7) | Piezo ($29) | BlackBox ($10) |
|---------|-------------------|---------------|-------------|----------------|
| Menu bar app | No | Yes | No (window) | Yes |
| Multi-channel (3+) | No | No | No | **Yes (1–64+)** |
| Continuous recording | No | No | No | **Yes** |
| Split per channel | No | No | No | **Yes** |
| Silence detection | No | No | No | **Yes** |
| File rotation | No | No | No | **Yes** |
| App Store | N/A | Yes | No | **Yes** |
| Price | Free | $7 | $29 | **$10** |

BlackBox occupies a unique position: professional multi-channel recording capability at an indie price point, delivered through a simple menu bar interface. No other $10 app offers 64-channel recording with continuous mode and per-channel splitting.
