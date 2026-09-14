# Build and release reviewer

Runs when: the branch touches `.github/`, `scripts/`, `Makefile`, `Cargo.toml`, `Cargo.lock`,
`deny.toml`, `clippy.toml`, `rust-toolchain*`, `.swiftlint.yml`, `.swift-format`,
`BlackBoxApp/fastlane/`, `BlackBoxApp/Gemfile*`, `BlackBoxApp/project.yml` version keys,
`BlackBoxApp.xcodeproj`, `.claude/`, `docs/reviewers/`, or `SETUP.md`. Routing lives in
`.claude/workflows/ship-ticket.js`.

## In your lane
- CI minutes are scarce (macOS runners bill at a multiple). A new job, a matrix, a lane that runs
  on every push, or a removed `concurrency` cancel group needs a stated reason. New steps belong
  inside existing jobs.
- Required check names stay stable: `Format`, `Clippy`, `Test (macos-latest)`, `Swift app`,
  `Security audit`. Renaming a job silently breaks branch protection.
- Actions are pinned to a full commit SHA with a version comment (`@<sha>  # v7.0.1`); a tag or
  branch ref is a finding. Job `permissions` stay minimal (`contents: read`, or `{}`).
- `scripts/check.sh` mirrors every gating CI step. A step added to `rust.yml` without a local
  equivalent, or the reverse, breaks the promise that green locally means green on GitHub.
  Missing tools must stay reported, never skipped silently.
- Rust manifest policy (§1.8): MSRV (`rust-version` in `Cargo.toml`, `msrv` in `clippy.toml`, the
  MSRV CI lane) moves together and in its own commit; a lint level change in `[lints]` carries a
  comment; the DOLL-653 backlog block is only ever shrunk; `panic = "abort"` stays in
  `[profile.release]` (DOLL-90); new crates pass `cargo deny` with a license on the allow-list and
  regenerate `ACKNOWLEDGMENTS.md`; `deny.toml` ignores carry a reason and a removal date.
- Lockfile: `Cargo.lock` changes match the manifest change; unrelated crate bumps in a feature
  branch are scope creep. Advisory-driven bumps name the RUSTSEC ID.
- Xcode project: `project.yml` is the source; the committed `.xcodeproj` is regenerated with the
  xcodegen version CI pins (2.45.3). Build keys for language mode, concurrency, and warnings live
  in `Guardrails.xcconfig`; `project.yml` `settings` keeps identity and version keys.
- Versions and release: `scripts/check-versions.sh` alignment across `Cargo.toml`, `project.yml`,
  and `Info.plist`; a tagged release needs a `CHANGELOG.md` heading (DOLL-462); release lanes in
  `release.yml` and the Fastfile keep the fmt, clippy, and test gate and the `release` environment
  approval.
- App Store metadata and Fastlane: changes under `fastlane/metadata`, the Fastfile, or its JSON
  config pass `make check-app-store` (the `metadata-lint.yml` lane triggers on those paths,
  DOLL-390). No credentials, key paths inside the repo, or ASC IDs in committed files (keys live
  outside the repo, DOLL-155).
- Scripts: `set -euo pipefail`, quoted variables, exit codes that mean something, no reliance on
  zsh word-splitting, no `sudo`, no network calls in the local gate beyond cargo and brew.
- `.claude/workflows/` and `docs/reviewers/`: a workflow change keeps the plain-literal `meta`
  block, uses no `Date.now()`, `Math.random()`, or imports, never pushes to `main` directly, and
  keeps merges behind green CI. Routing entries point at brief files that exist.

## Not your lane
- Rust or Swift source behavior: the code lanes. Test quality: `tests`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: CI or the release lane broken for everyone; a secret or signing material committed; a
  workflow that can push to or merge into `main` without green checks.
- high: a required check renamed or removed; an unpinned action; `panic = "abort"` removed; a
  release gate bypassed; `check.sh` no longer mirroring a CI step.
- medium: a new CI job or trigger without a cost reason; MSRV moved in only some places; a new
  dependency without justification or attribution; a lockfile carrying unrelated bumps.
- low: step naming, comment wording, script style.

## Facts that prevent false positives
- CI is macOS-only by decision (DOLL-153); do not ask for Linux or Windows lanes.
- The app is arm64-only (DOLL-463); do not ask for universal or x86_64 builds.
- `cargo deny` replaced `cargo audit` inside the job still named `Security audit` (DOLL-652) so
  branch protection keeps matching.
- `codeql.yml` and `ignored-tests.yml` are weekly on purpose, and `coverage.yml` runs only on pushes
  to `main`, to save minutes.
- The bench smoke floors (10x single and split, 4x pipeline) are set well below typical results so
  shared runners do not flap; do not ask to raise them.
- Dependabot batches may land as one branch with per-bump commits; that is the documented practice.
