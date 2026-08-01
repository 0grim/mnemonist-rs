#!/usr/bin/env bash
#
# Live project status, DERIVED rather than maintained.
#
# Every number here is read from an artifact that some gate already had to
# produce: tests/scope.txt, fuzz/log.txt, bench/results.json, docs/modules/,
# and git. Nothing is hand-updated, so nothing can go stale — which is the
# point, because the hand-maintained log in planning/NOTES.md did go stale.
#
# Usage:  scripts/status.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

KICKOFF=$(date -u -d '2026-07-31 18:00' +%s)
FREEZE=$(date -u -d '2026-08-03 18:00' +%s)
NOW=$(date -u +%s)

bar() { printf '%.0s─' $(seq 1 "$1"); echo; }

echo
echo "mnemonist-rs — status at $(date -u '+%Y-%m-%d %H:%M UTC')"
bar 66
printf "  H+%dh%02dm   ·   %dh%02dm to code freeze\n" \
  $(( (NOW-KICKOFF)/3600 )) $(( ((NOW-KICKOFF)%3600)/60 )) \
  $(( (FREEZE-NOW)/3600 ))  $(( ((FREEZE-NOW)%3600)/60 ))

# ---------------------------------------------------------------- coverage
TOTAL=0
for t in tests/original/test/*.js; do TOTAL=$((TOTAL + $(wc -l < "$t"))); done

# Space-separated on purpose: the membership tests below are glob matches on
# " $DONE_UNITS ", which never match against newline separators.
DONE_UNITS=$(grep -vE '^[[:space:]]*(#|$)' tests/scope.txt 2>/dev/null | tr '\n' ' ' || true)
DONE_LINES=0
for u in $DONE_UNITS; do
  [ -f "tests/original/test/$u.js" ] && DONE_LINES=$((DONE_LINES + $(wc -l < "tests/original/test/$u.js")))
done

# Pending = has a divergence doc but is not yet in scope.txt.
PENDING_UNITS=""; PENDING_LINES=0
for d in docs/modules/*.md; do
  [ -e "$d" ] || continue
  u=$(basename "$d" .md)
  case " $DONE_UNITS " in *" $u "*) continue;; esac
  PENDING_UNITS="$PENDING_UNITS $u"
  [ -f "tests/original/test/$u.js" ] && PENDING_LINES=$((PENDING_LINES + $(wc -l < "tests/original/test/$u.js")))
done

echo
echo "  Coverage by upstream test weight"
bar 66
printf "  %-34s %6d lines  %3d%%\n" "DONE (in scope.txt)"        "$DONE_LINES"    $(( DONE_LINES*100/TOTAL ))
printf "  %-34s %6d lines  %3d%%\n" "pending (doc, awaiting gates)" "$PENDING_LINES" $(( PENDING_LINES*100/TOTAL ))
printf "  %-34s %6d lines\n"        "repo total (42 files)"       "$TOTAL"

# ------------------------------------------------------------ per-unit gates
echo
echo "  Evidence per unit          shim  test  doc  fuzz  bench   scope"
bar 66
for u in $DONE_UNITS $PENDING_UNITS; do
  [ -f "tests/bridge/$u.js" ]        && s="  ok " || s="  -- "
  [ -f "tests/original/test/$u.js" ] && t="  ok " || t="  -- "
  [ -f "docs/modules/$u.md" ]        && d=" ok " || d=" --  "
  grep -v '^[[:space:]]*#' fuzz/log.txt 2>/dev/null | grep -q "module=$u " && f="  ok " || f="  -- "
  node -e 'const r=require("./bench/results.json");process.exit((r.modules||{})[process.argv[1]]?0:1)' "$u" 2>/dev/null \
     && b="  ok " || b="  -- "
  case " $DONE_UNITS " in *" $u "*) sc="  DONE";; *) sc=" pend";; esac
  printf "  %-24s %s  %s  %s  %s  %s  %s\n" "$u" "$s" "$t" "$d" "$f" "$b" "$sc"
done

# --------------------------------------------------------------- in flight
echo
echo "  In flight (isolated worktrees)"
bar 66
git worktree list 2>/dev/null | grep -v "^$ROOT " | grep -v prunable \
  | sed 's|.*/\.claude/worktrees/||' | awk '{printf "  %s  %s\n", $2, $3}' \
  | head -8
[ -z "$(git worktree list | grep worktrees)" ] && echo "  (none)"

# ------------------------------------------------------------------ health
echo
echo "  Health"
bar 66
sha256sum -c tests/SHA256SUMS --quiet 2>/dev/null \
  && echo "  hashes      PASS ($(wc -l < tests/SHA256SUMS) files)" \
  || echo "  hashes      *** FAIL ***"
printf "  commits     %s on main, %s\n" \
  "$(git rev-list --count HEAD)" \
  "$(git status -sb | head -1 | grep -oE '\[.*\]' || echo 'in sync with origin')"
# pgrep exits non-zero on no matches AND prints 0, so `|| echo 0` would emit twice.
NODE_PROCS=$(pgrep -c node 2>/dev/null); CARGO_PROCS=$(pgrep -c 'cargo|rustc' 2>/dev/null)
printf "  machine     load %s   node:%s cargo:%s\n" \
  "$(cut -d' ' -f1 /proc/loadavg)" "${NODE_PROCS:-0}" "${CARGO_PROCS:-0}"
echo
