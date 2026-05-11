#!/usr/bin/env bash
# DOLL-190: parity check between src/ffi.rs and include/blackbox_ffi.h.
#
# The header is hand-maintained (no cbindgen). This script:
#
# 1. Extracts every `pub extern "C" fn` from src/ffi.rs.
# 2. Extracts every function declaration from include/blackbox_ffi.h.
# 3. Reports any symbol that exists in one but not the other.
#
# Exits 0 on match, 1 on drift. Wire into CI via a new step in the
# rust.yml workflow (or its own job) so missing-header bugs surface
# at the PR boundary instead of mid-Swift-build.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FFI_RS="$REPO_ROOT/src/ffi.rs"
FFI_H="$REPO_ROOT/include/blackbox_ffi.h"

if [[ ! -f "$FFI_RS" ]]; then
    echo "Error: $FFI_RS not found" >&2
    exit 1
fi

if [[ ! -f "$FFI_H" ]]; then
    echo "Error: $FFI_H not found" >&2
    exit 1
fi

# Extract function names from `pub extern "C" fn NAME(` in ffi.rs.
rust_symbols=$(
    grep -E '^pub extern "C" fn ' "$FFI_RS" \
        | sed -E 's/^pub extern "C" fn ([a-zA-Z0-9_]+).*/\1/' \
        | sort -u
)

# Extract function names from declarations in blackbox_ffi.h.
# Pattern: `<return-type> NAME(` (one per declared function).
# We deliberately match lines that look like function declarations,
# not typedefs / structs / macros.
header_symbols=$(
    # Match any line starting with a type token, then optional `*`s,
    # then a blackbox_* symbol, then `(`. Captures the symbol.
    grep -E '^[a-zA-Z_][a-zA-Z0-9_ *]*[[:space:]\*]blackbox_[a-zA-Z0-9_]+ *\(' "$FFI_H" \
        | sed -E 's/.*[[:space:]\*](blackbox_[a-zA-Z0-9_]+)[[:space:]]*\(.*/\1/' \
        | sort -u
)

missing_in_header=$(comm -23 <(echo "$rust_symbols") <(echo "$header_symbols"))
missing_in_rust=$(comm -13 <(echo "$rust_symbols") <(echo "$header_symbols"))

status=0

if [[ -n "$missing_in_header" ]]; then
    echo "FFI drift: declared in src/ffi.rs but missing from include/blackbox_ffi.h:" >&2
    echo "$missing_in_header" | sed 's/^/  - /' >&2
    status=1
fi

if [[ -n "$missing_in_rust" ]]; then
    echo "FFI drift: declared in include/blackbox_ffi.h but missing from src/ffi.rs:" >&2
    echo "$missing_in_rust" | sed 's/^/  - /' >&2
    status=1
fi

if [[ "$status" -eq 0 ]]; then
    count=$(echo "$rust_symbols" | grep -c '.' || true)
    echo "FFI header parity OK ($count symbols)."
fi

exit "$status"
