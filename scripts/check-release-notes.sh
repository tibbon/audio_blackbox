#!/usr/bin/env bash
#
# Fail when the App Store "What's New" text has not changed since the
# previous release tag. release_notes.txt is what `fastlane metadata` sends
# as the version's release notes, so an unchanged file ships the previous
# version's notes (1.5.0 was tagged with the 1.4.0 text and fixed afterwards).
#
# Usage:
#   ./scripts/check-release-notes.sh [COMMIT] [CURRENT_TAG]
#
# COMMIT defaults to HEAD. CURRENT_TAG, when given, is left out of the search
# for the previous tag (a tag run passes its own tag, which points at COMMIT).
# Needs the tags and the history back to the previous tag. Exits 0 when there
# is no earlier v* tag.

set -euo pipefail

cd "$(dirname "$0")/.."

NOTES="BlackBoxApp/fastlane/metadata/en-US/release_notes.txt"
COMMIT="${1:-HEAD}"
CURRENT_TAG="${2:-}"

describe=(git describe --tags --abbrev=0 --match 'v*')
if [[ -n "$CURRENT_TAG" ]]; then describe+=(--exclude "$CURRENT_TAG"); fi

if ! PREV_TAG="$("${describe[@]}" "$COMMIT" 2>/dev/null)"; then
  echo "No earlier v* tag before ${COMMIT}; nothing to compare ${NOTES} against."
  exit 0
fi

if git diff --quiet "$PREV_TAG" "$COMMIT" -- "$NOTES"; then
  echo "error: ${NOTES} is unchanged since ${PREV_TAG}, so the App Store would show that version's \"What's New\" text." >&2
  echo "       Write notes for this version (see CHANGELOG.md [Unreleased]), run 'make check-app-store', and merge that first." >&2
  exit 1
fi
echo "${NOTES} has changed since ${PREV_TAG}"
