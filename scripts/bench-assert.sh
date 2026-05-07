#!/usr/bin/env bash
#
# Run a bench-writer command and fail if its real-time throughput multiplier
# falls below a floor. Used in CI to catch perf regressions that would
# otherwise pass the smoke test (which only checks exit code).
#
# Usage:
#   ./scripts/bench-assert.sh <min_realtime_multiplier> -- <bench-writer args...>
#
# Example:
#   ./scripts/bench-assert.sh 10 -- target/release/bench-writer \
#       --channels 64 --seconds 2 --mode single
#
# bench-writer prints a line of the form:
#   Real-time:   45.3x (need >1.0x)
# This script greps that line, parses the multiplier, and exits 1 if it is
# below the floor.

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 <min_realtime_multiplier> -- <bench-writer args...>" >&2
    exit 2
fi

min="$1"
shift
if [[ "${1:-}" == "--" ]]; then
    shift
fi

if [[ $# -lt 1 ]]; then
    echo "Error: no bench-writer command provided" >&2
    exit 2
fi

# bench-writer prints to stderr; capture both streams while preserving live output.
out=$("$@" 2>&1 | tee /dev/stderr)

# Allow grep to "fail" (no match) without tripping set -e / pipefail; the
# empty-string check below is the real failure handler.
mult=$(printf '%s\n' "$out" | grep -oE 'Real-time:[[:space:]]+[0-9.]+x' | grep -oE '[0-9.]+' | head -1 || true)

if [[ -z "$mult" ]]; then
    echo "FAIL: could not parse 'Real-time: NN.Nx' from bench-writer output" >&2
    exit 2
fi

if awk -v m="$mult" -v t="$min" 'BEGIN { exit !(m+0 < t+0) }'; then
    echo "FAIL: Real-time ${mult}x < ${min}x floor" >&2
    exit 1
fi

echo "OK: Real-time ${mult}x >= ${min}x floor"
