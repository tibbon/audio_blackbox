# Review checklist and agent guardrails

Scope: this app — a SwiftUI/AppKit menu-bar shell over a Rust core behind a hand-maintained
C ABI, with a real-time CoreAudio capture path. Written for a coding agent as much as for a
human reviewer. The lint configs enforce what can be enforced mechanically; this file covers
what cannot, plus the things the lints only flag for a human to judge.

Mechanical enforcement lives in:

- `Cargo.toml [lints.*]` — lint levels (rustc, rustdoc, clippy). Groups at priority -1, then
  individual entries. The "Backlog (DOLL-653)" block lists lints parked as `allow` with counts.
- `clippy.toml` — thresholds, test-only relief, and the hard bans (`thread::sleep`, `home_dir`,
  `partial_cmp`) with the reason each diagnostic prints.
- `deny.toml` — advisories, license allow-list, duplicate and wildcard bans, sources.
- `.swift-format` / `.swiftlint.yml` — Swift formatting and correctness rules, including the
  custom rules for things no built-in rule catches (`print`, GCD hops, `Task.detached`,
  `assumeIsolated`, escaping `withUnsafe*` pointers, `.foregroundColor`, `NavigationView`, ...).
- `BlackBoxApp/Guardrails.xcconfig` — Swift 6 language mode, complete strict concurrency,
  `MainActor` default isolation, warnings as errors, hardening floor.
- `scripts/check.sh` — the full loop. `make check` runs it. Green means done.

`docs/reviewers/` splits this checklist into review lanes. The `/ship-ticket` workflow runs them
until review converges (AGENTS.md "Workflow").

## 0. Rules of engagement for agents

- Run `make check` (or `scripts/check.sh rust` / `swift` for one side) before claiming anything
  is done. "It compiles" is not done. Paste the last lines of its output in the PR.
- Never silence a lint to make it pass. Rust: `#[expect(lint, reason = "...")]` only, never
  `#[allow]` (`clippy::allow_attributes` is deny). Package-wide policy goes in `Cargo.toml` with a
  comment, not in source. Swift: `// swiftlint:disable:next rule - reason`, never `disable all`,
  and the reason states the invariant, not "lint is wrong".
- Never weaken or delete a test to make a change pass. If a test is wrong, say so in the PR
  description and change it in its own commit.
- Never add a dependency to avoid writing twenty lines. New deps pass `cargo deny`, regenerate
  `ACKNOWLEDGMENTS.md` (`python3 scripts/gen-acknowledgments.py`), and get a one-line
  justification in the PR.
- Never change a public API, WAV/file format, buffer layout, `StatusFlags`, or an FFI signature
  without calling it out explicitly in the PR description and updating `include/blackbox_ffi.h`
  by hand in the same commit (`make check-ffi-header`).
- Match the surrounding code's idioms. The crate uses `thiserror` + `BlackboxError`; do not
  introduce `anyhow`. The app uses `@Observable`; do not add `ObservableObject`.
- Use current APIs. Training data is stale. "Stop and check" signals: `NavigationView`,
  `.foregroundColor`, single-parameter `onChange`, `ObservableObject` for new models,
  `Thread`/GCD for new concurrency, `NSLock` in new code, `UIKit` anything.
- When unsure whether something is safe (unsafe block, `@unchecked Sendable`, the audio
  callback), write the invariant down as a `// SAFETY:` comment. If you cannot state the
  invariant, the code is not ready.
- Reference the Linear ticket (`DOLL-N`) in comments only where the diff alone does not explain
  why; see `docs/decisions.md` for the map.

## 1. Rust core

### 1.1 Errors and panics
- Library code never panics on bad input. `unwrap`, `expect`, `panic!`, `todo!`,
  `unimplemented!`, and `[]` indexing on external data fail review outside tests
  (`unwrap_used` is deny; tests are exempt via `clippy.toml`).
- Every fallible public fn returns `Result<_, BlackboxError>`. `Box<dyn Error>` only in
  binaries and tests.
- Errors carry enough context to act on (what failed, with what input) and are safe to log
  (no secrets, no full file contents). Parse errors include the source error (`map_err_ignore`).
- `#[must_use]` on builders, handles, and anything whose dropped result is a bug.
- Panics that can still happen are documented under `# Panics`. There is no `catch_unwind`
  on the FFI path by design: release builds use `panic = "abort"` so a production panic
  surfaces as a crash report instead of unwinding across the ABI (DOLL-90). Do not add one.

### 1.2 unsafe
- Every `unsafe` block has a `// SAFETY:` comment naming the invariant and does one operation
  (`undocumented_unsafe_blocks` is deny; `multiple_unsafe_ops_per_block` warns).
- `unsafe fn` bodies use explicit inner `unsafe {}` blocks (`unsafe_op_in_unsafe_fn`).
- Every `unsafe fn` and every raw-pointer-taking public fn has a `# Safety` doc section.
- No `mem::forget`; use `ManuallyDrop` or `Box::into_raw`/`Arc::into_raw` with the matching
  `from_raw` documented at both sites (see `macos_sample_rate_listener.rs` for the one
  deliberate leak and why).
- No `transmute` where a cast or `from_ne_bytes` works. Pointer casts use `.cast()` /
  `.cast_mut()` / `.cast_const()`, never `as`, and never change alignment.
- 2024 edition: foreign items live inside `unsafe extern "C" { }`, exports use
  `#[unsafe(no_mangle)]`.

### 1.3 Ownership and performance
- A `clone()` added to satisfy the borrow checker is a review flag: ask what the ownership
  should be. `Arc::clone(&x)`, never `x.clone()`, so the refcount bump is visible.
- `Arc<Mutex<T>>` is not a default architecture. Ask who owns it, who mutates it, and whether
  there is exactly one writer. The FFI handle's lock order is documented in `src/ffi.rs`
  (DOLL-124); never take two inner locks in a new order.
- Shared buffers are `Arc<[T]>`, not `Arc<Vec<T>>` (`rc_buffer`).
- Hot paths take `&[T]`/`&mut [T]`, not `Vec<T>`. Iterate with `zip`, `chunks_exact`,
  `iter_mut` instead of index arithmetic so bounds checks hoist and loops vectorize.
- No allocation inside per-sample loops. Preallocate in `new()`; `src/tests/alloc_tests.rs`
  wraps the global allocator in a counter and asserts zero allocations on the hot path —
  extend it when you add a hot-path feature.
- Large arrays are not on the stack (`large_stack_arrays`, `large_stack_frames`); the audio
  callback and writer thread have small stacks.
- `HashMap` iteration order never reaches an output (`iter_over_hash_type`).
- Float comparisons use tolerances; NaN handling is explicit; sorting with
  `partial_cmp().unwrap()` is banned (use `total_cmp`).

### 1.4 Numeric and DSP code
- Every `as` cast is reviewed for truncation and sign. Length/size conversions for the C ABI
  use `try_from` and return an error (`cast_sign_loss`, `cast_possible_wrap` warn;
  `cast_possible_truncation` / `cast_precision_loss` are parked in the DOLL-653 backlog).
- Constants carry units in their names or docs (`sample_rate_hz`, `gate_timeout_secs`).
- Sample conversion and silence detection have reference tests with the tolerance stated in
  the test. No `assert_eq!` on floats.

### 1.5 Concurrency
- `Send`/`Sync` are derived, not hand-written. A manual `unsafe impl Send` needs a SAFETY
  comment that names the synchronization on the Swift side too.
- No `std::thread::spawn` in library code without a shutdown path (owned `JoinHandle`,
  cancellation flag, join on drop). Join results are checked, not `let _`-dropped.
- No lock held across a call back into Swift (deadlocks with the main actor).
- `thread::sleep` is banned. The two production exceptions (writer-thread idle backoff,
  DOLL-270; CLI status tick) carry an `#[expect]` with the reason. Tests wait on state with
  the helpers in `src/test_utils.rs`, not on time.
- Atomic ordering follows `ARCHITECTURE.md § Atomic ordering` (DOLL-101).

### 1.6 API surface, docs, layout
- `pub` means "Swift, a binary, or another crate calls this". Everything else is
  `pub(crate)` (`unreachable_pub`). Public types implement `Debug`.
- `cargo doc` builds with `-D warnings`. Doc comments say what and why, not the item's name.
- Integration tests live in `src/tests/`; the inline `mod tests` in `src/lib.rs` is for tiny
  smoke tests only.

### 1.7 Testing
- Tests go through the public API. Tests that reach into private state to assert
  implementation details are flagged.
- No `sleep` for synchronization in tests (see 1.5).
- The FFI test module calls every exported function with null, zero, and oversized inputs and
  asserts error codes, not crashes. Extend it when you add an export.

### 1.8 Dependencies and build
- `cargo deny check` (advisories, licenses, bans, sources) and `cargo machete` are clean.
- MSRV is `rust-version` in `Cargo.toml`; CI pins it and `scripts/check.sh` checks it when the
  toolchain is installed. Bump it deliberately, in its own commit, with the CI ref.
- The app is arm64-only by decision (DOLL-463); do not add universal builds.
- `panic = "abort"` in release is an invariant (DOLL-90).

## 2. The FFI boundary (Rust <-> Swift)

Most crashes in this architecture live here. Review every change as if it were a network
protocol.

- One header, `include/blackbox_ffi.h`, hand-maintained (no cbindgen), checked by
  `scripts/check-ffi-header.sh` (DOLL-190). Every `pub extern "C" fn` added or removed is
  mirrored there in the same commit.
- Ownership is stated per function in the doc comment: who allocates, who frees, whether the
  pointer is valid after return.
- Handles are opaque on the C side and `Box<BlackboxHandle>` on the Rust side;
  `blackbox_create` / `blackbox_destroy` are paired; destroy of null is a no-op. The magic word
  is a best-effort guard, not a safety mechanism: destroying a handle twice, concurrently or
  one after the other, can read freed memory. Never rely on it.
- Strings: Rust returns `*mut c_char`, freed only through `blackbox_free_string`; Swift copies
  immediately with `String(cString:)` then frees. `CString::new(x)?.as_ptr()` as one expression is a
  dangling pointer (`dangling_pointers_from_temporaries` is deny).
- Buffers: `(ptr, len)` pairs; `len` is in elements unless the name says bytes; both sides
  check `len` before touching memory.
- Thread contract per function, written in the header comment: "main thread only",
  "any thread, not reentrant", or "realtime safe". Existing exports predate this rule and
  have none yet; every new or changed export states one.
- Errors: integer codes (`BLACKBOX_ERR_*`) plus `blackbox_get_last_error`. Swift maps codes to
  one typed `Error` enum in one place (`RustBridge.swift`).
- `#[repr(C)]` on every crossing type; `StatusFlags` has compile-time size and per-field offset
  asserts that name the header (DOLL-354). No data-carrying enums, no `Option<T>` except
  `Option<&T>` / `Option<NonNull<T>>` / `Option<extern "C" fn>`, no `String`, `Vec`, `&str`,
  `&[T]`.
- Swift wrappers:
  - `final class` owning the handle; `deinit` calls free; `Sendable` only with a `// SAFETY:`
    comment naming the Rust-side synchronization.
  - Every Swift `Array`/`Data`/`String` crosses inside `withUnsafe...`/`withCString`; the
    pointer never escapes the closure (custom lint catches `{ $0 }` / `{ $0.baseAddress }`).
  - `withExtendedLifetime(self)` around calls that use a pointer derived from `self`.
  - `MemoryLayout<T>.stride`, not `.size`, for element math.

## 3. Swift

### 3.1 Concurrency (Swift 6 mode, MainActor default isolation)
- `@unchecked Sendable`, `nonisolated(unsafe)`, `Task.detached`, `MainActor.assumeIsolated`,
  `unsafeBitCast` each need a same-file `// SAFETY:` justification. A PR that adds several to
  quiet the compiler is rejected.
- CPU-bound and disk work runs off the main actor (`@concurrent` functions or a nonisolated
  async func), UI mutation on the main actor. Check the direction of every hop.
- Structured over unstructured: `.task {}` in views (auto-cancelled), `async let`/`TaskGroup`
  for fan-out, `Task {}` only when an owner stores and cancels it.
- Long-lived tasks check `Task.isCancelled`, and cancellation actually stops the work.
- Actor reentrancy: state read before an `await` may be stale after it. Look for `await`
  between "check" and "act".
- No GCD in new code except at a legacy boundary (CoreAudio callbacks, Carbon event handlers,
  DOLL-161). `DispatchQueue.main.async` inside an async context is a bug.
- Locks: `Mutex` from `Synchronization` (macOS 15+) or `OSAllocatedUnfairLock`. Never held
  across an `await`.
- Timers: `.task { while !Task.isCancelled { try await Task.sleep(...) } }` or `TimelineView`,
  not `Timer.scheduledTimer`.

### 3.2 Memory and lifetime
- Closures stored on `self` or registered with system objects (`NotificationCenter`, KVO,
  `NSEvent.addLocalMonitor`) capture `[weak self]` and are removed in `deinit` or held in a
  token array. Observer tokens are never discarded.
- Any class that owns a Rust handle, file descriptor, or observer has a `deinit`, and that
  `deinit` is reachable (no retain cycle).
- Large data (sample buffers, `Data`) is not copied per frame.

### 3.3 SwiftUI and AppKit on macOS
- macOS idioms, not iOS: `Settings` scene, `commands {}` for menus, `.keyboardShortcut`,
  `MenuBarExtra`, `NSHostingView` / `NSViewRepresentable` for AppKit content. Any `UIKit`
  import fails lint.
- `body` is cheap: no decoding, sorting, formatting, or Rust calls in `body`. Precompute in the
  model and pass values.
- `ForEach` uses stable identity, never indices. `.id()` is not used to force refreshes.
- No `AnyView`. `@ViewBuilder`, `Group`, generics instead.
- `@Observable` models; `@State` only for view-local value state; `@Bindable` to pass
  observables down; `@Environment` for dependencies.
- `onChange(of:)` two-parameter or zero-parameter form, `.foregroundStyle`. Deprecated API is a
  blocker, not a warning to ignore.
- Every user-facing string is localizable (the String Catalog check in CI enforces the
  catalog is in sync); every image control has an accessibility label; keyboard navigation
  works.
- `@AppStorage`/`UserDefaults` for small settings only: no secrets, no blobs. Keys live in
  `SettingsKeys.swift`.

### 3.4 Pointers and C interop
- `withUnsafe*` results never escape the closure.
- `bindMemory` on memory already bound to another type is UB. Prefer `withMemoryRebound` or
  `UnsafeRawBufferPointer.load(as:)`.
- `@convention(c)` closures capture nothing; context goes through the raw pointer parameter.
- `Unmanaged` retain/release is balanced in the same type; `passUnretained` only when the
  callee cannot outlive the call, with the SAFETY comment saying who unregisters first.

### 3.5 Errors and logging
- Errors are typed enums (`BlackBoxError`) with `LocalizedError` where shown to users. `try?`
  that hides a failure the user should see is a bug.
- `os.Logger` per subsystem/category; `print` fails lint. Interpolated values default to
  private; `privacy: .public` only for non-user data.
- `Task { try await ... }` without a `catch` fails lint (`unhandled_throwing_task`).

### 3.6 Sandbox, entitlements, signing
- App Sandbox and Hardened Runtime on. Entitlements are the minimum set
  (`BlackBox.entitlements`); each addition is justified in the PR.
- File access outside the container goes through `NSOpenPanel`/`fileImporter` and
  security-scoped bookmarks. `startAccessingSecurityScopedResource()` has its `Bool` checked
  and is balanced by `stopAccessingSecurityScopedResource()`.
- `NSMicrophoneUsageDescription` and friends exist for every TCC-protected resource; denial is
  handled without crashing.
- Secrets in Keychain. Nothing sensitive in `UserDefaults`, plists, or logs.

### 3.7 Performance
- Instruments before optimizing: Time Profiler, Allocations, Animation Hitches.
- The main thread does no file IO, decoding, or Rust processing longer than a millisecond.
  The status poll reads atomics through `blackbox_get_status`, never the recorder mutex.
- No polling beyond the documented status/meter cadence. Background work is cancelled when
  its window closes.

### 3.8 Testing
- New tests may use Swift Testing (`@Test`, `#expect`); existing files are XCTest. Never mixed
  in one file.
- `scripts/check.sh sanitize` runs TSan then ASan+UBSan locally; new concurrency code needs a
  TSan-clean run before it is called done (CI does not run sanitizers — minutes are scarce).
- FFI wrapper tests cover error paths and null/empty cases.
- UI logic is testable without views: models and reducers are plain types.

## 4. Real-time audio thread rules

Applies to Swift and Rust code reachable from the cpal/CoreAudio input callback.

- No allocation: no `Vec::push`, `String`, `Box::new`, `format!`, no heap-capturing closures.
  The callback only pushes `f32` samples into the `rtrb` ring; everything else happens on the
  writer thread.
- No blocking locks. `try_lock` only with a bounded fallback.
- No logging, no `os_log`/`print`, no `Task`, no `NotificationCenter`, no class allocation.
- No blocking IO, no `sleep`.
- UI communication via the lock-free SPSC ring and atomics; status is published through
  `Arc<AtomicBool>` / `AtomicU64` flags with the ordering rules in `ARCHITECTURE.md`.
- Buffers and state are preallocated when the stream is built; the callback only reads and
  writes them. `src/tests/alloc_tests.rs` is the proof — keep it passing.
- Swift never touches the callback path; it lives in Rust entirely.

## 5. Cross-cutting agent failure modes (what to grep for in review)

- Lint silencing: new `#[allow]`, `swiftlint:disable`, `@unchecked`, `nonisolated(unsafe)`,
  `try!`, `as!`, force unwraps. `#[expect]` without a reason is a compile error.
- Test tampering: loosened assertions, deleted tests, `XCTSkip`/`#[ignore]`, widened
  tolerances without a reason.
- Deprecated APIs or iOS idioms in macOS code.
- Copy-paste drift: two implementations of the same helper (Rust and Swift, or two Swift
  files). Ask which one is canonical. (`bench_writer.rs` deliberately copies
  `f32_to_wav_sample`; the comment says why.)
- Over-generalization: traits, generics, protocols with one implementer, introduced "for
  flexibility".
- Under-ownership: `clone()`, `Arc<Mutex<>>`, `[unowned self]`, `Any` casts used to avoid
  thinking about ownership.
- Ceremony instead of behavior: a "manager", "service", or "helper" that forwards calls
  without adding an invariant.
- Silent fallbacks: `?? default`, `unwrap_or_default()`, `try?`, `let _ = result` that turn
  errors into wrong-but-plausible output. Especially bad in sample conversion.
- Missing boundaries: Rust reading env vars or home paths outside `config.rs`; Swift assuming
  non-sandboxed file access.
- Stale contracts: a Rust struct or export changed without `include/blackbox_ffi.h` and
  `RustBridge.swift` updated; `project.yml` changed without `make xcodegen`.
- Fake verification: "tests pass" without `check.sh` output; "verified on device" for a
  change that touches nothing device-specific.
- Comment rot: comments that restate the code or describe an earlier version. A comment says
  why, or states an invariant.

## 6. Definition of done

1. `make check` is green: fmt, clippy with `-D warnings` on all three feature sets, rustdoc,
   tests, deny, machete, MSRV, FFI header parity, attribution, swift-format, swiftlint
   `--strict`, xcodebuild test with warnings as errors under Swift 6 strict concurrency,
   swiftlint analyze, String Catalog sync, project.yml/pbxproj parity.
2. New unsafe, FFI, real-time, or concurrency code has the SAFETY, ownership, and
   thread-contract comments listed above; new Swift concurrency code has a TSan-clean
   `scripts/check.sh sanitize` run.
3. New behavior has a test; changed numeric behavior has a tolerance and a reference.
4. The PR description lists: public API changes, FFI/header changes, buffer layout changes,
   new dependencies, and every lint expectation added, with its reason.
