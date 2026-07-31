#!/usr/bin/env bash
#
# Gate 10 runner: benchmark one module against the vendored upstream JS and
# append a keyed entry to bench/results.json.
#
# Everything methodological lives in bench/drive.js and bench/methodology.md;
# this only builds the Rust side and hands over. Unattended and resumable by
# design (DESIGN.md 5.2, "Driver"): results.json is merged, not overwritten, so
# per-module runs accumulate.
#
# Usage:  bench/run.sh [module]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULE="${1:-static-disjoint-set}"

# The pure-Rust path, linked against mnemonist-core directly. Never the napi
# cdylib: bridge overhead would poison the comparison (5.1).
cargo build --release -p bench-runner --manifest-path "$ROOT/Cargo.toml" >/dev/null

echo "== bench · $MODULE · $(node -v) · $(rustc --version) =="

exec node "$ROOT/bench/drive.js" "$MODULE"
