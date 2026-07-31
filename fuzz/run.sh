#!/usr/bin/env bash
#
# Gate 9 runner: fuzz one module for at least 60 seconds and append the result
# to fuzz/log.txt.
#
# The binary prints one machine-readable summary line on stdout and everything
# human-facing on stderr, so this script only has to timestamp and append. A
# divergence exits 1 and is NOT logged as a clean run; a harness failure (no
# node, dead pipe) exits 2 and is not logged at all, because "the oracle never
# started" must never end up in the log looking like "zero divergences".
#
# Usage:  fuzz/run.sh <module> [seconds] [seed]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULE="${1:?usage: fuzz/run.sh <module> [seconds] [seed]}"
SECONDS_BUDGET="${2:-60}"
SEED="${3:-42}"
LOG="$ROOT/fuzz/log.txt"

cargo build --release -p difffuzz --manifest-path "$ROOT/Cargo.toml" >/dev/null

echo "== difffuzz · $MODULE · ${SECONDS_BUDGET}s · seed $SEED · $(node -v) =="

set +e
SUMMARY="$("$ROOT/target/release/difffuzz" \
  --module "$MODULE" --duration "$SECONDS_BUDGET" --seed "$SEED")"
STATUS=$?
set -e

case "$STATUS" in
  0) printf '%s  %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$SUMMARY" >> "$LOG"
     echo "logged: $SUMMARY" ;;
  1) printf '%s  %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$SUMMARY" >> "$LOG"
     echo "DIVERGENCE — logged, and the minimised seed is in crates/difffuzz/proptest-regressions/" >&2 ;;
  *) echo "harness failure (exit $STATUS): nothing logged" >&2 ;;
esac

exit "$STATUS"
