#!/usr/bin/env python3
"""Generate (or check) ACKNOWLEDGMENTS.md from cargo metadata.

DOLL-369: the third-party attribution was hand-maintained and drifted — it
listed a crate that had been removed (objc2-app-kit) and omitted ~50 crates the
shipped binary actually links. This regenerates the file mechanically from
`cargo metadata` over the shipped feature set (`--no-default-features
--features ffi`), so it can't silently drift. CI runs this with `--check` to
fail the build if the committed file is out of sync.

Versions are intentionally omitted: license + repository are what matter for
attribution and they're stable across routine dependency bumps, so the file
only changes when a dependency is added, removed, or relicensed — exactly the
drift worth catching, without churning on every `cargo update`.

Usage:
  scripts/gen-acknowledgments.py            # rewrite ACKNOWLEDGMENTS.md
  scripts/gen-acknowledgments.py --check    # exit 1 if it would change
"""

import collections
import json
import os
import subprocess
import sys

FEATURE_ARGS = ["--no-default-features", "--features", "ffi"]
ROOT_CRATE = "blackbox"
OUTPUT = os.path.join(os.path.dirname(__file__), "..", "ACKNOWLEDGMENTS.md")


def shipped_crate_names():
    """Names of crates in the normal-dependency graph for the shipped features."""
    out = subprocess.run(
        ["cargo", "tree", *FEATURE_ARGS, "-e", "normal", "--prefix", "none", "-f", "{p}"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    names = set()
    for line in out.splitlines():
        line = line.strip()
        if line:
            names.add(line.split()[0])
    names.discard(ROOT_CRATE)
    return names


def package_index():
    out = subprocess.run(
        ["cargo", "metadata", *FEATURE_ARGS, "--format-version", "1"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    meta = json.loads(out)
    return {p["name"]: p for p in meta["packages"]}


def render():
    names = shipped_crate_names()
    pkgs = package_index()

    by_license = collections.defaultdict(list)
    for name in names:
        p = pkgs.get(name, {})
        license_ = p.get("license") or "(see repository)"
        by_license[license_].append((name, p.get("repository")))

    lines = [
        "# Open Source Acknowledgments",
        "",
        "BlackBox Audio Recorder links the following open source crates. We are",
        "grateful to their authors and contributors.",
        "",
        "> **Generated file — do not edit by hand.** Produced by",
        "> `scripts/gen-acknowledgments.py` from `cargo metadata` over the shipped",
        "> feature set (`--no-default-features --features ffi`). Run the script and",
        "> commit the result when dependencies change; CI checks it stays in sync",
        "> (DOLL-369).",
        "",
    ]
    for license_ in sorted(by_license):
        lines.append(f"## {license_}")
        lines.append("")
        for name, repo in sorted(by_license[license_]):
            lines.append(f"### {name}")
            lines.append("")
            lines.append(f"- **License**: {license_}")
            if repo:
                lines.append(f"- **Repository**: <{repo}>")
            lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main():
    content = render()
    path = os.path.normpath(OUTPUT)
    if "--check" in sys.argv[1:]:
        with open(path, encoding="utf-8") as f:
            current = f.read()
        if current != content:
            sys.stderr.write(
                "ACKNOWLEDGMENTS.md is out of sync with the linked crates.\n"
                "Run scripts/gen-acknowledgments.py and commit the result (DOLL-369).\n"
            )
            sys.exit(1)
        print("ACKNOWLEDGMENTS.md is up to date.")
    else:
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        print(f"Wrote {path}")


if __name__ == "__main__":
    main()
