# NOTES.md — running capture log

**Purpose:** raw material for the post-event write-up (**Write-Up Side Quest: $100 × 3, deadline
Aug 10 — a full week after code freeze**, judged on insight not followers). Not prose. Append
freely, fix nothing, delete nothing.

**The leverage:** the write-up costs zero hackathon hours *only if* the material is captured while
it happens. Nobody reconstructs a surprise at hour 70.

---

## Capture checklist — grab these DURING the event

- [ ] Terminal output of the **first** green upstream test run against the Rust port (screenshot + text)
- [ ] `SHA256SUMS` verification output
- [ ] **Every fuzz divergence**: the raw failure, the proptest-minimised repro, and the fix
- [ ] `proptest-regressions/` contents as they accumulate
- [ ] Wall-clock when each module lands (feeds a "what 72h actually looks like" chart)
- [ ] Benchmark numbers, including any **regressions** (honest ones are more interesting)
- [ ] Every moment of genuine surprise — those *are* the article
- [ ] Dead ends and what they cost in hours
- [ ] Anything an LLM got confidently wrong about the port (relevant to the event's AI framing)

---

## Bug candidates (upstream)

Must be filed upstream **during the event** to count for **Bug Catcher (+3, $100)**.
Status: `unverified` → `verified` → `filed #NNN` / `intentional`.

### B-1 — `iter`/`forEach` asymmetry on plain objects
`status: unverified` · `obliterator v2.0.5`
`take({a: 1})` **throws** (`iter.js` has no plain-object branch) while `forEach({a: 1}, cb)`
**iterates the values** (branch 5, `for…in`). Two helpers in the same library disagree about
whether a plain object is iterable.
**Likely intentional?** Possibly — `iter` must return an *iterator*, `forEach` only needs to visit.
Worth asking upstream regardless; even "intentional" is a documentation gap.

### B-2 — `toArray` produces sparse arrays when `guessLength` lies
`status: unverified` · `mnemonist utils/iterables.js`
`toArray` preallocates `new Array(guessLength(target))` then fills with `array[i++] = value`.
`guessLength` trusts `.length` then `.size` without validating against actual yield count.
Result on mismatch: **a sparse array with holes**, distinguishable from `undefined` in JS.
Sharpest case: `toArray({length: 5})` → `forEach` plain-object branch enumerates own properties
**including `length` itself** → `[5, <4 empty items>]`.
**This is the strongest candidate.** Concrete, reproducible in isolation, clearly unintended.

### B-3 — `take` with omitted `n`
`status: unverified` · `obliterator take.js`
`l = arguments.length > 1 ? n : Infinity`, then on early exhaustion `if (i !== n) array.length = i`.
With `n` omitted, `n === undefined`, so `i !== n` is **always** true. Benign today (no-op on a
growing array) but the guard doesn't express what it appears to intend.
**Low severity** — code-smell tier, not a behaviour bug. File only if others land.

### B-4 — `forEach` falsy guard rejects empty string and zero
`status: unverified` · `obliterator foreach.js`
`if (!iterable) throw`. So `forEach('', cb)` **throws** while `forEach('a', cb)` iterates.
An empty string is a legitimately iterable value that should yield zero times. Same for `0`
and `false` reaching a numeric path.
**Arguably intentional** as an input guard, but the empty-string case looks like a genuine miss.

### B-5 — `toString()` called on arbitrary input during dispatch
`status: unverified` · `obliterator foreach.js`
Branch 1 tests `iterable.toString() === '[object Arguments]'`. This **invokes `toString` on an
arbitrary user value** during type dispatch — a custom `toString` can throw, or return that exact
string and hijack the branch.
**Adversarially interesting**; low real-world impact.

### B-7 — `StaticDisjointSet.union` compares ranks of the ITEMS, not the ROOTS
`status: unverified — strong candidate` · `mnemonist static-disjoint-set.js`
Union-by-rank requires comparing the ranks of the two **roots**. Upstream reads the ranks of the
original arguments and then writes to the root:
```js
var xRoot = this.find(x), yRoot = this.find(y);
var xRank = this.ranks[x],       // <-- x, not xRoot
    yRank = this.ranks[y];       // <-- y, not yRoot
if (xRank < yRank)      this.parents[xRoot] = yRoot;
else if (xRank > yRank) this.parents[yRoot] = xRoot;
else { this.parents[yRoot] = xRoot; this.ranks[xRoot]++; }   // <-- writes xRoot
```
Reading at `x`/`y` but writing at `xRoot` is internally inconsistent, and non-root ranks are never
updated — so they stay 0 forever and the `else` branch fires almost always. The rank heuristic is
effectively disabled, degrading `find()` toward O(n) in the worst case.

**Results stay correct** — union-find is correct regardless of which tree is attached to which — so
this is a *performance* bug, not a correctness one. Note the consequence for us: **differential
fuzzing will never catch it**, because a faithful port reproduces it exactly. Found by reading.

**We reproduce it, we do not fix it.** `find(x)` returns a root, and which element becomes root is
observable; "fixing" it would be a silent behavioural divergence. Goes in `DECISIONS.md` as a
deliberate bug-for-bug reproduction, and upstream as an issue.
*Also a good write-up beat: the class of bug that differential testing structurally cannot find.*

### B-8 — `SparseSet.add(m)` with `m >= length` corrupts the set, three defects deep
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-set.js`
Neither guard in `add` fires for an out-of-range member, because `sparse[m]` is `undefined` and
every comparison against `undefined` is false. Three separate silent failures then follow in three
consecutive lines:
```js
this.dense[this.size] = member;    // (1) TRUNCATES — add(300) on a length-10 set stores 44
this.sparse[member]   = this.size; // (2) DROPPED — out-of-range typed-array store is a no-op
this.size++;                       // (3) happens anyway
```
Measured: `new SparseSet(10); add(300)` → `size === 1`, `dense === [44, 0, …]`, `sparse` untouched,
`has(300) === has(44) === false`. The member is stored, counted, iterable and unfindable under
either name.
**We reproduce it.** Unlike `StaticDisjointSet`'s out-of-range read, every step here is a
well-defined read, truncating store or dropped store, so the faithful port is expressible — and it
is cheaper than a guard as well as more useful. See `docs/modules/sparse-set.md`.

### B-9 — and therefore `size` can exceed `length`, so upstream's own iterator yields `undefined`
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-set.js`
The second-order consequence of B-8(3), and the more interesting half. `values()` freezes `size`
and `dense` is a fixed-length typed array, so:
```js
var u = new SparseSet(2);
u.add(100); u.add(101); u.add(102); u.add(103);   // size 4, length 2
[...u]  // → [100, 101, undefined, undefined]
```
**This is DESIGN.md §3.7's shrink window, reached through the public API in four calls** — two, on
a zero-length set. §3.7 chose Option A on the grounds that no upstream *test* reaches the window,
which measured Option B as costing zero on the 40% axis. That remains true and the conclusion was
right, but for a stronger reason than recorded: the window is not exotic, and the differential
fuzzer finds it in 0.3s when the port takes Option B.

### B-10 — `SparseSet.delete` past capacity writes a string-keyed expando onto a typed array
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-set.js`
```js
index = this.dense[this.size - 1];        // undefined once size > length
this.dense[this.sparse[member]] = index;  // LANDS, as 0 — a NaN element store is 0
this.sparse[index]              = ...;    // does NOT land — sparse[undefined] is a PROPERTY
```
`new SparseSet(3)`, add `0/1/2/99`, `delete(1)` → `dense = [0, 0, 2]`, `sparse = [0, 1, 2]`,
`sparse.undefined = 1`.
**This one caught the port.** The first cut wrote `sparse[0]`, which is what reading the three
lines as a unit produces rather than statement by statement. Fixed, pinned by a test, and then used
as fuzzer falsification sabotage A — caught in 6.6s and shrunk to seven ops.
*Lesson worth keeping: for a bug-for-bug port, read JS one statement at a time and confirm each
against the runtime. Reading for intent is how you write the correct version by accident.*

### B-30 — `forEach` on a truthy primitive dies in the `in` operator, not in its own guard
`status: VERIFIED against Node 24.18.1` · `obliterator v2.0.4/2.0.5 foreach.js`
A truthy primitive — a number, a boolean, a symbol, a bigint — survives `if (!iterable) throw`,
is not an indexed sequence, and has no `.forEach`. It then reaches

```js
if (SYMBOL_SUPPORT && Symbol.iterator in iterable && ...)
```

and `in` **requires an object**. So:

```js
forEach(5, cb)  // TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in 5
```

Measured on Node 24.18.1 for `5`, `true`, `10n` (which stringifies as `10`, not `10n`) and
`Symbol(x)`. The error a caller sees names V8's operator rather than the library that called it,
and the library's *own* guard for "this is not iterable" never fires — a caller who passes a
number gets a message that reads like a bug in obliterator's internals.

**Related, same file, same dispatch:** `forEach(Object.create(null), cb)` throws
`TypeError: iterable.toString is not a function`, because branch 1 calls `toString()` unguarded
on an arbitrary value (that is B-5, with a second symptom). Both are reproduced verbatim by the
port and pinned in `tests/boundary/foreach.js`.

**Severity: low, but it is a genuine gap in a guard that exists.** Two lines would close it
(`typeof iterable !== 'object' && typeof iterable !== 'function'` before the `in`), and a
one-line doc note would close it as documentation. Worth filing alongside B-4, which is the same
guard's other blind spot.

### B-6 — `Stack.values()` captures `items.length`, not `this.size`
`status: unverified` · `mnemonist stack.js`
Other structures capture `this.size`. These coincide for `Stack` today; the inconsistency is latent
rather than active.
**Probably not a bug** — log it, don't file it.

> Differential fuzzing has not run yet. Expect the best candidates to come from there, not from
> reading. Add them here with the minimised repro attached.

---

## Log

### Pre-kickoff — 2026-07-31

**Repo/track selection.** **Track G** (JS→Rust), solo. *Note: the website's track table says F is
JS→Go/Rust and G is C→Zig; the later admin FAQ says F is JS→Go and G is JS→Rust, dropping C→Zig
entirely. Planned as F for most of the day, corrected to G once the FAQ landed. Track is declared
at submission, not registration, so the cost was zero — but a good reminder that the "official"
page is not always the current source of truth.* The decisive criterion turned out not to be LOC
or difficulty but **test-corpus portability** — whether the original suite can run against a port
at all. That reframing eliminated most of the pool immediately.

**mnemonist is 15,386 LOC** (shipped source; my first `-maxdepth 2` sweep said 15,841 and swept in
`experiments/`/`docs/` — the correction was only ~625 lines, the size problem was real).
Chosen anyway because it is **perfectly modular**: 1 file per structure, 1:1 matching `test/<name>.js`,
tests importing by direct relative path, no `.mocharc`. A scoped subset port is therefore clean —
rare at this size.

**Four files gate 90% of the repo.** `obliterator/foreach` (22 dependents), `obliterator/iterator`
(18), `utils/typed-arrays` (14), `utils/iterables` (12). The dependency graph, not the LOC table,
determined the wave order.

**I mischaracterised `test/_utils.js` as a helper.** It is a real test file — 389 lines, 20
`describe` blocks, covering five utils modules. Consequence: Wave 0's utils work earns direct
test credit instead of being pure infrastructure. *Worth including in the write-up as an example
of how a wrong early assumption quietly reshapes a plan.*

**obliterator turned out to be the whole story.** Reading the source rather than inferring:
- `Iterator.prototype[Symbol.iterator] = function () { return this; }` — **self-returning, not
  restartable.** Idiomatic Rust `IntoIterator` has the wrong semantics.
- `fromSequence` is **hybrid**: length captured at creation, elements read lazily. Not snapshot,
  not live — the weirder of the two possible answers, and the same pattern recurs in every
  structure's own `values()` (`Stack`, `FixedDeque` confirmed).
- `forEach` has **5-branch dispatch where the callback's second argument changes type per branch** —
  number for sequences/iterators, *string key* for plain objects, host-defined for anything owning
  its own `.forEach` (a JS `Map` yields `(value, key)`).

**The grep that changed the architecture.** All ~26 `forEach` call sites across all 30 importing
modules are `forEach(iterable, cb)` inside `.from()` statics or iterable-accepting constructors,
on the **user-supplied argument** — never on a structure's own data. So `forEach` is a *boundary*
function. It moved out of the core entirely into the napi crate: more idiomatic, less work, and
correctly located. *A good write-up beat: one grep collapsing the ugliest item on the critical path.*

**Two-level `Symbol.iterator`.** Collection-level is a **factory** (`[...stack]` twice works);
iterator-level is **identity** (`const it = stack.values(); [...it]` twice does not). One level
apart, opposite semantics. A uniform "iterable" abstraction gets exactly one wrong.

**napi-rs already has the right semantics — measured, not assumed.** Smoke crate, napi 3.6.1,
Node 24.18.1: `c[Symbol.iterator]() === c` → `true`; first `[...c]` → `[1,2,3]`; second → `[]`;
`next(); next(); [...c]` → `[3]`. D-06 and half of D-07 need **no custom work**.
*Good beat: the FFI layer handed over the exact JS semantics that idiomatic Rust would have broken.*

**Node 26 breaks the test suite before a line of port code exists.** 26.5.1 runs fine standalone,
but mocha 9.1.3's bundled `yargs` dies: `require is not defined in ES module scope` (Node 26
ESM/CJS interop). **Node 24.18.1 is the newest that runs the upstream suite with zero deviation
from upstream devDeps.** Also: **22.23.2 segfaults on exec (exit 139)** — bad build, unrelated.
*Strong beat: "the newest runtime is not a neutral choice when your proof depends on a 2021 runner."*

**Environment archaeology.** Windows `link.exe` on PATH resolves to Git/scoop's **GNU coreutils
`link` 8.32**, shadowing MSVC's linker despite VS 2022 being installed — a cdylib build would fail
with errors that look nothing like PATH. Sidestepped by making Linux primary. Also: `rustup update
stable` hit a component conflict and left WSL's toolchain without `rustc`/`cargo` (clean reinstall
fixed it), and starting Docker Desktop put WSL into `getpwuid` failures → `E_UNEXPECTED`
(`wsl --shutdown` fixed it). **~90 minutes of pre-kickoff environment work that would otherwise
have been hour-3 hackathon work.**

**Upstream baseline:** `525 passing · 1 pending · 0 failing · 90ms` on Node 24.18.1, clean clone.
`npm install` clean, 165 packages, no native-build failures.

**Admin ruling (verbatim, see DESIGN.md header).** FFI bridge ratified; `tests/original/` +
kickoff SHA-256 named explicitly; *"unsafe at the FFI boundary is fine and expected — what counts
against you is unsafe code spread through the core port logic"* — which is exactly the crate split
we had already chosen. 1:1 native tests accepted as a fallback. **Tests now optional for
qualification**, but still the strongest proof.
*Still unanswered: repo size / scoped subset.*

---

**Second correction of the day: "no matching test file" ≠ "untested."** I had written off 1,086 LOC
across four LRU modules because none had a `test/<name>.js`. `test/lru-cache.js` requires all four
directly — 835 of those LOC are covered and scoreable. Combined with the `test/_utils.js` mistake
earlier, that is **two wrong coverage inferences in one day, both from reasoning about filenames
instead of grepping requires.** *Good write-up beat: the cheapest possible check kept beating my
structural intuition.*

**Evidence-driven resolution of the shrink-window question.** Rather than guess whether to
reproduce JS's iterator-invalidation behaviour, grepped all 41 test files for *stored* iterators —
the only sites that can observe mutation. 24 sites; every one constructs, stores, drains, asserts
`done`, with **no mutation in between**. So the cheap fallback was measured to cost nothing, which
turned a risky choice into a safe one. *Beat: how to make an architecture decision with a grep
instead of an argument.*

**Denominator honesty.** Decided to hash **all 41** upstream test files at kickoff rather than only
the in-scope subset, then report both numbers. Hashing only what we plan to run would mean choosing
our own denominator after picking modules. The timestamped full hash is proof we committed before
knowing outcomes. *Beat: subset ports have an integrity problem nobody talks about, and it has a
one-command fix.*

---

### H+2 — first real module ported (`StaticDisjointSet`)

**The rank bug has a second-order consequence that nearly bit us.** B-7 leaves non-root ranks
permanently zero, so the equal-ranks branch fires on almost every union, so one root's rank climbs
once per union — far past the `log2(size)` the array was sized for. And `ranks` is *always* a
`Uint8Array` in practice. So it **wraps**: a 300-element set ends with `ranks[0] == 43`. Node agrees
exactly. A naive `Vec<u32>` port diverges silently and no test catches it, because upstream's own
suite never builds a set that large.

*Two bugs compounding — one upstream logic error making an otherwise-unreachable overflow reachable
— is the best single argument for differential testing we have so far. Neither is visible from
reading one file.*

**Validated every case against real Node rather than reasoning about it.** All 10 scenarios matched.
That is now the working method: when a JS semantic is in question, run it, don't argue about it.

**Pinned the rank bug with a regression test** on a concrete input where it changes the elected root
(size 8; unions `(0,1) (0,2) (3,4) (1,3)` → upstream elects `3`, correct union-by-rank elects `0`).
A future "cleanup" now fails the suite instead of silently diverging.

**Process note:** the same apostrophe-quoting trap that broke `&'static str` earlier also truncated
a commit message (`find()'s` closed the outer `bash -lc '...'`). Dodged in Rust source by staging
files, forgotten for the commit body. *Small recurring tax of driving a WSL repo from a Windows
shell — the reason we moved the session into WSL.*

### H+5 — harnesses built, `StaticDisjointSet` backfilled to full DoD

**The fuzzer found nothing, and that is the interesting result.** 4.23 M operations across two
seeds, zero divergences. Expected, and worth saying out loud: **a faithful port reproduces
upstream's bugs, so differential fuzzing structurally cannot find them.** B-7 was found by reading.
What the fuzzer is actually for on a bug-for-bug port is the *opposite* direction — catching the
port drifting away from upstream, **including drifting towards correctness**.

**So the fuzzer was falsified by "fixing" B-7.** Gate 6's lesson applies to the fuzzer itself: one
that has never been observed to catch anything is a second green light, not a check. Changing
`ranks[x]` to `ranks[x_root]` in the core — the single most plausible way a future cleanup breaks
this port — was caught in **129 cases, 0.3 s**, and proptest shrank a 600-op program to three ops:
`new(23); union(10,7); union(11,7); find(10)` → upstream 11, "corrected" port 10. That seed is now
committed as a regression guard with a provenance header, because an unlabelled `cc` line in
`proptest-regressions/` would read as a real defect that was found and fixed.
*Write-up beat: the most valuable thing my differential fuzzer caught was my own code being too
correct.*

**proptest's default `max_shrink_iters` is tuned for small values and quietly gives up.** At the
default it stopped at a "minimal" 29-op program that was mostly noise, with a warning easy to miss
in the scroll. Raised to 2²², the same failure minimises to 3 meaningful ops. **The shrink budget
is the difference between a repro you can file upstream and a wall of text.**

**The benchmark's first result was too good, which was the tell.** Port won every metric on the
1e6 workload — p50, p99, RSS, startup. Against a library that is already typed-array-backed, that
should not happen, and §5.1 says so explicitly. Swept the size (200 → 5k → 65k → 1e6 → 4e6) looking
for the boundary and found it at **4e6: p99 275 ns vs 102 ns, the port 2.7× SLOWER**, while p50
stays 1.7× faster.

Cause is our own design, not the workload: `PointerVec` backs *every* logical width with a
`Vec<u32>`, so our `ranks` is 4× upstream's `Uint8Array`. At 4e6 that is 32 MB of structure vs
20 MB — exactly the 32 MB L3 boundary on this 7600X. **The port wins the median and loses the
tail, which is the inverse of the usual Rust-vs-V8 story**, and it only shows up because §5.2's
batch-level p99 exists to show it.
*Beat: "I went looking for the workload where my port loses, and the honest answer changed the
headline from 'faster' to 'faster at the median, worse at the tail'."*

**Two harness decisions that turned out to matter more than expected:**

1. **Both bench sides emit a checksum over every non-mutating op's result, and the driver refuses
   to write results unless all 20 runs agree.** Intended as cheap paranoia; it turns "same
   workload" from an assertion into a verified claim — same ops *and* same answers — and it
   incidentally re-proves the rank bug is reproduced, since a corrected port would elect different
   roots and move the checksum.
2. **Percentiles are computed once in the driver over both sides**, not twice in the two runners.
   §5.2 asks for "same percentile maths"; implementing it twice and hoping is strictly weaker.

**Cost of the persistent oracle, quantified at last:** ~23,600 op/s *including* a full
`mapping()` + `compile()` comparison after every op. At one `node` spawn per op the 120 s campaign
would have taken ~33 hours. D-23 paid for itself on the first module.

**Small trap:** JS bitwise operators produce *signed* 32-bit results, so the xorshift32 twin needs
`>>> 0` or the two streams part company within a handful of draws — silently producing two
different benchmarks. `--dump-prng` + `diff` catches it in one second; reasoning about it would
have taken longer and been less convincing.

### H+5 — the RSS lesson: a fix that worked for a reason we got wrong

**The prediction.** `PointerVec` backed every logical width with `Vec<u32>`, so at 4e6 items our
`ranks` was 16 MB where upstream's `Uint8Array` is 4 MB — 32 MB of structure against upstream's
20 MB, straddling this CPU's 32 MB L3. That was offered as the cause of a 2.7× p99 tail regression,
with a confident mechanism attached.

**The fix worked, emphatically.** Per-width backing store → p99 at `mixed-4e6` went
**275.0 → 43.6 ns/op** against upstream's 134.9. A 2.7× loss became a 3.1× win.

**The mechanism was wrong, and one number proved it.** If footprint were the cause, resident memory
should have dropped ~12 MB. `structure_rss_delta_mb` moved **12.8 → 13.0**. Nothing.

**Why.** `ranks` is `vec![0; n]`, and because of the rank bug (B-7) almost every entry is *never
written* — only roots are ever bumped. Linux does not fault in untouched zero pages, so the extra
12 MB was **never resident and never appeared in RSS in the first place**. We reasoned confidently
about memory that did not exist.

**Two generalisable lessons:**
1. **RSS measures resident, not allocated.** For zero-initialised or sparsely-written structures
   the two diverge without limit. Allocating 16 MB and touching 4 KB of it costs 4 KB of RSS.
   Any argument of the form "we allocate more, therefore we are slower" needs a residency check
   before it is believed.
2. **Check a causal story against a metric that would falsify it.** The fix and the explanation
   were bundled together, and only splitting them — "if footprint is the cause, RSS must drop" —
   exposed that one was right and the other wasn't. A correct prediction of *outcome* is not
   evidence for the predicted *mechanism*.

Current best hypothesis is address-space stride rather than resident size: at `u32` the same
indices span 4× the pages (4096 vs 1024 at 4 KB), and TLB pressure lands in the tail rather than
the median. **Unconfirmed** — needs `perf stat -e dTLB-load-misses` on both revisions. Recorded as
a hypothesis, not a finding.

**Bonus lesson from the same episode: benchmark noise is larger than it looks.** A run taken while
the machine was saturated inflated *both* sides 2–3×, and upstream's own p99 swung **102 → 135**
between two clean runs on the same host. Absolute ns/op are not comparable across runs; only the
within-run A/B comparison is sound. §5.2's interleaving requirement is what made the conclusion
survive a bad measurement rather than being poisoned by it — and the honest reporting rule is that
small ratios read as "roughly 2×", never as three significant figures.

*Write-up beats: "the fix worked and my explanation didn't" is a better story than a clean win, and
"RSS measures resident, not allocated" is the kind of thing people rediscover painfully.*

### H+5 — `cargo build` does not compile your tests

An agent died mid-task leaving uncommitted work. `cargo build --release` was **clean**. `cargo test`
had **three compile errors**, all on one line:

```rust
assert_eq!(values, PointerVec::U8(vec![300 % 256, 0]));
```

Both literals infer as `u8` from the `Vec<u8>` context, so `300` and `256` are out of range, `256`
truncates to `0`, and the modulo panics at compile time. Widened to `(300u32 % 256) as u8`, which
keeps the arithmetic documenting the truncation it tests instead of hardcoding `44`.

**The lesson is not the literal.** It is that `#[cfg(test)]` blocks are not compiled by `cargo
build`, so **a green build carries no information about whether test code even parses.** Any gate
that means to say "this compiles" must run `cargo test`, not `cargo build`. `tests/verify.sh`
already does — which is worth noting as a design choice that paid off rather than luck.

### H+? — `&self` is a promise the FFI boundary cannot keep

Found while porting `stack`/`queue`, by differentially probing the *bridge* rather than the core.
Three defects, two of them in code that was already green and one of them in a module already in
`tests/scope.txt`.

**1. `noalias` ate a mutation.** napi hands the same wrapped struct to JS as `&self` for one
method and `&mut self` for another, and JavaScript calls the second from inside a callback the
first is still running:

```js
q.forEach(function (value, i) { if (i === 0) { q.dequeue(); q.dequeue(); } });
```

Upstream yields `1, 4, undefined, undefined` — its `forEach` re-reads `this.items` every
iteration and the second dequeue rebinds it. The port yielded `1, 2, 3, 4`. rustc marks a `&T`
parameter `noalias readonly` whenever `T: Freeze`, so LLVM hoisted the read straight out of the
loop; the *same object* reported the new `offset` correctly through a separate call one line
later.

The fix is not a barrier, it is the type: a struct holding a `RefCell` inline is not `Freeze`, so
`&self` carries neither attribute. The bridges now hold `RefCell<Core*>` and release every borrow
before any JS call.

**This is also present in the `sparse-set` bridge, which is already in `tests/scope.txt`.**
Measured: a `forEach` callback that deletes yields `[1, 4, 3, 4]` against upstream's `[1, 4]`. Not
fixed in this pass — out of lane — and the same `RefCell` change fixes it.

**2. napi's iterator installs a `#.return` that latches.** `obliterator/iterator` has no `return`
and no `done` flag, so breaking out of a `for…of` leaves the cursor where it stopped and a later
`next()` resumes. napi's `#[napi(iterator)]` sets `next`/`return`/`throw` as own properties on
every instance, and its `return` writes `[[GeneratorState]] = true` **before** the Rust
`complete()` runs — so `complete` cannot prevent it and every later `next()` answered
`{done: true}`. The addon now deletes the method at load, which is exactly upstream's situation.

This **corrects a claim in `crates/mnemonist-napi/src/sparse_set.rs`'s docs** — that napi's
default `complete` is observably the same as having no `return`. It was reasoned about, not
measured, and it is wrong.

**3. `napi_create_reference` rejects primitives.** Below Node-API 10, and napi-rs 3.12 does not
export `node_api_module_get_api_version_v1`, so an addon built with it is a version-8 module
however its Cargo features are set — moving the workspace from `napi9` to `napi10` changes
nothing. Measured. Storing arbitrary JS values therefore needs an enum: references for
object/function/symbol, by value for primitives, which is observationally exact because
primitives are immutable and compared by value.

*Write-up beat: every one of these is the FFI layer being more honest than the type system. (5) in
the angle list gets stronger — "the boundary gave me the semantics idiomatic Rust would have
broken" now has a companion, "and it took the semantics `&T` would have broken away from me".*

### The pattern these three share

Three separate incidents today, all the same meta-failure — *the check I ran did not check what I
believed it checked*:

| Incident | What it appeared to verify | What it actually verified |
|---|---|---|
| Falsification that stayed green | that the suite exercises Rust | nothing — it sabotaged a branch the test never takes |
| RSS as evidence for the L3 hypothesis | that the footprint shrank | nothing — the pages were never resident to begin with |
| `cargo build` clean | that the code compiles | that *non-test* code compiles |
| `cases=16666` in a fuzz campaign | 16,666 distinct programs | 32 programs, plus two saved seeds re-run ~8,300 times each |
| `sparse-set` bridge green on `forEach` | that the loop re-reads `size` | nothing — the read was hoisted; no test or fuzz case mutates from a bridge callback |
| A cursor's `complete()` returning `None` | that `break` leaves the walk resumable | the opposite — napi had already latched `[[GeneratorState]]` |
| `cargo build … \| tail -1` before a fuzz run | that the sabotage was compiled in | tail's exit status; the run used a stale binary and reported "clean" |

Each one produced a **confident green signal that was empty**, and in every case the failure was
invisible until something forced the question "what would this look like if it were broken?"

*This is the strongest write-up thread we have. It is the rigor gap in miniature — not "the port
was wrong" but "the evidence that the port was right did not say what we thought". A hackathon
premised on proving behavioural equivalence is exactly the place to argue that verification needs
verifying.*

## Write-up angle candidates

1. **"The rigor gap, measured."** The event's own thesis, tested: what differential fuzzing
   actually finds in a well-tested JS library. Strongest if the fuzzer produces real divergences.
2. **"Your iterator semantics are load-bearing."** Self-returning cursors, hybrid live/snapshot
   capture, two-level `Symbol.iterator` — the parts of JS that idiomatic Rust silently changes.
   *Most reusable by other people; probably the best insight-per-word.*
3. **"Node 26 broke my test suite before I wrote any code."** Short, punchy, concrete.
4. **"One grep moved the hardest code out of my core."** Boundary-vs-core as a porting principle.
5. **"The FFI layer gave me the semantics idiomatic Rust would have broken."** Counterintuitive,
   and it inverts the usual framing of FFI as a necessary evil.

Pick after the event based on what the fuzzer actually found. **(2) and (5) are strong regardless
of outcome; (1) depends on results.**
