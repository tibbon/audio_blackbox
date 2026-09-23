# Development Setup

How to build, test, and release BlackBox Audio Recorder. For what the app does, see [README.md](README.md); for how to land a change, see [AGENTS.md](AGENTS.md).

Most work needs only [Build and test](#build-and-test). Releases run in GitHub Actions; [local Fastlane](#local-fastlane-optional) is only for driving one by hand.

## Build and test

### Prerequisites

The app is Apple Silicon only (`ARCHS = arm64`) and needs macOS 15 or later.

| Tool | Install | Why |
| --- | --- | --- |
| Xcode + Command Line Tools | App Store | Builds the SwiftUI app. `xcode-select -p` should point into `Xcode.app`. |
| Rust | [rustup](https://rustup.rs) | Edition 2024. Keep `stable` current (`rustup update`); CI's clippy runs on stable. |
| Rust 1.98 (MSRV) | `rustup toolchain install 1.98.1` | `make check` runs the same MSRV check CI does. |
| XcodeGen | `brew install xcodegen` | Regenerates the `.xcodeproj` from `project.yml`. CI pins 2.46.0 and fails if they differ. |
| SwiftLint | `brew install swiftlint` | Runs with `--strict`. CI pins 0.65.1 and `make check` warns on a different version. `swift format` ships with Xcode. |
| cargo-deny, cargo-machete | `cargo install cargo-deny@0.20.2 cargo-machete@0.9.2 --locked` | Dependency advisories, licenses, unused deps. CI pins these versions. |
| Node.js | `brew install node` | `make check` tests the Claude workflow scripts. |
| GitHub CLI | `brew install gh` | PRs and release dispatch. |

### First run

```bash
git clone git@github.com:tibbon/audio_blackbox.git
cd audio_blackbox
make setup      # installs the pre-commit hook, then runs `make verify`
make run-app    # build the Rust library and the app, then launch it
```

### Everyday commands

```bash
make check        # everything CI gates on; green means done
make check-rust   # just the Rust half
make check-swift  # just the Swift half
make lint-swift   # swift-format + swiftlint only (seconds)
make fmt          # autoformat Rust and Swift
make test         # cargo test
make run          # run the CLI recorder
make xcodegen     # regenerate the .xcodeproj after editing project.yml
make help         # everything else
```

Run `make check` before pushing: CI runs the same gates, and Actions minutes are limited. [docs/REVIEW-CHECKLIST.md](docs/REVIEW-CHECKLIST.md) lists what the gates enforce and what a reviewer still has to judge.

If you run `xcodebuild` directly, run `make rust-lib` first. The app links `target/release/libblackbox.a`, and the `make` app targets build it for you.

## Claude Code: `/ship-ticket`

`/ship-ticket DOLL-N` takes a Linear ticket from plan to merged PR; `/ship-ticket review` reviews the current branch. See the Workflow section of [AGENTS.md](AGENTS.md#workflow). It needs dynamic workflows enabled (`/config`), the Linear MCP server connected, and `gh` logged in.

A run launches many agents and can take an hour, so permission prompts stall it. Allow its commands in `.claude/settings.local.json` (not committed):

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

Pushing, opening the PR, and merging still prompt. Add `Bash(git push:*)`, `Bash(gh pr create:*)`, and `Bash(gh pr merge:*)` only if you want those unattended too.

## Releasing

Releases run in GitHub Actions (`.github/workflows/release.yml`).

1. Run `scripts/bump-version.sh X.Y.Z`, add a `## [X.Y.Z]` entry to `CHANGELOG.md`, rewrite the App Store "What's New" text in `BlackBoxApp/fastlane/metadata/en-US/release_notes.txt` (check it with `make check-app-store`), and merge that PR. `make release` and the workflow both refuse a tag when `release_notes.txt` has not changed since the previous tag.
2. Run `make release VERSION=X.Y.Z`. It checks that the version matches `Cargo.toml` and every manifest, that the working tree is clean, and that `HEAD` is `origin/main`, then tags and pushes `vX.Y.Z`. The workflow re-checks the tag against `Cargo.toml`, the manifests, and main, but tag with `make release` rather than `git tag` so a mistake fails on your machine before it starts a release run.
3. The workflow reruns the test gate, waits for approval on the protected `release` environment, then builds, signs, and uploads to TestFlight.
4. To submit to the App Store, dispatch the workflow's `metadata` lane, then its `submit_review` lane. Each one needs its own approval.

After Apple approves, the version waits in **Pending Developer Release** until you click **Release This Version** in App Store Connect. A pending version blocks creating the next one, so release it promptly. Build numbers come from TestFlight (`max(remote, local) + 1`), so the committed build number only needs to match across manifests.

## Local Fastlane (optional)

Only needed to run lanes from your own machine.

### Ruby and Fastlane

Match CI's Ruby 4.0. The system Ruby is too old.

```bash
brew install ruby
echo 'export PATH="'"$(brew --prefix ruby)"'/bin:$PATH"' >> ~/.zshrc && exec zsh

cd BlackBoxApp
gem install bundler -v 4.0.20              # matches Gemfile.lock
bundle config set --local path vendor/bundle
bundle install                             # pinned fastlane 2.240.0
```

Install into `vendor/bundle` as shown. A plain `bundle install` into Homebrew's gem directory fails on Ruby 4.0 with `Permission denied ... rdoc_plugin.rb` (DOLL-658). The `make fl-*` targets call `fastlane` directly, so either put the pinned version on your `PATH` or run lanes from `BlackBoxApp` as `bundle exec fastlane <lane>`.

### Credentials

The `fl-*` targets read a gitignored `.env` at the repo root. Start from the template with `cp .env.example .env` and fill in:

- **`ASC_KEY_ID` and `ASC_KEY_PATH`**: the App Store Connect API key. The `.p8` lives outside the repo, at `~/Library/Application Support/com.dollhousemediatech.blackbox/keys/`, so it can't be committed by accident (DOLL-155). Copy it from a trusted machine and `chmod 600` it.
- **`ASC_ISSUER_ID`**: stored only in GitHub Actions secrets, which you can't read back. Copy it from App Store Connect → Users and Access → Integrations → App Store Connect API.
- **`DEVELOPMENT_TEAM`**: leave empty to use `BlackBoxApp/fastlane/Appfile`.

Signed local builds (`make archive`, `make dmg`) also need your Apple signing identity in the login keychain. Export it as a `.p12` from a machine that has it.

### Lanes

```bash
make fl-beta       # build and upload to TestFlight
make fl-metadata   # push App Store metadata and "What's New" (run make check-app-store first)
make fl-check      # precheck metadata for common rejection reasons
make fl-submit     # submit the latest build for review
```

## Remote development over SSH

Do the [build and test](#build-and-test) setup on the remote Mac, then add a host alias to your local `~/.ssh/config`:

```sshconfig
Host mini
    HostName <host-or-tailscale-name>
    User <you>
    IdentityFile ~/.ssh/id_ed25519
    IdentitiesOnly yes
    IdentityAgent none   # avoids "Too many authentication failures" when an agent offers many keys
```

The remote login shell is zsh. Run scripted commands under `zsh -l` so Homebrew and Cargo are on `PATH`.
