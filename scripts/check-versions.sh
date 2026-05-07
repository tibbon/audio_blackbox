#!/usr/bin/env bash
#
# Verify that the marketing version (X.Y.Z) and build number (CFBundleVersion)
# are in sync across every file that declares one. Run before tagging a
# release; non-zero exit code means a mismatch and a release should not
# proceed until it is reconciled (typically via scripts/bump-version.sh).
#
# Usage:
#   ./scripts/check-versions.sh
#
# Outputs every value found, then a verdict. Exits 1 on mismatch, 2 on
# parse error (a field could not be located).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO="$REPO_ROOT/Cargo.toml"
MAKEFILE="$REPO_ROOT/Makefile"
PROJECT_YML="$REPO_ROOT/BlackBoxApp/project.yml"
PLIST="$REPO_ROOT/BlackBoxApp/BlackBoxApp/Info.plist"

red()    { printf '\033[0;31m%s\033[0m\n' "$1"; }
green()  { printf '\033[0;32m%s\033[0m\n' "$1"; }
yellow() { printf '\033[0;33m%s\033[0m\n' "$1"; }

# Extract a single value with a regex; fail loudly if not found.
# Args: <description> <file> <regex with ONE capturing group>
extract() {
    local desc="$1" file="$2" regex="$3"
    local value
    value=$(grep -E "$regex" "$file" | head -1 | sed -E "s/.*$regex.*/\1/" || true)
    if [[ -z "$value" ]]; then
        red "FAIL: could not extract $desc from $file"
        exit 2
    fi
    printf '%s' "$value"
}

# Extract the i-th occurrence (1-indexed) of a regex.
extract_nth() {
    local desc="$1" file="$2" regex="$3" nth="$4"
    local value
    value=$(grep -E "$regex" "$file" | sed -n "${nth}p" | sed -E "s/.*$regex.*/\1/" || true)
    if [[ -z "$value" ]]; then
        red "FAIL: could not extract $desc (#$nth) from $file"
        exit 2
    fi
    printf '%s' "$value"
}

# Extract a string value following a <key> in a plist.
plist_string_after_key() {
    local key="$1" file="$2"
    /usr/libexec/PlistBuddy -c "Print :$key" "$file" 2>/dev/null || {
        red "FAIL: could not read $key from $file"
        exit 2
    }
}

# ── Marketing version (X.Y.Z) ───────────────────────────────────────────
CARGO_PKG_VERSION=$(extract_nth      "Cargo.toml package.version"            "$CARGO"       '^version = "([0-9]+\.[0-9]+\.[0-9]+)"' 1)
CARGO_BUNDLE_VERSION=$(extract_nth   "Cargo.toml metadata.bundle.version"    "$CARGO"       '^version = "([0-9]+\.[0-9]+\.[0-9]+)"' 2)
MAKEFILE_APP_VERSION=$(extract       "Makefile APP_VERSION"                  "$MAKEFILE"    '^APP_VERSION = ([0-9]+\.[0-9]+\.[0-9]+)')
YML_MARKETING_VERSION=$(extract      "project.yml MARKETING_VERSION"         "$PROJECT_YML" 'MARKETING_VERSION: "([0-9]+\.[0-9]+\.[0-9]+)"')
YML_SHORT_VERSION=$(extract          "project.yml CFBundleShortVersionString" "$PROJECT_YML" 'CFBundleShortVersionString: "([0-9]+\.[0-9]+\.[0-9]+)"')
PLIST_SHORT_VERSION=$(plist_string_after_key "CFBundleShortVersionString" "$PLIST")

# ── Build number (integer) ──────────────────────────────────────────────
YML_PROJECT_VERSION=$(extract        "project.yml CURRENT_PROJECT_VERSION"   "$PROJECT_YML" 'CURRENT_PROJECT_VERSION: "([0-9]+)"')
YML_BUNDLE_VERSION=$(extract         "project.yml CFBundleVersion"           "$PROJECT_YML" 'CFBundleVersion: "([0-9]+)"')
PLIST_BUNDLE_VERSION=$(plist_string_after_key "CFBundleVersion" "$PLIST")

echo
echo "Marketing version (CFBundleShortVersionString):"
printf '  %-40s %s\n' "Cargo.toml [package].version"            "$CARGO_PKG_VERSION"
printf '  %-40s %s\n' "Cargo.toml [metadata.bundle].version"    "$CARGO_BUNDLE_VERSION"
printf '  %-40s %s\n' "Makefile APP_VERSION"                    "$MAKEFILE_APP_VERSION"
printf '  %-40s %s\n' "project.yml MARKETING_VERSION"           "$YML_MARKETING_VERSION"
printf '  %-40s %s\n' "project.yml CFBundleShortVersionString"  "$YML_SHORT_VERSION"
printf '  %-40s %s\n' "Info.plist CFBundleShortVersionString"   "$PLIST_SHORT_VERSION"

echo
echo "Build number (CFBundleVersion):"
printf '  %-40s %s\n' "project.yml CURRENT_PROJECT_VERSION"     "$YML_PROJECT_VERSION"
printf '  %-40s %s\n' "project.yml CFBundleVersion"             "$YML_BUNDLE_VERSION"
printf '  %-40s %s\n' "Info.plist CFBundleVersion"              "$PLIST_BUNDLE_VERSION"
echo

mismatched=0

# All marketing versions must match the Cargo.toml package version.
for pair in \
    "Cargo.toml [metadata.bundle].version|$CARGO_BUNDLE_VERSION" \
    "Makefile APP_VERSION|$MAKEFILE_APP_VERSION" \
    "project.yml MARKETING_VERSION|$YML_MARKETING_VERSION" \
    "project.yml CFBundleShortVersionString|$YML_SHORT_VERSION" \
    "Info.plist CFBundleShortVersionString|$PLIST_SHORT_VERSION"
do
    name="${pair%|*}"
    value="${pair##*|}"
    if [[ "$value" != "$CARGO_PKG_VERSION" ]]; then
        red "MISMATCH: $name is '$value', expected '$CARGO_PKG_VERSION' (from Cargo.toml [package].version)"
        mismatched=1
    fi
done

# All build numbers must match the project.yml CURRENT_PROJECT_VERSION.
for pair in \
    "project.yml CFBundleVersion|$YML_BUNDLE_VERSION" \
    "Info.plist CFBundleVersion|$PLIST_BUNDLE_VERSION"
do
    name="${pair%|*}"
    value="${pair##*|}"
    if [[ "$value" != "$YML_PROJECT_VERSION" ]]; then
        red "MISMATCH: $name is '$value', expected '$YML_PROJECT_VERSION' (from project.yml CURRENT_PROJECT_VERSION)"
        mismatched=1
    fi
done

if [[ $mismatched -ne 0 ]]; then
    yellow "Run scripts/bump-version.sh (with no args to just bump the build number, or with X.Y.Z to set the marketing version)."
    exit 1
fi

green "OK: all version fields aligned at $CARGO_PKG_VERSION (build $YML_PROJECT_VERSION)"
