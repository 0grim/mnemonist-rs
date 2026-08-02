#!/usr/bin/env bash
#
# Five-minute demo, narrated (DESIGN.md 12b). A script rather than something
# performed live: it is rehearsable, re-runnable, and immune to a fumbled
# command on camera. Echoes what it is about to do before every command, so
# it can be screen-captured in one take.
#
# Deliberately not `set -e`: one soft step (the fuzz sample, or a broken
# Docker on the recording machine) must not abort the whole take. Each step
# reports what actually happened rather than assuming success -- the same
# rule the rest of this project runs on (CLAUDE.md: no confident green that
# isn't real). If Docker is unavailable, steps 3 and 4 say so on screen and
# fall back to the closest non-Docker evidence instead of silently skipping.
#
# Usage:  scripts/demo.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DEMO_START=$(date +%s)
STEP_START=$DEMO_START

step() {
  local now
  now=$(date +%s)
  echo
  echo "############################################################"
  printf "# STEP %s   (previous step: %ds)\n" "$1" "$((now - STEP_START))"
  echo "# $2"
  echo "############################################################"
  STEP_START=$now
}

docker_usable() {
  command -v docker >/dev/null 2>&1 && timeout 8 docker info >/dev/null 2>&1
}

# ------------------------------------------------------------- step 1: env
step 1 "Environment: versions, commit, track + scope"
echo "\$ node -v && rustc --version && cargo --version"
node -v
rustc --version
cargo --version
echo "\$ git rev-parse HEAD"
git rev-parse HEAD
echo "\$ grep -E '^(track|source|kickoff_hash)[[:space:]]*=' .port-mortem.toml"
grep -E '^(track|source|kickoff_hash)[[:space:]]*=' .port-mortem.toml

# ------------------------------------------------ step 2: originals untouched
step 2 "Parity proof: upstream tests byte-identical to the kickoff hash"
echo "\$ sha256sum -c tests/SHA256SUMS --quiet"
if sha256sum -c tests/SHA256SUMS --quiet; then
  echo "PASS -- $(wc -l < tests/SHA256SUMS) upstream test files unmodified since kickoff"
else
  echo "FAIL -- tests/original/ has diverged from the kickoff hash"
fi
echo
echo "\$ git diff --stat HEAD -- tests/original/"
git diff --stat HEAD -- tests/original/
echo "(empty output above: the working tree matches exactly what was committed and hashed)"

# ---------------------------------------------------- step 3: one-command build
step 3 "One-command build"
if docker_usable; then
  echo "\$ docker build -t port-mortem ."
  docker build -t port-mortem .
else
  echo "Docker is not usable in this environment (checked: command -v docker && docker info)."
  echo "Falling back to the Rust half of the same build directly -- the Dockerfile builder"
  echo "stage runs this exact command inside the image:"
  echo "\$ cargo build --release -p mnemonist-napi"
  cargo build --release -p mnemonist-napi
fi

# --------------------------------------------- step 4: the FFI-rule rebuttal
step 4 "The FFI-rule rebuttal: mnemonist-core builds and tests with no Node"
if docker_usable; then
  echo "\$ docker build -t pm-core --target core . && docker run --rm pm-core"
  docker build -t pm-core --target core . && docker run --rm pm-core
else
  echo "Docker is not usable here, so the containerized 'no Node in this image' proof cannot"
  echo "run on camera in this environment. Showing the same two assertions the core target"
  echo "makes, run locally instead -- this substitutes for, and is not equivalent to, the"
  echo "containerized proof:"
  echo "\$ grep -q 'forbid(unsafe_code)' crates/mnemonist-core/src/lib.rs && echo OK"
  grep -q 'forbid(unsafe_code)' crates/mnemonist-core/src/lib.rs && echo OK
  echo "\$ cargo tree -p mnemonist-core   # 1 line == zero dependencies == no JS runtime reachable"
  cargo tree -p mnemonist-core
fi

# --------------------------------------------------------- step 5: the money shot
step 5 "The money shot: unmodified upstream suite, through the bridge"
echo "\$ ./tests/run.sh"
./tests/run.sh

# --------------------------------------------------- step 6: fuzz sample + totals
step 6 "Live fuzz sample, plus the accumulated campaign totals"
echo "\$ fuzz/run.sh heap 5 42"
fuzz/run.sh heap 5 42 || echo "(non-zero exit is itself informative here -- see fuzz/log.txt)"
echo
echo "\$ tail -5 fuzz/log.txt"
tail -5 fuzz/log.txt
TOTAL_OPS=$(grep -v '^[[:space:]]*#' fuzz/log.txt | grep -oE 'ops=[0-9]+' | cut -d= -f2 | awk '{s+=$1} END {print s+0}')
TOTAL_CAMPAIGNS=$(grep -vc '^[[:space:]]*#' fuzz/log.txt)
echo "accumulated: $TOTAL_CAMPAIGNS campaigns, $TOTAL_OPS ops logged"

# --------------------------------------------- step 7: a caught divergence + fix
step 7 "A real divergence the fuzzer caught, and its fix"
REGR="crates/difffuzz/proptest-regressions/lru-cache.txt"
echo "\$ wc -l $REGR"
wc -l "$REGR"
echo "\$ grep -v '^#' $REGR | head -c 240   # the minimised op sequence itself, not the header"
grep -v '^#' "$REGR" | head -c 240; echo " ...(truncated)"
echo
echo "What it caught: forEach advanced its cursor BEFORE the callback ran; upstream advances"
echo "AFTER. No test in the original suite mutates from inside a forEach callback, so gate 4"
echo "could not have found this -- the differential fuzzer's first campaign against this"
echo "grammar did. Fixed with ForEachWalk (mnemonist_core::structures::lru_cache), which splits"
echo "'read the current position' from 'advance' into two calls the caller controls."
echo "Full writeup: docs/modules/lru-cache.md, 'Bugs this found' #2."

# ----------------------------------------------- step 8: benchmarks, honestly
step 8 "Benchmarks -- including a row where the port loses"
echo "\$ node -e '...' bench/results.json   # bit-set / mixed-1e6"
node -e "
const r = require('./bench/results.json');
const m = r.modules['bit-set'].workloads['mixed-1e6'];
console.log('bit-set / mixed-1e6, port vs upstream:');
for (const row of m.regressions) {
  console.log('  ' + row.metric.padEnd(16) + 'port=' + row.port + '  original=' + row.original + '  ratio=' + row.ratio + 'x (port slower)');
}
"
echo "Declared, not hidden: gate 10 requires a regressions array on every workload, even an"
echo "empty one -- a regression has to be stated, never just absent."

# --------------------------------------------------- step 9: scope declaration
step 9 "Scope: what shipped, and what's still on the roadmap"
echo "\$ scripts/status.sh"
./scripts/status.sh | head -9
echo "..."
echo "Full unit-by-unit evidence table: scripts/status.sh. What's next: planning/ROADMAP.md."

DEMO_END=$(date +%s)
echo
echo "############################################################"
printf "# DONE -- total wall time: %ds\n" "$((DEMO_END - DEMO_START))"
echo "############################################################"
