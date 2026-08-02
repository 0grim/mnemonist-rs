#!/usr/bin/env bash
#
# Executable Definition of Done (planning/DESIGN.md 1.1).
#
# Driven by tests/scope.txt: for every unit declared done, assert the evidence
# actually exists. That makes the done marker un-cheatable -- a unit cannot be
# listed without the artifacts to back it.
#
# Runs EVERY gate and reports all failures. It does not stop at the first,
# because at hour 50 you want the whole list, not a bisect.
#
# Usage:  tests/verify.sh
set -uo pipefail          # deliberately not -e: we collect failures

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PASS=0; FAIL=0; FAILED_GATES=()

ok()   { printf '  \033[32mPASS\033[0m  %s\n' "$1"; PASS=$((PASS+1)); }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; FAIL=$((FAIL+1)); FAILED_GATES+=("$1"); }
note() { printf '        %s\n' "$1"; }

scoped_units() { grep -vE '^[[:space:]]*(#|$)' tests/scope.txt 2>/dev/null || true; }

echo
echo "Definition of Done -- planning/DESIGN.md 1.1"
echo "============================================"

UNITS=$(scoped_units)
if [ -z "$UNITS" ]; then
  echo
  echo "  tests/scope.txt is empty: no unit claims to be done."
  echo "  Repo-wide gates still run."
  echo
fi

# ---------------------------------------------------------------- repo-wide
echo
echo "Repo-wide"
echo "---------"

sha256sum -c tests/SHA256SUMS --quiet 2>/dev/null \
  && ok "gate 5  original test suite unmodified ($(wc -l < tests/SHA256SUMS) files hashed)" \
  || bad "gate 5  tests/original/ HAS BEEN MODIFIED"

grep -q '^#!\[forbid(unsafe_code)\]' crates/mnemonist-core/src/lib.rs 2>/dev/null \
  && ok "gate 2a forbid(unsafe_code) declared in core" \
  || bad "gate 2a forbid(unsafe_code) missing from crates/mnemonist-core/src/lib.rs"

# The port must not need a JS runtime. One line of `cargo tree` means no deps.
CORE_DEPS=$(cargo tree -p mnemonist-core 2>/dev/null | wc -l)
if [ "$CORE_DEPS" -eq 1 ]; then
  ok "gate 2b core has zero dependencies (no JS runtime reachable)"
else
  bad "gate 2b core dependency tree is $CORE_DEPS lines, expected 1"
  note "$(cargo tree -p mnemonist-core 2>/dev/null | tail -n +2 | head -5)"
fi

cargo build --release -p mnemonist-core >/dev/null 2>&1 \
  && ok "gate 2c core builds standalone" \
  || bad "gate 2c core fails to build standalone"

cargo fmt --all --check >/dev/null 2>&1 \
  && ok "        cargo fmt clean" \
  || bad "        cargo fmt would reformat -- run: cargo fmt --all"

cargo clippy --all-targets -- -D warnings >/dev/null 2>&1 \
  && ok "        clippy clean (-D warnings)" \
  || bad "        clippy reports warnings -- run: cargo clippy --all-targets -- -D warnings"

# Output is captured rather than discarded: this gate once reported FAILING
# with no indication of what failed, and `cargo test` run by hand immediately
# afterwards was green. A gate that says only "FAILING" invites re-running
# until it passes, which is precisely the habit the Definition of Done exists
# to prevent -- so when it fails it must say what failed, and the flake has to
# be visible rather than smoothed over by a second attempt.
if TEST_OUT=$(cargo test 2>&1); then
  ok "gate 7  Rust native tests pass"
else
  bad "gate 7  Rust native tests FAILING"
  note "$(echo "$TEST_OUT" | grep -E '^test .* FAILED|^error|panicked at' | head -5)"
  note "re-run before assuming a flake; a green second attempt is not a passing gate"
fi

# ------------------------------------------------------------------ per-unit
for unit in $UNITS; do
  echo
  echo "Unit: $unit"
  printf -- '-%.0s' $(seq 1 $((6 + ${#unit}))); echo

  # gate 3 -- a shim must exist, or the original test cannot resolve its require
  [ -f "tests/bridge/$unit.js" ] \
    && ok "gate 3  bridge shim present" \
    || bad "gate 3  tests/bridge/$unit.js missing"

  # gate 4 -- the original test file, unmodified, green through the bridge
  if [ -f "tests/original/test/$unit.js" ]; then
    if OUT=$(./tests/run.sh "test/$unit.js" 2>&1); then
      ok "gate 4  original test green ($(echo "$OUT" | grep -oE '[0-9]+ passing' | head -1))"
    else
      bad "gate 4  original test FAILING"
      note "$(echo "$OUT" | grep -E 'failing|AssertionError' | head -3)"
    fi
  else
    bad "gate 4  tests/original/test/$unit.js does not exist"
  fi

  # gate 8 -- divergence doc, present and substantive, with the sections that matter
  DOC="docs/modules/$unit.md"
  if [ -f "$DOC" ] && [ "$(wc -l < "$DOC")" -ge 40 ]; then
    MISSING=""
    for section in "What upstream tests" "What upstream does NOT test" \
                   "What we test in addition" "Bugs this found" \
                   "Deliberate divergences" "Fuzz + bench"; do
      grep -qF "## $section" "$DOC" || MISSING="$MISSING '$section'"
    done
    [ -z "$MISSING" ] \
      && ok "gate 8  divergence doc complete ($(wc -l < "$DOC") lines)" \
      || bad "gate 8  divergence doc missing section(s):$MISSING"
  else
    bad "gate 8  $DOC missing or a stub (<40 lines)"
  fi

  # gate 6 -- NOT verified here, only that it was recorded. Performing it
  # generically would be mutation testing; a weak version would give false
  # confidence, which is the exact failure this gate exists to catch.
  if [ -f "$DOC" ] && grep -qiE "falsif|sabotag" "$DOC"; then
    ok "gate 6  falsification recorded in the doc (manual step, not re-run here)"
  else
    bad "gate 6  no falsification recorded in $DOC"
  fi

  # gate 9 -- a logged campaign with zero divergences.
  #
  # Commented lines are skipped, which is load-bearing rather than tidy: a
  # campaign whose coverage was later found to be overstated is commented out
  # with its reason instead of deleted, and summing it back in would restate
  # the very number the correction withdrew.
  CAMPAIGNS=$(grep -v '^[[:space:]]*#' fuzz/log.txt 2>/dev/null | grep "module=$unit " || true)
  if [ -n "$CAMPAIGNS" ]; then
    BAD_RUNS=$(echo "$CAMPAIGNS" | grep -vc "divergences=0")
    TOTAL_OPS=$(echo "$CAMPAIGNS" | grep -oE 'ops=[0-9]+' | cut -d= -f2 \
                | awk '{s+=$1} END {print s+0}')
    if [ "$BAD_RUNS" -eq 0 ]; then
      ok "gate 9  fuzz clean ($(echo "$CAMPAIGNS" | grep -c .) campaign(s), ${TOTAL_OPS} ops)"
    else
      bad "gate 9  $BAD_RUNS fuzz campaign(s) reported divergences"
    fi
  else
    bad "gate 9  no fuzz campaign logged for $unit in fuzz/log.txt"
  fi

  # gate 10 -- benchmark entry. Results are nested per workload, and a
  # regression must be *stated*, never absent.
  if [ -f bench/results.json ]; then
    BENCH=$(node -e '
      const unit = process.argv[1];
      const r = require("./bench/results.json");
      const m = (r.modules || {})[unit];
      if (!m || !m.workloads) { console.log("MISSING"); process.exit(0); }
      const names = Object.keys(m.workloads);
      if (!names.length) { console.log("MISSING"); process.exit(0); }
      let regs = 0;
      for (const n of names) {
        const w = m.workloads[n];
        if (!w.port || !w.original) { console.log("INCOMPLETE:" + n); process.exit(0); }
        if (!Array.isArray(w.regressions)) { console.log("NOREGFIELD:" + n); process.exit(0); }
        regs += w.regressions.length;
      }
      console.log("OK:" + names.length + ":" + regs);
    ' "$unit" 2>/dev/null)
    case "$BENCH" in
      OK:*) ok "gate 10 bench recorded ($(echo "$BENCH" | cut -d: -f2) workload(s), $(echo "$BENCH" | cut -d: -f3) declared regression(s))" ;;
      INCOMPLETE:*) bad "gate 10 bench workload '${BENCH#INCOMPLETE:}' missing port or original figures" ;;
      NOREGFIELD:*) bad "gate 10 bench workload '${BENCH#NOREGFIELD:}' has no regressions array -- a regression must be stated, not omitted" ;;
      *) bad "gate 10 no bench entry for $unit in bench/results.json" ;;
    esac
  else
    bad "gate 10 bench/results.json missing"
  fi
done

# ------------------------------------------------------------------- verdict
echo
echo "============================================"
if [ "$FAIL" -eq 0 ]; then
  printf '  \033[32mALL GATES PASS\033[0m  (%d checks, %d unit(s) in scope)\n\n' \
    "$PASS" "$(echo "$UNITS" | grep -c . || echo 0)"
  exit 0
fi

printf '  \033[31m%d GATE(S) FAILED\033[0m  (%d passed)\n\n' "$FAIL" "$PASS"
for g in "${FAILED_GATES[@]}"; do echo "    - $g"; done
echo
exit 1
