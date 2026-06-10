#!/usr/bin/env python3
"""Sync Localizable.xcstrings from compiler-extracted .stringsdata (DOLL-449).

Xcode's IDE updates the String Catalog on every build, but `xcodebuild` does
not write back to the catalog — so a CLI-only workflow (this repo's) leaves it
empty/stale. This script replicates the IDE's sync deterministically:

  1. Build with extraction enabled so the compiler emits .stringsdata:
       xcodebuild build -project BlackBoxApp/BlackBoxApp.xcodeproj \
         -scheme BlackBoxApp -destination platform=macOS \
         SWIFT_EMIT_LOC_STRINGS=YES CODE_SIGN_IDENTITY="-"
  2. Run this script, pointing at the DerivedData (or any) root containing
     the emitted *.stringsdata files.

Merge semantics (mirrors Xcode):
  - keys extracted from source but absent from the catalog are ADDED
    (empty entry = source-language fallback, ready for translation);
  - existing entries keep their localizations untouched;
  - catalog entries no longer extracted are DROPPED if they carry no
    localizations, or marked "extractionState": "stale" if they do
    (a translator's work is never silently deleted).

Output is byte-deterministic (sorted keys, fixed formatting), so CI can run
the sync and `git diff --exit-code` the catalog to catch drift.

Usage:
  python3 scripts/sync-string-catalog.py [--stringsdata-root DIR] [--catalog FILE]
"""

import argparse
import json
import plistlib
import sys
from pathlib import Path

DEFAULT_CATALOG = Path("BlackBoxApp/BlackBoxApp/Localizable.xcstrings")
DEFAULT_ROOT = (
    Path.home() / "Library/Developer/Xcode/DerivedData"
)


def extracted_keys(root: Path) -> dict[str, str]:
    """Collect Localizable-table keys (key -> comment) from *.stringsdata under root."""
    keys: dict[str, str] = {}
    files = sorted(root.rglob("*.stringsdata"))
    if not files:
        sys.exit(f"error: no .stringsdata files under {root} — "
                 "build with SWIFT_EMIT_LOC_STRINGS=YES first")
    for f in files:
        # Only the app's own build products; ignore unrelated projects when
        # pointed at the whole DerivedData directory.
        if "BlackBoxApp" not in str(f):
            continue
        raw = f.read_bytes()
        # .stringsdata is JSON in current Xcode; older toolchains emitted
        # binary/XML plists. Accept both.
        try:
            data = json.loads(raw)
        except (json.JSONDecodeError, UnicodeDecodeError):
            try:
                data = plistlib.loads(raw)
            except plistlib.InvalidFileException:
                continue
        for entry in data.get("tables", {}).get("Localizable", []):
            key = entry["key"]
            comment = entry.get("comment") or ""
            # First non-empty comment wins; later duplicates don't clobber.
            if key not in keys or (not keys[key] and comment):
                keys[key] = comment
    if not keys:
        sys.exit("error: .stringsdata files found but no Localizable keys — "
                 "wrong build directory?")
    return keys


def sync(catalog_path: Path, keys: dict[str, str]) -> tuple[int, int, int]:
    catalog = json.loads(catalog_path.read_text())
    strings = catalog.setdefault("strings", {})

    added = stale = dropped = 0
    for key, comment in keys.items():
        if key not in strings:
            entry: dict = {}
            if comment:
                entry["comment"] = comment
            strings[key] = entry
            added += 1
        else:
            # Re-extracted: clear any stale marker from a previous sync.
            if strings[key].get("extractionState") == "stale":
                del strings[key]["extractionState"]

    for key in list(strings):
        if key in keys:
            continue
        if strings[key].get("localizations"):
            strings[key]["extractionState"] = "stale"
            stale += 1
        else:
            del strings[key]
            dropped += 1

    catalog["strings"] = dict(sorted(strings.items()))
    # Match Xcode's xcstrings JSON style: 2-space indent, " : " separators,
    # raw UTF-8, trailing newline.
    out = json.dumps(catalog, indent=2, separators=(",", " : "),
                     ensure_ascii=False, sort_keys=False)
    catalog_path.write_text(out + "\n")
    return added, stale, dropped


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--stringsdata-root", type=Path, default=DEFAULT_ROOT,
                    help="directory searched recursively for *.stringsdata")
    ap.add_argument("--catalog", type=Path, default=DEFAULT_CATALOG)
    args = ap.parse_args()

    keys = extracted_keys(args.stringsdata_root)
    added, stale, dropped = sync(args.catalog, keys)
    print(f"{args.catalog}: {len(keys)} extracted keys — "
          f"{added} added, {stale} marked stale, {dropped} dropped")


if __name__ == "__main__":
    main()
