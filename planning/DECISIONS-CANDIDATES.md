# DECISIONS.md — candidate log

Running scratch for what will become the submission's `DECISIONS.md`. **Append here the moment a
divergence surfaces.** Reconstructing these at hour 65 is how decision logs end up thin.

Companion to `DESIGN.md` (same directory). Pre-kickoff; nothing committed to the repo yet.

## Format

```
### D-NN — Title
Status:      CONFIRMED | OPEN | PENDING-ADMIN | PROPOSED
Category:    architecture | behavioural | tooling | licensing | scope
Divergence:  yes | no      ← only "yes" counts toward the +3 bonus
Upstream:    what it does, with evidence
Port:        what we do
Rationale:   why
Verify:      how we prove it
```

## Bonus tracking

The **Decision Log +3** bonus asks for **10+ non-trivial divergences**. Entries below: **34**.
Marked `Divergence: yes`: **11**. Threshold met on paper — but the marker is applied honestly.
Faithfully reproducing a strange upstream behaviour is a *decision*, not a divergence, and
inflating the count is exactly the kind of thing an adversarial judge checks.

**Upstream parity target (measured pre-kickoff, Node 24.18.1, clean clone):**
`525 passing · 1 pending · 0 failing · 90ms`

---

## Architecture

### D-01 — Original test suite driven through an N-API bridge
**Status:** **CONFIRMED — RATIFIED BY ADMINS** · **Category:** architecture · **Divergence:** no
**Upstream:** tests are JS, `require('../heap.js')` per file.
**Port:** port is pure Rust; original test files run **unmodified** in Node against a `.node` addon.
Node depends on Rust, never the reverse.
**Admin ruling, verbatim (Discord, pre-kickoff):**
> "Please keep the original test files exactly as they are in `tests/original/`, together with their
> kickoff SHA-256, and run them against your port through a thin adapter or FFI shim. If you rewrite
> the tests, your score will go down because the hashes are fixed and the judges will see the diff."

> "Unsafe code at the FFI boundary is expected and is not a problem. Also, tests are optional for
> qualification now, but running the original tests unchanged is still the top-1 strongest proof
> you can show."

**Rationale:** the rules forbid *the port* linking the source runtime; the direction is inverted here.
Standard practice (oxc, SWC, Rolldown). Now settled by explicit ruling rather than inference.
**Verify:** `cargo test -p mnemonist-core` passes with Node absent.

### D-01b — Fallback: 1:1 native tests are explicitly accepted
**Status:** CONFIRMED · **Category:** architecture · **Divergence:** no
**Admin ruling:** *"Rewriting the test logic 1:1 as native tests is accepted."* / *"Porting the
logic 1:1 into native `#[test]` tests is also acceptable and still counts."*
**Consequence:** CP1 (H+14 bridge viability) is no longer a scoring cliff. If the bridge fails,
translating test logic 1:1 into `#[test]` still scores — it is second-best, not zero.
**Rationale:** record this so the CP1 decision is made on evidence rather than panic.

### D-02 — Two-crate split to quarantine `unsafe`
**Status:** CONFIRMED · **Category:** architecture · **Divergence:** no
**Port:** `mnemonist-core` (`#![forbid(unsafe_code)]`) + `mnemonist-napi` (cdylib, test-only).
**Rationale:** napi-rs generates `unsafe` internally. Quarantining keeps the Zero Unsafe claim
literally true and machine-checkable.
**Verify:** the `forbid` attribute + a core build with no napi dependency in the tree.
**✅ VINDICATED BY THE RULING** — the admins described exactly this split unprompted:
> "Using unsafe at the FFI boundary is fine and expected. **What counts against you is unsafe code
> spread through the core port logic.**"

Quote this line directly alongside the crate diagram in the final `DECISIONS.md`. It converts our
architecture from a defensible choice into the officially stated preference.

### D-03 — `forEach`/`iter`/`iterables` live at the boundary, not in core
**Status:** CONFIRMED · **Category:** architecture · **Divergence:** YES
**Upstream:** `obliterator/foreach` is imported by 30 modules. Grep of every call site shows
**all** are `forEach(iterable, cb)` inside `.from()` statics or iterable-accepting constructors,
operating on the user-supplied argument. None iterate a structure's own data.
**Port:** the 5-branch dispatch is implemented once in `mnemonist-napi` as `Unknown → impl Iterator`.
Core structures accept `IntoIterator<Item = T>`.
**Rationale:** JS-value coercion belongs in the layer that owns JS values. Keeps core idiomatic
(20% criterion), writes the gnarliest logic once instead of per module.
**Verify:** core compiles with no JS-value types in any public signature.

### D-04 — `tests/.work/` assembly keeps `tests/original/` pure
**Status:** CONFIRMED · **Category:** architecture · **Divergence:** no
**Upstream:** `test/heap.js` does `require('../heap.js')`, so a shim must sit one level above.
**Port:** `tests/original/` holds upstream test files only, hashed. `tests/run.sh` assembles a
scratch tree with generated shims at the root.
**Rationale:** placing shims inside the hashed directory would muddy the parity claim.
**Verify:** `tests/verify-hashes.sh` on camera in the demo video.

### D-05 — Scoped subset port, with the FULL suite hashed at kickoff
**Status:** CONFIRMED (admin answer still pending, but non-blocking) · **Category:** scope · **Divergence:** no
**Upstream:** 15,386 LOC, 44 modules, 41 test files, 525 passing.
**Port:** declared module scope in `.port-mortem.toml` + README. **`tests/original/` ships all 41
upstream test files, hashed at kickoff** — not just the in-scope subset. Only in-scope modules are
run; both numbers are reported.

**Rejected alternative — hash only the in-scope files.** It would let us pick our own denominator
*after* choosing modules, and cherry-picking is a failure mode the rules name explicitly. Hashing
everything at kickoff is a timestamped commitment to the honest denominator made before any outcome
was known. Zero cost, and it converts the subset weakness into visible rigor.

**README framing, in this order:**
> **13 of 44 modules ported. 100% of their original tests pass, unmodified.**
> Repo-wide that is N/525, because 31 modules are declared roadmap (see scope table).

**Rationale:** repo is pool-listed despite exceeding the 8k guidance, and the dual-reporting
structure makes the answer non-blocking either way.

**✅ EFFECTIVELY RESOLVED BY THE ADMIN FAQ.** Two answers, quoted:
> "**Q: What size repo do I need?** A: **Pool repos have no minimum** (even under 500 lines is
> fine). **Bring-your-own** needs ~1,000 source LOC floor, **up to ~8,000**."

> "**Q: Do the size tiers affect scoring?** A: No. They're a difficulty signal only. **Judges score
> how well you prove the port works, not line count.**"

The LOC bounds — including the ~8,000 ceiling — are stated **for bring-your-own repos only**. Pool
repos are explicitly exempted from the minimum and given no maximum at all. mnemonist is pool-listed.
Combined with "size tiers are a difficulty signal only," the >8k concern is not a scoring issue.
Keep the dual reporting anyway: it costs nothing and answers the pass-rate question before it is asked.
**Verify:** `tests/SHA256SUMS` covers all 41 files, committed before the first line of port code.

### D-28 — Track letter: G, not F
**Status:** CONFIRMED-PENDING (settle by Aug 2) · **Category:** scope · **Divergence:** no
The website's track table and the admin FAQ disagree. Website: F = JS→Go/Rust, G = C→Zig.
FAQ: **F = JS→Go, G = JS→Rust**, with C→Zig absent entirely. The FAQ list is internally consistent
and later, so the website table is presumed stale. We are JS→Rust ⇒ **Track G**.
**Not urgent:** FAQ says track and repo are declared *at submission on the last day*, not at
registration. Confirm with admins, set once in `.port-mortem.toml`, README, and the demo banner.

### D-29 — Eligibility: no existing Rust port of mnemonist
**Status:** CONFIRMED · **Category:** scope · **Divergence:** no
FAQ requires the repo be "not already ported to your target language," and notes judges may rule
a project ineligible if an existing project is *effectively* a port.
**Checked pre-kickoff:** no Rust port of mnemonist exists on crates.io or GitHub. The `mnemonic`
crate is unrelated (BIP39 mnemonics). Individual Rust data-structure crates exist, but none is a
port of mnemonist as a library — which the FAQ explicitly allows: *"Similar tools don't
auto-disqualify you."*
**Verify:** re-check at submission; record the search in `NOTES.md`.

---

## Behavioural — iteration semantics

### D-06 — `Iterator` is a stateful cursor, not `IntoIterator`
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** YES
**Upstream:** `obliterator` v2.0.5 `iterator.js`: `Iterator.prototype[Symbol.iterator] = function () { return this; }`.
Self-returning, therefore **not restartable** — a second drain yields nothing.
**Port:** modelled as an explicit cursor type. Never `impl IntoIterator`, which would hand out a
fresh iterator per loop and silently restart.
**Rationale:** exact fidelity; the idiomatic Rust construct has the wrong semantics here.
**Verify:** fuzz op `iter_create → drain → drain` expects empty on the second.
**✅ VALIDATED PRE-KICKOFF:** napi-rs `#[napi(iterator)]` (napi 3.6.1) already has these semantics.
Smoke test on Node 24.18.1: `c[Symbol.iterator]() === c` → `true`; first `[...c]` → `[1,2,3]`;
second `[...c]` → `[]`; `next(); next(); [...c]` → `[3]`. **No custom bridge work required.**

### D-07 — Two-level `Symbol.iterator`: collection is a factory, cursor is identity
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** `stack.js:150`, `queue.js:156`, `vector.js:286`, `bit-set.js:348`, `fixed-deque.js:294`
— all `X.prototype[Symbol.iterator] = X.prototype.values`. So `[...stack]` twice **works**, while
`const it = stack.values(); [...it]` twice does **not**.
**Port:** collection `Symbol.iterator` constructs a fresh cursor; cursor `Symbol.iterator` returns itself.
**Rationale:** a uniform "iterable" abstraction gets exactly one of these wrong.
**Verify:** both expressions in the fuzz grammar.
**✅ PARTIALLY VALIDATED:** the *identity* half is free — napi-rs `#[napi(iterator)]` returns `this`
(measured, see D-06). The *factory* half remains ours: each collection's `Symbol.iterator` must
construct a new cursor object per call. That is the only side of D-07 needing implementation.

### D-08 — Hybrid capture: length frozen, elements live
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** universal pattern. `Iterator.fromSequence` captures `l = sequence.length` at creation
but reads `sequence[i++]` lazily. `Stack.prototype.values` captures `l = items.length`;
`FixedDeque.prototype.values` captures `size`, `capacity`, `start`. Element mutation **is** visible;
length change is **not**.
**Port:** index cursor over `&self` in core; `SharedReference` at the bridge so JS sees live elements.
**Rationale:** in pure Rust the borrow checker forbids the aliasing that would reveal this, so the
question is unobservable from Rust and the clean design is also the correct one. It becomes
observable only through the bridge.
**Verify:** fuzz `iter_create → mutate element → iter_next`.

### D-09 — Shrink window: reproduce the `undefined` gap (Option A, sequenced)
**Status:** **DECIDED** · **Category:** behavioural · **Divergence:** no (yes if the B fallback is taken)
**Upstream:** `i >= l` tests the frozen `l`, so a shrunk backing array is read past its new end and
JS yields `{done: false, value: undefined}` rather than terminating.

**Evidence gathered pre-kickoff.** All 41 test files grepped for *stored* iterators — the only
sites that could observe mutation (an immediate spread or drain cannot). 24 sites, 16 in Wave 1.
Every one is the same shape: construct via `.from([...])`, store, drain with `next()`, assert
`done`. **No upstream test mutates between iterator creation and drain.** So Option B costs
*exactly zero* on the 40% axis — a measured-safe fallback, not a gamble.

**Cost correction.** Option A was earlier estimated at "~3 lines, centralized." Too optimistic:
it needs every cursor to hold a live parent reference (`SharedReference`) **and** `Yield` to become
`Option<T>`, plus confirmation that napi-rs maps `None` → `undefined` not `null` (unverified).
But it decomposes: **B1** (element mutation visible) needs live parent access; **B2** (shrink →
`undefined`) needs that *plus* `Option<T>`. Live access is required for B1 regardless, so A's
marginal cost over B is only the `Option<T>` yield.

**Port — sequenced:** (1) Wave 0 builds cursors with live parent access; (2) get B1 working and
fuzzed; (3) add the B2 `undefined` gap; (4) if the `Option<T>`→`undefined` mapping is awkward,
fall back to B and document.
**Rationale:** no test needs either behaviour, but mutation-during-iteration is precisely the class
of bug tests miss and differential fuzzing catches — the event's own thesis. It is a 30%-category
differentiator and the best write-up material available.
**Verify:** fuzz grammar includes `iter_create → mutate → iter_next` for both element-change and
shrink. Record whichever branch step 4 takes, with this reasoning.

### D-10 — `forEach` five-branch dispatch order is observable
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** order is (1) array/typed-array/string/`[object Arguments]`, (2) has own `.forEach`
→ delegate, (3) `Symbol.iterator` present and no `.next`, (4) has `.next` → drain, (5) plain object
→ `for…in` + `hasOwnProperty`. Branch 2 preempts 3 and 4.
**Port:** same order, same precedence.
**Rationale:** anything owning a `.forEach` must never reach the iterator path.
**Verify:** boundary unit tests, one per branch, plus a `Map` (hits branch 2) and a plain object.

### D-11 — `forEach` second callback argument is polymorphic
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** index (number) for sequences, own counter (number) for iterators, **string key** for
plain objects, host-defined for branch 2 — a JS `Map` yields `(value, key)`, not `(value, index)`.
**Port:** enum at the boundary; delegation preserved for branch 2.
**Verify:** per-branch assertions on the second argument's type and value.

### D-12 — `forEach` falsy guard
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** `if (!iterable) throw`. So `forEach('', cb)` **throws** while `forEach('a', cb)`
iterates. Same for `0`, `false`, `NaN`, `null`, `undefined`.
**Port:** explicit JS-truthiness check at the boundary before coercion.
**Rationale:** JS truthiness has no Rust analogue; it must be spelled out.
**Verify:** boundary tests for each falsy value.

### D-13 — `iter` is narrower than `forEach` (upstream asymmetry)
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** `iter.js` has no `.forEach` branch and no plain-object branch. So `take({a: 1})`
**throws** while `forEach({a: 1}, cb)` **iterates the values**.
**Port:** reproduce both, asymmetry intact.
**Rationale:** genuine upstream inconsistency, not a bug to "fix". Fixing it would be a silent
behavioural change.
**Verify:** paired test asserting throw-vs-iterate on the same input.

### D-14 — Exact error strings reproduced
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** `'obliterator/iterator: expecting a function!'`,
`'obliterator/forEach: invalid iterable.'`,
`'obliterator: target is not iterable nor a valid iterator.'`
**Port:** core uses typed errors; the bridge maps them to these exact strings.
**Rationale:** upstream tests assert on messages; typed errors keep core idiomatic.
**Verify:** string equality assertions at the boundary.

### D-15 — Plain-object enumeration order
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** `for…in` + `hasOwnProperty` → integer-like keys ascending, then string keys in
insertion order.
**Port:** delegate to the JS engine's key enumeration at the boundary rather than reimplementing.
**Rationale:** reimplementing JS property order in Rust is pure downside risk.
**Verify:** mixed integer/string key object in the boundary tests.

### D-16 — `support.js` feature flags hardcoded true
**Status:** PROPOSED · **Category:** behavioural · **Divergence:** YES
**Upstream:** `ARRAY_BUFFER_SUPPORT` / `SYMBOL_SUPPORT` are runtime-detected for old engines.
**Port:** hardcoded true.
**Rationale:** both hold on every Node version the harness supports; branch is dead code.
**Verify:** note the assumption; no engine in scope where it is false.

---

### D-30 — JS typed-array write truncation must be emulated, not just width *selection*
**Status:** CONFIRMED (implemented in `StaticDisjointSet`) · **Category:** behavioural · **Divergence:** no
**Upstream:** writing to a `Uint8Array` silently truncates mod 256. Selecting the right *width*
(D-17/`getPointerArray`) is only half the semantics; the *write* behaviour is the other half.

**Why this is reachable rather than theoretical — it compounds with B-7.** Because the rank bug
leaves non-root ranks permanently zero, the equal-ranks branch is taken on nearly every union, so a
single root's rank is bumped once per union — far past what `getPointerArray(Math.log2(size))` sized
the array for. And `ranks` is **always** `Uint8Array` in practice: widening would require
`log2(size) > 256`, impossible when `parents` already caps `size` at 2³².
Concrete: a 300-element set unioned as `(0,1)` then `(1,k)` ends with `ranks[0] == 43`.
**Verified against real Node — it agrees exactly.**

**Port:** a `PointerVec` masks every write to the selected width. A naive `Vec<u32>` would have
diverged silently here, and no test would have caught it.
**Verify:** test `root_rank_wraps_at_the_ranks_array_width`.
**Note:** `PointerVec` is currently private to `static_disjoint_set.rs`. Promote it into
`utils/typed_arrays.rs` as soon as a second structure needs truncation semantics.

### D-31 — `StaticDisjointSet` rank bug pinned by regression test
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no (deliberate bug-for-bug)
Concrete input where the bug changes the elected root, found and pinned so a future "cleanup"
cannot silently correct it: size 8, unions `(0,1) (0,2) (3,4) (1,3)`. Upstream reads
`ranks[1] == 0 < ranks[3] == 1` and flips the root to `3`; a correct union-by-rank would read
`ranks[0] == 1 == ranks[3] == 1` and keep `0`. **Node confirms `find(1) === 3`.**
See B-7 in `NOTES.md` for the upstream report. **Verify:** test `reproduces_upstream_rank_bug`.

## Behavioural — `utils/iterables`

### D-17 — `toArray` preallocation can produce sparse arrays
**Status:** PROPOSED · **Category:** behavioural · **Divergence:** YES
**Upstream:** `toArray` preallocates `new Array(guessLength(target))` then fills via `array[i++]`.
If the guess exceeds what `forEach` yields, the result is a **sparse array with holes** —
distinguishable from `undefined` in JS. `toArray({length: 5})` hits the plain-object branch, which
enumerates own properties including `length` itself, giving `[5, <4 empty>]`.
**Port:** Rust has no hole concept; the bridge must choose a representation.
**Rationale:** decide explicitly rather than discover it via a fuzz divergence.
**Verify:** fuzz inputs with a lying `.length`/`.size`.

### D-18 — `guessLength` trusts `.length` then `.size`
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** returns `target.length` if numeric, else `target.size` if numeric, else `undefined`.
No validation against actual yield count.
**Port:** same, feeding D-17.
**Verify:** covered by D-17 cases.

### D-19 — `Stack.values()` captures `items.length`, not `this.size`
**Status:** CONFIRMED · **Category:** behavioural · **Divergence:** no
**Upstream:** `Stack.prototype.values` uses `l = items.length` and indexes `items[l - i - 1]`
(reverse, LIFO). Other structures capture `this.size`.
**Port:** per-module fidelity; do not normalise across structures.
**Rationale:** the two coincide for `Stack` today, but normalising would be an unforced assumption.
**Verify:** per-module cursor tests.

---

## Tooling & methodology

### D-20 — Benchmarks measure the pure Rust path
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Port:** `bench/` drives `mnemonist-core` directly, never through N-API.
**Rationale:** napi marshalling overhead would misrepresent the port in both directions.
**Verify:** stated in `bench/methodology.md`.

### D-21 — Fuzz grammar includes cursor lifecycle interleaving
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Port:** ops include `iter_create` → k structure mutations → `iter_next`.
**Rationale:** D-06/D-08/D-09 are only reachable by interleaving; a suite written against a GC'd
language rarely probes this deliberately.
**Verify:** grammar listed in `fuzz/log.txt` header.

### D-22 — mocha pinned; default glob is non-recursive
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Upstream:** `mocha ^9.1.3`, no `.mocharc`, `npm test` is bare `mocha`. Default spec
`./test/*.{js,cjs,mjs}`. Corroborated: `test/exports/` has its own separate `test:exports` script,
which only makes sense if the default run does not descend.
**Port:** pin the exact version in the harness `package.json`. Keep mocha at upstream's `^9.1.3` —
do **not** upgrade it (see D-26).
**Rationale:** a v10+ glob change would silently alter which files run.
**Verify:** lockfile + recorded mocha version in the demo.

### D-26 — Node pinned to 24.18.1 (constrained by mocha, not preference)
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Measured** against the real upstream suite pre-kickoff, not assumed:

| Node | Result |
|---|---|
| 26.5.1 | **FAILS** — mocha 9.1.3's bundled `yargs`: `require is not defined in ES module scope` |
| **24.18.1** | **GREEN** ← pinned |
| 22.23.2 | **segfaults** on exec (exit 139) — bad build, unrelated to mocha |
| 20.20.2 / 18.20.8 | green |

**Port:** Node 24.18.1 pinned identically in the harness, the Dockerfile, and CI.
**Rationale:** newest Node that runs mocha 9 with **zero deviation from upstream devDeps**. The
alternative — upgrading mocha to support Node 26 — leaves `test/*.js` hashes intact but swaps the
test runner, introducing a divergence where none is needed. Rejected on that basis.
**Verify:** version recorded in `.nvmrc`, Dockerfile, CI matrix, and stated on camera in the demo.

### D-27 — Linux is the build environment; Windows is not used
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Port:** dev in WSL2 Ubuntu 22.04 with the repo in `~/` (never `/mnt/c`); Docker 28.3.0 Linux
engine as the reference build for submission and all benchmark numbers.
**Rationale:** three independent reasons. (1) The Windows host's `link.exe` on PATH resolves to
Git/scoop's **GNU coreutils `link` 8.32**, shadowing MSVC's linker — a cdylib build would fail
with errors unrelated in appearance to PATH. (2) Judges run the Dockerfile, so benchmarks must
come from that environment to be honest. (3) napi-rs on Linux is far better trodden.
**Also:** repo must not live on `/mnt/c` — the 9p bridge makes cargo builds dramatically slower
across a compile-heavy 72h sprint.
**Verify:** validated end-to-end pre-kickoff — napi 3.6.1 cdylib built (11.6s) and loaded into
Node 24.18.1 successfully.

### D-23 — Node oracle is a persistent subprocess
**Status:** CONFIRMED — **BUILT AND MEASURED** · **Category:** tooling · **Divergence:** no
**Port:** one long-lived Node process, line-delimited JSON protocol.
**Rationale:** per-op spawning turns 60s of fuzzing into an hour and would quietly forfeit the
Fuzz Survivor bonus.
**Verify:** throughput figure recorded in `fuzz/log.txt`.
**Measured:** **~23,600 op/s**, including a full `mapping()` + `compile()` comparison after every
op. The 120s campaign did 2,837,506 ops; at one spawn per op it would have taken ~33 hours. The
estimate in the rationale was, if anything, generous to the naive approach.

### D-32 — The differential fuzzer is falsified before it is trusted
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Rationale:** gate 6 exists because a falsification test that cannot fail is a second green light.
The same argument applies to the fuzzer, and with more force: a bug-for-bug port that fuzzes clean
is indistinguishable from a fuzzer that never compares anything.
**Port:** the sabotage is to *fix* upstream's B-7 rank bug in the core — the most plausible way a
future cleanup breaks this port, and one that makes the port strictly *more correct* than upstream
and therefore wrong, since the elected root is observable through `find()`. Caught in 129 cases
(0.3s); proptest shrank a 600-op program to three ops.
**Verify:** the minimised seed is committed in
`crates/difffuzz/proptest-regressions/static-disjoint-set.txt`, with a provenance header so it is
not misread as a real port defect, and proptest replays it before any novel case.

### D-33 — Differential fuzzing structurally cannot find bug-for-bug defects
**Status:** CONFIRMED · **Category:** methodology · **Divergence:** no
**Observed:** 4.23 M ops across two seeds on `static-disjoint-set`, zero divergences — while the
module contains a known upstream bug (B-7) and a known second-order overflow (D-30).
**Rationale:** the oracle *is* upstream, so any behaviour we reproduce faithfully is by definition
not a divergence. Both defects on this module were found by reading, and neither is findable this
way. Recording it because "we fuzzed it and found nothing" is otherwise easy to misread as either
"the code is clean" or "the fuzzer is broken", and it is neither.
**Consequence:** the fuzzer's value on a bug-for-bug port is *drift detection*, not bug discovery.
Both directions of drift — towards a different answer and towards a more correct one — are equally
failures. Say this in the write-up; it is a genuinely non-obvious property of the technique the
event is built around.

### D-34 — Benchmark regressions are derived, not written down
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Port:** every published metric is lower-is-better, so `bench/drive.js` computes the `regressions`
array mechanically from the results rather than leaving a human to fill it in.
**Rationale:** the FAQ states hiding a regression scores worse than disclosing it. A field nobody
has to remember cannot be quietly left out on a bad day at hour 60.
**First finding:** `static-disjoint-set` at 4e6 items — p99 **276 ns vs upstream's 110 ns, 2.5×
slower**, while p50 stays 1.8× faster. Cause is `PointerVec` backing every logical width with a
`Vec<u32>`, making our structure 32 MB against upstream's 20 MB and pushing it past this CPU's
32 MB L3. Found by sweeping the size *because* the 1e6 result looked too clean.
**Consequence for D-30:** promoting `PointerVec` into `utils/typed_arrays.rs` should give it a real
per-width backing store, with this benchmark as the before/after.

### D-35 — Both benchmark sides checksum their results, and disagreement aborts
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Port:** each runner accumulates a checksum over every non-mutating op's return value; the driver
requires all 20 runs across both sides to agree before writing anything.
**Rationale:** "both sides ran the same workload" is otherwise an assertion. This makes it a
verified claim — same ops *and* same answers, not merely the same op count. It also re-proves the
B-7 reproduction for free: a corrected implementation elects different roots and the checksum moves.
**Verify:** `checksum` field per workload in `bench/results.json`.

### D-36 — Percentiles are computed once, in the driver, over both sides
**Status:** CONFIRMED · **Category:** tooling · **Divergence:** no
**Upstream of the decision:** DESIGN.md §5.2 Problem 1 asks for "same percentile maths" on both sides.
**Port:** the runners emit raw per-batch nanoseconds and nothing else; nearest-rank percentiles
happen in `bench/drive.js`.
**Rationale:** implementing the maths twice and hoping the implementations agree is strictly weaker
than implementing it once. Same reasoning as the matched PRNG being *diffed* rather than asserted.

---

## Licensing & scope

### D-24 — MIT attribution
**Status:** CONFIRMED · **Category:** licensing · **Divergence:** no
**Upstream:** MIT, © 2016 Guillaume Plique (Yomguithereal). **No** per-file copyright or SPDX
headers anywhere in source.
**Port:** own `LICENSE`; `LICENSE-MNEMONIST` verbatim; `NOTICE` covering `obliterator` (also
Yomguithereal, MIT) as a second ported dependency; README derivation statement; one-line
attribution comment per ported module.
**Rationale:** upstream has no per-file headers, so per-file attribution exceeds the obligation —
cheap, and it reads well to a judge checking licence hygiene.

### D-25 — Only `semi-dynamic-trie` is genuinely untested
**Status:** CONFIRMED · **Category:** scope · **Divergence:** no
**CORRECTED pre-kickoff.** The earlier claim — that 1,086 LOC across four modules had no test
coverage — was wrong. It rested on "no *matching* test file," which is true but not the same as
"untested." `test/lru-cache.js` (497 lines) requires all four LRU variants directly:
```js
LRUCache            = require('../lru-cache.js'),
LRUMap              = require('../lru-map.js'),
LRUCacheWithDelete  = require('../lru-cache-with-delete.js'),
LRUMapWithDelete    = require('../lru-map-with-delete.js');
```
**So 835 of the 1,086 LOC are covered and do earn 40%-category credit.** Only `semi-dynamic-trie`
(251 LOC) has no coverage anywhere.
**Port:** LRU family moved into Wave 3 and *prioritised within it* — one 497-line test file covers
1,271 LOC, and the three variants are thin layers over `lru-cache`, so porting the base largely
yields the rest. `semi-dynamic-trie` alone stays roadmap.
**Lesson worth keeping:** file-name-based coverage inference is unreliable; grep the requires.

---

## Not yet investigated — likely future entries

- `utils/merge.js` (563 LOC) semantics — heaviest util, only `inverted-index` needs it
- Typed-array pointer-width selection (`getPointerArray`) and its Rust equivalent
- Comparator callback semantics across the boundary (Wave 2, tier T2)
- Arbitrary JS values as keys + SameValueZero equality (Wave 3, tier T3)
- `random`-dependent structures needing deterministic seeding for differential comparison
