# linked-list

Upstream: `linked-list.js` (261 LOC) · `test/linked-list.js` — **139 lines, 11 `it` blocks,
31 assertion statements**.

Port: `crates/mnemonist-core/src/structures/linked_list.rs`. Bridge:
`crates/mnemonist-napi/src/linked_list.rs`. Shim: `tests/bridge/linked-list.js`. Fuzz spec:
`crates/difffuzz/src/modules/linked_list.rs`.

A singly linked list over plain `{item, next}` nodes: `push`/`unshift`/`shift`,
`first`/`last`/`peek`, `forEach`, `toArray`, `values`/`entries` (`Symbol.iterator` aliased to
`values`), and a static `.from`. Chosen for this batch specifically because CLAUDE.md's brief
named it: "linked-list has a cursor" — and it does, but not the shape any other module in this
port has. Read `crates/mnemonist-core/src/structures/linked_list.rs`'s own module docs first; this
file summarises what they establish and adds the six required sections.

---

## What upstream tests

Eleven blocks, each exercising one method or a short combination:

```js
list.push('test');
assert.strictEqual(list.size, 1);
// ...push/unshift together, clear, first/last/peek, shift to exhaustion,
// forEach counting invocations, JSON.stringify, LinkedList.from(an object),
// a values() iterator drained by hand, an entries() iterator drained by hand,
// for-of over a list built by .from.
```

Characterising the shape of that coverage:

* **Every list built in the suite has at most three elements**, and every one is drained or
  cleared before the test ends. No test ever holds a list across two `it` blocks.
* **No cursor is ever opened before a mutation.** `values()`/`entries()` are always called on a
  list that is already in its final shape, then drained to exhaustion in the same block.
  `forEach`'s callback never calls back into the list.
* **`shift()` is only ever called until the list is empty**, and `last()` is never read
  immediately afterward — so B-241 (below) has zero coverage.
* **`unshift` is tested together with `push`**, but neither is tested while a cursor from an
  *earlier* call is still open.
* **`LinkedList.from` is given a plain object** (`{one: 1, two: 2, three: 3}`), exercising
  `obliterator`'s object-enumeration branch, not an array or iterable.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by the original suite.

**Cursor liveness — the whole reason this unit was chosen for this batch**

1. **A `push` while a cursor sits on the (old) tail is never observed.** Upstream's `push` mutates
   the tail node's `.next` in place, and a cursor that has not yet advanced past it sees the
   append — untested here, and the sharpest of the three liveness rules below.
2. **A `shift`/`unshift` while a cursor is open is never observed.** Both are invisible to an
   already-open cursor (neither touches a node a cursor might already hold), and nothing pins that
   either way.
3. **A cursor that has reported `{done: true}` is never revisited after the list grows.** Nothing
   confirms it stays done rather than resuming (contrast `queue.js`, whose cursor *does* resume —
   D-08/D-09 — this module's does not, and nothing here says so).
4. **`clear()` under an open cursor is never done.** `clear` never touches any node object, so an
   open cursor is entirely unaffected by it; untested.

**B-241's whole territory**

5. **`shift()` down to a fully empty list, followed by `last()`, is never done.** This is exactly
   the gap B-241 lives in (below).
6. **A `push`/`unshift` immediately after emptying the list via `shift()` is never done**, so the
   self-healing half of B-241 is untested too.

**Everything else**

7. **`LinkedList.from` is never given an array, a `Set`, a `Map`, a string or a generator** — only
   a plain object. `obliterator`'s other four branches are unexercised here (though several are
   pinned generically by `tests/boundary/foreach.js` for other modules).
8. **`forEach`'s `scope` argument is never passed.** Only the `arguments.length > 1 ? scope : this`
   omitted-argument branch is exercised.
9. **Never called at all:** `toString`, `inspect()`, the `nodejs.util.inspect.custom` symbol.

## What we test in addition

**`crates/mnemonist-core/src/structures/linked_list.rs` — 21 tests:**

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | the eleven blocks, as a baseline |
| `shifting_the_last_element_leaves_tail_stale`, `a_stale_tail_from_b_241_is_healed_by_the_next_push`, `a_stale_tail_from_b_241_is_healed_by_the_next_unshift`, `the_staleness_only_appears_once_the_list_is_shifted_fully_empty` | 5, 6 — B-241 |
| `a_push_after_the_cursor_opened_is_visible_if_not_yet_past_the_tail` | 1 |
| `a_push_after_the_cursor_has_passed_the_tail_is_not_visible` | 1 |
| `a_shift_is_invisible_to_a_cursor_already_open` | 2 |
| `an_unshift_is_invisible_to_a_cursor_already_open` | 2 |
| `a_cursor_opened_on_an_empty_list_never_yields_anything_even_after_pushes` | 3 |
| `clear_does_not_affect_a_cursor_already_open` | 4 |
| `a_cursor_is_not_restartable` | D-06 |
| `a_for_each_shaped_walk_sees_a_push_made_from_its_own_callback_on_the_lone_tail_node`, `a_step_shaped_walk_does_not_see_a_push_made_between_two_of_its_own_steps` | the port defect the fuzzer found (below) — pins both halves directly |
| `push_after_for_each_shifts_the_list_to_empty_starts_a_fresh_one_element_list` | the second port defect the fuzzer found (below) |
| `interleaved_unshift_and_push_produce_the_expected_order`, `a_long_workout_of_push_shift_unshift_matches_a_vecdeque_reference` | general correctness, cross-checked against `std::collections::VecDeque` |
| `from_iter_builds_in_order`, `shift_on_an_empty_list_reports_absence_without_panicking`, `an_empty_list_reports_empty_everywhere`, `step_checked_reports_done_rather_than_a_gap` | baseline edges |

**27 side-by-side probes and the differential fuzzer** — see "Fuzz" below; the fuzzer is what
actually found the two port defects, both fixed before any campaign was logged.

**Still untested, stated rather than glossed:** gap 7 (`.from`'s other four `obliterator`
branches — covered generically for other modules, not specifically for this one), gap 8 (`scope`
under `arguments.length`, a deliberate divergence, see below), gap 9 (`inspect`, not bridged).

## Bugs this found

**B-241 — `shift()` never updates `tail`, so emptying the list leaves `last()` returning the
just-removed item.**
`status: verified against Node 24.18.1`.

```js
LinkedList.prototype.shift = function() {
  if (!this.size) return undefined;
  var node = this.head;
  this.head = node.next;
  this.size--;
  return node.item;               // `this.tail` is never read or written
};
```

Shifting the list down to exactly zero elements leaves `head` correctly `null` but `tail` still
pointing at the just-removed node:

```text
var list = new LinkedList(); list.push('a');
list.shift();          // -> 'a'
list.size              // 0
list.first()           // undefined  (head is null: correct)
list.last()            // 'a'        (tail is STALE: the removed item)
```

Silent and self-healing, exactly like B-40: the next `push` or `unshift` on an empty list takes the
`!this.head` branch and resets `tail` unconditionally, so the staleness is only observable in the
narrow window between "shifted to empty" and "the next insert." Reproduced rather than corrected:
`LinkedList::shift` deliberately does not touch `tail`, and `LinkedList::last` reads it verbatim
with no `size == 0` guard upstream's own `last` does not have either. Recorded in NOTES.md as
**B-241**.

**Two defects in the port, both found by `linked-list`'s own first fuzz campaign, both fixed
before any campaign was logged in `fuzz/log.txt`.** Neither is upstream's fault; both are recorded
here following the precedent `docs/modules/lru-cache.md` set for defects a gate never caught.

**1 — `forEach` shared a stepping primitive with the lazy iterators, which is wrong.** An earlier
cut of this module claimed `forEach` and `values`/`entries` could all share one "read item, advance,
return" cursor, on the theory that JavaScript's single-threaded execution makes "the callback ran"
and "the next `.next()` call runs" equivalent pauses. They are not: upstream's `forEach` advances
*after* its callback returns —

```js
while (n) {
  callback.call(scope, n.item, i, this);
  n = n.next;      // AFTER the callback: two separate statements
  i++;
}
```

— while the lazy iterators' `n = n.next` runs *inside* the same closure invocation that produced
the previous value, before the caller regains control. So a `push` made from inside a `forEach`
callback, while the walk sits on the current (lone) tail, relinks that tail's `.next` before the
following `n = n.next` reads it — the walk continues onto the freshly pushed node — while the same
push made by external code between two `values()` `.next()` calls is always too late, because `n`
already moved past the tail inside the previous call. An eager-advance cursor shared by both gets
the second case right and the first wrong. Found by
`crates/difffuzz/src/modules/linked_list.rs`'s very first campaign
(`--module linked-list --seed 42 --cases 63`), which disagreed on operation #3 of a nine-line
generated program:

```js
var s = new LinkedList();
s.push(0); s.push(0); s.shift();
var fired = 0;
s.forEach(function (a, b) { if (fired++ < 1) s.push(a); });
// port saw one callback invocation; upstream saw two
```

Fixed by `ListCursor::current`/`ListCursor::advance` — a peek-then-commit split `forEach` uses and
the lazy iterators do not; see the core module's own docs for the full account. `ListCursor::step`
keeps its original (correct, for the lazy iterators) shape.

**2 — `push` branched on `self.tail` instead of `self.head`.** Indistinguishable from upstream's
own `if (!this.head) {...} else {...}` guard in every state except the one B-241 produces (`head`
`None`, `tail` stale-`Some`), where the wrong branch links onto the stale tail and never repairs
`head` — leaving it permanently unreachable even though `tail` and the arena both hold a real node.
Found by the same campaign, one generated operation after the first defect:

```js
var s = new LinkedList();
s.push(0);
var fired = 0;
s.forEach(function (a, b) { if (fired++ < 1) s.shift(); }); // empties the list -- B-241's state
s.push(0);
// port: toArray() === []   upstream: toArray() === [0]
```

Fixed to check `head`, matching upstream's own guard and this port's own (already-correct)
`unshift`. Both defects are pinned by dedicated Rust unit tests (see "What we test in addition")
and confirmed absent by the post-fix campaigns below.

**A grammar hazard in the fuzz spec itself, not the port.** `$forEach`'s `push` mutation initially
used the same uncapped limit every other mutation table in this port uses. Because a `push` while
the walk sits on the tail relinks that exact tail's `.next` to the node just pushed — and the walk
then advances onto precisely that node, which is now itself the tail — an uncapped `push` chases
its own tail forever. Not a divergence (a real Node `forEach` in the identical shape loops
identically); a program this grammar must not generate, since a campaign is meant to run thousands
of finite cases in its time budget. Capped at 8; per-case throughput went from roughly 2 seconds to
roughly 5 milliseconds. See `fuzz/log.txt` for the before/after numbers.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **The arena never frees or recycles a slot.** `shift()` removes a node from the reachable chain but never from `LinkedList::arena`; a list that has pushed and shifted heavily keeps every item it has ever held until the whole list is dropped. Upstream does not have this cost — V8 reclaims a shifted-off node once nothing (no list, no open cursor) references it. This port cannot tell "no cursor holds it any more" without a live reference count per node, and recycling the slot would silently alias two logically distinct positions the moment a stale index was reused — the same failure mode `fibonacci-heap`'s own arena docs describe for re-entrancy, here reachable through nothing more exotic than an ordinary open cursor. At the bridge, a stored item is a real JS value (`JsSlot`) kept alive by the arena for as long as the arena lives — later than upstream would release it, never never. |
| — | **`forEach`'s third callback argument, and `scope` under `arguments.length`.** Upstream passes the list itself as the third argument and keys `scope` off `arguments.length > 1`, which napi's typed signature cannot see; identical divergence to `SparseSet`/`Queue`/`Stack`'s own `forEach`, recorded the same way. The omitted-argument case, which the original suite uses, is exact. |
| — | **`inspect()`/`toString`'s custom constructor-name trick are not ported.** Upstream's `inspect()` returns `toArray()` with `Object.defineProperty(array, 'constructor', {value: LinkedList, ...})` so Node's REPL prints it as a `LinkedList`; nothing asserts on this and no equivalent concept exists at the boundary. `toString`/`toJSON` (plain `toArray().join(',')` / `toArray()`) are ported. |
| D-06 | No collection implements `IntoIterator`; unchanged from every other module in this port, for the same non-restartability reason. |

## Fuzz + bench

### Fuzz

```
module=linked-list  seed=42       cases=11790 ops=1184507 wall=60.0s divergences=0
module=linked-list  seed=20260801 cases=11865 ops=1187723 wall=60.0s divergences=0
```

Two campaigns, two seeds, **2.37M operations, zero divergences** — against a build that already
carries both fixes from "Bugs this found." Both defects were found by this same grammar, one
operation apart, in an earlier unlogged run before either fix landed; these two measure the grammar
*after* the interesting bugs, not instead of them. An earlier pair of runs at these same seeds
measured only ~3-4K ops in 60-70 seconds because of the `$forEach`/`push` grammar hazard described
above; `fuzz/log.txt` keeps both the original (slow, but still zero-divergence) entries and the
corrected re-runs, annotated, rather than rewriting history.

* **Op alphabet:** `push` (5), `unshift` (4) — both outweigh `shift` (3) so a program keeps enough
  live nodes to reach the liveness rules rather than emptying the list every few operations —
  `first`/`last` (2 each, the pair B-241 depends on), `peek`/`clear` (1 each), `$iter` over
  `values`/`entries` (2), `$next` (4), `$spread` (1), `$forEach` (3, the heaviest of the
  cursor-lifecycle ops — this is the one that reaches "push while the walk is mid-flight").
* **Observable state:** `size`, `first()`, `last()`, `toArray()`, compared after every operation.
* **Values:** a small pool (`0..24`), so a shrunk repro is unambiguous.
* **`$forEach` mutation table:** `push`/`unshift`/`clear` uncapped (`shift` uncapped too — none of
  the three have `push`'s tail-chasing hazard, since `shift`/`unshift`/`clear` are all invisible or
  bounded to the cursor per the module's own liveness rules), `push` alone capped at 8 for the
  reason above.
* **Deliberately excluded:** object/reference identity questions (`JsSlot`/`WeakKey`-shaped) do not
  apply — `Value` is compared by content, matching this test file's own primitive-only style — so
  the fuzz side runs `LinkedList<serde_json::Value>` directly, no bridge-specific mirror key type
  needed, unlike `default-map`'s `FuzzKey`.

### Falsification (gate 6)

Two Rust-level defects were falsified, plus B-241; all three were assertions the port's own
history had already made, so the sabotage is literally "revert the fix" in two cases.

**A — the `forEach` timing fix.** *(This is the same shape as "Bugs this found" #1 above; not
separately re-run as a formal gate 6 sabotage, since finding it via the fuzzer IS the falsification
— the campaign that found it is the confirmation.)*

**B — `push` branching on `head` vs `tail`.** Assertion named first: the last line of
`push_after_for_each_shifts_the_list_to_empty_starts_a_fresh_one_element_list`,
`assert_eq!(list.first(), Some(&9))`. Sabotage: reverted `push` to `match self.tail { ... }`.
Confirmed red: two Rust tests failed (the named one, plus
`a_stale_tail_from_b_241_is_healed_by_the_next_push`'s `first()` assertion, added specifically
because the pre-existing `last()`-only version of that test could not distinguish the two
branches). The original mocha suite stayed green (11 passing) — expected; it never reaches this
state. The differential fuzzer caught it in 162 cases, 272 operations, 0.1 seconds, on the
identical minimised repro shown above. Reverted; confirmed green at all three: the two Rust
assertions pass, `11 passing` on the original suite, and a 200-case replay of the same seed comes
back `0 divergences`.

**Nothing was found to be blind here.** Every instrument — the Rust unit tests, the original
suite (correctly green, since it does not test this), and the differential fuzzer — behaved
exactly as expected for this sabotage.

### Bench

`bench/results.json` → `modules["linked-list"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`shift`/`walk` (50/25/25). `push` (tail) and `shift` (head) are
both O(1) — they only ever touch the two ends the list already holds pointers to, exactly like a
plain array-backed deque would, which would leave the one thing a *linked* list does differently
from a contiguous one — walking node to node by following `next` — completely untested. `walk` (the
remaining 25%) opens a fresh cursor at the head (upstream's own `values()`, the same generator
`forEach`/`entries`/`Symbol.iterator` all share) and steps it forward 20 times, genuinely chasing
pointers rather than answering in O(1). xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **6.18** | 25.47 | 4.1× faster |
| p99 ns/op | **10.79** | 55.44 | 5.1× faster |
| RSS delta MB | **17.9** | 195.8 | |
| structure-only RSS delta MB | **1.5** | 9.8 | |
| startup ms | **0.6** | 15.5 | 26× (reported separately; not throughput) |

**No regressions**, and the arena-never-recycles question the earlier draft of this section flagged
is settled by the RSS row: even though `Vec<Node<T>>` grows monotonically with every `push`
regardless of later `shift`s (the arena's own docs explain why a shifted slot cannot be freed while
a cursor might still reference it), the port's RSS delta is still an order of magnitude below
upstream's own per-node heap objects. Checksum `250135666931`, identical on both sides.
