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
