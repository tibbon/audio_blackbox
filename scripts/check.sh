#!/usr/bin/env bash
# scripts/check.sh — the full guardrail loop. Green means done (DOLL-652).
#
# Usage:  scripts/check.sh              everything CI gates on (rust + swift)
#         scripts/check.sh rust         Rust only: fmt, clippy x3 feature sets, rustdoc,
#                                       tests, bench smoke, deny, machete, MSRV, FFI header,
#                                       attribution
#         scripts/check.sh swift        Swift only: swift-format, swiftlint, xcodebuild test,
#                                       swiftlint analyze, string-catalog sync, pbxproj parity,
#                                       Release-configuration build
#         scripts/check.sh tooling      Claude workflow scripts parse and pass their mocked scenarios (fast)
#         scripts/check.sh sanitize     Swift tests under TSan, then ASan+UBSan (slow; local only)
#         scripts/check.sh all          rust + swift + sanitize
#
# Every step here mirrors a CI lane (.github/workflows/rust.yml) or is stricter
# than one, so a green run locally means a green run on GitHub. Actions minutes
# are scarce; run this before pushing, not instead of it.
#
# Missing optional tools are reported and skipped, never silently ignored:
#   brew install swiftlint            cargo install cargo-deny cargo-machete --locked
#   rustup toolchain install 1.98.1   (MSRV tests; must match rust-version in Cargo.toml)
set -euo pipefail

cd "$(dirname "$0")/.."

XCODE_PROJECT="${XCODE_PROJECT:-BlackBoxApp/BlackBoxApp.xcodeproj}"
XCODE_SCHEME="${XCODE_SCHEME:-BlackBoxApp}"
SWIFT_DIRS=(BlackBoxApp/BlackBoxApp BlackBoxApp/BlackBoxAppTests)
MSRV="$(sed -nE 's/^rust-version = "([0-9.]+)"/\1/p' Cargo.toml)"
BUILD_LOG="target/xcodebuild.log"

section="${1:-default}"
missing=()

step()  { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
have()  { command -v "$1" >/dev/null 2>&1; }
skip()  { printf '   (skipped: %s)\n' "$*"; missing+=("$*"); }
# changed_by <cmd...> -- <file>: run cmd, succeed iff <file> changed as a result.
changed_by() {
  local cmd=() before after
  while [[ "$1" != "--" ]]; do cmd+=("$1"); shift; done; shift
  before="$(shasum "$1")"
  "${cmd[@]}" || { echo "   ${cmd[*]} failed"; exit 1; }
  after="$(shasum "$1")"
  [[ "$before" != "$after" ]]
}
pretty(){ if have xcbeautify; then xcbeautify --quiet; else grep -E 'error|warning:|Test Suite|Executed|\*\*' || true; fi; }

xcb() {
  xcodebuild -project "$XCODE_PROJECT" -scheme "$XCODE_SCHEME" \
    -destination 'platform=macOS' CODE_SIGN_IDENTITY="-" "$@"
}

# ---------------------------------------------------------------- Rust
check_rust() {
  step "rustfmt"
  cargo fmt --all -- --check

  # Cargo features are not additive for tests, so the three feature sets are
  # three separate passes — the same three lanes CI runs (DOLL-346, DOLL-450).
  local feature_sets=("--no-default-features" "--features=ffi" "--features=benchmarking")
  for feats in "${feature_sets[@]}"; do
    step "clippy $feats (all targets, warnings are errors)"
    cargo clippy --all-targets "$feats" -- -D warnings
  done

  step "rustdoc with warnings as errors"
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --all-features

  step "tests (no features)"
  cargo test --no-default-features -- --test-threads=1
  step "tests (ffi surface)"
  cargo test --features ffi ffi_tests -- --test-threads=1

  step "benchmark smoke (same throughput floors as the CI Benchmark lane)"
  cargo build --release --bin bench-writer --features benchmarking
  ./scripts/bench-assert.sh 10 -- target/release/bench-writer --channels 64 --seconds 2 --mode single
  ./scripts/bench-assert.sh 10 -- target/release/bench-writer --channels 16 --seconds 2 --mode split
  ./scripts/bench-assert.sh 4 -- target/release/bench-writer --channels 64 --seconds 2 --mode pipeline

  step "Cargo.lock is up to date"
  cargo update --locked --dry-run

  step "dependency hygiene: cargo deny (advisories, licenses, bans, sources)"
  if have cargo-deny; then cargo deny check; else skip "cargo-deny not installed"; fi

  step "dependency hygiene: cargo machete (unused dependencies)"
  if have cargo-machete; then cargo machete; else skip "cargo-machete not installed"; fi

  step "MSRV ${MSRV}: cargo test (the toolchain releases ship with)"
  # rust-version is "1.98"; the installed toolchain is named "1.98.<patch>-<host>".
  local msrv_toolchain
  msrv_toolchain="$(rustup toolchain list | awk -v m="$MSRV." 'index($1, m) == 1 { print $1; exit }')"
  if [[ -n "$msrv_toolchain" ]]; then
    rustup run "$msrv_toolchain" cargo test --no-default-features -- --test-threads=1
  else
    skip "toolchain $MSRV not installed (rustup toolchain install $MSRV.0)"
  fi

  step "FFI header parity (hand-maintained include/blackbox_ffi.h, DOLL-190)"
  ./scripts/check-ffi-header.sh

  step "third-party license attribution (DOLL-369)"
  python3 scripts/gen-acknowledgments.py --check
}

# ---------------------------------------------------------------- Swift
check_swift_lint() {
  step "swift-format (lint only; 'swift format --in-place --recursive ${SWIFT_DIRS[*]}' to fix)"
  swift format lint --strict --recursive "${SWIFT_DIRS[@]}"

  step "swiftlint --strict ('swiftlint --fix' autocorrects the mechanical rules)"
  if have swiftlint; then swiftlint lint --strict --quiet; else skip "swiftlint not installed (brew install swiftlint)"; fi
}

check_swift() {
  check_swift_lint

  step "Rust static library for the app (release, ffi) — must precede xcodebuild"
  cargo build --release --features ffi

  step "xcodebuild test (Debug; warnings are errors via Guardrails.xcconfig)"
  mkdir -p target
  # SWIFT_EMIT_LOC_STRINGS feeds the string-catalog check below (DOLL-449).
  # The full log is kept for `swiftlint analyze`, which needs the swiftc invocations.
  xcb test SWIFT_EMIT_LOC_STRINGS=YES 2>&1 | tee "$BUILD_LOG" | pretty

  step "swiftlint analyze (unused declarations and imports, captured variables)"
  if have swiftlint; then
    swiftlint analyze --strict --quiet --compiler-log-path "$BUILD_LOG"
  else
    skip "swiftlint not installed"
  fi

  # Both generators below are compared before/after, not against the git index,
  # so uncommitted-but-correct changes on a branch don't trip them.
  step "String Catalog in sync with the sources (DOLL-449)"
  if changed_by python3 scripts/sync-string-catalog.py -- BlackBoxApp/BlackBoxApp/Localizable.xcstrings; then
    echo "Localizable.xcstrings was out of sync; it has been updated — commit it."
    exit 1
  fi

  step "project.pbxproj matches project.yml (DOLL-160)"
  if have xcodegen; then
    if changed_by sh -c 'cd BlackBoxApp && xcodegen generate --quiet' -- BlackBoxApp/BlackBoxApp.xcodeproj/project.pbxproj; then
      echo "project.pbxproj is out of sync with project.yml (or your xcodegen differs from CI's pinned 2.46.0); it has been regenerated — commit it."
      exit 1
    fi
  else
    skip "xcodegen not installed (brew install xcodegen)"
  fi

  # CI's Swift lane ends with this build: Release turns on whole-module
  # optimization and drops DEBUG-only code, so it can fail where Debug passes.
  step "xcodebuild build (Release configuration, as the CI Swift lane does)"
  xcb -configuration Release build 2>&1 | pretty
}

# ---------------------------------------------------------------- Claude workflows (local only)
# .claude/workflows/*.js run as /<name> commands (DOLL-654). The runtime only
# reports a syntax error when someone launches the workflow, so parse them here.
check_workflows() {
  step "Claude workflow scripts parse, and /ship-ticket passes its mocked scenarios"
  local files=() f tmp
  for f in .claude/workflows/*.js; do [[ -e "$f" ]] && files+=("$f"); done
  if ((${#files[@]} == 0)); then echo "   (no workflow scripts)"; return; fi
  if ! have node; then skip "node not installed (brew install node)"; return; fi
  mkdir -p target
  for f in "${files[@]}"; do
    if [[ "$(head -n 1 "$f")" != "export const meta = {" ]]; then
      echo "$f: the first line must be 'export const meta = {' (a plain literal)"; exit 1
    fi
    # These throw inside the runtime (they would break resume), and imports fail before launch.
    if grep -nE 'Date\.now\(|Math\.random\(|new Date\(\)|(^|[^.[:alnum:]_])import[[:space:]]*[({"'"'"']' "$f"; then
      echo "$f: Date.now(), Math.random(), new Date() and import are not available in workflow scripts"; exit 1
    fi
    tmp="target/workflow-parse-$(basename "$f" .js).cjs"
    { echo '(async () => {'; sed 's/^export const meta = {/const meta = {/' "$f"; echo '})'; } > "$tmp"
    node --check "$tmp" || { echo "$f does not parse"; exit 1; }
  done
  # Every control-flow path of the workflow, driven by mocked agents (DOLL-654).
  node scripts/test-ship-ticket.mjs
}

# ---------------------------------------------------------------- Sanitizers (local only)
check_sanitize() {
  step "Rust static library for the app (release, ffi)"
  cargo build --release --features ffi

  # TSan and ASan cannot run together. Only the Swift side is instrumented;
  # the Rust lib is a plain release build, so this catches Swift-side races
  # around the bridge, not races inside the writer thread (cargo test covers those).
  step "Swift tests under Thread Sanitizer"
  xcb test -enableThreadSanitizer YES 2>&1 | pretty

  step "Swift tests under Address + Undefined Behavior Sanitizer"
  xcb test -enableAddressSanitizer YES -enableUndefinedBehaviorSanitizer YES 2>&1 | pretty
}

case "$section" in
  default)  check_workflows; check_rust; check_swift ;;
  all)      check_workflows; check_rust; check_swift; check_sanitize ;;
  rust)     check_rust ;;
  swift)    check_swift ;;
  lint)     check_swift_lint ;;
  tooling)  check_workflows ;;
  sanitize) check_sanitize ;;
  *) echo "unknown section: $section (rust | swift | lint | tooling | sanitize | all)"; exit 2 ;;
esac

if ((${#missing[@]})); then
  step "green, with skipped steps:"
  printf '   - %s\n' "${missing[@]}"
  exit 1
fi
step "all checks green"
