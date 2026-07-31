#!/usr/bin/env bash
#
# Minimal harness runner. Deliberately crude: it assembles the work tree and
# runs the specs named on the command line, nothing more.
#
# The full design (scope.txt selection, tiered hash verification, repo-wide
# mode) is specified in planning/DESIGN.md 2.3 and lands at D1, once a second
# module has shown what the generator actually needs to emit.
#
# Usage:  tests/run.sh [spec ...]        default: every spec with a shim
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$ROOT/tests/.work"
ADDON="$WORK/node_modules/@port/addon"

# 1. Never run against modified originals.
( cd "$ROOT" && sha256sum -c tests/SHA256SUMS --quiet ) \
  || { echo "FATAL: tests/original/ has been modified. Refusing to run." >&2; exit 2; }

# 2. Build the bridge (cargo is incremental).
cargo build --release -p mnemonist-napi --manifest-path "$ROOT/Cargo.toml" >/dev/null

# 3. Assemble the work tree, preserving node_modules.
mkdir -p "$WORK"
find "$WORK" -mindepth 1 -maxdepth 1 \
     ! -name node_modules ! -name package-lock.json -exec rm -rf {} +
cp -R "$ROOT/tests/original/test" "$WORK/test"      # byte-identical originals
cp -R "$ROOT/tests/bridge/."      "$WORK/"          # our shims
cp    "$ROOT/tests/harness-package.json" "$WORK/package.json"

# 4. Publish the addon as a resolvable package so shims can require it from
#    any depth.
mkdir -p "$ADDON"
cp "$ROOT/target/release/libmnemonist_napi.so" "$ADDON/addon.node"
printf '{"name":"@port/addon","main":"addon.node"}' > "$ADDON/package.json"

# 5. Deps, only when missing.
[ -d "$WORK/node_modules/mocha" ] \
  || ( cd "$WORK" && npm install --no-audit --no-fund --silent )

# 6. Default to every spec we have a shim for.
cd "$WORK"
if [ "$#" -gt 0 ]; then
  SPECS=("$@")
else
  SPECS=()
  for shim in "$ROOT"/tests/bridge/*.js; do
    name="$(basename "$shim")"
    [ -f "test/$name" ] && SPECS+=("test/$name")
  done
fi

[ "${#SPECS[@]}" -gt 0 ] || { echo "FATAL: no specs to run." >&2; exit 2; }

echo "== mnemonist-rs · $(node -v) · ${#SPECS[@]} spec(s) =="
npx mocha --reporter spec "${SPECS[@]}"
