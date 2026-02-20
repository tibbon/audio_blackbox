#!/usr/bin/env bash
#
# Bump version and/or build number across all project files.
#
# Usage:
#   ./scripts/bump-version.sh          # Just increment build number
#   ./scripts/bump-version.sh 0.2.0    # Set version + increment build number
#
# Updates:
#   - Cargo.toml (package version) — only when version argument given
#   - Makefile (APP_VERSION) — only when version argument given
#   - BlackBoxApp/BlackBoxApp/Info.plist (CFBundleShortVersionString + CFBundleVersion)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLIST="$REPO_ROOT/BlackBoxApp/BlackBoxApp/Info.plist"

# Always increment CFBundleVersion (build number)
CURRENT_BUILD=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$PLIST" 2>/dev/null || echo "0")
NEW_BUILD=$((CURRENT_BUILD + 1))
sed -i '' "/<key>CFBundleVersion<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_BUILD<\/string>/;}" "$PLIST"
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

    # Cargo.toml — package version
    sed -i '' "s/^version = \"[0-9]*\.[0-9]*\.[0-9]*\"/version = \"$NEW_VERSION\"/" "$REPO_ROOT/Cargo.toml"
    echo "  Updated Cargo.toml"

    # Makefile — APP_VERSION
    sed -i '' "s/^APP_VERSION = .*/APP_VERSION = $NEW_VERSION/" "$REPO_ROOT/Makefile"
    echo "  Updated Makefile"

    # Info.plist — CFBundleShortVersionString
    sed -i '' "/<key>CFBundleShortVersionString<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_VERSION<\/string>/;}" "$PLIST"
    echo "  Updated CFBundleShortVersionString to $NEW_VERSION"

    echo ""
    echo "Version $NEW_VERSION (build $NEW_BUILD)"
else
    CURRENT_VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$PLIST" 2>/dev/null || echo "unknown")
    echo ""
    echo "Version $CURRENT_VERSION (build $NEW_BUILD)"
fi
