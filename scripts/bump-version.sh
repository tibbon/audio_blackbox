#!/usr/bin/env bash
#
# Bump version and/or build number across all project files.
#
# Usage:
#   ./scripts/bump-version.sh          # Just increment build number
#   ./scripts/bump-version.sh 0.2.0    # Set version + increment build number
#
# Updates:
#   - Cargo.toml (package version AND package.metadata.bundle version)
#   - Makefile (APP_VERSION)
#   - BlackBoxApp/BlackBoxApp/Info.plist (CFBundleShortVersionString + CFBundleVersion)
#   - BlackBoxApp/project.yml (CFBundleShortVersionString + CFBundleVersion)
#   - BlackBoxApp/BlackBoxApp.xcodeproj/project.pbxproj (regenerated from
#     project.yml via xcodegen, so the committed project stays in sync — CI
#     enforces this match, DOLL-160). Requires `xcodegen` on PATH.
#
# Portability: uses `sed -i.bak` everywhere (both BSD and GNU sed accept
# this form) and cleans up the .bak files at the end (DOLL-193). The
# script is still macOS-only in practice — PlistBuddy is only available
# on macOS — but the sed pattern works on GNU sed for any contributor
# who runs the marketing-version path on Linux.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLIST="$REPO_ROOT/BlackBoxApp/BlackBoxApp/Info.plist"
PROJECT_YML="$REPO_ROOT/BlackBoxApp/project.yml"
CARGO_TOML="$REPO_ROOT/Cargo.toml"
MAKEFILE="$REPO_ROOT/Makefile"

# Helper: portable in-place sed. Both BSD and GNU sed accept -i.bak;
# we delete the backup immediately. This avoids the BSD-only -i '' /
# GNU-only --in-place / -i divergence.
sed_inplace() {
    local pattern="$1"
    local file="$2"
    sed -i.bak -E "$pattern" "$file"
    rm -f "${file}.bak"
}

# Always increment CFBundleVersion (build number)
CURRENT_BUILD=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$PLIST" 2>/dev/null || echo "0")
NEW_BUILD=$((CURRENT_BUILD + 1))
sed -i.bak "/<key>CFBundleVersion<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_BUILD<\/string>/;}" "$PLIST"
rm -f "${PLIST}.bak"
sed_inplace "s/CFBundleVersion: \"[^\"]*\"/CFBundleVersion: \"$NEW_BUILD\"/" "$PROJECT_YML"
sed_inplace "s/CURRENT_PROJECT_VERSION: \"[^\"]*\"/CURRENT_PROJECT_VERSION: \"$NEW_BUILD\"/" "$PROJECT_YML"
echo "  CFBundleVersion: $CURRENT_BUILD → $NEW_BUILD"

# Optionally update marketing version if argument provided
if [[ $# -ge 1 ]]; then
    NEW_VERSION="$1"

    # Validate version format (semver-like: X.Y.Z)
    if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "Error: Version must be in X.Y.Z format (e.g. 0.2.0)"
        exit 1
    fi

    echo "Bumping version to $NEW_VERSION..."

    # Cargo.toml — both [package].version AND [package.metadata.bundle].version
    # (DOLL-193: the prior `^version = ` pattern relied on BSD-sed's
    # multi-line default; this version uses `$`-anchored end-of-line to
    # match unambiguously on both BSD and GNU sed, hitting every
    # standalone `version = "X.Y.Z"` line.)
    sed_inplace "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"\$/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
    echo "  Updated Cargo.toml (package + metadata.bundle)"

    # Makefile — APP_VERSION
    sed_inplace "s/^APP_VERSION = .*/APP_VERSION = $NEW_VERSION/" "$MAKEFILE"
    echo "  Updated Makefile"

    # Info.plist — CFBundleShortVersionString
    sed -i.bak "/<key>CFBundleShortVersionString<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_VERSION<\/string>/;}" "$PLIST"
    rm -f "${PLIST}.bak"
    echo "  Updated Info.plist"

    # project.yml — CFBundleShortVersionString + MARKETING_VERSION
    sed_inplace "s/CFBundleShortVersionString: \"[^\"]*\"/CFBundleShortVersionString: \"$NEW_VERSION\"/" "$PROJECT_YML"
    sed_inplace "s/MARKETING_VERSION: \"[^\"]*\"/MARKETING_VERSION: \"$NEW_VERSION\"/" "$PROJECT_YML"
    echo "  Updated project.yml"

    echo ""
    echo "Version $NEW_VERSION (build $NEW_BUILD)"
else
    CURRENT_VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$PLIST" 2>/dev/null || echo "unknown")
    echo ""
    echo "Version $CURRENT_VERSION (build $NEW_BUILD)"
fi

# Regenerate the committed Xcode project so project.pbxproj tracks the
# project.yml edits above (DOLL-160 CI fails the build otherwise). Done last
# so it picks up every change. xcodegen is required — fail loudly if missing
# rather than leaving a half-bumped tree.
if ! command -v xcodegen >/dev/null 2>&1; then
    echo "Error: xcodegen not found on PATH — install it (brew install xcodegen) and" >&2
    echo "       re-run 'make xcodegen' so project.pbxproj matches the bumped project.yml." >&2
    exit 1
fi
( cd "$REPO_ROOT/BlackBoxApp" && xcodegen generate )
echo "  Regenerated project.pbxproj"
