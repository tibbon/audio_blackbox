# Development Setup

How to get a machine ready to build, run, and release BlackBox Audio Recorder.
For what the app *is*, see [README.md](README.md); for release notes see
[CHANGELOG.md](CHANGELOG.md).

There are two tiers of setup:

1. **Core development** — build, test, and run the app. This is all most work needs.
2. **Local releases** *(optional)* — cut TestFlight / App Store builds from your
   machine with Fastlane. Releases normally go through **GitHub Actions**
   (see [Releasing](#releasing)), so this tier is only for driving a release by hand.

---

## 1. Core development

### Prerequisites

| Tool | Notes |
| --- | --- |
| **Xcode** (full) + Command Line Tools | Required to build the SwiftUI app. `xcode-select -p` should point into `Xcode.app`. |
| **Rust** via [rustup](https://rustup.rs) | Edition 2024; MSRV **1.95** (`rust-version` in `Cargo.toml`). Stable works for day-to-day; releases pin 1.95. |
| **Homebrew** | For the tools below. |
| **XcodeGen** | `brew install xcodegen` — regenerates `BlackBoxApp.xcodeproj` from `project.yml` (the `.xcodeproj` is committed and CI checks it matches; CI pins 2.45.3). |
| **SwiftLint** | `brew install swiftlint` — `scripts/check.sh` and CI run it with `--strict`. `swift format` ships with Xcode. |
| **cargo-deny, cargo-machete** | `cargo install cargo-deny cargo-machete --locked` — dependency advisories/licenses and unused-dep detection; CI runs both. |
| **Rust MSRV toolchain** | `rustup toolchain install 1.95.0` — lets `scripts/check.sh` run the MSRV check CI runs. Keep `stable` current (`rustup update`): CI's clippy is stable, and lints move between releases. |
| **GitHub CLI** | `brew install gh` — for PRs and release dispatch. |
| **Node.js** | `brew install node` — `scripts/check.sh` parses the Claude workflow scripts and runs their mocked scenarios (`scripts/test-ship-ticket.mjs`). |

macOS on Apple Silicon is the supported/shipped target (the app is arm64-only).
The CLI binary also builds on Linux (`libasound2-dev` + `pkg-config`), but there
is no Linux GUI.

### Clone and first-time setup

```bash
git clone git@github.com:tibbon/audio_blackbox.git ~/code/personal/audio_blackbox
cd ~/code/personal/audio_blackbox

# Set your git identity if this is a fresh machine
git config --global user.name  "Your Name"
git config --global user.email "you@example.com"

make setup   # installs the pre-commit hook and runs `make verify`
```

`make setup` fails fast if `cargo` is missing and warns if Xcode is absent.

### Everyday commands

```bash
make build          # debug build of the Rust crate
make test           # cargo test
make lint           # fast Rust lint: fmt + clippy (no features)
make lint-swift     # swift-format + swiftlint --strict (seconds)
make fmt            # autoformat Rust and Swift
make check          # the full guardrail loop (scripts/check.sh) — green means done
make check-rust     # Rust half: clippy x3 feature sets, rustdoc, tests, deny, machete, MSRV
make check-swift    # Swift half: format, swiftlint, xcodebuild test, analyze, catalog sync
make check-sanitize # Swift tests under TSan then ASan+UBSan (slow; local only)
make verify         # check + App Store metadata lint
make run            # run the CLI directly

make app            # build the SwiftUI app bundle (rust-lib + xcodebuild)
make run-app        # build and launch the app
make xcodegen       # regenerate the .xcodeproj from project.yml
```

Run `make help` for the full list. Run `make check` before pushing —
GitHub Actions runs the same gates and CI minutes are limited. The rules the
gates enforce, and the ones a reviewer still has to judge, are in
[docs/REVIEW-CHECKLIST.md](docs/REVIEW-CHECKLIST.md).

### Claude Code workflow

`/ship-ticket DOLL-N` takes a ticket from plan to merged PR, and
`/ship-ticket review` reviews the current branch (see AGENTS.md "Workflow").
It needs Claude Code with dynamic workflows enabled (`/config`), the Linear MCP
server connected, and `gh` logged in. `scripts/check.sh tooling` parses the
workflow script; it needs `node` (`brew install node`).

A run launches dozens of agents and can take an hour, so permission prompts
stall it. Allow the commands its agents run in `.claude/settings.local.json`
(local, not committed), for example:

```json
{
  "permissions": {
    "allow": [
      "Workflow(ship-ticket)",
      "Bash(./scripts/check.sh:*)",
      "Bash(cargo fmt:*)", "Bash(cargo clippy:*)", "Bash(cargo test:*)", "Bash(cargo build:*)",
      "Bash(swiftlint lint:*)", "Bash(swift format:*)", "Bash(make xcodegen)",
      "Bash(git status:*)", "Bash(git diff:*)", "Bash(git log:*)", "Bash(git show:*)",
      "Bash(git fetch:*)", "Bash(git merge-base:*)", "Bash(git switch:*)",
      "Bash(git add:*)", "Bash(git commit:*)", "Bash(git rebase:*)",
      "Bash(gh pr view:*)", "Bash(gh pr checks:*)", "Bash(gh pr list:*)", "Bash(gh run view:*)"
    ]
  }
}
```

Pushing, opening the PR and merging are left to the normal permission
prompt or auto mode. Add `Bash(git push:*)`, `Bash(gh pr create:*)` and
`Bash(gh pr merge:*)` only if you want those unattended too.

---

## 2. Local releases (optional)

Releases are normally cut in the cloud (see [Releasing](#releasing)). Set this up
only if you want to run `fastlane` from your machine.

### Toolchain

Fastlane needs a modern Ruby (the system Ruby is too old). Match CI's **Ruby 3.3**:

```bash
brew install ruby@3.3
echo 'export PATH="'"$(brew --prefix ruby@3.3)"'/bin:$PATH"' >> ~/.zshrc
exec zsh                                   # pick up the new PATH

cd BlackBoxApp
gem install bundler -v 4.0.9               # matches Gemfile.lock "BUNDLED WITH"
bundle install                             # installs the pinned fastlane (2.236.1)
fastlane --version                         # should print 2.236.1
```

The `make fl-*` targets call `fastlane` directly (they don't use `bundle exec`),
so `fastlane` must be on your `PATH` at the pinned version.

### App Store Connect credentials

The `fl-*` targets source a **`.env`** at the repo root (gitignored). Copy the
template and fill it in:

```bash
cp .env.example .env
```

```dotenv
ASC_KEY_ID="<key id>"          # e.g. the AuthKey_<KEY_ID>.p8 filename
ASC_ISSUER_ID="<issuer id>"    # App Store Connect → Users and Access →
                               #   Integrations → App Store Connect API → Issuer ID
ASC_KEY_PATH="$HOME/Library/Application Support/com.dollhousemediatech.blackbox/keys/AuthKey_<KEY_ID>.p8"
DEVELOPMENT_TEAM=""            # empty → falls back to BlackBoxApp/fastlane/Appfile
```

Two things are **not** stored in this repo and must be provisioned per machine:

- **The `.p8` API key** lives *outside* the repo (see `ASC_KEY_PATH` above), so a
  real key is never one `git add` away from leaking. Copy it from a trusted
  machine to that path and `chmod 600` it.
- **The Issuer ID** is not on disk anywhere — it is only in the GitHub Actions
  secrets (which are write-only). Read it from App Store Connect and paste it in.

### Signing certificates

Local *signed* builds (`make archive` / `make app` for distribution, `make dmg`)
need your Apple signing identity in the **login keychain**. Export it as a `.p12`
from a machine that has it (Keychain Access → export the identity) and import it
on the new machine. Cloud releases sign inside CI, so this is only for local
signed archives.

### Release commands

```bash
make fl-beta       # build, upload to TestFlight
make fl-metadata   # push App Store metadata / "What's New"
make fl-submit     # submit the latest build for App Store review
make fl-check      # precheck metadata for common rejection reasons
```

---

## Releasing

The canonical release path is **GitHub Actions** (`.github/workflows/release.yml`),
not local Fastlane. In brief:

1. `scripts/bump-version.sh X.Y.Z`, add a `## [X.Y.Z]` entry to `CHANGELOG.md`,
   then PR and merge. `scripts/check-versions.sh` must pass (all version fields
   aligned) and the CHANGELOG heading is required.
2. Tag `vX.Y.Z` and push → the workflow re-runs the full test gate, then pauses on
   the protected **`release`** environment for manual approval → the `beta` lane
   builds, signs, and uploads to **TestFlight**.
3. App Store submission is a separate `workflow_dispatch` of the `metadata` then
   `submit_review` lanes (each re-runs the gate and needs its own approval).

Notes:

- `submit_review` uses `automatic_release: false`, so after Apple approves, the
  version sits in **"Pending Developer Release"** until you click **Release This
  Version** in App Store Connect. A held/pending version also *blocks creating the
  next version*, so release it promptly.
- The build number is derived from Apple's state (`max(remote, local) + 1`), so
  the committed build number only needs to be self-consistent.

---

## Remote development over SSH

To develop on another machine over SSH, complete the [core development](#1-core-development)
setup there, then add a host alias to your **local** `~/.ssh/config` for a clean
connection:

```sshconfig
Host mini
    HostName <host-or-tailscale-name>
    User <you>
    IdentityFile ~/.ssh/id_ed25519
    IdentitiesOnly yes
    # If your global `Host *` routes through an SSH agent (e.g. 1Password) that
    # offers many keys, `IdentityAgent none` here avoids "Too many authentication
    # failures" by using only the key above.
    IdentityAgent none
```

Then `ssh mini`. Note that the login shell is zsh — if you script remote commands,
run them under `zsh -l` so Homebrew and Cargo are on `PATH`.
