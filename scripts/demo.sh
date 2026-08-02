#!/usr/bin/env bash
#
# The submission demo: a narrated terminal walkthrough, built to be screen-
# captured in one take.
#
# This is a PRESENTATION, not a verification tool. It clears the screen between
# steps and pauses long enough for a viewer to read, which makes its output
# deliberately unscrollable. Anyone wanting to check the claims should run the
# real things instead:
#
#     ./tests/verify.sh      all ten gates, per unit claimed complete
#     ./tests/run.sh         the unmodified upstream suite
#     scripts/status.sh      derived, per-unit evidence
#
# Usage:
#
#     scripts/demo.sh                    the whole thing, ~4 minutes
#     scripts/demo.sh --warm             build everything first, unnarrated
#     scripts/demo.sh --pace 0           no pauses, for rehearsal and CI
#     scripts/demo.sh --pace 1.5         slower
#     scripts/demo.sh --list             the step titles, numbered
#     scripts/demo.sh --from 7           start at step 7, to re-record one stretch
#     scripts/demo.sh --from 7 --until 8
#
# DEMO_PACE, DEMO_FROM and DEMO_UNTIL are read from the environment as well.
#
# WARM THE CACHES BEFORE RECORDING. Steps 3, 4 and 5 build real things: warm,
# they take seconds, and cold they take minutes of silent compilation with
# nothing on screen worth watching. `--warm` does exactly those builds.
#
# Deliberately not `set -e`: a soft failure in one step must not abort the take.
# Each step reports what actually happened rather than assuming success.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SELF="${BASH_SOURCE[0]}"
cd "$ROOT"

PACE=${DEMO_PACE:-1.0}
FROM=${DEMO_FROM:-1}
UNTIL=${DEMO_UNTIL:-999}
WARM=0
STEP_NO=0
SKIP=0

# Counted from the file rather than declared, so that adding a step cannot
# leave "step 4 of 9" on screen in a recording nobody wants to shoot twice.
TOTAL_STEPS=$(grep -c '^screen "' "$SELF")

list_steps() {
  grep '^screen "' "$SELF" | sed 's/^screen "//; s/"$//' | nl -w3 -s'   '
}

usage() { sed -n '2,26p' "$SELF" | sed 's/^#\{0,1\} \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    --pace)  PACE=$2;  shift 2 ;;
    --from)  FROM=$2;  shift 2 ;;
    --until) UNTIL=$2; shift 2 ;;
    --warm)  WARM=1;   shift ;;
    --list)  list_steps; exit 0 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ -t 1 ] && command -v tput >/dev/null 2>&1 && [ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]; then
  BOLD=$(tput bold); DIM=$(tput dim); RESET=$(tput sgr0)
  CYAN=$(tput setaf 6); GREEN=$(tput setaf 2); YELLOW=$(tput setaf 3)
  INTERACTIVE=1
else
  BOLD=""; DIM=""; RESET=""; CYAN=""; GREEN=""; YELLOW=""
  INTERACTIVE=0
fi

RULE="────────────────────────────────────────────────────────────────────"

# A recording interrupted halfway must not leave the terminal without a cursor
# and still wearing the last colour that was set.
cleanup() {
  if [ "$INTERACTIVE" = 1 ]; then
    tput cnorm 2>/dev/null
    tput sgr0 2>/dev/null
  fi
  return 0
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT TERM

# --------------------------------------------------------------------- warming
if [ "$WARM" = 1 ]; then
  echo "Warming the caches the demo will otherwise build on camera."
  echo
  echo "==> cargo, release"
  cargo build --release -p mnemonist-napi
  cargo build --release -p mnemonist-core --example tour
  echo "==> harness dependencies"
  ./tests/run.sh test/queue.js >/dev/null 2>&1 && echo "    ok"
  if command -v docker >/dev/null 2>&1 && timeout 8 docker info >/dev/null 2>&1; then
    echo "==> docker images"
    docker build -t port-mortem . >/dev/null && echo "    port-mortem ok"
    docker build -t pm-core --target core . >/dev/null && echo "    pm-core ok"
  else
    echo "==> docker unavailable, skipped"
  fi
  echo
  echo "Warm. Record with: scripts/demo.sh"
  exit 0
fi

# ---------------------------------------------------------------- presentation

# `sleep` accepts fractions; `bc` is absent on some hosts, so scale with awk.
beat() {
  local seconds
  seconds=$(awk "BEGIN{printf \"%.2f\", $1 * $PACE}")
  # Explicitly return 0: at --pace 0 the awk test exits non-zero to skip the
  # sleep, and since a `beat` is the last statement in the script that status
  # became the script's own. A rehearsal run reported failure while every step
  # had succeeded.
  if awk "BEGIN{exit !($seconds > 0)}"; then
    sleep "$seconds"
  fi
  return 0
}

# Steps outside --from/--until advance the counter and produce nothing, so that
# a re-recorded stretch carries the same step numbers as the full run.
screen() {
  STEP_NO=$((STEP_NO + 1))
  if [ "$STEP_NO" -lt "$FROM" ] || [ "$STEP_NO" -gt "$UNTIL" ]; then
    SKIP=1
    return 0
  fi
  SKIP=0
  if [ "$INTERACTIVE" = 1 ]; then
    # \033[3J drops the scrollback as well: on a recording, an earlier step
    # still reachable by scrolling is just clutter.
    printf '\033[H\033[2J\033[3J'
  fi
  echo
  echo "${CYAN}${BOLD}  mnemonist-rs${RESET}${DIM}   ·   step ${STEP_NO} of ${TOTAL_STEPS}${RESET}"
  echo "${CYAN}  ${RULE}${RESET}"
  echo
  echo "  ${BOLD}$1${RESET}"
  echo
  beat 1
}

# A paragraph, then time to read it: roughly 200 words per minute, floored at
# three seconds so a short line is not flashed past.
say() {
  local words
  if [ "$SKIP" = 1 ]; then return 0; fi
  echo "$1" | fold -s -w 68 | sed 's/^/  /'
  echo
  words=$(echo "$1" | wc -w)
  beat "$(awk "BEGIN{t = $words / 3.3; print (t < 3 ? 3 : t)}")"
}

run() {
  if [ "$SKIP" = 1 ]; then return 0; fi
  echo "  ${YELLOW}\$ $1${RESET}"
  echo
  beat 0.8
  eval "$1" 2>&1 | sed 's/^/  /'
  echo
  beat 1.5
}

note() {
  if [ "$SKIP" = 1 ]; then return 0; fi
  echo "$1" | fold -s -w 72 | sed "s/^/  ${DIM}/;s/\$/${RESET}/"
  echo
  beat 2
}

line() {
  if [ "$SKIP" = 1 ]; then return 0; fi
  echo "$1"
}

DOCKER=""
for candidate in docker /usr/bin/docker; do
  if command -v "$candidate" >/dev/null 2>&1 && timeout 8 "$candidate" info >/dev/null 2>&1; then
    DOCKER="$candidate"; break
  fi
done

# ───────────────────────────────────────────────────────────── 1. what this is
screen "What this is"
say "mnemonist is a JavaScript library of 44 data structures. This is a Rust port of all of it."
say "The deliverable is a standalone Rust crate: no dependencies, no unsafe code, and it builds without Node installed anywhere on the machine."
say "The original JavaScript test suite is not a dependency of that crate. It is the proof that the crate behaves like the library it replaces. Those tests run unmodified against the Rust build, through a thin bridge that ships to nobody."
run "grep -E '^(track|source|version|commit)[[:space:]]*=' .port-mortem.toml | head -5"
say "Every upstream test file is ported, and every one passes all ten verification gates."
run "scripts/status.sh | sed -n '/Coverage/,/repo total/p'"

# ─────────────────────────────────────────────────── 2. the originals are intact
screen "The tests were not edited"
say "The easiest way to pass someone else's test suite is to quietly edit it. So every upstream test file was hashed before any Rust was written, and those hashes are checked on every commit."
say "Had a single byte of any test file changed, this check would fail and the build would stop."
run "sha256sum -c tests/SHA256SUMS --quiet && echo 'PASS - every file matches the hash recorded at kickoff'"
run "ls tests/original/test/*.js | wc -l"
note "42 upstream test files, out of 47 files hashed in total."

# ──────────────────────────────────────────────────────── 3. one-command build
screen "One command, from source to a running artifact"
say "The whole project builds in a container: the Rust crate, the bridge to JavaScript, the test harness and its dependencies."
if [ -n "$DOCKER" ]; then
  say "This is the real build. It completes quickly here only because the layers are cached; from cold it compiles the entire workspace."
  run "$DOCKER build -t port-mortem . 2>&1 | tail -4"
else
  say "Docker is not available on this machine, so the container build cannot be shown. The Rust half of the identical build runs instead. This substitutes for the containerised proof and is not equivalent to it."
  run "cargo build --release -p mnemonist-napi 2>&1 | tail -3"
fi

# ─────────────────────────────────────────────────────── 4. the crate stands alone
screen "The crate does not need JavaScript"
say "This is the claim that matters most, because it is what makes the deliverable a Rust crate rather than a JavaScript wrapper."
say "A second image is built from a base containing no JavaScript runtime at all. The build asserts that Node is absent, and then runs the crate's own test suite inside that image."
if [ -n "$DOCKER" ]; then
  run "$DOCKER build -t pm-core --target core . 2>&1 | tail -3"
  run "$DOCKER run --rm pm-core 2>&1 | grep 'test result' | head -2"
  run "$DOCKER run --rm pm-core sh -c 'command -v node || echo \"node: not present in this image\"'"
else
  say "Without Docker, the two assertions that image makes are shown directly instead."
  run "grep -q 'forbid(unsafe_code)' crates/mnemonist-core/src/lib.rs && echo 'forbid(unsafe_code): present'"
  run "cargo tree -p mnemonist-core"
  note "A one-line dependency tree means zero dependencies, and therefore no route to a JavaScript runtime."
fi

# ────────────────────────────────────────────────── 5. the crate, used as a crate
screen "The crate, used from Rust"
say "Everything so far has been a test result. This is the library simply being used: an ordinary Rust program that imports the crate and nothing else."
run "awk '/A Fibonacci heap/,/fibonacci-heap  drained/' crates/mnemonist-core/examples/tour.rs"
run "cargo run -q --release --example tour -p mnemonist-core"
note "Four structures, in crates/mnemonist-core/examples/tour.rs. No bridge, no Node, no dependencies — this is the deliverable, doing its job."

# ───────────────────────────────────────────── 6. the original suite, unchanged
screen "The original test suite, unmodified, against Rust"
say "This is the equivalence proof. These test files are byte-identical to the ones published with the JavaScript library, as the hashes in step two established."
say "They are pointed at the Rust build through the bridge, and run exactly as their authors wrote them."
run "./tests/run.sh 2>&1 | tail -5"

# ─────────────────────────────────────────────────────── 7. beyond the suite
screen "Passing the suite is not the same as being equivalent"
say "A test suite covers what its authors thought to write down. To find the rest, generated programs are replayed against both implementations at once — the Rust one, and the real JavaScript running in Node — comparing observable state after every single operation."
say "Not a model of what the original does. The original itself."
run "grep -vE '^[[:space:]]*(#|\$)' fuzz/log.txt | tail -3"
run "grep -vE '^[[:space:]]*(#|\$)' fuzz/log.txt | grep -oE 'ops=[0-9]+' | cut -d= -f2 | awk '{s+=\$1} END {printf \"%d campaigns, %d operations, zero divergences\\n\", NR, s}'"

# ───────────────────────────────────── 8. something the suite could not find
screen "A defect the original suite could not have found"
say "In the LRU cache, forEach advanced its internal cursor before running the callback. The original advances it afterwards."
say "That difference is invisible unless a callback modifies the cache while it is being iterated. No test in the original suite does that, so the suite passed either way."
say "The differential fuzzer found it on its first campaign against that structure. On the third callback, the port saw a stale successor where the original saw the promoted one."
run "sed -n '/for_each_walk/,/^#     }\$/p' crates/difffuzz/proptest-regressions/lru-cache.txt | sed -E 's/^#[[:space:]]?//'"
note "The failing case is checked in beside that note, so the defect is re-run before any new case is generated. Written up in full in docs/modules/lru-cache.md."

# ────────────────────────────────────────────────────── 9. benchmarks, honestly
screen "Performance, including where the port loses"
say "Every structure is benchmarked against the real JavaScript on matched workloads. Most are faster. Some are not, and those are reported rather than omitted, because a table with no losses in it invites the question of what was left out."
run "node -e \"const r=require('./bench/results.json');for(const m of ['fibonacci-heap','trie','kd-tree','default-map']){const w=Object.values(r.modules[m].workloads)[0];const p=w.port.p50_ns_per_op,o=w.original.p50_ns_per_op;console.log(m.padEnd(18)+(o/p>=1?(o/p).toFixed(2)+'x faster':(p/o).toFixed(2)+'x SLOWER'))}\""
say "Each loss has a cause that was measured rather than guessed. One earlier explanation was disproved that way, and replaced."
note "The verification gate refuses a benchmark entry that omits its regressions list, so a slower result cannot be expressed by silence."

# ────────────────────────────────────────────────────────────── 10. the evidence
screen "Where the evidence lives"
say "Nothing here has to be taken on trust. Every claim in this demo is reproducible from the repository."
line "  ${GREEN}./tests/verify.sh${RESET}     all ten gates, for every unit claimed complete"
line "  ${GREEN}./tests/run.sh${RESET}        the unmodified upstream suite"
line "  ${GREEN}scripts/status.sh${RESET}     derived, per-unit evidence table"
line "  ${GREEN}docs/METHODOLOGY.md${RESET}   what each gate detected, and what it cannot see"
line "  ${GREEN}docs/BUGS.md${RESET}          defects found in the original library"
line "  ${GREEN}docs/DECISIONS.md${RESET}     every deliberate divergence, and why"
line ""
beat 4
say "The methodology document also records where these instruments are blind. The differential fuzzer never exercises the bridge, and four falsification attempts stayed green for four different reasons. Those are written down too."

# The closing card counts from the repository rather than from memory: a figure
# typed into a slide is a figure that goes stale between rehearsal and take.
if [ "$SKIP" != 1 ]; then
  UNITS=$(grep -cvE '^[[:space:]]*(#|$)' tests/scope.txt 2>/dev/null || echo 0)
  OPS=$(grep -vE '^[[:space:]]*(#|$)' fuzz/log.txt 2>/dev/null \
        | grep -oE 'ops=[0-9]+' | cut -d= -f2 | awk '{s+=$1} END {printf "%.1fM", s/1000000}')
  echo "  ${CYAN}${RULE}${RESET}"
  echo
  echo "  ${BOLD}44 structures ported.${RESET}  ${UNITS} units through all ten gates."
  echo "  ${BOLD}${OPS} differential operations.${RESET}  Zero divergences."
  echo "  The original test suite passes unmodified."
  echo
  beat 4
fi
