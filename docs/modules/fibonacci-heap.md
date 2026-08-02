# fibonacci-heap

Upstream: `fibonacci-heap.js` (321 LOC) · `test/fibonacci-heap.js` — **115 lines, 6 `it` blocks, 29
assertion statements**.

Port: `crates/mnemonist-core/src/structures/fibonacci_heap.rs`.
Bridge: `crates/mnemonist-napi/src/fibonacci_heap.rs`. Shim: `tests/bridge/fibonacci-heap.js`.
Fuzz spec: `crates/difffuzz/src/modules/fibonacci_heap.rs`.

`test/fibonacci-heap.js` `require`s only `../fibonacci-heap.js`; that file's own require-closure
also needs `./utils/comparators.js` (79 LOC), but that file is already a ported unit — the
`DEFAULT_COMPARATOR`/`reverseComparator` machinery this module reuses verbatim rather than
reimplementing, per DESIGN.md's own T2-tier table (`heap` 576 · `fixed-reverse-heap` 209 ·
`fibonacci-heap` 321). So this unit's own LOC is the 321 alone, and it opens no new capability tier:
T2 (a comparator called from inside a Rust sift, and able to re-enter the very structure it is
comparing) was already established by `heap`, and this module is the second and sharpest test of it.

---

## What upstream tests

Six `it` blocks, every one over small, fresh heaps with a total comparator:

* **`push`/`size`** — two pushes, one assertion on `size`. `size` is never checked against anything
  but a successful push.
* **`peek`** — the empty-heap `undefined` case, then two pushes checked against ascending minimums.
* **`pop`** — the file's longest block: four values pushed, then drained one at a time with `size`
  checked after every pop, ending on `pop()` returning `undefined` on the now-empty heap. Four
  elements is also the largest heap the whole file ever builds.
* **`MaxFibonacciHeap`** — the identical four-push/drain shape, under the reversed comparator.
* **A custom comparator** — `assert.throws` on a string argument (matching `/function/`, not the
  exact wording), then one heap and one max-heap each pushing two `{value: ...}` objects and
  asserting `peek()` via `deepStrictEqual`.
* **`FibonacciHeap.from`** — a `Set` of three numbers, checked for `size` and `peek()`.

Every comparator in the file is total, pure and side-effect-free. No heap here is ever popped enough
times to force `consolidate` through more than a single, trivial merge — four elements is at most
one degree-1 link.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by `test/fibonacci-heap.js`.

**`consolidate`'s real job — repeated, multi-level degree merging**

1. **A heap large enough that one `pop` forces several links, not zero or one.** The largest heap in
   the whole file has four elements; `consolidate`'s `while (A[d])` loop, which is the entire reason
   this structure is asymptotically better than a binary heap for a `push`-heavy workload, never runs
   more than once. See "Fuzz + bench" for measured evidence this port's own tests and fuzz grammar do
   reach it, repeatedly.
2. **Interleaved push/pop against a heap that has already consolidated at least once.** Every block
   either only pushes, or pushes everything and then only pops — never both, repeatedly, against the
   same instance.
3. **The tie-break itself.** `push`'s `<=` (favouring the most recently pushed node on an exact tie)
   and its interaction with `consolidate`'s degree-bucket restructuring are never exercised by any
   duplicate value anywhere in the file.

**The comparator as a re-entrant callback — the entire regime, same as `heap`'s own gap**

4. **A comparator that mutates the heap it is comparing.** Three distinct shapes exist and are
   fuzzed here: growing it (`push` from inside a comparison), shrinking it (a **nested** `pop` from
   inside another `pop`'s own `consolidate`), and resetting it (`clear()` mid-`consolidate`). None
   appears in the original suite.
5. **A comparator that throws.** Never used upstream.
6. **`FibonacciHeap.from` with a custom comparator.** Both call sites in the file use the default.
7. **`from` on an empty iterable**, and on anything other than a `Set`.

**What genuinely cannot be reached, by anyone**

8. **`decreaseKey` and cascading cuts.** Read before assuming these are merely untested: there is
   **no `decreaseKey`, no `delete`, no node `mark` field, no cut, no cascading cut anywhere in
   upstream's source or its `.d.ts`** — confirmed by reading `~/upstream-mnemonist/fibonacci-heap.js`
   and `fibonacci-heap.d.ts` directly and by grepping the entire upstream repository for
   `decreaseKey`/`mark`/cut-shaped names, all with zero hits. This is upstream's own limitation. No
   op alphabet, however wide, can exercise a code path that is not there; stated here rather than
   silently treated as a gap in this port's own fuzz grammar (see "Fuzz + bench").

**Never called at all**

9. `inspect()` and the `nodejs.util.inspect.custom` symbol — a Node display convenience with no
   upstream assertion, same policy `heap`/`fixed-reverse-heap` already established.
10. `FibonacciHeap.MinFibonacciHeap` (`FibonacciHeap` itself under an alias) and the
    `instanceof`/`.constructor` relationship between `FibonacciHeap` and `MaxFibonacciHeap` — see
    "Bugs this found", B-221.

## What we test in addition

**Rust native tests** — `crates/mnemonist-core/src/structures/fibonacci_heap.rs` (12):

| Test | Closes gap |
|---|---|
| `push_increments_size`, `peek_reads_the_minimum_without_removing_it`, `pop_drains_in_ascending_order`, `a_max_heap_drains_in_descending_order`, `a_custom_comparator_orders_by_a_projected_field`, `from_iter_builds_a_heap_from_an_iterable` | the upstream blocks, as a baseline |
| `consolidation_merges_trees_across_many_pushes_and_pops` | 1 — 64 pushes then a full drain, asserting both the sorted output and a measured floor (`merges() >= 6`) on how many links one `pop` forces |
| `interleaved_push_and_pop_stays_sorted_and_merges_repeatedly` | 2 — a 400-step xorshift32-seeded interleaving of push and pop, checked against a reference `Vec` sort at every third step, plus a measured floor on total merges |
| `push_favours_the_most_recently_pushed_node_on_a_tie` | 3 — pins that a tie really does take the `<=` branch (see "Gate 6" for the sharper way this rule is actually pinned) |
| `a_comparator_may_re_enter_and_push` | 4 (growing) |
| `a_comparator_that_clears_the_heap_mid_pop_does_not_panic` | 4 (resetting) — and pins NOTES.md B-220's exact `-1`, not merely "doesn't crash" |
| `a_pop_after_b_220s_negative_size_panics_matching_upstreams_null_dereference` (`#[should_panic]`) | the follow-on half of B-220: the *next* `pop` after the corruption |

The shrinking (via a **nested** `pop`, gap 4's shrinking shape) is not a separate native test: it is
what the fuzz grammar's `fibPopper` factory exercises on every campaign, and it is what found this
module's own arena-recycling defect (see "Bugs this found").

**Differential fuzzer** — see "Fuzz + bench" for the campaign and the measured evidence that
`consolidate`'s degree-merge path fires repeatedly, not once, and that a nested `pop` from inside
another `pop` survives without panicking or deadlocking.

**Still untested, stated rather than glossed:** gap 9 (`inspect`, not ported — no upstream
assertion), gap 6/7 (`from` with a custom comparator or an empty source — mechanically identical to
`push`, which is exhaustively covered, so not independently pinned), and gap 8, which is untested
because it is unreachable, not because it was skipped.

## Bugs this found

Three upstream defects, **B-220 through B-222**, verified either by tracing upstream's own
deterministic control flow or by the differential fuzzer itself. Full write-ups in
`planning/NOTES.md`; in brief:

| ID | Defect | Found by |
|---|---|---|
| B-220 | a comparator that `clear()`s the heap from inside a `pop`'s `consolidate` leaves `this.size` at `-1` (JS has no unsigned integers), and the *next* `pop` then crashes reading `null.child` — because `-1` is truthy and does not satisfy the `!this.size` empty guard | writing this unit's own re-entrancy tests; independently reproduced constantly once the fuzz grammar existed |
| B-221 | `MaxFibonacciHeap.prototype = FibonacciHeap.prototype` — the identical `instanceof`-blurring anti-pattern `heap.js`'s B-75 already documents, one file over | reading, cross-referenced against B-75 |
| B-222 | a `clear()` fired from inside a **`push`'s** tie-break (not a `pop`'s `consolidate` — B-220's site) leaves `root` `null` while `min` is restored to a real node by that same `push`'s own assignment one line later; the next `pop` then crashes on a *different* `TypeError` (`reading 'right'`, from `consolidate`'s `consumeLinkedList(null)`, not `reading 'child'`) | the differential fuzzer, inside its first few hundred generated cases once the `fibClearer` factory existed |

**B-220 is the one worth reading twice**, because the two halves are almost independent bugs:

```js
// pop()'s tail:
if (z === z.right) { this.min = null; this.root = null; }
else { this.min = z.right; consolidate(this); }
this.size--;                                              // AFTER consolidate, not before
return z.item;
```

A `clear()` mid-`consolidate` sets `this.size = 0`; the pending `this.size--` then computes `-1`.
That alone is silent (nothing throws yet) — but the *next* `pop`'s own guard, `if (!this.size)
return undefined;`, is a **falsy** check, and `-1` is truthy. So the corrupted heap does not report
itself empty; it proceeds into `var z = this.min;` — `null` — and `z.child` throws.

**Two real defects in this port, both found while landing this unit, both fixed:**

1. **An arena that recycled a popped node's slot.** The first cut of `Arena` freed a node's slot on
   `pop` and reused it on the next `create_node`. A `fibPopper` comparator running a **nested** `pop`
   from inside another `pop`'s `consolidate` can free a node the *outer* call's own `nodes` snapshot
   still holds an id for — panicking immediately once a later `create_node` reused that id (found by
   the differential fuzzer inside its first fifty generated cases: `new
   FibonacciHeap({"$factory":"fibPopper"}); push(0); push(0);` was enough), or, worse, silently
   handing the outer call a *different, unrelated* node under the id it expected. JavaScript has no
   such hazard — an object stays valid for as long as anything references it, suspended re-entrant
   call frames included. Fixed by never recycling a slot at all; see `Arena`'s own module docs and
   D-173.
2. **The fuzz harness's own panic-message recovery**, not the port: `catch_unwind`'s `Err` payload,
   downcast to `&'static str`/`String`, is the textbook way to recover an `.expect(...)` panic's
   message — and it is what an isolated `rustc -O` repro of the exact same `.expect()` call produces.
   It is **not** what this crate's actual release-profile binary produces at this call site,
   measured directly (the downcast failed silently, logged, and did not reproduce in isolation).
   Fixed by reading the message through a custom `std::panic::set_hook` instead, which renders via
   `Display` regardless of the payload's concrete representation. See
   `crates/difffuzz/src/modules/fibonacci_heap.rs`'s `pop`/`install_panic_capture`/`bare_message` doc
   comments for the full account.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-170 | **`MaxFibonacciHeap` is installed as evaluated JavaScript, not a second native class.** | Reproduces B-221's prototype-sharing exactly, the same mechanism `crate::heap`'s `install_heap_statics` already uses for `MaxHeap`; a second `#[napi]` class would silently repair the `instanceof` blur instead. |
| D-171 | **`size` is `i64`, not `usize`.** | `usize` cannot represent B-220's `-1`; clamping or panicking on the arithmetic itself would each be a different, more "defensive" behaviour than upstream's own silent corruption. Matches `multi-set`'s D-163 precedent. |
| D-172 | **B-220/B-222's crashes are Rust panics whose message IS upstream's exact `TypeError` text.** | `mnemonist-core` has no exceptions. The message text (not a description of the Rust-side invariant) is what lets the fuzz harness recover the exact upstream wording without a hand-maintained translation table. Noted inconsistency with `_utils`'s D-104, which chose `Result<_, KWayError>` for B-180 instead — that call site already returns a `Result` its callers handle routinely; this one is reachable only through one adversarial re-entrant sequence, and threading a new error variant through every `pop`/`consolidate` caller for it was judged disproportionate. |
| D-173 | **Nodes live in an arena of `NodeId`s, never `Rc<RefCell<Node>>`, and a popped slot is never recycled.** | A literal `Rc` translation of upstream's circular doubly-linked list is a strong-reference cycle that never reaches zero; a *recycling* arena panics or silently aliases two nodes under re-entrant nested `pop`s (see "Bugs this found"). Both are unobservable through the public API either way — nothing exposes node identity or arena occupancy. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion, same policy as `heap`/`fixed-reverse-heap`. |
| — | **Comparator-is-a-function validation lives at the bridge, not in core.** | `typeof comparator !== 'function'` is a JavaScript-value question; core's `FibonacciHeap::new` takes any `C: Comparator<T, E>`, which is checked at compile time instead. Same split `crate::heap`'s `Heap::new` already uses. |

## Fuzz + bench

### Fuzz

```
module=fibonacci-heap seed=42       cases=4713 ops=943340 wall=60.0s divergences=0
module=fibonacci-heap seed=20260801 cases=4702 ops=945478 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1.89M operations, zero divergences**. Reproduce with e.g.
`target/release/difffuzz --module fibonacci-heap --seed 42 --cases 4713`.

* **Op alphabet:** `push(v)` (weight 7, `v` drawn from `0..24` — small and repetitive, so ties are
  frequent rather than incidental) · `pop()` (5) · `clear()` (1). `program_len` is widened to
  `1..400` (`heap`'s own grammar uses `1..200`): this structure's whole point is `consolidate`'s
  degree-merging, which needs a population built up across many pushes before a `pop`'s degree
  bucketing has real work to do, not a handful of elements.
* **Constructor alphabet:** six comparator factories, mirrored name for name between
  `fuzz/oracle.js` and `crates/difffuzz/src/modules/fibonacci_heap.rs`. `ascending`/`descending`/
  `boom` are reused **verbatim** from `heap`'s own table — none of the three touches `.items`, so
  none needed a fibonacci-heap-specific version. Three are new, because this structure has no public
  backing array to mutate through: `fibPushy` (`instance.push(99)`), `fibPopper` (a **nested**
  `instance.pop()` — the shape that found this port's own arena defect), `fibClearer`
  (`instance.clear()` — B-220/B-222's trigger).
* **Observable state, compared after every op:** `size` and `peek`. There is no `.items` to compare
  against — `push`/`peek`/`pop`/`clear` are this structure's entire public surface. `size` is
  compared as the signed `i64` both sides can produce (D-171); a campaign that clamped or ignored
  negative values here would have missed B-220 entirely.

**Both crash sites (B-220, B-222) fire constantly across a normal campaign** — visible directly in
the stderr of any run, `Cannot read properties of null (reading 'child')` and `(reading 'right')`
both appear dozens of times per 60-second campaign — and every one is caught by the harness's
`catch_unwind`/panic-hook machinery and compared as a `{"$throw": ...}` value against upstream's own
thrown `TypeError`, matching exactly: `divergences=0` is not "this campaign never reached the bug",
it is "the port and upstream agree on the bug".

**Measured evidence that `consolidate` actually merges trees, repeatedly — not inferred from op
weights.** `grammar_self_check` (`crates/difffuzz/src/modules/fibonacci_heap.rs`, no oracle, no
`node`) runs 400 generated programs directly against the core structure and counts
[`FibonacciHeap::merges`](../../crates/mnemonist-core/src/structures/fibonacci_heap.rs), a
diagnostic counter incremented once per `link` call (not part of upstream's API):

```
fibonacci-heap grammar: 81477 ops, 16815 tree merges across 400 programs (369 of them saw at least one)
```

92% of generated programs (369/400) triggered at least one degree-merge, and the total is over 16.8
thousand links across those programs — this is not a grammar that merely proves the heap can store
numbers.

**Cascading cuts are not measured, because they cannot be reached — stated plainly rather than
inferred from a clean campaign that never got there.** There is no `decreaseKey` anywhere in
upstream's `fibonacci-heap.js` (see "What upstream does NOT test", gap 8, and the core module's own
docs). No fuzz grammar, however wide, can exercise a cut when there is no operation that could ever
trigger one. This is recorded as a genuine, structural gap in this unit's coverage — not upstream's
fault, and not this port's — rather than quietly omitted from this report.

### Falsification (gate 6)

Two attempts, targeting `consolidate` directly — the sharp target for this structure, not
`push`/`peek` — with contrasting outcomes, both reported honestly.

**Attempt 1 — confirmed red, dramatically.**

**The assertion named before running:** `interleaved_push_and_pop_stays_sorted_and_merges_repeatedly`'s
final `assert_eq!(popped, reference);`, plus `consolidation_merges_trees_across_many_pushes_and_pops`'s
sorted-drain assertion and, at the bridge, `test/fibonacci-heap.js`'s `should be possible to pop the
heap.` (`assert.strictEqual(heap.pop(), 1)`).

**The sabotage:** `consolidate`'s swap condition, `if (heap.comparator(x.item, y.item) > 0) { swap
}`, flipped from `> 0.0` to `< 0.0` — one character, in the branch that decides which of two
same-degree trees becomes the parent.

**Confirmed red, at every level checked:**

* Rust: **5 of 12 native tests failed**, both named targets included, plus two more
  (`a_max_heap_drains_in_descending_order`, `a_comparator_may_re_enter_and_push`) that depend on
  correct consolidation incidentally.
* The bridge: **the Node process aborted outright** —
  `thread '<unnamed>' panicked at ...: Cannot read properties of null (reading 'child') ... fatal
  runtime error: failed to initiate panic, error 5, aborting`. The sabotage corrupted the tree
  structure badly enough to reach B-220's own panic site through completely ordinary (non-re-entrant)
  operation, and a Rust panic crossing the N-API boundary is not a catchable JS exception — it takes
  the whole process down. About as red as a falsification gets.

**Reverted; confirmed green again** at both levels: `12/12` native tests, `6/6` original suite.

**Attempt 2 — stayed green in this unit's own gates, caught by the fuzzer and by a sibling unit.**
Reported as such, per this project's own standard for a falsification that does not fail where
expected (a recent `_utils` batch had the identical outcome twice, for the same underlying reason:
the sabotaged behaviour is genuinely unobservable to the instrument in question).

**The assertion named before running:** none could be named with confidence in advance — the honest
prediction, stated before running, was "this may not be observable within this unit's own tests,
because ties between identical values carry no information."

**The sabotage:** `push`'s tie-break, `<= 0.0` (favour the just-pushed node on an exact tie) flipped
to `< 0.0` (favour the existing `min`) — B-220/B-221's neighbouring line, and the exact rule D-105's
own fix in `utils/merge.rs` depends on.

**Result, confirmed and reported honestly:**

* This unit's 12 native tests: **all still pass**, `push_favours_the_most_recently_pushed_node_on_a_tie`
  included — both pushed values in that test are literally `5`, so which physical node "wins" a tie
  is unobservable through `peek()`.
* The original suite: **6/6 still pass** — no block in `test/fibonacci-heap.js` ever pushes two equal
  values.
* **The differential fuzzer caught it**, inside 425 generated cases:
  `new FibonacciHeap({"$factory":"fibClearer"}); push(10); push(10);` diverges on `peek()` after the
  very first `push` (`port: undefined`, `upstream: 10`) — a different comparator factory than the one
  being sabotaged exposed it, because `fibClearer`'s own re-entrant `clear()` interacts with the
  tie-break rule in a way two plain pushes do not.
* **The sibling unit's own regression caught it too:** `mnemonist_core::utils::merge`'s
  `merge_k_matches_upstreams_real_heap_on_the_case_that_found_d_105` failed — D-105's fix depends on
  this exact rule.

**Reverted; confirmed green again** at all four: `12/12`, `6/6`, a clean 3,000-case fuzz replay, and
`cargo test` workspace-wide (606 `mnemonist-core` tests passing).

This pair is the complete picture gate 6 is for: one sabotage that nothing could have missed, and one
that two of three instruments genuinely could not see — and the third (the fuzzer) is exactly the
instrument this event's whole thesis says should catch what example-based tests cannot.

### Bench

`bench/results.json` → `modules["fibonacci-heap"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-2e5`** — 200,000 mixed `push`/`pop`/`peek` (50/25/25), default numeric comparator, `size`
200,000 (not this batch's usual 1e6 — see below for why). The load-bearing check is
`FibonacciHeap::merges`, a public counter of `link` calls (two trees becoming one): measured
directly at **195,920 merges over 50,000 `pop` calls** for this exact op mix — ~3.9 merges per
`pop`, confirming `consolidate` does real, repeated multi-tree linking rather than degenerating to
"pop one thing, link nothing" the way a push-only stream would. xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **1251.72** | 31260.54 | 25× faster |
| p99 ns/op | **2667.66** | 64227.06 | 24× faster |
| RSS delta MB | **5.6** | 291.8 | |
| structure-only RSS delta MB | **0.1** | 6.7 | |
| startup ms | **0.6** | 15.3 | 26× (reported separately; not throughput) |

**`size`/`ops` are 200,000, not this batch's usual 1e6 — sanity-checked before committing** (the
`bit_set.rs` `rank` lesson `methodology.md` documents). A 1e6-op pass was timed by hand first: the
port completed in ~6 seconds, but upstream took **over 2 minutes**, and the profile was the
give-away — 92 seconds of *system* time against 52 seconds of *user* time, the signature of heavy
memory churn (V8 GC pressure over a very large, long-lived node graph) rather than of comparator or
algorithmic cost. At 200,000 ops the same ~20× ratio persists (see the table above) but both sides
finish in seconds, which is what makes an interleaved, warmed-up A/B/A/B pass practical at all — the
size reduction changes wall-clock cost, not the workload's shape: the merges-per-pop ratio above is
unchanged from the 1e6-op measurement (985,004 merges / 250,000 pops there, same ~3.9 ratio).

**No regressions.** Checksum `5003154165`, identical on both sides.
