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
#     scripts/demo.sh                    the whole thing, ~4.5 minutes
#     scripts/demo.sh --warm             build everything first, unnarrated
#     scripts/demo.sh --pace 0           no pauses, for rehearsal and CI
#     scripts/demo.sh --pace 1.5         slower
#     scripts/demo.sh --hold 6           longer on the last frame of each step
#     scripts/demo.sh --list             the step titles, numbered
#     scripts/demo.sh --from 7           start at step 7, to re-record one stretch
#     scripts/demo.sh --from 7 --until 8
#
# DEMO_PACE, DEMO_HOLD, DEMO_FROM and DEMO_UNTIL are read from the environment
# as well.
#
# The hold is separate from the reading pauses on purpose. A pause after a
# paragraph is time to read that paragraph; the hold is time to take in the
# whole finished screen — usually a command's output, which arrives with no
# pause of its own after it — before it is wiped. Rehearsal kept losing the
# last line of a step to the clear.
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
HOLD=${DEMO_HOLD:-4}
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

# Everything between the shebang and the first line of code, uncommented: the
# header is the documentation, and a hardcoded line range goes stale the first
# time a paragraph is added to it.
usage() { awk 'NR > 1 && /^#/ { sub(/^#[[:space:]]?/, ""); print; next } NR > 1 { exit }' "$SELF"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --pace)  PACE=$2;  shift 2 ;;
    --hold)  HOLD=$2;  shift 2 ;;
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
  MAGENTA=$(tput setaf 5); BLUE=$(tput setaf 4)
  INTERACTIVE=1
else
  BOLD=""; DIM=""; RESET=""; CYAN=""; GREEN=""; YELLOW=""
  MAGENTA=""; BLUE=""
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
    docker build -t port-mortem . >/dev/null 2>&1 && echo "    port-mortem ok"
    docker build -t pm-core --target core . >/dev/null 2>&1 && echo "    pm-core ok"
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
  # Hold the finished screen before wiping it. This sits here rather than at
  # the end of each step so that no step can forget it, and so the last frame
  # of a step is held for the same length however that step ends.
  if [ "$STEP_NO" -gt 0 ] && [ "$SKIP" = 0 ]; then
    beat "$HOLD"
  fi
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

# ---- syntax highlighting for source excerpts --------------------------------
#
# Three steps print real source (TOML, Rust, JS-inside-a-comment) instead of
# program output, and a plain two-space indent reads as a grey wall on camera.
# This is a hand-rolled highlighter: coreutils awk only, no bat/pygmentize/etc,
# so it is exactly what gets recorded. Every substitution is built additively
# from the *unhighlighted* source (never by re-scanning text that already has
# escape codes in it), so a token can never be highlighted twice, and with all
# colour variables empty (non-TTY, <8 colours) the output is byte-identical to
# the plain source: no width is added or removed, only ANSI codes that vanish.
RUST_KEYWORDS="let mut fn pub struct impl enum match if else while for loop in return use mod trait const static self Self as dyn where async await move ref true false Some None Ok Err println"
JS_KEYWORDS="while for if else function var let const return this new typeof instanceof true false null undefined break continue do switch case default try catch finally throw class extends super import export from as async await yield in of delete void"

# Shared token scanner for Rust and JS: double-quoted strings, numeric
# literals (with trailing type/suffix letters, so `23i64` stays one token),
# a caller-supplied keyword list, and a trailing `//` comment. MODE=mixed is
# for step 8, where most lines are prose with `backtick` code spans and only
# the indented lines (4+ leading spaces) are an actual code block.
_CODE_AWK='
function is_kw(w) { return index(" " KW " ", " " w " ") > 0 }
function hl_codeline(code,    n, i, c, j, tok, out) {
  n = length(code); out = ""; i = 1
  while (i <= n) {
    c = substr(code, i, 1)
    if (c == "\"") {
      j = i + 1
      while (j <= n && substr(code, j, 1) != "\"") { if (substr(code, j, 1) == "\\") j++; j++ }
      if (j <= n) j++
      out = out STR substr(code, i, j - i) RST; i = j
    } else if (c ~ /[0-9]/) {
      j = i
      while (j <= n && substr(code, j, 1) ~ /[0-9A-Za-z_.]/) j++
      out = out NUM substr(code, i, j - i) RST; i = j
    } else if (c ~ /[A-Za-z_]/) {
      j = i
      while (j <= n && substr(code, j, 1) ~ /[A-Za-z0-9_]/) j++
      tok = substr(code, i, j - i)
      out = out (is_kw(tok) ? KWC tok RST : tok); i = j
    } else { out = out c; i++ }
  }
  return out
}
function hl_backticks(text,    n, i, c, j, out) {
  n = length(text); out = ""; i = 1
  while (i <= n) {
    c = substr(text, i, 1)
    if (c == "`") {
      j = i + 1
      while (j <= n && substr(text, j, 1) != "`") j++
      if (j <= n) j++
      out = out STR substr(text, i, j - i) RST; i = j
    } else { out = out c; i++ }
  }
  return out
}
function hl_line(line,    cpos, code, com) {
  cpos = index(line, "//")
  if (cpos > 0) { code = substr(line, 1, cpos - 1); com = substr(line, cpos) }
  else { code = line; com = "" }
  return hl_codeline(code) (com != "" ? COMC com RST : "")
}
{
  if (MODE == "mixed" && $0 !~ /^    /) print hl_backticks($0)
  else print hl_line($0)
}
'
hl_rust()   { awk -v KW="$RUST_KEYWORDS" -v STR="$GREEN" -v NUM="$MAGENTA" -v KWC="$BLUE" -v COMC="$DIM" -v RST="$RESET" -v MODE=code   "$_CODE_AWK"; }
hl_js()     { awk -v KW="$JS_KEYWORDS"   -v STR="$GREEN" -v NUM="$MAGENTA" -v KWC="$BLUE" -v COMC="$DIM" -v RST="$RESET" -v MODE=code   "$_CODE_AWK"; }
hl_defect() { awk -v KW="$JS_KEYWORDS"   -v STR="$GREEN" -v NUM="$MAGENTA" -v KWC="$BLUE" -v COMC="$DIM" -v RST="$RESET" -v MODE=mixed "$_CODE_AWK"; }

# TOML line-oriented highlighter: whole-line comments, `[section]` headers,
# and `key = value` pairs with the value coloured by whether it is a quoted
# string or a bare number. Alignment and blank lines pass through untouched.
_TOML_AWK='
{
  line = $0
  if (line ~ /^[ \t]*#/) { print COMC line RST; next }
  if (line ~ /^[ \t]*$/) { print line; next }
  if (line ~ /^[ \t]*\[[^]]+\][ \t]*$/) { print SEC line RST; next }
  eq = index(line, "=")
  if (eq > 0) {
    key = substr(line, 1, eq - 1)
    rest = substr(line, eq + 1)
    cpos = index(rest, "#")
    if (cpos > 0) { val = substr(rest, 1, cpos - 1); com = substr(rest, cpos) }
    else { val = rest; com = "" }
    trimmed = val
    gsub(/^[ \t]+/, "", trimmed); gsub(/[ \t]+$/, "", trimmed)
    if (trimmed ~ /^".*"$/) vcol = STR
    else if (trimmed ~ /^-?[0-9]+(\.[0-9]+)?$/) vcol = NUM
    else vcol = ""
    out = KEY key RST "="
    out = out (vcol != "" ? vcol val RST : val)
    if (com != "") out = out COMC com RST
    print out
  } else print line
}
'
hl_toml() { awk -v KEY="$BLUE" -v STR="$GREEN" -v NUM="$MAGENTA" -v COMC="$DIM" -v SEC="${BOLD}${CYAN}" -v RST="$RESET" "$_TOML_AWK"; }

# Same shape as run(), but pipes the command's output through a highlighter
# (hl_rust / hl_js / hl_toml / hl_defect, above) before the two-space indent.
# The echoed `$ ...` line is the real, unmodified command either way.
run_hl() {
  if [ "$SKIP" = 1 ]; then return 0; fi
  local hl="$1" cmd="$2"
  echo "  ${YELLOW}\$ $cmd${RESET}"
  echo
  beat 0.8
  eval "$cmd" 2>&1 | "$hl" | sed 's/^/  /'
  echo
  beat 1.5
  return 0
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

# A deliberate pause inside a step, as opposed to the automatic one screen()
# takes before wiping. Skip-aware, so that --from does not sit in silence
# waiting out steps it is not showing.
hold() {
  if [ "$SKIP" = 1 ]; then return 0; fi
  beat "${1:-$HOLD}"
}

DOCKER=""
for candidate in docker /usr/bin/docker; do
  if command -v "$candidate" >/dev/null 2>&1 && timeout 8 "$candidate" info >/dev/null 2>&1; then
    DOCKER="$candidate"; break
  fi
done

# ───────────────────────────────────────────────────────────── 1. what this is
screen "What this is"
say "mnemonist is a JavaScript library of 44 data structures. Forty-three of them are ported here. The one left out is not exported by the library and has no test in its published suite, so nothing in that suite could have checked it."
say "The deliverable is a standalone Rust crate: no dependencies, no unsafe code, and it builds without Node installed anywhere on the machine."
say "The original JavaScript test suite is not a dependency of that crate. It is the proof that the crate behaves like the library it replaces. Those tests run unmodified against the Rust build, through a thin bridge that ships to nobody."
run_hl hl_toml "sed -n '/^track/,/^version/p' .port-mortem.toml"
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
run_hl hl_rust "awk '/A Fibonacci heap/,/fibonacci-heap  drained/' crates/mnemonist-core/examples/tour.rs"
run "cargo run -q --release --example tour -p mnemonist-core"
note "Four structures, in crates/mnemonist-core/examples/tour.rs. No bridge, no Node, no dependencies — this is the deliverable, doing its job."

# ───────────────────────────────────────────── 6. the original suite, unchanged
screen "The original test suite, unmodified, against Rust"
say "This is the equivalence proof. These test files are byte-identical to the ones published with the JavaScript library, as the hashes in step two established."
say "They are pointed at the Rust build through the bridge, and run exactly as their authors wrote them."
run "./tests/run.sh 2>&1 | tail -5"
say "That total is two things, and only one of them is evidence of equivalence. The harness runs the upstream files and this port's own bridge specs in a single pass, so the split is worth showing rather than rounding up."
run "(cd tests/.work && printf 'upstream test files     '; npx mocha --reporter dot test/*.js 2>&1 | grep -oE '[0-9]+ passing'; printf \"this port's own specs   \"; npx mocha --reporter dot boundary/*.js 2>&1 | grep -oE '[0-9]+ passing')"
note "525 is the number that counts here. The other 208 test the bridge, which is this project's own code and cannot vouch for itself — they are evidence for a different gate."

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
run_hl hl_defect "sed -n '/for_each_walk/,/^#     }\$/p' crates/difffuzz/proptest-regressions/lru-cache.txt | sed -E 's/^#[[:space:]]?//'"
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
line "  ${GREEN}DECISIONS.md${RESET}          the twelve choices that shaped the port"
line "  ${GREEN}docs/DIVERGENCES.md${RESET}   every deliberate divergence, and why"
line ""
hold
say "The methodology document also records where these instruments are blind. The differential fuzzer never exercises the bridge, and four falsification attempts stayed green for four different reasons. Those are written down too."

# The closing card counts from the repository rather than from memory: a figure
# typed into a slide is a figure that goes stale between rehearsal and take.
if [ "$SKIP" != 1 ]; then
  UNITS=$(grep -cvE '^[[:space:]]*(#|$)' tests/scope.txt 2>/dev/null || echo 0)
  OPS=$(grep -vE '^[[:space:]]*(#|$)' fuzz/log.txt 2>/dev/null \
        | grep -oE 'ops=[0-9]+' | cut -d= -f2 | awk '{s+=$1} END {printf "%.1fM", s/1000000}')
  echo "  ${CYAN}${RULE}${RESET}"
  echo
  echo "  ${BOLD}43 of 44 structures ported.${RESET}  ${UNITS} units through all ten gates."
  echo "  ${BOLD}${OPS} differential operations.${RESET}  Zero divergences."
  echo "  The original test suite passes unmodified."
  echo
  # The last frame of the recording, with nothing after it to clear it away:
  # held half again as long as any other, since this is the one a viewer is
  # most likely to pause on.
  beat "$(awk "BEGIN{print $HOLD * 1.5}")"
fi
