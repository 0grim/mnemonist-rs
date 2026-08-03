#!/usr/bin/env bash
#
# Minimal harness runner. Deliberately crude: it assembles the work tree and
# runs the specs named on the command line, nothing more.
#
# Selection is driven by tests/scope.txt; hash verification is tiered, and the
# repo-wide mode checks every upstream file rather than only the selected ones.
#
# Usage:  tests/run.sh [spec ... | all]  default/"all": every spec with a shim
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$ROOT/tests/.work"
ADDON="$WORK/node_modules/@port/addon"

# 1. Never run against modified originals.
( cd "$ROOT" && sha256sum -c tests/SHA256SUMS --quiet ) \
  || { echo "FATAL: tests/original/ has been modified. Refusing to run." >&2; exit 2; }

# 2. Build the bridge (cargo is incremental). Skipped when PM_NO_BUILD=1: the
#    Docker `parity` image has no Rust toolchain -- the compiled .so already
#    arrived from the `builder` stage.
[ "${PM_NO_BUILD:-}" = 1 ] \
  || cargo build --release -p mnemonist-napi --manifest-path "$ROOT/Cargo.toml" >/dev/null

# 3. Assemble the work tree, preserving node_modules.
mkdir -p "$WORK"
find "$WORK" -mindepth 1 -maxdepth 1 \
     ! -name node_modules ! -name package-lock.json -exec rm -rf {} +
cp -R "$ROOT/tests/original/test" "$WORK/test"      # byte-identical originals
cp -R "$ROOT/tests/bridge/."      "$WORK/"          # our shims
cp    "$ROOT/tests/harness-package.json" "$WORK/package.json"

# 3b. Our own specs for things upstream has no test file for -- the obliterator
#     primitives, which are a runtime dependency there and ported code here.
#     Kept out of test/ so the originals directory stays exactly the originals.
[ -d "$ROOT/tests/boundary" ] && cp -R "$ROOT/tests/boundary" "$WORK/boundary"

# 4. Deps, only when missing.
#
#    BEFORE publishing the addon, not after: npm prunes anything in
#    node_modules that its manifest does not mention, and @port/addon is
#    deliberately not in the manifest. Publishing first meant the very first
#    run on a fresh tree deleted the addon and failed to resolve it, while
#    every run after that passed -- a fresh-clone-only failure.
[ -d "$WORK/node_modules/mocha" ] \
  || ( cd "$WORK" && npm install --no-audit --no-fund --silent )

# 5. Publish the addon as a resolvable package so shims can require it from
#    any depth.
mkdir -p "$ADDON"
cp "$ROOT/target/release/libmnemonist_napi.so" "$ADDON/addon.node"
printf '{"name":"@port/addon","main":"addon.node"}' > "$ADDON/package.json"

# 6. Default to every spec we have a shim for. "all" is accepted as an alias
#    for the default: every ported module already has a shim (42/42 upstream
#    test files), so there is currently no repo-wide/in-scope split left to
#    make -- unlike the scope.txt-filtered selection sketched in `docs/METHODOLOGY.md`'s gate 3,
#    which would matter once a module is deliberately excluded from scope.txt
#    while its shim still exists. Revisit if that ever happens.
cd "$WORK"
if [ "$#" -gt 0 ] && [ "$1" != "all" ]; then
  SPECS=("$@")
else
  SPECS=()
  for shim in "$ROOT"/tests/bridge/*.js; do
    name="$(basename "$shim")"
    [ -f "test/$name" ] && SPECS+=("test/$name")
  done
  for spec in boundary/*.js; do
    [ -f "$spec" ] && SPECS+=("$spec")
  done
fi

[ "${#SPECS[@]}" -gt 0 ] || { echo "FATAL: no specs to run." >&2; exit 2; }

echo "== mnemonist-rs · $(node -v) · ${#SPECS[@]} spec(s) =="
npx mocha --reporter spec "${SPECS[@]}"
