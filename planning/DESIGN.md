# Port Mortem 2026 — Design Doc

**Status:** pre-kickoff design. Nothing here is committed to the repo until **2026-07-31 18:00 UTC** (20:00 local WEDT).
**Entry:** **Track G** — JavaScript → Rust. Solo. *(Was "F" — see §0 below.)*

### §0 TRACK LETTER — the website and the FAQ disagree

| | Website (`/2026/` + repo pool) | Admin FAQ (later, and self-consistent) |
|---|---|---|
| F | JavaScript → **Go/Rust** | JavaScript → **Go** |
| G | **C → Zig** | JavaScript → **Rust** |

The FAQ list (A C→Rust · B Zig→Rust · C TS→Go · D Python→Rust · E Go→Rust · F JS→Go · G JS→Rust ·
H Open) drops C→Zig entirely, so the website table is most likely stale rather than the FAQ being
loose. **We are JavaScript→Rust, therefore Track G under the FAQ.**

**This is not urgent.** Per the FAQ, *"You declare your track and repo at submission on the last
day"* — registration asks for neither. So confirm with the admins and settle it by Aug 2, when the
submission form arrives. Everything downstream (`.port-mortem.toml`, README, demo banner) reads the
letter from one place; change it once when confirmed.

Nothing else about the plan depends on the letter: the pair, the repo, and the target language are
all unchanged.
**Target:** `Yomguithereal/mnemonist` (MIT), 15,386 LOC shipped source, 41 test files.
**Ambition:** full port. **Hackathon deliverable:** Waves 0+1 complete (~4,000 LOC), remainder as roadmap.

**1. FFI approach — RATIFIED BY ADMINS.** Verbatim (Discord, pre-kickoff):

> "Please keep the original test files exactly as they are in `tests/original/`, together with their
> kickoff SHA-256, and run them against your port through a thin adapter or FFI shim. If you rewrite
> the tests, your score will go down because the hashes are fixed and the judges will see the diff.
> **Using unsafe at the FFI boundary is fine and expected. What counts against you is unsafe code
> spread through the core port logic.** Rewriting the test logic 1:1 as native tests is accepted."

> "Unsafe code at the FFI boundary is expected and is not a problem. Also, **tests are optional for
> qualification now**, but running the original tests unchanged is still the top-1 strongest proof
> you can show."

Three consequences: the §3.1 crate split is now vindicated by the ruling's own wording; the
`tests/original/` + kickoff-SHA-256 layout (§2.1–2.2) matches the vocabulary they used; and the
CP1 fallback is no longer a scoring cliff, because 1:1 native tests are explicitly "accepted" and
"still count."

**2. Repo size / scoped subset — STILL UNANSWERED.** The ruling addressed FFI and tests only.
Nothing on mnemonist's 15,386 LOC against the 8k guidance, or on declaring a module scope. Ask
again pre-kickoff. Not a blocker — worst case is presentational (see DIV-PROJ-8 and §6 scope
declaration) — but resolve it if an answer is cheap.

---

## 1. Core principles

**P1 — Every module ships complete or not at all.**
Complete = port + original test file green + fuzz clean + bench recorded + DECISIONS entries.
Never carry a half-finished module into freeze. An unknown cutoff makes partial work pure loss.

**P2 — Order the queue so any cutoff leaves a coherent story.**
Not "42 modules, 30% done" but "the contiguous-memory subset, complete."

**P3 — Machinery before modules.**
With 42 targets, a generic bridge / scaffold / fuzz driver amortizes. Hand-crafting module #1 is a trap.

**P4 — The port crate never knows JS exists.**
`mnemonist-core` builds, tests, and benches with Node absent. This is both good architecture and the one-line rebuttal to the FFI rule.

**P0 — DEFINITION OF DONE. A unit is not ported until every gate below is green.**
See §1.1. Nothing else in this document overrides it. "Compiles and the test passes" is a
workflow smoke-test, not a delivered unit.

**P5 — Commit incrementally, from kickoff, always. This is a disqualification risk.**
Admin FAQ, verbatim: *"We check Git history and expect the first port commit after kickoff with
real incremental commits. **A single dump or predated work risks disqualification.**"*
- First commit is the hashed test suite (§10 step 4), **before** any port code exists.
- Commit per module and per milestone — never batch a wave into one commit.
- Long uncommitted stretches followed by a squash is precisely the anti-pattern being screened for.
- Planning docs from `scratchpad/` may be committed **at or after** kickoff as planning artifacts —
  they are not port code, and their content is openly about pre-kickoff scouting, which the FAQ
  explicitly permits (*"You can scout, read tests, check LOC/license, and plan now"*).

---

## 1.1 The unit, and the Definition of Done

### What a unit is

**A unit is the complete require-closure of one upstream test file** — not one source module.

Forced by the harness, not chosen: every `require('../x.js')` in an upstream test file sits at the
top of the file, so a single missing module throws before any `it()` executes and the **whole file
fails with zero partial credit.** You cannot half-land a test file.

Most files are 1:1 with a module. These are not:

| Test file | Require-closure | LOC |
|---|---|---|
| `_utils.js` | `typed-arrays` + `binary-search` + `hash-tables` + `iterables` + `merge` | ~1,166 |
| `lru-cache.js` | `lru-cache` + `lru-map` + `lru-cache-with-delete` + `lru-map-with-delete` | 1,271 |
| `sort.js` | `sort/quick` + `sort/insertion` + `utils/typed-arrays` | ~350 |
| `multi-set.js` | `multi-map` + `multi-set` | 853 |
| `multi-map.js` | `multi-map` + `vector` | 781 |
| `heap.js` | `heap` + `utils/comparators` | 655 |
| `kd-tree.js` | `kd-tree` + `utils/comparators` | 526 |

**This corrects an earlier claim in §6** that `test/_utils.js` accrues partial credit as utils land.
It does not. All five utils modules must exist before one assertion runs.

### Definition of Done — all ten gates, no exceptions

| # | Gate | Evidence |
|---|---|---|
| 1 | Every module in the closure ported to `mnemonist-core` | builds |
| 2 | `#![forbid(unsafe_code)]` intact; core has no JS awareness | `cargo build -p mnemonist-core` with Node absent |
| 3 | napi bridge + shim per module | — |
| 4 | **Original test file green, unmodified** | `tests/run.sh <file>` |
| 5 | `sha256sum -c tests/SHA256SUMS` PASS | — |
| 6 | **Falsification check**: sabotage the core, gate goes red | proves the suite exercises Rust, not a JS fallback |
| 7 | Rust native tests covering what upstream's do not | `cargo test -p mnemonist-core` |
| 8 | **Divergence doc** `docs/modules/<unit>.md` | see below |
| 9 | **Differential fuzz** 60s+, zero divergences | `fuzz/log.txt` entry + committed `proptest-regressions/` |
| 10 | **Benchmark** vs upstream JS | keyed entry in `bench/results.json` |

Plus always: `cargo fmt`, `cargo clippy --all-targets -- -D warnings` clean.

**The done marker is `tests/scope.txt`.** A unit is added to it in the final commit of its series,
and only when gates 1–10 are green. If it is not in `scope.txt`, it does not exist: `run.sh`,
the README scope table, and `.port-mortem.toml` all read from that file.

### Gate 6 exists because of a real miss

The first falsification attempt on `StaticDisjointSet` sabotaged `if x_root == y_root` — a branch
that test never takes, because every union in it merges distinct sets. It stayed green and proved
nothing. **A falsification test that cannot fail is just a second green light.** Choose the
sabotage by naming the assertion it must break, then confirm red *and* confirm green after revert.

### Gate 8 — the divergence doc

One file per unit, `docs/modules/<unit>.md`. This is the artifact that makes the rigor gap
*visible* rather than merely claimed, and it feeds `DECISIONS.md` and the write-up directly.

```markdown
# <unit>
Upstream: <files> · <n> LOC · test/<file>.js (<n> lines, <n> assertions)

## What upstream tests
Bullets. Be specific about the shape of the coverage.

## What upstream does NOT test          <-- the point of the document
Behaviours reachable in the public API that the original suite never exercises.

## What we test in addition
Our native tests, mapped 1:1 to the gaps above.

## Bugs this found
Defects only visible because our coverage exceeds upstream's. Cross-ref NOTES.md B-nn.

## Deliberate divergences
Where the port differs and why. Cross-ref DECISIONS-CANDIDATES.md D-nn.

## Fuzz + bench
Ops fuzzed, seed, duration, divergences. Benchmark headline including any regression.
```

### Commit granularity

A unit lands as a short series, not one dump — P5 still applies:
`feat(core)` → `feat(napi)` → `test(fuzz)` + `perf(bench)` → `docs(module)` + scope.txt.
**The scope.txt commit is what marks it done.** Never begin the next unit before the current one
is in `scope.txt`.

### Consequence: `StaticDisjointSet` is NOT done

It has gates 1–7 (and gate 6 only after the second attempt). It is missing **8, 9, 10**. It is a
validated workflow smoke-test, not a delivered unit, and it must be backfilled — see §7.2.

---

## 2. Repository layout

```
port-mortem/
├── README.md                  migration rationale, scope declaration, build
├── DECISIONS.md               every non-trivial divergence + why
├── Dockerfile                 one command → runnable artifact
├── .port-mortem.toml          track letter, source URL, kickoff hash
├── Cargo.toml                 workspace
├── crates/
│   ├── mnemonist-core/        THE PORT. pure Rust. #![forbid(unsafe_code)]
│   │   ├── src/
│   │   │   ├── cursor/        Iterator cursor semantics (§3.4) — NOT foreach
│   │   │   ├── utils/         typed_arrays, bitwise, comparators, binary_search
│   │   │   └── structures/    one per data structure; accept IntoIterator
│   │   └── tests/             port-side unit tests (tests/port equivalent)
│   ├── mnemonist-napi/        cdylib. TEST HARNESS ONLY. never a dep of core
│   │   └── src/coerce.rs      forEach/iter/iterables JS-value dispatch (§3.5)
│   └── difffuzz/              generic op-sequence differential fuzzer
├── tests/
│   ├── original/              ← HASHED AT KICKOFF. byte-identical upstream
│   │   └── test/*.js
│   ├── bridge/                generated CJS shims (heap.js, bit-set.js, …)
│   ├── SHA256SUMS             produced at kickoff, verified at submission
│   └── run.sh                 assembles work tree, runs mocha
├── fuzz/
│   ├── harness.rs             → crates/difffuzz
│   └── log.txt                per-module runs, 60s+, zero divergences
└── bench/
    ├── methodology.md
    └── results.json           per-module keyed
```

### 2.1 The module-resolution trick

Upstream tests import by **direct relative path**: `test/heap.js` does `require('../heap.js')`.
So the shim must sit one level above the test file. Mirror the upstream root:

```
tests/.work/            (generated, gitignored)
├── test/heap.js        ← copied from tests/original/test/heap.js  (UNMODIFIED)
├── heap.js             ← copied from tests/bridge/heap.js         (our shim)
├── utils/…             ← our shims
└── node_modules/       ← obliterator + oracle deps for Wave 5 tests
```

`tests/run.sh` assembles `.work/`, runs `npx mocha test/`, tears down.
`tests/original/` therefore stays **pure upstream and cleanly hashable** — no shims mixed in.

Each shim is trivial:
```js
// tests/bridge/heap.js
module.exports = require('../../target/release/mnemonist_napi.node').Heap;
```

### 2.2 Hashing protocol — hash ALL 41 test files, run the declared scope

**DECIDED.** Vendor and hash the **entire** upstream `test/` directory at kickoff, not just the
in-scope files. Run only the modules actually ported, and report **both** numbers.

At kickoff, before writing any port code:
```bash
find tests/original -type f -print0 | sort -z | xargs -0 sha256sum > tests/SHA256SUMS
git commit -m "chore: vendor + hash full original test suite at kickoff"
```

**Why the full suite.** Hashing only the in-scope files would let us choose our own denominator
*after* picking modules — and cherry-picking is a failure mode the rules name explicitly. Hashing
all 41 at kickoff is timestamped proof we committed to the honest denominator before knowing any
outcome. It converts the subset-scope weakness into visible rigor at zero cost.

**Dual reporting.** README leads with both figures, in this order:

> **13 of 44 modules ported. 100% of their original tests pass, unmodified.**
> Repo-wide that is N/525, because 31 modules are declared roadmap (see scope table).

`tests/verify-hashes.sh` re-checks at submission and prints PASS/FAIL into the demo video.
Any file that must change gets its own DECISIONS.md entry with the diff inline.

**Upstream baseline to beat within scope:** see §11.7 — 525 passing / 1 pending / 0 failing.

### 2.3 `tests/run.sh` — harness assembly (spec; implement at kickoff)

Five non-obvious problems this has to solve. The script itself is trivial once they're settled.

#### Problem 1 — shim depth
Tests `require('../heap.js')`, so shims sit one level above `test/`. But `utils/` shims live at
`.work/utils/`, needing `../../addon`. Hard-coding relative depth per shim is fragile.

**Solution: install the addon as a resolvable package.** Node's resolution walks parent
directories, so `require('@port/addon')` works from *any* depth:
```
.work/node_modules/@port/addon/
├── package.json        {"name":"@port/addon","main":"addon.node"}
└── addon.node          ← copied from target/release/libmnemonist_napi.so
```
Every generated shim, root or nested, is then depth-independent:
```js
// tests/bridge/heap.js
const A = require('@port/addon');
const Heap = A.Heap;
Heap.MaxHeap = A.MaxHeap;      // upstream attaches this as a static
module.exports = Heap;
```

#### Problem 2 — export shapes differ per module
A naive `module.exports = addon.X` is wrong for several modules. Upstream attaches statics and
sentinels: `Heap.MaxHeap`, `Trie.SENTINEL`, `Vector` subclasses, and `utils/comparators.js` exports
*named* functions (`test/heap.js` does `require('../utils/comparators.js').DEFAULT_COMPARATOR`).
**The scope manifest must record each module's exact export shape**, and the shim generator emits
accordingly. Getting this wrong produces `undefined is not a constructor` far from the cause.

#### Problem 3 — unported modules abort the run
`.work/test/` holds all 41 files (§2.2), but shims exist only for in-scope modules. Bare `mocha`
would glob all 41, and 28 would throw `Cannot find module '../trie.js'` at load time.
**Solution: never run bare mocha — always pass an explicit spec list**, derived from `scope.txt`.

#### Problem 4 — the two reporting modes (§2.2 dual reporting)
- `run.sh` → in-scope specs only. **Exit code is mocha's.** This gates CI and the demo.
- `run.sh all` → all 41. Unported modules fail as expected; **exits 0 regardless**, and prints the
  repo-wide passing count for the README's second figure. Failing here would be meaningless.

#### Problem 5 — `node_modules` churn
Tests themselves require real JS `obliterator` (`test/trie.js` does `require('obliterator/take')`).
That is legitimate — the *test* uses a JS utility to consume *our* cursor, which incidentally
proves compatibility with the real thing. Reinstalling per run is far too slow, so preserve
`node_modules` across assembly and reinstall only when the manifest changes.

#### Assembly script

```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$ROOT/tests/.work"
STAMP_V="$ROOT/tests/.verify-stamp"

MODE=scope; SKIP_VERIFY=0
for arg in "$@"; do case "$arg" in
  scope|all)     MODE="$arg" ;;
  --skip-verify) SKIP_VERIFY=1 ;;
  *) echo "usage: run.sh [scope|all] [--skip-verify]" >&2; exit 64 ;;
esac; done

# 1. Integrity gate — tiered (see "Verification tiers" below).
FORCE_FULL=0
if [ "${CI:-}" = true ] || [ "${PM_VERIFY_ALWAYS:-}" = 1 ]; then
  SKIP_VERIFY=0; FORCE_FULL=1          # CI and the demo can never skip
fi

if [ "$SKIP_VERIFY" = 1 ]; then
  echo "!!  --skip-verify: suite integrity NOT checked. Local iteration only." >&2
  echo "!!  Results are UNVERIFIED and must not be reported or recorded." >&2
  UNVERIFIED=" [UNVERIFIED]"
else
  UNVERIFIED=""
  # Full hash check only when something under tests/original/ actually changed.
  if [ "$FORCE_FULL" = 1 ] || [ ! -f "$STAMP_V" ] \
     || [ -n "$(find "$ROOT/tests/original" -type f -newer "$STAMP_V" -print -quit)" ]; then
    ( cd "$ROOT" && sha256sum -c tests/SHA256SUMS --quiet ) \
      || { echo "FATAL: tests/original/ modified. Refusing to run." >&2; exit 2; }
    touch "$STAMP_V"
  fi
fi

# 2. Build the addon (cargo is incremental).
cargo build --release -p mnemonist-napi --manifest-path "$ROOT/Cargo.toml"

# 3. Assemble .work, preserving node_modules.
mkdir -p "$WORK"
find "$WORK" -mindepth 1 -maxdepth 1 \
     ! -name node_modules ! -name package-lock.json -exec rm -rf {} +
cp -R "$ROOT/tests/original/test" "$WORK/test"     # all 41, byte-identical
cp -R "$ROOT/tests/bridge/."      "$WORK/"         # generated shims
cp    "$ROOT/tests/harness-package.json" "$WORK/package.json"

# 4. Publish the addon as a resolvable package (Problem 1).
mkdir -p "$WORK/node_modules/@port/addon"
cp "$ROOT/target/release/libmnemonist_napi.so" "$WORK/node_modules/@port/addon/addon.node"
printf '{"name":"@port/addon","main":"addon.node"}' \
  > "$WORK/node_modules/@port/addon/package.json"

# 5. Install deps only when the manifest changed.
STAMP="$WORK/node_modules/.install-stamp"
if [ ! -f "$STAMP" ] || [ "$WORK/package.json" -nt "$STAMP" ]; then
  ( cd "$WORK" && npm install --no-audit --no-fund --silent ) && touch "$STAMP"
fi

# 6. Select specs (Problem 3).
cd "$WORK"
echo "== mnemonist port · mode=$MODE${UNVERIFIED} · $(node -v) =="
if [ "$MODE" = all ]; then
  npx mocha --reporter spec test/*.js || true      # Problem 4: never fail here
  exit 0
fi
mapfile -t MODULES < <(grep -vE '^[[:space:]]*(#|$)' "$ROOT/tests/scope.txt")
SPECS=(); for m in "${MODULES[@]}"; do
  [ -f "test/$m.js" ] && SPECS+=( "test/$m.js" )
done
[ ${#SPECS[@]} -gt 0 ] || { echo "FATAL: empty scope." >&2; exit 2; }
npx mocha --reporter spec "${SPECS[@]}"
```

#### Verification tiers

Three tiers, so the fast path is also the safe path and the escape hatch is hard to misuse.

| Tier | When | Cost | Behaviour |
|---|---|---|---|
| **Cached** (default) | local iteration | ~0 after first run | Full `sha256sum -c` only if any file under `tests/original/` is newer than `.verify-stamp` |
| **Forced** | `CI=true` or `PM_VERIFY_ALWAYS=1` | full check | Always verifies; **`--skip-verify` is ignored** |
| **Skipped** | explicit `--skip-verify` | none | Loud stderr warning; run tagged `[UNVERIFIED]` |

**The cached tier should make the flag unnecessary.** `find -newer` catches any accidental edit —
including a `git checkout`, which stamps mtime to now. It does not defend against someone
deliberately preserving mtimes, but the threat model here is *us editing a test file by accident*,
not an adversary. So the default is both fast and safe, and `--skip-verify` exists only for the
case where even the `find` is unwelcome.

**Two guards against the flag leaking into anything that matters:**
1. `CI=true` (set automatically by GitHub Actions) and `PM_VERIFY_ALWAYS=1` (for the demo) both
   override it. A skipped verification cannot reach CI or the recording.
2. Every run prints a banner — `== mnemonist port · mode=scope [UNVERIFIED] · v24.18.1 ==` — so an
   unverified run **self-labels in its own output**. A screenshot of one can't be mistaken for a
   clean run later, which is the realistic way this would otherwise go wrong at hour 68.

Never bake `--skip-verify` into `Dockerfile`, the CI workflow, `scripts/demo.sh`, or any npm script.

#### `tests/scope.txt` — single source of truth
One module name per line, `#` comments allowed. **Three consumers:** `run.sh` spec selection, the
README scope table, and `.port-mortem.toml`. Keeping one list prevents the classic failure where
the README claims a module the harness never runs.

#### Notes
- **Copy, don't symlink**, `tests/original/test` → `.work/test`. A symlink means a stray write
  reaches the hashed originals. The integrity gate in step 1 makes drift detectable anyway.
- `libmnemonist_napi.so` is the Linux artifact name; `.dylib`/`.dll` differ. Linux-only is fine
  (§11.5) — but do not hard-code the path in the Dockerfile *and* here without a shared variable.
- Add `tests/.work/` to `.gitignore` at kickoff.

---

## 3. The N-API bridge

### 3.1 Crate split (protects the Zero Unsafe +5)

`napi-rs` generates `unsafe` internally. Quarantining it means `#![forbid(unsafe_code)]` on
`mnemonist-core` stays literally true and machine-verifiable:

```
mnemonist-core   → #![forbid(unsafe_code)]   ← the artifact judges score
mnemonist-napi   → depends on core           ← test scaffolding, excluded from the claim
```

State this explicitly in README and DECISIONS.md, with the `cargo test -p mnemonist-core`
command that passes on a machine with no Node installed.

### 3.2 Iterator protocol — the critical path

30 of 44 modules depend on `obliterator`. Build this **once**, in Wave 0, generically.

**CONFIRMED** against `docs.rs/napi` **3.12.0** — use the `napi` 3.x crate:

```rust
pub trait Generator {
    type Yield: ToNapiValue;
    type Next: FromNapiValue;
    type Return: FromNapiValue;

    fn next(&mut self, value: Option<Self::Next>) -> Option<Self::Yield>;   // required

    fn complete(&mut self, value: Option<Self::Return>) -> Option<Self::Yield> { ... }
    fn catch<'env>(&'env mut self, env: Env, value: Unknown<'env>)
        -> Result<Option<Self::Yield>, Unknown<'env>> { ... }
}
```

Applied with `#[napi(iterator)]` on the struct. Only `next` must be implemented.
Note the asymmetry: `Yield: ToNapiValue`, but `Next`/`Return` are `FromNapiValue`.
One thing to check at first build: whether `()` satisfies `FromNapiValue` for the unused
`Next`/`Return` slots (`napi::Undefined` is an alias for `()`). If not, use `Unknown`.

Upstream `obliterator` (v2.0.5) is a **runtime** dependency — linking it is forbidden, so
`iterator` / `iter` / `foreach` / `take` must be ported into Rust. Their exact semantics are
mapped in §3.4; they are far more idiosyncratic than they look and are the richest single
source of silent divergence in this port.

### 3.3 Bridge capability tiers (drives wave order)

| Tier | Capability | Unlocks |
|---|---|---|
| T0 | scalars + typed arrays | Wave 1 |
| T1 | iterator protocol | Wave 1 (most of it) |
| T2 | comparator callbacks JS→Rust | Wave 2 |
| T3 | arbitrary JS values as keys, SameValueZero equality | Wave 3 |
| T4 | string-heavy + external oracle deps | Waves 4–5 |

Never start a wave before its tier is proven by a pilot.

### 3.4 `obliterator` v2.0.5 — exact semantics (READ BEFORE IMPLEMENTING)

Source read directly. These are the behaviours a naive Rust port gets wrong.

**`Iterator` — self-returning and NOT restartable.**
```js
Iterator.prototype[Symbol.iterator] = function () { return this; };
```
Iterating the same instance twice **continues from where it stopped**; it does not restart.
Idiomatic Rust `IntoIterator` yields a fresh iterator each time — that is a divergence.
Model it as a **stateful cursor object**, never as `impl IntoIterator`.

**`Iterator.fromSequence` — hybrid live/snapshot.**
```js
var i = 0, l = sequence.length;              // length captured AT CREATION
return new Iterator(function () {
  if (i >= l) return {done: true};
  return {done: false, value: sequence[i++]}; // element read LAZILY
});
```
Element mutations during iteration **are** visible; length changes are **not**. This resolves the
snapshot-vs-live question in the risk register: it is *neither*, it is hybrid. Fuzz iteration
interleaved with mutation specifically for this.

Constructor throws verbatim `'obliterator/iterator: expecting a function!'` on a non-function.
`Iterator.is(v)` duck-types: `v instanceof Iterator || (typeof v === 'object' && v !== null && typeof v.next === 'function')`.

**`forEach(iterable, callback)` — 5-branch dispatch, order is observable.**

| # | Test | Callback 2nd arg |
|---|---|---|
| 1 | `Array.isArray` ∥ `ArrayBuffer.isView` ∥ `typeof === 'string'` ∥ `toString() === '[object Arguments]'` | `i` (number) |
| 2 | `typeof iterable.forEach === 'function'` → delegates | **whatever the host `forEach` passes** |
| 3 | `Symbol.iterator in iterable && typeof iterable.next !== 'function'` → coerce to iterator | — |
| 4 | `typeof iterable.next === 'function'` → drain | `i` (number, own counter) |
| 5 | plain object → `for…in` + `hasOwnProperty` | **`k` (string key)** |

Three traps:
- **The second argument changes type by branch** — number for sequences and iterators, string for
  plain objects, host-defined for branch 2. A JS `Map` hits branch 2, so its callback gets
  `(value, key)`, not `(value, index)`.
- **Branch 2 precedes iterator handling.** Anything owning a `.forEach` never reaches branch 3/4.
- **Falsy guard:** `if (!iterable) throw`. So `forEach('', cb)` throws while `forEach('a', cb)`
  iterates. Same for `0`, `false`, `NaN`. Error text: `'obliterator/forEach: invalid iterable.'`

Plain-object order must follow JS property enumeration order: integer-like keys ascending first,
then string keys in insertion order.

**`iter(target)` — coercion, and deliberately narrower than `forEach`.**
Order: indexed sequence → `Iterator.fromSequence`; non-object/null → throw; `Symbol.iterator` → call it;
`.next` → return as-is; else throw `'obliterator: target is not iterable nor a valid iterator.'`

**`iter` has no `.forEach` branch and no plain-object branch.** So `take({a: 1})` **throws** while
`forEach({a: 1}, cb)` **iterates the values**. This asymmetry between the two helpers is real,
upstream, and exactly the kind of thing that belongs in `DECISIONS.md`.

**`take(iterable, n)`** — `l = arguments.length > 1 ? n : Infinity`. Preallocates `new Array(l)`
when finite, truncates via `array.length = i` if the source runs dry early. `take(it, 0)` → `[]`.

`support.js` gates on `ARRAY_BUFFER_SUPPORT` / `SYMBOL_SUPPORT`; both are true on any modern Node,
so hardcode true and note it in `DECISIONS.md`.

### 3.5 `forEach` is a BOUNDARY function — do not port it into core

**Verified by grep across all 30 modules that import it.** Every single call site is
`forEach(iterable, cb)` inside a `.from()` static or an iterable-accepting constructor, operating
on the **user-supplied argument**. Not one call site iterates a structure's own internal data.
(The `this.items.forEach(...)` / `METHODS.forEach(...)` lines in `bi-map`, `multi-map`, `multi-set`,
`fuzzy-map`, `symspell` are native `Array`/`Map` `.forEach`, unrelated to obliterator.)

`utils/iterables.js` is a thin layer over the same thing — `toArray`, `toArrayWithIndices`,
`guessLength`, `isArrayLike` — and is likewise only reached from `.from()`/constructor paths.

**Consequence for the crate split:**

| Concern | Crate | Form |
|---|---|---|
| `forEach` 5-branch dispatch, falsy guard, plain-object key order | `mnemonist-napi` | `Unknown` → `impl Iterator<Item = T>` coercion, written **once** |
| `iter` coercion + exact error strings | `mnemonist-napi` | same |
| `iterables::{toArray, toArrayWithIndices, guessLength}` | `mnemonist-napi` | same |
| `Iterator` cursor semantics (§3.4) | **both** | core: index cursor. napi: `#[napi(iterator)]` wrapper |
| Structures | `mnemonist-core` | accept `IntoIterator<Item = T>` |

This is strictly better on three axes: the core stays idiomatic Rust (**20% category**), the
gnarliest logic is written once instead of per module, and it lands in the layer that actually
owns JS-value semantics. A Rust caller doing `Stack::from(vec)` gets the natural thing; the
five-branch coercion exists only where a JS value can appear.

**Caveat to verify per module in Wave 2+:** the claim is proven for all current call sites. If a
Wave 3/4 module turns out to call `forEach` on internal data, revisit — but nothing in the graph
suggests it will.

### 3.6 Two-level `Symbol.iterator` — factory vs identity

Confirmed in source (`stack.js:150`, `queue.js:156`, `vector.js:286`, `bit-set.js:348`,
`fixed-deque.js:294`), all of the form:

```js
Stack.prototype[Symbol.iterator] = Stack.prototype.values;
```

So there are **two different behaviours one level apart**:

| Expression | Behaviour |
|---|---|
| `[...stack]` twice | **Works twice.** Collection-level `Symbol.iterator` is a *factory* — calls `values()`, fresh `Iterator` each time |
| `const it = stack.values(); [...it]` twice | **Second is empty.** Iterator-level `Symbol.iterator` is *identity* (`return this`), so the cursor is already exhausted |

A port that models "iterable" with one uniform concept gets one of these wrong. In the bridge:
the collection's `Symbol.iterator` must **construct** a new `#[napi(iterator)]` cursor object;
the cursor's own `Symbol.iterator` must **return itself**. napi-rs's `#[napi(iterator)]` already
implements the identity half correctly — the factory half is on you.

### 3.7 DECIDED — Option A, sequenced (was an open choice)

**Evidence gathered pre-kickoff.** Grepped all 41 test files for stored iterators — the only sites
that *could* observe mutation, since an immediate spread or drain cannot. 24 sites exist (16 in
Wave 1). Inspected: **every one follows the identical pattern** —

```js
var stack = Stack.from([1, 2, 3]);
var iterator = stack.values();
assert.strictEqual(iterator.next().value, 3);   // …drain, no mutation…
assert.strictEqual(iterator.next().done, true);
```

**No upstream test mutates a structure between iterator creation and drain.** Therefore Option B
costs *exactly zero* in the 40% category — which makes it a genuinely safe fallback rather than
a gamble.

**Correcting an earlier estimate:** Option A was described below as "~3 lines, centralized." That
was optimistic. It needs every cursor to hold a live parent reference (`SharedReference`) **and**
`Yield` to become `Option<T>` to express `undefined`, plus verification that napi-rs maps `None`
to `undefined` rather than `null` (unverified).

**But the cost decomposes into two separable behaviours:**

| | Behaviour | Requires |
|---|---|---|
| **B1** | element mutation during iteration is visible | live parent access |
| **B2** | shrink below frozen length yields `undefined` | live parent access **+** `Option<T>` yield |

Live parent access is needed for B1 regardless. So A's *marginal* cost over B is only the
`Option<T>` yield — small, given live cursors exist.

**Decision: Option A, in this order.**
1. Wave 0 — build cursors with live parent access. Scaffold generator makes per-module cost
   near-zero after the first.
2. Get **B1** working and fuzzed.
3. Add the **B2** `undefined` gap as a small increment.
4. If the `Option<T>` → `undefined` mapping proves awkward, **fall back to B and document it.**
   Safe: measured to cost nothing on the 40% axis.

**Rationale.** No test needs either behaviour — but mutation-during-iteration is exactly the class
of bug a test suite misses and differential fuzzing catches, which is the event's whole thesis.
Covering it is a 30%-category differentiator and the strongest write-up material available.

Whichever way step 4 lands, the rejected option and this reasoning go into `DECISIONS.md`.

---

<details>
<summary>Original framing of the choice (retained for the decision log)</summary>

### OPEN CHOICE — shrink-window behaviour

Settled already (§3.4): cursors are index-based over `&self` in core, with `SharedReference` at the
bridge so JS sees live element data. What remains open is the **shrink window** — what happens when
the collection shrinks below the cursor's captured length mid-iteration.

Upstream, `i >= l` tests the **frozen** `l`, so the cursor reads past the shrunk backing array and
JS yields `{done: false, value: undefined}` — `undefined` values rather than termination.
Confirmed universal: `Iterator.fromSequence`, `Stack.prototype.values` (`l = items.length`),
`FixedDeque.prototype.values` (`l = this.size`, `c = this.capacity`, `i = this.start` all frozen).

#### Option A — reproduce the shrink window

Bridge cursor keeps the captured `l` and compares the index against **both** `l` and the parent's
current length. In the gap (`i < l` but `i >= current_len`) it yields JS `undefined`.

- **Cost:** ~3 lines, in one place, inherited by every module. Cursor must reach the parent's live
  length — already required by `SharedReference`, so no new machinery.
- **Core impact:** none. Core cursors stay `Option<T>`; the gap logic is bridge-only.
- **Fuzz grammar:** enable `iter_create → shrink ops → iter_next` interleaving.
- **Risk:** the `undefined` gap must not leak into the core's type signatures. If it starts to,
  that's the signal to switch to Option B.

#### Option B — terminate cleanly, document the divergence

Bridge cursor terminates when the index reaches the parent's *current* length. Shrink-during-
iteration ends the iteration instead of yielding `undefined`.

- **Cost:** zero. It is the natural Rust behaviour.
- **Core impact:** none.
- **Fuzz grammar:** must **exclude** shrink-during-iteration, with the exclusion stated explicitly
  in `fuzz/log.txt` — a silently narrowed grammar reads as "covered everything" when it wasn't.
- **Risk:** if any upstream test exercises the shrink window, it fails and costs 40%-category
  points. Cheap pre-check: grep the Wave 1 test files for mutation between `values()` and drain.

#### Comparison

| | A — reproduce | B — diverge |
|---|---|---|
| Fidelity | exact | documented divergence |
| Implementation | ~3 lines, centralized | none |
| Fuzz coverage | full grammar | narrowed, must be disclosed |
| 40% risk if a test hits it | none | test fails |
| 30% strength | strong — fuzzed under mutation | weaker — a whole op class excluded |
| 20% strength | equal — core is clean either way | equal |

**Leaning A**, because the cost is near-zero and centralized, it removes a 40% risk outright, and
"we preserved JS iterator-invalidation semantics at the boundary while keeping the core
borrow-checked" is a materially stronger claim than an exclusion note. **B remains fully
defensible** — the rubric rewards honest disclosure, and the FAQ is explicit that hiding a
limitation scores worse than stating it.

Decide at Wave 0, before the first cursor is written. Whichever is chosen, the *other* option and
this rationale go into `DECISIONS.md` — a rejected alternative with reasoning is exactly the
decision-log quality the 20% criterion asks for.

</details>

### 3.8 T3 — reproducing JavaScript `Map` (BUILT, piloted on `default-map`)

T3 is not a family of related structures. It is **one capability that eleven modules share**:
`default-map`, `bi-map`, `fuzzy-map`, `fuzzy-multi-map`, `multi-map`, `multi-set`, `lru-map` and
`lru-map-with-delete` all keep their state in a `new Map()`. So "port T3" means, precisely,
"reproduce `Map`" — written once, exactly as `obliterator` was written once in §3.4.

**Corrections to what §3.3 assumed.** Verified against the vendored sources, not inferred:

| module | actually backed by | consequence |
|---|---|---|
| `set.js` | **native `Set`, and it is not a wrapper at all** — free functions over `Set`s | not a T3 module; needs `Set`, not `Map` |
| `lru-cache`, `lru-cache-with-delete` | **a plain object `{}`** | keys are *string-coerced by the index* while `entries()` reads the raw key array, and `test/lru-cache.js:65` asserts both halves |
| `sparse-map` | three typed arrays | T0, not T3 |
| `fuzzy-multi-map` | a `MultiMap`, not a `Map` directly | T3 transitively |

#### The split: core owns order, the bridge owns equality

```
mnemonist_core::map::OrderedMap<K: Hash + Eq + Clone, V>   insertion order, liveness, tombstones
mnemonist_napi::js_key::JsKey                              SameValueZero
mnemonist_napi::js_value::{Received, Retained, Loaned}     JS values held across calls
```

Core never sees a JavaScript value. SameValueZero is a property of the **key type**, so it lives
entirely in the bridge and core simply requires `Hash + Eq`. A Rust caller gets an ordinary
insertion-ordered map.

#### The four behaviours `std::collections::HashMap` does not have

All four confirmed against Node 24.18.1 rather than read off the spec:

1. **Guaranteed insertion order.** Every T3 test file asserts it.
2. **Delete-then-reinsert moves the key to the end; overwrite does not.** `set` on a present key
   updates in place and keeps the *original* key — which is what makes `-0` come back as `+0`.
3. **Iterators are live.** An entry appended behind a cursor **is** visited; one deleted ahead of
   it is **skipped**; a cursor that has once reported `{done: true}` stays detached even if the map
   grows; and `clear()` followed by `set()` **is** visible to a cursor that has not yet finished,
   while `clear()` followed by `next()` detaches it first. The order of those last two operations
   is the whole distinction.
4. **SameValueZero**, delegated to the key type.

**This is a different discipline from §3.4's cursor, and must not be merged with it.** An
`obliterator` cursor freezes a length at construction and reads elements lazily (hybrid capture,
DIV-PROJ-10); a `Map` cursor owns its entry list, skips tombstones and sees appends. Both are faithful —
to different things. `MapCursor` therefore does *not* implement `cursor::Sequence`, and there is no
`Step::Gap`: a `Map` cursor has no frozen length to run past.

#### Representation, and the one genuinely hard part

```rust
slots:   Vec<Slot<K, V>>        // Slot { id: u64, entry: Option<(K, V)> }, sorted by id
index:   HashMap<K, usize>      // key -> physical slot
live:    usize                  // Map.prototype.size
next_id: u64                    // never reset, not even by clear()
```

`delete` **tombstones** — O(1), and V8's own `OrderedHashMap` does not shift either. Tombstones
accumulate, so `slots` is **compacted** once the dead outnumber the live (amortised O(1), since
each pass halves). Compaction moves entries, which would invalidate a cursor holding a physical
index.

**So a cursor does not hold one.** Every slot carries a monotonically increasing `id`, assigned at
insertion and never reused, so `slots` is *always* sorted strictly ascending by `id` no matter how
many compactions have run. A `MapCursor` stores the id it wants next and locates it by binary
search, with a physical-index hint that makes the uncompacted step O(1) and that is **validated,
never trusted**.

V8 solves the same problem by chaining old tables to new ones and transitioning live iterators
through a recorded hole list. The id is that idea with the bookkeeping deleted: it needs no
communication between the map and its cursors at all, which leaves `MapCursor` `Copy`, borrow-free
and impossible to invalidate — exactly what the FFI boundary needs, where a JS cursor outlives the
call that produced it and the map stays mutable underneath it.

`clear()` does **not** reset `next_id`, and that is load-bearing: keeping it monotonic is what makes
post-`clear` entries visible to an unfinished cursor.

**Known cost, stated:** the key is stored twice. `indexmap` avoids that with `hashbrown`'s raw-entry
API; core is zero-dependency by declaration and `std` exposes no equivalent on stable. Mitigated by
making the bridge's string keys `Rc<str>`, so the second copy is a refcount rather than the text.

#### `JsKey` — SameValueZero without a hand-written `PartialEq`

```rust
enum JsKey { Undefined, Null, Bool(bool), Number(u64 /* normalised f64 bits */), String(Rc<str>) }
```

`Hash` and `Eq` are **derived**. `Number` holds the bits of an *already normalised* double — every
`NaN` folded to one canonical `NaN`, `-0.0` folded to `+0.0` — so the derived pair is SameValueZero
by construction and there is no way for equality and hashing to disagree. That disagreement is the
failure mode a hand-written `PartialEq` beside a derived `Hash` invites, and its symptom is two map
entries under one key with every other test still passing.

**Object keys are rejected, loudly.** `Map` compares objects by identity and there is no identity
hash for a JS object reachable from Rust. Two designs are implementable and both cost something
real: tagging each object with a hidden id under a private `Symbol` is O(1) but mutates the
caller's object and fails on a frozen one; an association list of `napi_ref` probed with
`napi_strict_equals` touches nothing but is O(n) per operation and holds a strong reference to
every key it has ever seen.

**Neither was built, because no upstream test in the entire T3 family uses an object key.** Audited
across all ten test files: every key that reaches a `Map` is a string or a number. `fuzzy-map` and
`fuzzy-multi-map` *accept* objects at the public API but hash them to strings before the `Map` sees
one. Machinery no test can reach is worse than a stated limit; a silently wrong answer is worse
than both. Revisit only if a module lands that needs it.

#### JS values, which for `default-map` was the larger half

`map.get('one').push(1)` mutates a stored array in place and reads it back, so values must be the
caller's actual objects. `Retained` is split on the one question that matters — *does this value
have an identity a caller could observe?*

| value | stored as |
|---|---|
| object, function, symbol, bigint | a counted `napi_ref` |
| null, boolean, number, string | by value |

Forced twice over. **Required:** `napi_create_reference` rejects a number at `NAPI_VERSION` 9,
which the addon declares — measured, it is what made two of seven upstream assertions fail on the
first run. **And right:** a `napi_ref` is a V8 global handle, and one per stored value would mean a
million global handles for a million-entry `lru-cache` against upstream's inline SMIs. Nothing is
observable either way, because a JS primitive has no identity.

`Received` exists because napi's own `FromNapiValue for Option<T>` maps **both** `undefined` and
`null` to `None`. `null` is a perfectly good `Map` value that must round-trip —
`test/lru-cache.js` asserts exactly that — and only `undefined` is absence. Core spells that
absence `None`: `DefaultMap<K, V>` stores `Option<V>`.

A `napi_ref` is not freed by dropping the Rust value that holds it, because
`napi_delete_reference` needs an `Env` and `Drop` has none. Every removal path releases explicitly,
and `#[napi(custom_finalize)]` covers the last one.

#### What this costs the next ten modules

A T3 module is now: a core type over `OrderedMap`, a `#[napi]` class using `JsKey`/`Received`/
`Loaned`, one `MapBridgeCursor` per iterator flavour, one row in `ITERATOR_FACTORIES` — **which is
not always `values`**; `DefaultMap`'s last line aliases `entries` — and a `ModuleSpec`. The oracle
needed three additive changes for `default-map` (`Map` encoding, argument decoding for
`undefined`/`-0`/`NaN`, named factories) and should need none for the rest.

---

## 4. Differential fuzzer

Generic **operation-sequence state machine**, parameterized per module. Written once, covers
every module shipped. This is the single strongest artifact for the 30% category and the +5 bonus.

```
seed → generate op sequence → apply to Rust (in-process)
                            → apply to Node original (subprocess)
                            → compare observable state after EVERY op
```

**Performance rule:** spawn Node **once** and stream line-delimited JSON. Spawning per op makes
60 seconds of fuzzing take an hour.

```
→ {"op":"set","args":[42]}
← {"ok":true,"state":{"size":1,"bits":"…"}}
```

Per-module spec declares the op alphabet and arg generators (sketch, not compilable):
```
ModuleSpec {
    name: "bit-set",
    ctor: |rng| vec![json!(rng.gen_range(1..=4096))],
    ops: &[
        Op { name: "set",   args: |rng, cap| vec![json!(rng.gen_range(0..cap))] },
        Op { name: "reset", args: |rng, cap| vec![json!(rng.gen_range(0..cap))] },
        Op { name: "rank",  args: |rng, cap| vec![json!(rng.gen_range(0..cap))] },
        Op { name: "clear", args: |_, _| vec![] },
    ],
    observe: &["size", "capacity", "toJSON"],
}
```

### 4.1 Use `proptest` — do not hand-roll generation or shrinking

**Decided.** `proptest` provides op-sequence generation *and* automatic shrinking. Shrinking is the
expensive part to write and the valuable part to have: it turns a 4,000-op divergence into a
minimal repro you can paste into a `DECISIONS.md` entry or an upstream issue for the
**Bug Catcher +$100**.

Shape: the op sequence is the generated value, the differential comparison is the property.

```
proptest! {
    #[test]
    fn port_matches_original(ops in prop::collection::vec(any_op(), 1..500)) {
        let mut rust = RustBitSet::new(4096);
        let mut node = oracle.new_instance("bit-set", 4096);
        for op in ops {
            let a = rust.apply(&op);
            let b = node.apply(&op);          // persistent subprocess, line-delimited JSON
            prop_assert_eq!(a, b, "divergence at {:?}", op);
        }
    }
}
```

Why not the alternatives: `cargo-fuzz`/libFuzzer is byte-oriented, needs nightly, and shrinks bytes
rather than op sequences. `arbitrary` alone gives generation without shrinking. Hand-rolled
bisection is a day of work to reimplement what `proptest` already does well.

It also aligns with the rubric's own language — *"Property tests survive translation; example-based
tests don't."* Using the standard property-testing crate and naming it is worth stating explicitly
in the write-up and `DECISIONS.md`.

**Persist the regression corpus.** `proptest` writes failing seeds to `proptest-regressions/`.
Commit that directory — it is machine-checkable evidence that a divergence was found, minimised,
and fixed, which is exactly the rigor the event says is missing.

`fuzz/log.txt` records per module: seed, op count, wall time, divergences. Target 60s+ each, zero.

---

## 5. Benchmarks

`bench/results.json` — **keyed per module from the start.** Retrofitting this at hour 50 is misery.

```json
{
  "methodology": "bench/methodology.md",
  "host": {"cpu": "...", "cores": 0, "governor": "...", "ram_gb": 0,
           "rustc": "1.97.1", "node": "24.18.1", "in_docker": true},
  "protocol": {"warmup": 3, "measured": 10, "batch_k": 1000, "interleaved": true},
  "baseline_rss_mb": {"node": 0, "rust": 0},
  "modules": {
    "bit-set": {
      "workload": "1e6 mixed set/reset/rank over cap 1e6, xorshift32 seed 42",
      "original": {"p50_ns_per_op": 0, "p99_ns_per_op": 0,
                   "rss_total_mb": 0, "rss_delta_mb": 0, "startup_ms": 0},
      "port":     {"p50_ns_per_op": 0, "p99_ns_per_op": 0,
                   "rss_total_mb": 0, "rss_delta_mb": 0, "startup_ms": 0}
    }
  }
}
```

Rules: **p99 over averages** (the rubric says so explicitly). Same workload, same seed, both sides.
Report regressions honestly — the FAQ states hiding one scores worse than disclosing it.
Benchmark the **pure Rust** path, never through N-API; napi overhead would poison the comparison
and misrepresent the port.

**Tools — superseded by §5.2 and §12c.2, which are authoritative.** This line originally read
"`criterion`, `hyperfine`, `/usr/bin/time -v`" and contradicted both. Corrected:

| Need | Tool | Why not the obvious choice |
|---|---|---|
| Comparative p50/p99 | **matched hand-rolled harness**, same shape both sides | `criterion` has no Node counterpart; criterion-vs-loop is two methodologies in one table (§5.2 Problem 1) |
| Rust-only regression tracking | `criterion` is fine here | — |
| Startup | `hyperfine` | the one place a uniform external tool *is* fair: it times whole processes identically |
| RSS | **in-process** `getrusage` / `process.resourceUsage().maxRSS` | `/usr/bin/time -v` is GNU `time`, absent from `node:slim`, and mixing it with an in-process reading on the other side breaks the same-methodology rule (§12c.2) |

### 5.1 Workload design

**The fairness rule: both sides must execute the identical op sequence.** If each implementation
rolls its own from `rand`/`Math.random`, you are partly benchmarking two PRNGs.

**Mechanism — a matched PRNG, not a serialised file.** Specify one tiny generator (xorshift32,
fixed seed) and implement it identically in ~10 lines on each side. A serialised `workload.jsonl`
would be ~30 MB for 1e6 ops and drags JSON parsing into the picture; the matched PRNG has zero I/O
and is provably identical. **Verify once:** dump the first 1000 values from each implementation and
`diff`. Materialise the op array *before* the timed region so generation is never measured.
(Serialisation remains the fallback for op types too complex to generate identically.)

**Warm up V8, and say so.** Node needs JIT warmup; measuring cold JS against optimised Rust is a
dishonest win and a judge will spot it. Protocol per module: **3 warmup runs, 10 measured**, report
p50 / p99 / min plus peak RSS. State the protocol in `methodology.md`.

**Measure the pure paths.** Rust native binary vs `node script.js`. Never through N-API.

| Module group | Workload |
|---|---|
| `bit-set`, `bit-vector` | 1e6 random `set`/`reset`/`get`/`rank` over capacity 1e6; growth path for bit-vector |
| `sparse-set`, `sparse-map`, `sparse-queue-set` | 1e6 `add`/`remove`/`has` cycles, capacity 1e6 |
| `vector`, `hashed-array-tree` | sequential push to 1e6, then 1e6 random reads; include resize |
| `fixed-deque`, `circular-buffer` | ring steady state — push/shift churn at capacity, 1e6 ops |
| `stack`, `queue` | 1e6 push/pop churn |
| `sort/` | 1e5 elements × 4 distributions: random, sorted, reverse, few-unique |

**Three metrics, three different stories — report all:**
- **Throughput / p99 per-op** — the real comparison. p99 is where Node's GC pauses show up, and
  it is what the rubric asks for.
- **Peak RSS** — typed arrays vs Rust arrays; expect a genuine win, quantify it.
- **Startup** — `hyperfine` on process start. Node's ~30–40ms boot dominates short workloads. This
  is a real but *cheap* win; report it separately and label it as startup, not throughput. Folding
  it into per-op numbers would be misleading.

**Expect to lose somewhere and report it.** mnemonist is already typed-array-backed and well
optimised; Rust will not dominate uniformly on raw bit operations. The FAQ is explicit that hiding
a regression scores worse than disclosing it — and a benchmark table where the port loses two rows
is far more credible than one where it wins everything.

### 5.2 How the benchmarks actually run

Three methodological problems have to be solved before any number is trustworthy.

#### Problem 1 — do NOT use criterion for the comparative table
Criterion has no Node counterpart. Criterion-vs-hand-rolled-loop is two methodologies in one table,
and a judge who notices will discount every row. **Use a matched harness written the same way on
both sides**: same warmup count, same measured count, same monotonic clock semantics, same
percentile maths. Criterion is still the right tool for Rust-only regression tracking during
Wave 1 — just keep it out of the comparison.

#### Problem 2 — batch the timing, or p99 measures the clock
`Instant::now()` / `process.hrtime.bigint()` cost ~20–30 ns. A `bit-set` `set` is ~1–2 ns. Timing
individual ops on the fast structures measures timer overhead almost exclusively.

**Time batches of K ops (K = 1000), record per-batch duration, report p99 across batches.**
Two things fall out of this, both good:
- Timer cost drops to ~0.03% of each sample instead of ~95%.
- **GC pauses land in exactly one batch each**, so batch-level p99 is precisely where V8's tail
  behaviour becomes visible. This is the metric that makes the GC-vs-predictable-latency argument,
  and it is more honest than a mean.

Report `p50_ns_per_op` and `p99_ns_per_op`, both derived from batch times divided by K.

#### Problem 3 — subtract the RSS baseline
`/usr/bin/time -v` peak RSS for Node includes ~40 MB of V8 before any data structure exists.
Reporting "5 MB vs 45 MB" as a data-structure result is the memory equivalent of claiming Node's
process startup as a throughput win.

**Measure a no-op baseline for each runtime** (empty script / empty `main`), then report **both**
`rss_total_mb` and `rss_delta_mb`. The delta is the honest structural comparison; the total is
still worth showing, clearly labelled as including runtime overhead.

#### Run protocol
- **Interleave A/B/A/B**, never all-Rust-then-all-Node. Thermal drift and background load are
  monotonic over a run; interleaving cancels them, sequential runs bake them into the result.
- 3 warmup + 10 measured per side (§5.1), warmup mandatory for V8.
- `taskset -c 2,3` to pin cores; record host CPU, core count, and governor in `results.json`.
- Startup measured separately with `hyperfine` — the one place a uniform external tool *is* fair,
  since it times whole processes identically.

#### Driver
`bench/run.sh <module>` → appends one keyed entry to `bench/results.json`.
Unattended and resumable: at ~30 s per module per side, 13 modules is roughly 30–45 minutes of
wall clock, which must run **while you write DECISIONS.md at CP4**, not interactively.

#### Docker
Specified in **§12c** as a dedicated `bench` stage carrying both implementations, with the four
non-obvious requirements in **§12c.2** — chief among them that the upstream JS baseline must be
vendored from the same clone as the hashed tests (the shims have replaced it), and that RSS is
measured in-process rather than via `/usr/bin/time -v`.

---

## 6. Wave plan

Critical path, from the measured dependency graph:
`obliterator/foreach` (22 dependents) · `obliterator/iterator` (18) · `utils/typed-arrays` (14) · `utils/iterables` (12)

### Wave 0 — machinery + two pilots (~12h)
Bridge skeleton, iterator protocol, `obliterator` port (§3.4), `utils/{typed-arrays,iterables,bitwise,comparators}`,
scaffold generator, `tests/run.sh` (§2.3), fuzz driver, bench harness, Dockerfile,
**CI workflow (§12a, ~30 min)**, hashing.

Pilots chosen to **stress** the machinery:
1. `static-disjoint-set` (195 LOC, typed-arrays only, **zero iterators**, 34-line test) — proves the pipeline end-to-end fast.
2. `sparse-set` (168 LOC, `obliterator/iterator`, 76-line test) — forces the iterator bridge at hour ~8, not hour ~40.

Do **not** pick two easy pilots.

**`test/_utils.js` is a real test file, not a helper.** 389 lines, **20 `describe` blocks**,
covering `utils/{typed-arrays, binary-search, merge, hash-tables, iterables}`. No other test file
requires it. So Wave 0's utils work **earns direct 40%-category credit** rather than being pure
infrastructure — a meaningful change to the wave's value.

| utils module | LOC | wave |
|---|---|---|
| `typed-arrays` | 187 | 0 (14 dependents) |
| `iterables` | 93 | 0 (12 dependents) |
| `bitwise` | 109 | 0 |
| `comparators` | 79 | 0 |
| `binary-search` | 216 | 1.5 |
| `hash-tables` | 107 | 1.5 |
| `merge` | 563 | 1.5 — heaviest single util; only `inverted-index` needs it |

**CORRECTED (§1.1): `test/_utils.js` accrues NO partial credit.** Its five requires are top-level,
so a single missing module throws before any `it()` runs and the entire file fails. All five —
including `merge` at 563 LOC — must land before one assertion scores. That makes the utils unit
~1,166 LOC, one of the largest in the repo, not the fill-in work described here originally.

### Wave 1 — contiguous/bit subset (~3,300 LOC) ← **THE SHIPPABLE MILESTONE**
`sparse-map` 243 · `sparse-queue-set` 218 · `bit-set` 379 · `bit-vector` 550 · `hashed-array-tree` 209 ·
`vector` 373 · `fixed-deque` 357 · `fixed-stack` 242 · `circular-buffer` 140 (→fixed-deque) ·
`queue` 215 · `stack` 210 · `sort/` 166

All depend only on Wave 0 primitives. Waves 0+1 ≈ 4,000 LOC — comfortably inside the stated bands,
and a genuinely coherent thesis: *the subset where JS typed arrays become real Rust arrays.*

#### Wave 1 module order

Ordering principles, in priority order:
1. **One new capability at a time** — never debug two unknowns at once.
2. **Introduce the hardest primitive on the simplest host structure.**
3. **Respect hard deps** (`circular-buffer` requires `fixed-deque`).
4. **Integration module last** — the one touching every primitive validates the whole stack.

| # | Module | src | test | New capability introduced | Why here |
|---|---|---|---|---|---|
| 1 | `hashed-array-tree` | 209 | 114 | — (zero deps, zero iterators) | Warm-up; proves the scaffold generalizes past the pilots |
| 2 | `sparse-map` | 243 | 139 | — | Same shape as pilot `sparse-set`; fast win confirming the pilot generalizes |
| 3 | `sparse-queue-set` | 218 | 134 | — | Completes the sparse family at one tier |
| 4 | `bit-set` | 379 | 189 | `utils/bitwise` | Small new primitive, well-tested host |
| 5 | `bit-vector` | 550 | 320 | growth/resize semantics | Reuses bitwise; largest bit module |
| 6 | `stack` | 210 | 126 | **`obliterator/foreach`** | Hardest primitive (5-branch dispatch) on the simplest possible structure |
| 7 | `queue` | 215 | 126 | — | Confirms the `foreach` port on a second trivial host |
| 8 | `fixed-stack` | 242 | 157 | **`utils/iterables`** | New primitive, small host |
| 9 | `fixed-deque` | 357 | 281 | wrap-around indexing | Prereq for #10 |
| 10 | `circular-buffer` | 140 | 339 | — (depends on #9) | **Best test:src ratio in the wave, 2.4:1** |
| 11 | `vector` | 373 | 234 | — (uses all four primitives) | Integration module: `foreach` + `iterator` + `iterables` + `typed-arrays` |
| 12 | `sort/` (quick 116, insertion 50) | 166 | 170 | — (standalone) | No deps; pure algorithm. Usable as filler at any point |
| | **Total** | **3,302** | **2,329** | | |

Steps 6 and 8 are the two genuine risk points. If either overruns, `sort/` and #1–5 already
constitute a shippable, coherent "bit and sparse structures" scope.

### Wave 2 — heaps (~1,940 LOC) · needs T2
`heap` 576 · `fixed-reverse-heap` 209 · `fibonacci-heap` 321 · `multi-set` 445 · `static-interval-tree` 387

### Wave 3 — maps/sets (~2,880 LOC) · needs T3
`default-map` 162 · `default-weak-map` 108 · `set` 356 · `bi-map` 195 · `multi-map` 408 ·
`fuzzy-map` 185 · `fuzzy-multi-map` 196 · **LRU family 1,271** (see below)

**The LRU family is better value than first assessed.** `test/lru-cache.js` (497 lines) requires
**all four** variants — `LRUCache` 436, `LRUMap` 261, `LRUCacheWithDelete` 287, `LRUMapWithDelete`
287. One test file covers 1,271 LOC, and the three variants are thin layers over `lru-cache`
(`lru-map` → `lru-cache`; both `*-with-delete` → their base). Porting `lru-cache` largely yields
the rest. **Prioritise this cluster within Wave 3.**

### Wave 4 — tries/strings (~2,500 LOC)
`trie-map` 477 · `trie` 167 · `inverted-index` 249 (+`utils/merge` 563) · `suffix-array` 353 · `multi-array` 447

### Wave 5 — spatial/probabilistic (~3,270 LOC)
`kd-tree` 447 · `vp-tree` 367 · `bk-tree` 180 · `symspell` 547 · `passjoin-index` 518 ·
`bloom-filter` 186 (+`murmurhash3` 87) · critbit pair 942

These are exactly the 6 test files needing external JS oracles (`lodash`, `static-kdtree`,
`damerau-levenshtein`, `seedrandom`, `leven`) — the test image must install them.

### Deprioritized — genuinely untested (251 LOC)
`semi-dynamic-trie` 251 — **the only** shipped module with no test coverage anywhere.

**RESOLVED pre-kickoff (was 1,086 LOC).** `test/lru-cache.js` requires all four LRU variants directly:
```js
LRUCache            = require('../lru-cache.js'),
LRUMap              = require('../lru-map.js'),
LRUCacheWithDelete  = require('../lru-cache-with-delete.js'),
LRUMapWithDelete    = require('../lru-map-with-delete.js');
```
So 835 of the 1,086 LOC **are** covered and do earn 40%-category credit. Moved into Wave 3.
Only `semi-dynamic-trie` stays deprioritized.

---

## 7. Schedule (H = hours since kickoff; kickoff Jul 31 18:00 UTC / 20:00 local)

| Block | UTC | H | Work |
|---|---|---|---|
| D1 eve | Jul 31 18:00 – Aug 1 00:00 | H0–H6 | Setup, vendor + hash tests, workspace, napi skeleton, `obliterator` port started |
| sleep | Aug 1 00:00 – 07:00 | H6–H13 | |
| D2 | Aug 1 07:00 – 23:00 | H13–H29 | Finish Wave 0, both pilots green, fuzz driver, bench harness. **CP1 @ H14.** Start Wave 1 |
| sleep | Aug 1 23:00 – Aug 2 06:00 | H29–H36 | |
| D3 | Aug 2 06:00 – 22:00 | H36–H52 | Wave 1 bulk. **CP2 @ H40.** **CP3 @ H52** |
| sleep | Aug 2 22:00 – Aug 3 05:00 | H52–H59 | |
| D4 | Aug 3 05:00 – 15:00 | H59–H69 | **CP4 @ H60 = FEATURE FREEZE.** Long fuzz runs, benches, DECISIONS.md, README |
| submit | Aug 3 15:00 – 18:00 | H69–H72 | Video, hash verify, Docker clean-build test, submit |

**Watch for the submission form ~Aug 2** (FAQ: *"We send a submission form ~1 day before"*). It
asks for team name, **track letter (§0)**, and repo — none of which were given at registration.
Do not leave first contact with the form to H70.

~48 working hours of 72. Solo, this is the honest number.

### 7.1 The first 14 hours, task by task

**Governing rule: the spec is a target, not a build order.** §2.3, §12a, §12c, and §12e are all
fully written — which makes it tempting to implement them. Don't. Build the *minimum* version of
each, and only generalise once a real module has forced the shape. Specifically:

- **Do not build the scaffold generator before porting one module by hand.** You don't yet know
  what to generate.
- **Do not build CI or Docker before the bridge is proven.** Ninety minutes on a green badge for a
  thing that might not work is the worst possible trade at H2.
- **Do not implement `run.sh` in full.** A five-line version that copies files and runs one spec is
  correct for H2; §2.3 is what it grows into by H18.

#### Day 1 evening — H0 to H6 (18:00–00:00 UTC / 20:00–02:00 local)

Six hours, ending at 2am local, on night one. **One goal: a single module green through the
bridge.** Everything else is a bonus. Do not over-schedule this block.

| Task | Est | Depends | Done when |
|---|---|---|---|
| **A** Kickoff checklist §10 — vendor, hash all 41, `.port-mortem.toml`, upstream green in `.work/` | 0:30 | — | Hashes committed *before* any port code exists |
| **B1** Workspace skeleton: root `Cargo.toml`, `mnemonist-core` (`#![forbid(unsafe_code)]`), `mnemonist-napi` (cdylib, napi 3.6.1) | 0:30 | A | `cargo build --release -p mnemonist-napi` succeeds, empty |
| **B2** `utils/typed_arrays` — **only** `getPointerArray` and what B3 needs | 0:30 | B1 | Unit-tested in core |
| **B3** `static-disjoint-set` core port (195 LOC, zero iterators) | 1:15 | B2 | Core `#[test]`s pass |
| **B4** napi wrapper: constructor, `find`, `union`, `connected`, getters | 0:45 | B3 | `.node` loads in Node, methods callable by hand |
| **B5** Crude `run.sh` + shim — hardcode the one spec, no scope.txt, no verification | 0:30 | B4 | Script runs mocha against `.work/` |
| **🚩 GATE** | | | **`npx mocha test/static-disjoint-set.js` → green** |
| Buffer | 2:00 | | Absorbs the first napi surprise, whatever it turns out to be |

Two hours of buffer in six is deliberate. The first end-to-end pass through an unfamiliar FFI
toolchain is where estimates fail, and this block has no slack elsewhere in the schedule behind it.

**If the gate is met before H4:** read `obliterator` source and sketch the cursor type. Do not
start `sparse-set` — begin it fresh on Day 2 rather than half-finished at 1am (**P1**).

#### Day 2 morning — H13 to H17 — the real unknown

| Task | Est | Done when |
|---|---|---|
| **C1** Cursor design: core index cursor + napi `SharedReference` to parent (§3.4, §3.7 B1) | 1:00 | Design settled, one cursor compiles |
| **C2** `obliterator::Iterator` cursor semantics in core | 0:45 | Self-returning, non-restartable, verified against §11.3 smoke results |
| **C3** `sparse-set` core port (168 LOC) + remaining `typed_arrays` | 1:00 | Core `#[test]`s pass |
| **C4** napi `#[napi(iterator)]` wiring + collection `Symbol.iterator` **factory** half (§3.6) | 0:45 | — |
| **🚩 CP1** | | **`test/sparse-set.js` green, iterator tests included** |

#### H17 to H29 — generalise, then start Wave 1

| Task | Est | Note |
|---|---|---|
| **D1** `run.sh` to full §2.3 spec — scope.txt, tiered verification, both modes | 1:00 | Now you know what it must do |
| **D2** Scaffold generator: core file + napi wrapper + shim + scope entry | 1:30 | Only now — two modules have shown the shape |
| **D3** Remaining Wave 0 utils: `iterables`, `bitwise`, `comparators` | 1:00 | Earns `test/_utils.js` partial credit |
| **D4** `forEach`/`iter` boundary coercion (§3.5) | 1:30 | Unblocks every `.from()` constructor |
| **E1** Node oracle: persistent subprocess, line-delimited JSON | 1:00 | Throughput sane |
| **E2** proptest op-sequence harness (§4.1) | 1:30 | First differential run on both pilots |
| **F1** Dockerfile (§12c) + `.dockerignore` | 1:00 | **Only after CP1** |
| **F2** CI (§12a) | 0:30 | **Only after CP1** |
| **G** Begin Wave 1 in order (§6): `hashed-array-tree` → `sparse-map` → … | rest | Target ≥2 modules before sleep |

**Adding a module should be a repeatable ~30–45 min operation by H29.** If it isn't, the scaffold
generator (D2) is the thing to fix before porting anything further — that ratio is what determines
how much of Wave 1 lands.

### 7.2 REVISED ORDER — harnesses before more modules

Adopting P0/§1.1 changes what comes next. Gates 9 and 10 need a differential fuzzer and a bench
harness that do not exist, so **every module ported before they exist lands non-compliant and needs
retrofitting.** Retrofitting N modules is strictly worse than building the harness once.

**Revised sequence:**

| Step | Work | Why here |
|---|---|---|
| **1** | `difffuzz`: persistent Node oracle + proptest op-sequence driver (§4, E1+E2) | Gate 9 for everything downstream |
| **2** | Bench harness: matched xorshift32, batch timing, RSS via rusage (§5.1–5.2) | Gate 10 for everything downstream |
| **3** | **Backfill `StaticDisjointSet` to full DoD** — gates 8, 9, 10 | Shakes out both harnesses on the *simplest* module, with no iterators in the way. Converts the smoke-test into the reference implementation of the process |
| **4** | Block C — `sparse-set` + cursor design | First unit that is DoD-compliant *natively* |
| **5** | Wave 1 remainder, re-sorted by test weight | See below |

Backfilling before Block C is deliberate: it means **one** module needs retrofit instead of two,
and the harnesses get debugged against a module with zero iterator surface.

**Re-sort Wave 1 by test weight once the cursor pattern is proven.** The original order optimised
for capability risk — correct while the toolchain was unknown, wasteful afterwards. Test lines are
where the 40% actually lives, and they are badly back-loaded:

| Module | test lines | src | current pos |
|---|---|---|---|
| `circular-buffer` | **339** | 140 | #10 (behind `fixed-deque`) |
| `bit-vector` | 320 | 550 | #5 |
| `fixed-deque` | 281 | 357 | #9 |
| `vector` | 234 | 373 | #11 |
| `bit-set` | 189 | 379 | #4 |

Those five are ~60% of Wave 1's total test weight. Capability order still constrains
(`circular-buffer` genuinely needs `fixed-deque`), but within a tier, sort by test lines.

### 7.3 Pipelining — measured from Block C, and the one gate that cannot pipeline

`sparse-set` took **95 minutes wall clock**, of which the port itself was ~14. Measured breakdown:
cursor 5 · port+bridge 9 · **fuzz-driver bug 34** · gate 9 fourteen · gate 10 eight · doc+scope 6,
plus ~19 lost to an API failure and recovery. The 34-minute block was a one-time discovery and does
not recur. What remains is roughly **10 minutes per module of pure machine time** — a fuzz campaign
is 120s + 60s by design, and gate 10 is 3 warmup + 10 measured, interleaved, per workload.

That machine time is what pipelining recovers. But not all of it:

| Gate | Overlaps? | Why |
|---|---|---|
| 1–4, 6, 7 (port, bridge, tests, falsification) | **yes** | CPU-bound but correctness-only; contention costs speed, not validity |
| 8 (divergence doc) | **yes** | writing |
| 9 (fuzz) | **yes** | a divergence is a divergence regardless of machine load |
| **10 (bench)** | **NO** | **a benchmark under load is not a slow benchmark, it is a wrong one** |

**Gate 10 must run on an idle machine, and this is measured rather than assumed.** A run taken
while the machine was saturated inflated *both* sides 2–3×, and upstream's own p99 swung 102 → 135
between two otherwise-clean runs. Interleaving protects the A/B comparison from drift; it does not
make a contended measurement publishable.

#### The pipeline

1. **Port + fuzz + document** modules through gate 9. These may overlap — multiple agents, or an
   agent porting module N+1 while N's fuzz campaign runs.
2. **Batch gate 10.** When a few modules are pending, stop everything and run one serial benchmark
   pass over all of them on a quiet machine.
3. **`scope.txt` entries land after their benchmark**, never before. The done marker keeps meaning
   all ten gates.

This deliberately leaves units *complete except gate 10* for a while. That is a controlled
deferral, not a P1 violation, precisely because `scope.txt` is what P0 defines as done — a module
pending benchmark is visibly not in scope, and `tests/verify.sh` will say so.

**Batching is also better measurement, not just faster.** One quiet pass over several modules
shares a single warm, idle machine state, so cross-module numbers become comparable in a way
that per-module runs scattered across hours never were.

### Checkpoints (each is a real GO/NO-GO)

- **CP1 — FFI viability. Expected ~H17, hard deadline H20.** Both pilots green through the full
  pipeline, iterator tests included?
  **NO →** stop fighting napi. Fall back to **1:1 native `#[test]` translation**, which the admins
  explicitly accept and which *still counts* (D-01b) — this is a demotion, not a cliff. Say so
  plainly in `DECISIONS.md`.
  **Early-warning trigger:** if the Day-1 gate (one module green, no iterators) has not been met
  by **H6**, treat that as CP1 turning amber and re-read the fallback before sleeping. The Day-1
  gate deliberately avoids iterators precisely so that failing it means *the toolchain* is wrong,
  not the hard part.
- **CP2 @ H40 — pace.** ≥6 Wave 1 modules complete? **NO →** cut Wave 1 scope to what's reachable
  and re-declare scope. Shipping 8 complete beats 12 half-done.
- **CP3 @ H52 — Wave 1 done?** **YES →** start Wave 2 *only* if ≥4 modules look reachable by H60;
  otherwise spend the time on fuzz depth, bench rigor, and decision-log quality (worth more:
  30% + 20% vs. marginal 40%). **NO →** finish Wave 1, nothing else.
- **CP4 @ H60 — HARD FEATURE FREEZE.** No new modules after this, no exceptions. Anything
  incomplete gets deleted from the submission and listed in the roadmap.

---

## 8. Scoring map

| Category | % | Where it's earned |
|---|---|---|
| Functionality | 40 | Unmodified upstream tests green via bridge; `SHA256SUMS` verified on camera; one-command Docker |
| Behavioral equivalence | 30 | Generic op-sequence differential fuzzer, 60s+/module, zero divergences; honest p99/RSS |
| Code quality | 20 | `#![forbid(unsafe_code)]` on core; idiomatic Rust; decision-log depth |
| Innovation | 10 | Generic state-machine fuzzer across N structures; any upstream bug found |

Bonuses in reach: **Fuzz Survivor +5** (designed in) · **Zero Unsafe +5** (crate split) ·
**Decision Log +3** (10+ divergences — iterator ordering alone will produce several) ·
**Bug Catcher +3 / $100** (file upstream *during* the event; shrinker makes the repro trivial).

---

## 9. Risk register

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| 1 | napi iterator bridge harder than expected | Med | Pilot #2 forces it at H8. CP1 kills the approach at H14 if red |
| 2 | Wave-1 pace slower than planned | Med | CP2 re-declares scope. P1 means shipped modules stay clean |
| 3 | Node subprocess makes fuzzing too slow | Med | Persistent process + line-delimited JSON, designed in from the start |
| 4 | `obliterator` semantics mismatch (self-returning iterator, hybrid live/snapshot, `forEach` branch dispatch) | High | **Source read — mapped in §3.4.** Residual risk is implementation slip, not unknown behaviour. Fuzz iteration interleaved with mutation; assert on exact error strings |
| 5 | Admin ruling goes against FFI or subset scope | Low–Med | Fall back to bignumber.js — decide before H0, not after |
| 6 | Docker build works locally, fails clean | Med | Build from a clean clone at H69, not at H71 |
| 7 | Solo fatigue → sloppy final hours | High | CP4 hard freeze reserves 12h for non-code deliverables |
| 8 | ~~Windows/WSL toolchain friction with napi~~ | **RETIRED** | Resolved pre-kickoff (§11.5). Linux primary; napi 3.6.1 built and loaded end-to-end; Windows linker trap identified and sidestepped |
| 9 | WSL distro instability under Docker Desktop startup | Low–Med | Observed once pre-kickoff (`getpwuid` failures → `E_UNEXPECTED`); fixed by `wsl --shutdown` + restart. If it recurs mid-event, that's the remedy — ~60s, no data loss |
| 11 | **Git history reads as a dump → disqualification** | Low but **fatal** | P5. Commit the hashed suite first, then per module. Never squash a wave. Judges inspect history explicitly (FAQ) |
| 12 | Wrong track letter on the submission form | Low | §0 — website and FAQ disagree; confirm with admins before Aug 2. Cheap to fix, embarrassing to miss |
| 10 | Node/mocha version drift across pinning sites | Med | Node **24.18.1** pinned in **four** places — `.nvmrc`, `Dockerfile` ARG, CI matrix (§12a), §11.6. A drift to 26 silently breaks the whole harness. Grep all four whenever either version changes |

---

## 10. Kickoff checklist (first 30 minutes, in order)

1. Confirm registration + Discord; capture the admin FFI/scope reply into `DECISIONS.md` verbatim.
2. `git init`, workspace `Cargo.toml`, `.gitignore` (`target/`, `tests/.work/`, `node_modules/`).
3. Vendor **all 41** upstream test files → `tests/original/test/` **unmodified** (§2.2 — hash the
   full suite, not just the in-scope subset). **From the same clone, also copy the upstream `.js`
   source modules → `bench/upstream/`** — the benchmark baseline, which the shims will otherwise
   replace and make unrecoverable at the correct commit (§12c.2).
4. Generate `tests/SHA256SUMS` over the whole directory. **Commit before any port code exists** —
   this timestamp is the proof we did not choose our denominator after the fact.
5. Capture `git rev-parse HEAD` of the upstream clone **before anything else moves**, then write
   `.port-mortem.toml` (§12d): track `G` (§0 — confirm before submission), source URL, and
   `kickoff_hash` = `sha256sum tests/SHA256SUMS`.
   Those fields are frozen from this moment on.
6. Pin Node **24.18.1** (`.nvmrc`), mocha `^9.1.3` — §11.6. Do not let either drift.
7. Confirm `npx mocha test/heap.js` runs green against **upstream** in `.work/` before any Rust exists —
   proves the harness independently of the port.
8. Then, and only then, start Wave 0.

---

## 11. Resolved reference decisions

### 11.1 Licensing — DECIDED
Upstream is **MIT, © 2016 Guillaume Plique (Yomguithereal)**. Source files carry **no** per-file
copyright or SPDX headers — only descriptive comments. MIT requires the notice to accompany
"all copies or substantial portions," so:

- `LICENSE` — your own MIT for original work
- `LICENSE-MNEMONIST` — upstream text **verbatim**, unaltered copyright line
- `README.md` — explicit "derived from Yomguithereal/mnemonist (MIT)" statement
- `NOTICE` — attribution for `obliterator` (also Yomguithereal, MIT) as a second ported dependency
- Per ported module, a one-line attribution comment. Upstream has none, so this isn't strictly
  required — it's cheap, exceeds the obligation, and reads well to a judge checking licence hygiene.

### 11.2 Mocha invocation — CONFIRMED
`mocha ^9.1.3`, **no `.mocharc`**, `npm test` is bare `mocha`. Default spec glob is
`./test/*.{js,cjs,mjs}` — **non-recursive**. Corroborated internally: `test/exports/` exists as a
subdirectory with its own separate `test:exports` script, which only makes sense if the default
run doesn't descend. So the harness must run all 42 top-level files in `test/` — including
`test/_utils.js` — and nothing under `test/exports/`.

Pin the exact mocha version in the harness `package.json`; a v10+ default-glob change would
silently alter which files run.

### 11.3 `napi-rs` Generator trait — CONFIRMED **AND BUILT**
Docs checked (§3.2), then **empirically validated** with a smoke crate in WSL:
`napi` 3.6.1 + `napi-derive` 3.6.1 + `napi-build` 2, `crate-type = ["cdylib"]`, built in 11.6s,
loaded into Node 24.18.1, called successfully.

- `()` **does** satisfy `FromNapiValue` for the unused `Next`/`Return` slots. No `Unknown` needed.
- Plain `cargo build --release` + copying `libX.so` → `X.node` is sufficient. `@napi-rs/cli` is a
  convenience, not a requirement.

**`#[napi(iterator)]` gives obliterator's cursor semantics for free.** Measured:

| Probe | Result | Matches |
|---|---|---|
| `c[Symbol.iterator]() === c` | `true` | §3.4 self-returning (`return this`) |
| first `[...c]` | `[1,2,3]` | — |
| second `[...c]` | `[]` | §3.4 **non-restartable** |
| `next(); next(); [...c]` | `[3]` | cursor state survives mixed consumption |

So **DIV-STACK-1 and the identity half of DIV-STACK-2 need no custom work.** What remains ours is the *factory*
half — the collection's `Symbol.iterator` must construct a fresh cursor each call (§3.6).

### 11.5 Environment — DECIDED AND BUILT
**Linux is the primary build/test environment; Windows is not used for building.**

| Component | Pinned | Note |
|---|---|---|
| Dev env | WSL2 Ubuntu 22.04 | repo in `~/`, **never** `/mnt/c` (9p is far slower for cargo) |
| Rust | 1.97.1 (WSL, rustup-managed) | matches Windows host version |
| **Node** | **24.18.1** | see §11.6 — this is a hard constraint, not a preference |
| Docker | 28.3.0, Linux engine | reference build for submission + benchmarks |
| napi | 3.6.1 / napi-derive 3.6.1 / napi-build 2 | validated end-to-end |

**Why not Windows:** the host's `link.exe` on PATH resolves to Git/scoop's **GNU coreutils `link`
8.32**, shadowing the real MSVC linker (VS 2022 *is* installed). Building a cdylib would fail with
errors that look nothing like a PATH problem. Fixable, but pointless — judges run the Dockerfile.

### 11.6 Node version — CONSTRAINED BY MOCHA, NOT BY CHOICE
Measured against the real upstream suite, not assumed:

| Node | Result |
|---|---|
| 26.5.1 | **FAILS** — mocha 9.1.3's bundled `yargs` dies: `require is not defined in ES module scope` |
| **24.18.1** | **GREEN** ← pinned |
| 22.23.2 | **segfaults** on exec (exit 139) — bad build, unrelated to mocha |
| 20.20.2 | green |
| 18.20.8 | green |

Node 24 is the newest that runs mocha 9 with **zero deviation from upstream devDeps**. Upgrading
mocha instead would keep `test/*.js` hashes intact but changes the test runner — a real divergence
where none is needed. Pin 24.18.1 in the harness, the Dockerfile, and CI identically.

### 11.7 Upstream baseline — MEASURED
`npx mocha` on a clean clone, Node 24.18.1: **525 passing, 1 pending, 0 failing, 90ms.**
That is the parity target. `npm install` completes clean (165 packages, no native-build failures).
Re-measure and record per-module counts at kickoff.

### 11.4 `obliterator` semantics — MAPPED
See §3.4. Source read in full. The snapshot-vs-live question is answered: **hybrid** — length
captured at creation, elements read lazily.

## 12a. CI — `.github/workflows/parity.yml`

~30 minutes in Wave 0, and the highest value-per-minute artifact available. Judging is fully
async at 1–2 h/week across 10–12 projects: **a judge sees a green badge before they run anything.**
It also catches Dockerfile rot continuously instead of at hour 69 (risk #6).

Three jobs, each proving a different claim:

```yaml
name: parity
on: [push, pull_request]

jobs:
  # 1. THE FFI-RULE REBUTTAL, AUTOMATED.
  #    Container image has no Node at all — if the core needed a JS runtime, this fails.
  core-standalone:
    runs-on: ubuntu-latest
    container: rust:1.97-slim
    steps:
      - uses: actions/checkout@v4
      - name: Assert Node is absent
        run: '! command -v node'
      - run: cargo test -p mnemonist-core --release
      - name: Assert the unsafe ban is still declared
        run: grep -q 'forbid(unsafe_code)' crates/mnemonist-core/src/lib.rs

  # 2. TEST PARITY. CI=true forces full hash verification inside run.sh (§2.3).
  parity:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with: { toolchain: '1.97.1' }
      - uses: Swatinem/rust-cache@v2
      - uses: actions/setup-node@v4
        with: { node-version: '24.18.1' }
      - name: In-scope parity (gates the badge)
        run: ./tests/run.sh
      - name: Repo-wide count (informational, never fails)
        run: |
          ./tests/run.sh all | tee /tmp/all.txt
          { echo '### Repo-wide (informational)';
            grep -E 'passing|failing|pending' /tmp/all.txt; } >> "$GITHUB_STEP_SUMMARY"

  # 3. ONE-COMMAND BUILD — the submission requirement, checked on every push.
  docker:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - run: docker build -t port-mortem .
```

**Why job 1 is the interesting one.** It is the automated form of demo step 4: a container with no
Node, running the core's own tests. If `mnemonist-core` ever acquires a JS dependency, CI goes red.
That turns "the port doesn't link the source runtime" from a claim in `DECISIONS.md` into a
continuously-verified property — directly addressing the rule that made us ask the admins in the
first place.

**On the `forbid` grep.** `#![forbid(unsafe_code)]` already fails the *build* if unsafe is
introduced, so compilation is the real check. The grep only guards against the attribute itself
being deleted — a one-line regression that would otherwise silently void the Zero Unsafe claim.

**Notes.**
- Both `$GITHUB_STEP_SUMMARY` figures feed the README's dual reporting (§2.2) without opening logs.
- `rust:1.97-slim` — confirm the tag exists at kickoff; fall back to `rust:slim` + `rustup
  toolchain install 1.97.1` if not.
- Node and Rust versions are pinned here **and** in `.nvmrc` and the Dockerfile. Drift between the
  three is risk #10; grep all three whenever either changes.
- **If CI breaks mid-event, disable the workflow — do not debug it.** It is a scoring aid, not a
  dependency. Losing an hour to a runner quirk at hour 50 would be a straight loss.
- Badge in README on line 1: `![parity](https://github.com/<user>/<repo>/actions/workflows/parity.yml/badge.svg)`

## 12b. Demo video — `scripts/demo.sh`

Five minutes, hard requirement, produced at hour 70 with no slack. Build it as a **script that
narrates itself** (`echo` between steps) rather than something performed live — it is then
rehearsable, re-runnable, and immune to a fumbled command on camera. Polish only if time remains.

Written during the event (it is a deliverable), but the running order is fixed now:

| # | ~time | Step | What it proves |
|---|---|---|---|
| 1 | 0:15 | Print env: node/rustc versions, git commit, track + scope | Reproducibility |
| 2 | 0:30 | `tests/verify-hashes.sh` → PASS, then `git diff` vs upstream tests → **empty** | Test parity, the 40% headline |
| 3 | 0:30 | One-command build (`docker build` or `cargo build --release`) | Build requirement |
| 4 | 0:30 | `docker run --rm pm-core` (the `core` stage — **an image with no Node**, §12c) | Port is standalone — the FFI-rule rebuttal, on camera |
| 5 | 1:30 | `npx mocha` on the unmodified upstream suite through the bridge → N passing | **The money shot** |
| 6 | 0:45 | Short live fuzz run + `cat fuzz/log.txt` totals | Behavioural equivalence, 30% |
| 7 | 0:30 | One `proptest-regressions/` entry + the divergence it caught and its fix | Rigor, and the best single artifact |
| 8 | 0:30 | Benchmark table, **including a row where the port loses** | Honest measurement |
| 9 | 0:20 | Scope declaration: shipped vs roadmap | Frames a subset as complete-in-scope |

**Practical notes.** Steps 6 and 8 cannot run to completion on camera — pre-generate the logs and
show a short live sample plus the accumulated totals. Step 4 needs no special preparation now that
§12c exists: `--target core` *is* the Node-free environment, and it is the same assertion CI job 1
runs, so the claim is reproducible three ways. **Rehearse once before recording**; the first take
always reveals a step that reads as ambiguous.

## 12c. `Dockerfile` — the one-command build

Hard submission requirement ("one command to a runnable artifact"). For a data-structures library
there is no CLI, so **the runnable artifact is the parity proof itself**: `docker run` executes the
unmodified upstream suite against the port.

Three stages, mirroring the CI jobs so a judge can reproduce either locally.

```dockerfile
# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.97.1
ARG NODE_VERSION=24.18.1

# ---------- builder ----------
FROM rust:${RUST_VERSION}-slim AS builder
WORKDIR /src
# Warm the dependency cache before copying sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/mnemonist-core/Cargo.toml crates/mnemonist-core/
COPY crates/mnemonist-napi/Cargo.toml crates/mnemonist-napi/
COPY crates/difffuzz/Cargo.toml       crates/difffuzz/
RUN mkdir -p crates/mnemonist-core/src crates/mnemonist-napi/src crates/difffuzz/src \
 && touch crates/mnemonist-core/src/lib.rs crates/mnemonist-napi/src/lib.rs \
 && echo 'fn main(){}' > crates/difffuzz/src/main.rs \
 && cargo fetch
COPY . .
RUN cargo build --release -p mnemonist-napi

# ---------- core: NO NODE. mirrors CI job 1 ----------
FROM rust:${RUST_VERSION}-slim AS core
WORKDIR /src
COPY --from=builder /src /src
RUN ! command -v node                       # fails the build if Node ever creeps in
CMD ["cargo", "test", "-p", "mnemonist-core", "--release"]

# ---------- tools: cached separately from the port build ----------
FROM rust:${RUST_VERSION}-slim AS tools
RUN cargo install hyperfine --root /out --locked

# ---------- bench: BOTH implementations, one container (§5.2) ----------
FROM node:${NODE_VERSION}-slim AS bench
WORKDIR /app
# Upstream JS baseline, vendored at the SAME commit as the hashed tests — see 12c.2.
COPY bench/upstream ./bench/upstream
COPY bench/package.json ./bench/
RUN cd bench && npm install --no-audit --no-fund --silent   # obliterator, for upstream
COPY --from=builder /src/target/release/bench-runner  ./bench/
COPY --from=builder /src/target/release/rss-baseline  ./bench/
COPY --from=tools   /out/bin/hyperfine /usr/local/bin/hyperfine
COPY bench ./bench
CMD ["./bench/run.sh", "all"]

# ---------- parity (default target — MUST REMAIN LAST) ----------
FROM node:${NODE_VERSION}-slim AS parity
WORKDIR /app
ENV PM_NO_BUILD=1
# Pre-install harness deps so `docker run` needs no network.
COPY tests/harness-package.json ./tests/.work/package.json
RUN cd tests/.work && npm install --no-audit --no-fund --silent \
 && touch node_modules/.install-stamp
COPY --from=builder /src/target/release/libmnemonist_napi.so ./target/release/
COPY tests ./tests
COPY .port-mortem.toml README.md DECISIONS.md ./
CMD ["./tests/run.sh"]
```

**Usage.**
```
docker build -t port-mortem .                       # default: parity stage
docker run --rm port-mortem                         # in-scope suite (the proof)
docker run --rm port-mortem ./tests/run.sh all      # repo-wide figure
docker build -t pm-core --target core . && docker run --rm pm-core   # core, no Node

# benchmarks — pin CPUs, mount bench/ so results.json escapes the container
docker build -t pm-bench --target bench .
docker run --rm --cpuset-cpus=2,3 -v "$PWD/bench:/app/bench" pm-bench
```

### Two amendments this forces on §2.3

Found by working the integration through rather than at hour 69:

**1. `run.sh` must be able to skip the cargo build.** Step 2 calls `cargo build`, but the parity
image has no Rust toolchain — the `.so` arrives from the builder stage. Guard it:
```bash
[ "${PM_NO_BUILD:-}" = 1 ] || cargo build --release -p mnemonist-napi --manifest-path "$ROOT/Cargo.toml"
```

**2. The npm-install trigger must compare content, not mtime.** The stamp check
(`package.json -nt .install-stamp`) breaks in Docker: step 3 copies a fresh `package.json` whose
mtime is copy-time, always newer than the baked-in stamp, forcing a reinstall on every container
start — and failing outright with no network. Replace with a content comparison:
```bash
if [ ! -f "$STAMP" ] || ! cmp -s "$ROOT/tests/harness-package.json" "$WORK/package.json.installed"; then
  ( cd "$WORK" && npm install --no-audit --no-fund --silent )
  cp "$ROOT/tests/harness-package.json" "$WORK/package.json.installed"; touch "$STAMP"
fi
```
Robust in both environments, and mtime-independent.

### Notes
- **`.dockerignore` is mandatory**, not hygiene — see §12c.1 below.
- Version ARGs must match `.nvmrc`, CI (§12a), and §11.6. **Four places now** — risk #10.
- `node:slim` is Debian-based, so `bash`, `sha256sum`, and `mapfile` are all present. A `-alpine`
  base would break `run.sh` (busybox `sha256sum` differs, no `mapfile` in `ash`).
- Benchmarks are taken from this image (§5). Container overhead is negligible for CPU-bound work
  on Linux, but record the host CPU in `bench/results.json` regardless.
- The `core` stage's `! command -v node` is the same assertion as CI job 1, so the FFI-rule
  rebuttal is reproducible by a judge with one command and no GitHub access.

### 12c.2 The bench stage — four non-obvious requirements

**1. The upstream JS baseline has to be vendored, and we do not currently have it.**
This is a real gap. `tests/original/` holds the upstream *tests*; the upstream *implementation*
modules are exactly what the shims replace. So the harness that proves parity has deliberately
removed the thing the benchmark needs to compare against.

**Do not `npm install mnemonist@0.40.4`.** We cloned `--depth 1` of master, which may sit ahead of
the 0.40.4 release — benchmarking a released tarball against tests hashed from master would compare
two different codebases. Instead, **at kickoff, copy the upstream `.js` source modules from the
same clone into `bench/upstream/`**, alongside vendoring the tests. One clone, one commit, one
`upstream.commit` in `.port-mortem.toml`, everything consistent.

`bench/package.json` needs only `obliterator` — upstream's single runtime dependency.

**2. Both sides must run in one container.** Splitting them across `parity` (Node, no Rust binary)
and something else means the two halves execute under different conditions and the comparison is
void. Hence a dedicated stage carrying `bench-runner`, the upstream JS, and `hyperfine`.

**3. Drop `/usr/bin/time -v`.** It is GNU `time`, not the shell builtin, and Debian slim does not
ship it — `apt-get install time` would be needed. Better: measure RSS **in-process on both sides**,
`getrusage` in Rust and `process.resourceUsage().maxRSS` in Node. Both return peak RSS in KB.
Uniform methodology, no extra package, and consistent with §5.2 Problem 1. This supersedes the
tool list in §5.

**4. `--cpuset-cpus` beats `taskset`.** Pin at the container boundary rather than inside the
process; it is one flag and it cannot be forgotten by a script. §5.2's `taskset` line applies only
to bare-metal runs.

**Stage ordering matters:** Docker's default target is the **last** stage, so `parity` must stay at
the bottom of the file. `tools` is separate from `builder` so that `cargo install hyperfine` caches
independently and does not rebuild whenever port sources change.

### 12c.1 `.dockerignore`

Not hygiene — a correctness and speed requirement. Without it, `COPY . .` in the builder stage
ships the host's `target/` (easily 1–3 GB with debug artifacts) and `tests/.work/node_modules`,
which both bloats the image and **invalidates layer caching on every build** because those paths
change constantly.

```gitignore
# Build outputs — large, and they churn on every local build
target/
**/node_modules/
tests/.work/

# VCS and editor state
.git/
.idea/
*.swp

# Generated evidence: belongs in the repo, not baked into the image
fuzz/corpus/
proptest-regressions/
bench/raw/

# Planning docs the image has no use for
NOTES.md
```

**Do not over-exclude.** The builder needs `Cargo.toml`, `Cargo.lock`, `crates/`, `build.rs`, and
`tests/`. Excluding any of those produces a confusing mid-build failure. `README.md`,
`DECISIONS.md`, and `.port-mortem.toml` are `COPY`d explicitly into the parity stage, so they must
*not* be ignored.

**One consequence of excluding `.git/`:** the image cannot derive the upstream commit at build
time. Pass it in instead — `--build-arg SOURCE_COMMIT=$(git -C … rev-parse HEAD)` — or rely on
`.port-mortem.toml` (§12d), which records it anyway.

## 12d. `.port-mortem.toml` — submission metadata

The brief specifies only three things — *track letter, source URL, kickoff hash* — and **publishes
no schema**. So: keep those three at top level under the names the brief itself uses, and namespace
everything else into tables, where any reasonable TOML parser will ignore what it doesn't know.

**"Kickoff hash" is singular, but we have 41 hashes.** Resolve it canonically: the kickoff hash is
the SHA-256 **of `tests/SHA256SUMS` itself**. One digest, deterministic, and it transitively covers
every test file. State that derivation in the file so nobody has to guess.

```toml
# Port Mortem 2026 — submission metadata
# No schema was published. The three fields named in the brief (track, source, kickoff hash)
# are top-level; everything else is namespaced. If a schema is published later, conform to it
# and move all extras under a single [x] table.

track        = "G"     # JS→Rust per the admin FAQ; website table is stale (§0). Confirm by Aug 2.
source       = "https://github.com/Yomguithereal/mnemonist"
# sha256 of tests/SHA256SUMS, which itself covers all 41 upstream test files.
kickoff_hash = "sha256:<digest>"

[upstream]
commit          = "<upstream SHA at clone time>"   # pin the exact version ported
version         = "0.40.4"
license         = "MIT"
source_language = "JavaScript"
target_language = "Rust"
loc_total       = 15386
modules_total   = 44
tests_total     = 525          # measured pre-kickoff, Node 24.18.1, clean clone

[port]
scope_manifest = "tests/scope.txt"    # single source of truth (§2.3)
modules_ported = 0                    # filled at freeze
loc_in_scope   = 0                    # filled at freeze

[verify]
build           = "docker build -t port-mortem ."
test            = "docker run --rm port-mortem"
test_repo_wide  = "docker run --rm port-mortem ./tests/run.sh all"
core_standalone = "docker build -t pm-core --target core . && docker run --rm pm-core"
hashes          = "./tests/verify-hashes.sh"

[results]                              # filled at freeze — see §2.2 dual reporting
in_scope_passing  = 0
in_scope_total    = 0
repo_wide_passing = 0
repo_wide_total   = 525
```

**Frozen vs. filled.** `track`, `source`, `kickoff_hash`, and everything under `[upstream]` are
written at kickoff and **never touched again** — `kickoff_hash` in particular is the commitment
that makes the parity claim meaningful. `[port]` and `[results]` are filled at freeze.

**Why `[verify]` earns its place.** A judge with 1–2 h/week across 10–12 projects should not have
to reverse-engineer how to run anything. Four copy-pasteable commands in the metadata file — build,
test, repo-wide count, and the no-Node core proof — is the cheapest possible reduction in the
friction between a judge and a green result.

**Why `upstream.commit` matters.** "mnemonist" alone is ambiguous across versions; without the SHA,
nobody can reproduce the parity claim later. We cloned `--depth 1`, so capture
`git rev-parse HEAD` at kickoff before anything else moves.

## 12e. `README.md` — structure

**Design constraint:** judging is fully async, 1–2 h/week across 10–12 projects. **The first screen
decides how the rest is read.** So the top of the file answers all four rubric questions before any
scrolling, and everything below is depth a judge reaches only if they want it.

Structure the whole file so a judge can *score* it without hunting. Sections map to rubric
criteria in rubric order.

```markdown
# mnemonist-rs — a Rust port of Yomguithereal/mnemonist
![parity](…/parity.yml/badge.svg)

> Track G (JavaScript → Rust) · Port Mortem 2026 · solo

**13 of 44 modules ported. 100% of their original tests pass, unmodified.**
Repo-wide that is N/525, because 31 modules are declared roadmap — see [Scope](#scope).

| | |
|---|---|
| Build | `docker build -t port-mortem .` |
| Run the original suite | `docker run --rm port-mortem` |
| Repo-wide count | `docker run --rm port-mortem ./tests/run.sh all` |
| Core, with no Node installed | `docker build -t pm-core --target core . && docker run --rm pm-core` |

**Test parity** · **Behavioural equivalence** · **Code quality** · **Decisions** · **Benchmarks**
(anchor links)

---

## Why the original tests run through FFI          ← PRE-EMPTS THE OBVIOUS OBJECTION
[admin ruling quoted verbatim]
The port itself links nothing. `--target core` above is an image with **no Node at all**.

## Migration rationale                              ← REQUIRED BY THE BRIEF
Why this repo · why Rust · why this subset

## Scope                                            ← GENERATED from tests/scope.txt
| Module | LOC | Tests | Status |
...31 roadmap rows collapsed in <details>

## Test parity (40%)
How the harness works, the hashing commitment, both figures

## Behavioural equivalence (30%)
Differential fuzzer, op-sequence shrinking, proptest-regressions/, fuzz totals

## Code quality (20%)
Crate split, `#![forbid(unsafe_code)]`, the unsafe-at-the-boundary argument

## Benchmarks
Summary table **including rows where the port loses** → bench/methodology.md

## Decisions
Three or four highlights → DECISIONS.md

## Limitations and what is not done
Explicit, unhedged

## Reproducing · Attribution · Licence
```

### The five things that make or break it

**1. Lead with both numbers, in that order.** "100% of in-scope, N/525 repo-wide" pre-empts the
single most damaging misreading — a judge computing 180/525 and scoring it as 34% completion.
Volunteering the weaker number first is what makes the stronger one credible.

**2. Address the FFI objection above the fold.** A judge skimming sees "FFI" and may pattern-match
it to the prohibited *"FFI into source-language runtime"*. Quote the admin ruling and point at the
no-Node image **before** they form that impression. This is the highest-leverage paragraph in the
document.

**3. Generate the scope table from `tests/scope.txt`.** Fourth consumer of the manifest (§2.3).
Hand-maintaining it guarantees the classic failure: the README claims a module the harness never
runs, which reads as dishonesty rather than as the oversight it is. Collapse the 31 roadmap rows
in `<details>` so the table doesn't bury everything below it.

**4. Section headings carry rubric weights.** `## Test parity (40%)` tells a judge working through
a scoring form exactly where to look. Costs nothing and makes the document navigable under time
pressure.

**5. State limitations unhedged.** The FAQ is explicit that hiding a regression scores worse than
disclosing one. A benchmark table with two losing rows and a limitations section that names
`semi-dynamic-trie`, the Wave-5 gap, and any divergence from §3.7 is *more* credible than a clean
sweep — and costs nothing we weren't already going to admit in `DECISIONS.md`.

### Writing order
Draft the top block (title → command table) **in Wave 0**, when the commands are being written
anyway and are fresh. Everything else is fill-in-the-blank at CP4 from `DECISIONS-CANDIDATES.md`,
`NOTES.md`, and the generated scope table — which is the point of keeping those three current.

## 12. Still open

- [x] ~~Admin ruling on FFI~~ — **RATIFIED**, quoted in the header and DIV-PROJ-2.
- [ ] **Admin ruling on repo size / scoped subset** — still unanswered. Ask again pre-kickoff.
      Not a blocker; worst case is presentational.
- [x] ~~§3.7 shrink-window~~ — **DECIDED: Option A, sequenced.** Evidence gathered: no upstream
      test mutates during iteration, so the B fallback is measured-free.
- [x] ~~`test/lru-cache.js` grep~~ — **RESOLVED:** covers all four LRU variants. 835 LOC moved
      into Wave 3; only `semi-dynamic-trie` (251) stays deprioritized.
- [x] ~~Hashing scope~~ — **DECIDED (§2.2):** hash all 41 test files at kickoff, run the declared
      scope, report both numbers.
- [x] ~~README / judge-facing framing~~ — **structure specced (§12e).** Top block drafted in
      Wave 0; the rest is fill-in-the-blank at CP4 from the three companion docs.

**Companion documents (same directory):**
- `DECISIONS-CANDIDATES.md` — running log for the submission's `DECISIONS.md`. Append whenever a
  divergence surfaces; do not reconstruct at hour 65.
- `NOTES.md` — raw capture log for the write-up ($300 pool, deadline **Aug 10**, a week after
  freeze) **and** the upstream bug-candidate list for Bug Catcher (+3, $100). Bugs must be filed
  upstream **during** the event to count.
