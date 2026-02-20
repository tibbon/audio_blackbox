#!/usr/bin/env bash
#
# Bump the version number across all project files atomically.
#
# Usage:
#   ./scripts/bump-version.sh 0.2.0
#
# Updates version in:
#   - Cargo.toml (package version + bundle metadata)
#   - Makefile (APP_VERSION)
#   - Info.plist (CFBundleShortVersionString)
#   - BlackBoxApp/BlackBoxApp/Info.plist (CFBundleShortVersionString)

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <new-version>"
    echo "Example: $0 0.2.0"
    exit 1
fi

NEW_VERSION="$1"

# Validate version format (semver-like: X.Y.Z)
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: Version must be in X.Y.Z format (e.g. 0.2.0)"
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "Bumping version to $NEW_VERSION..."

# 1. Cargo.toml — package version (line 3)
sed -i '' "s/^version = \"[0-9]*\.[0-9]*\.[0-9]*\"/version = \"$NEW_VERSION\"/" "$REPO_ROOT/Cargo.toml"
echo "  Updated Cargo.toml"

# 2. Makefile — APP_VERSION
sed -i '' "s/^APP_VERSION = .*/APP_VERSION = $NEW_VERSION/" "$REPO_ROOT/Makefile"
echo "  Updated Makefile"

# 3. Info.plist (Cocoa app)
sed -i '' "/<key>CFBundleShortVersionString<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_VERSION<\/string>/;}" "$REPO_ROOT/Info.plist"
echo "  Updated Info.plist"

# 4. BlackBoxApp Info.plist (SwiftUI app) — version string
sed -i '' "/<key>CFBundleShortVersionString<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_VERSION<\/string>/;}" "$REPO_ROOT/BlackBoxApp/BlackBoxApp/Info.plist"
echo "  Updated BlackBoxApp/BlackBoxApp/Info.plist (CFBundleShortVersionString)"

# 5. Auto-increment CFBundleVersion (build number) in both plists
SWIFTUI_PLIST="$REPO_ROOT/BlackBoxApp/BlackBoxApp/Info.plist"
CURRENT_BUILD=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$SWIFTUI_PLIST" 2>/dev/null || echo "0")
NEW_BUILD=$((CURRENT_BUILD + 1))

sed -i '' "/<key>CFBundleVersion<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_BUILD<\/string>/;}" "$SWIFTUI_PLIST"
sed -i '' "/<key>CFBundleVersion<\/key>/{n;s/<string>[^<]*<\/string>/<string>$NEW_BUILD<\/string>/;}" "$REPO_ROOT/Info.plist"
echo "  Updated CFBundleVersion: $CURRENT_BUILD → $NEW_BUILD"

echo ""
echo "Version bumped to $NEW_VERSION (build $NEW_BUILD) in all files."
echo "Remember to commit: git add -A && git commit -m 'Bump version to $NEW_VERSION'"
