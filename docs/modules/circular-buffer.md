# circular-buffer

Upstream: `circular-buffer.js` (140 LOC) + **`fixed-deque.js` (357 LOC)** + `utils/iterables.js`
(93 LOC) + `obliterator/iterator` · `test/circular-buffer.js` — **339 lines, 18 `it` blocks, 95
assertion statements**.

Port: `crates/mnemonist-core/src/structures/circular_buffer.rs`,
`crates/mnemonist-core/src/structures/fixed_deque.rs`,
`crates/mnemonist-core/src/structures/backing.rs`, `crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/circular_buffer.rs`, `crates/mnemonist-napi/src/fixed_deque.rs`
(shared `#.get`), `crates/mnemonist-napi/src/array_class.rs`,
`crates/mnemonist-napi/src/fixed_stack.rs` (shared `from`).

**Best test-to-source ratio in the library: 339 test lines against 140 source lines, 2.4:1.** Which
makes the finding below worth stating up front — the ratio is measured against the wrong
denominator. `circular-buffer.js` is 140 lines because it *pastes* `FixedDeque.prototype` and then
overrides two methods; the real surface behind those 339 lines is 497 LOC, and the ratio is 0.68:1.

---

## Is this its own unit, or does it share one with `fixed-deque`?

**Its own unit, and the two are separate — but its require-closure strictly contains the other's.**

DESIGN.md §1.1 defines a unit as the require-closure of one upstream *test file*. There are two
test files, so there are two units:

| Test file | Require-closure |
|---|---|
| `test/fixed-deque.js` | `fixed-deque.js` + `utils/iterables.js` + `obliterator/iterator` |
| `test/circular-buffer.js` | `circular-buffer.js` + **`fixed-deque.js`** + `utils/iterables.js` + `obliterator/iterator` |

`circular-buffer.js`'s second line is `require('./fixed-deque')`, and it uses the import for
`Object.keys(FixedDeque.prototype).forEach(paste)` at load time — so `fixed-deque.js` must exist and
be correct before a single `it()` in `test/circular-buffer.js` runs. The containment is one-way:
`fixed-deque` can be declared done without `circular-buffer`, and `circular-buffer` cannot be
declared done without `fixed-deque`'s *source* (though not its test file, and not its scope entry).

The port mirrors the same shape. `CircularBuffer` holds a `FixedDeque` and delegates everything but
`push` and `unshift`, so the shared implementation is shared rather than duplicated, and
`tests/bridge/circular-buffer.js` requires only the addon — the shim does not chain through the
deque's, because the addon exports both classes directly.

**Practical consequence for the scope table:** the two must be scoped in that order, and a
regression in `fixed-deque` fails both files. Neither is in `tests/scope.txt` yet — gate 10 is
outstanding for both.

## What upstream tests

Eighteen `it` blocks, sixteen of which are `test/fixed-deque.js`'s blocks with the names changed.
The two that are genuinely new are the ones about wrapping:

```js
it('should be possible to wrap buffer around when pushing.', …)    // push 1..8 on capacity 3
it('should be possible to wrap buffer around when unshifting.', …) // unshift 1..8 on capacity 3
it('peekLast should not be subject to one-off errors (#223).', …)  // a real regression test
```

Characterising the shape of that coverage:

* **The overwriting path is well covered in one direction at a time.** Eight pushes and eight
  unshifts, each on a fresh capacity-3 buffer, with `toArray()` and `size` asserted after each.
* **`push` and `unshift` are never mixed on a full buffer.** Both wrapping blocks use one insert
  method throughout, so the case where a `push` overwrites the element a preceding `unshift` put at
  the other end is never reached.
* **`#223` is a regression test**, and it is the only place a boolean `false` is stored — which
  matters, because a falsy element is exactly what an off-by-one in `peekLast` would hide.
* **The return values of `push` and `unshift` are asserted only in the non-overwriting case**
  (`unshift(13) === 4`, in the block inherited from the deque). What an *overwriting* insert
  returns is never checked.
* **`start` is asserted twice**, inherited from the deque's blocks.
* Everything else — `get`'s four calls on a full capacity-3 buffer, one immediate-drain iterator
  per method, three array classes, no oversized `from` — is the deque's coverage verbatim.

## What upstream does NOT test

**The overwriting inserts, which are the entire reason this class exists**

1. **What an overwriting `push` returns is never asserted.** It returns the size *unchanged*, and
   that is the **only** externally visible signal that an element was dropped — there is no other,
   no callback and no flag.
2. **What an overwriting `unshift` returns is never asserted**, likewise.
3. **`push` and `unshift` are never mixed on a full buffer**, so the case where each overwrites the
   other end's element is untested.
4. **A capacity-1 buffer is never built.** It is the degenerate ring: every insert replaces the one
   element and `start` never leaves zero.
5. **The order inside `push` is never observed.** The store happens *before* the fullness test, so a
   full buffer overwrites the slot `start` is on and only then steps past it. Reversing those two
   lines drops the wrong element and still passes several of the file's assertions.

**`from` — which does not overwrite**

6. **`from` is never called with an iterable longer than the capacity.** It copies by index and
   assigns `size`, so it does *not* push and therefore does *not* overwrite: an oversized iterable
   leaves `size > capacity` on the one class whose whole purpose is to prevent that, and the walk
   then goes round the ring more than once. `CircularBuffer.from([1,2,3], Array, 2).toArray()` is
   `[1, 2, 1]`.
7. **`from` is never called with a non-array-like iterable** — that branch is a `TypeError` (B-60).
8. **`from` is never called with an unguessable iterable and no capacity.**

**Everything inherited from `FixedDeque`, inherited untested too**

9. **`#.get` for `size <= index < capacity`, and for a negative index** — B-62, which this class
   gets *literally*, because upstream pastes the same function object.
10. **`forEach` on a non-full, empty or wrapped buffer, with a mutating callback, or with a
    `scope`.**
11. **`pop`, `shift` and `clear` leaving the elements in place.**
12. **Element coercion**, a non-constructor `ArrayClass`, and a non-integral capacity.
13. **Cursor re-draining, `[...buffer]`, and mutation during iteration** — the last of which is
    sharper here than anywhere else in the wave, because an overwriting `push` can rewrite a slot
    the cursor has not reached, so `next()` yields a value that was not in the buffer when the walk
    started.
14. **`inspect()`** and the inspect symbol. Worth a note of its own: the pasted `inspect` closes
    over `FixedDeque`, so `util.inspect` on a `CircularBuffer` reports the constructor as
    `FixedDeque`. Not ported, so not a divergence here, but it is upstream behaviour and it is
    untested.

## What we test in addition

`crates/mnemonist-core/src/structures/circular_buffer.rs` — 11 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all eighteen upstream blocks, `#223` included with its issue number |
| `a_push_that_overwrites_returns_the_unchanged_size` | 1 — the return sequence `1, 2, 3, 3, 3` and the `start` walk `0, 0, 0, 1, 2`, pinned against Node |
| `an_unshift_that_overwrites_returns_the_unchanged_size` | 2 |
| `a_full_push_overwrites_the_slot_start_is_on` | 5 — asserts the array *and* `start` after one overwriting push, which is what a reversed store/test pair would break |
| `push_and_unshift_overwrite_opposite_ends` | 3 |
| `get_is_bounded_by_the_capacity_here_too` | 9 |
| `from_bypasses_the_overwriting_that_this_class_exists_for` | 6 |
| `many_wraps_still_walk_in_order` | — thirteen pushes on a capacity-4 ring |
| `a_capacity_of_one_replaces_in_place` | 4 |
| `an_overwriting_push_is_visible_to_an_open_cursor` | 13 — the sharpest hybrid-capture case in the wave |
| `cursors_do_not_restart_but_the_buffer_can_be_walked_again` | 13 |

Plus the whole of `fixed_deque.rs`'s 12 tests and `backing.rs`'s 4, which this class inherits by
construction rather than by copy — the delegation is what makes that true rather than merely
claimed.

**Differential probes against the vendored upstream**, 27 cases for this class (50 across both this
and `fixed-deque`): everything in the deque's list, plus the two overwrite return-value sequences
with their `start` walks, thirteen pushes on a `Uint8Array` ring, a capacity-1 buffer, and an
overwriting push stepped against an open cursor. All agree.

**Still untested, stated rather than glossed:** gap 14 (`inspect` is not ported, and its
constructor-name quirk -- confirmed on Node 24.18.1:
`new CircularBuffer(Array, 3).inspect().constructor.name === 'FixedDeque'` -- is therefore
neither reproduced nor contradicted), and the same three
divergence-shaped gaps as `fixed-deque` — D-65, D-61, D-62.

## Bugs this found

**B-62 — `#.get` is bounded by the capacity, and has no lower bound.** Inherited *literally*: the
`get` on `CircularBuffer.prototype` is the same function object as the one on
`FixedDeque.prototype`. See `docs/modules/fixed-deque.md` for the transcript. Confirmed on this
class: `new CircularBuffer(Array, 3)`, push 1 and 2, `pop()` — then `size === 1` and
`get(1) === 2`.

**B-60 — `from` on a non-array-like iterable throws.** Shared with the other two; see
`docs/modules/fixed-stack.md`. Confirmed here:
`CircularBuffer.from(new Set([1,2,3]), Array, 3)` is `TypeError: iterables.forEach is not a
function`.

**Not a bug, but the sharpest observation in this file: `from` bypasses the class's whole purpose.**
`CircularBuffer.from` is the same fourteen lines as `FixedStack.from` and `FixedDeque.from`, so its
array-like branch copies by index and assigns `size` rather than pushing. An oversized iterable
therefore leaves a `CircularBuffer` in a state its own `push` can never produce —
`size > capacity`, elements repeating on the walk. Verified on Node 24.18.1:
`CircularBuffer.from([1,2,3], Array, 2)` gives `size 3`, `start 0`, `items [1,2,3]`,
`toArray() [1, 2, 1]`. Filed under B-60's umbrella rather than as its own ID, because the shared
`from` is one piece of code with two problems and the second is arguably a documentation gap rather
than a defect.

**What the fuzzer found: nothing new.** Two campaigns, 3.10 M operations, zero divergences — the
expected outcome (D-33).

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **`CircularBuffer` holds a `FixedDeque` rather than duplicating the ring.** | Upstream's own mechanism is `Object.keys(FixedDeque.prototype).forEach(paste)` — the pasted methods are the *same function objects*, not copies. Delegation is the closest Rust equivalent, and it makes "every `FixedDeque` defect is a defect here" structural rather than coincidental. Two copies of a wrap would be two places to get it wrong. |
| — | **`push` and `unshift` return `usize`, not `Result<usize>`.** | Neither can fail here, which is the entire difference from `FixedDeque`. Encoding that in the type is what stops a caller from handling an error that cannot happen. |
| — | **`inspect()` is not ported**, and with it the quirk that the pasted `inspect` reports the constructor as `FixedDeque`. | A Node display convenience with no upstream assertion. Noted rather than reproduced. |
| D-65 | **`get` with a non-numeric index returns `undefined`.** | See `docs/modules/fixed-deque.md`. |
| D-60, D-61, D-62, D-63, D-64 | See `docs/modules/fixed-stack.md`. | Shared by all three fixed-capacity modules. |
| D-06, D-07, D-39, D-43 | See `docs/modules/fixed-stack.md`. | Cursor and bridge decisions, shared repo-wide. |

## Fuzz + bench

### Fuzz

```
module=circular-buffer seed=42       cases=18564 ops=1899965 wall=90.0s divergences=0
module=circular-buffer seed=20260801 cases=11652 ops=1203311 wall=60.0s divergences=0
```

Two campaigns, two seeds, **3.10 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module circular-buffer --seed 42 --cases 18564`.

* **Op alphabet:** `push(v)` (weight 5) · `unshift(v)` (3) · `pop()` (2) · `shift()` (2) ·
  `peekFirst()` (1) · `peekLast()` (1) · `get(i)` (2) · `clear()` (1) · `$iter("values")` (2) ·
  `$next()` (4) · `$spread()` (1) · `$forEach(mutation, at)` (3).
* **Observable state, compared after every op:** `size`, `capacity`, `start`, `items`, `toArray()`.
* **Neither insert can throw here**, so a generated program never stops growing and spends almost
  all of its length *past* the capacity, where every insert overwrites and `start` walks. With
  capacities of 1..=8 and 200-op programs, a program wraps the ring tens of times. That is the
  contrast with the `fixed-deque` grammar, whose programs stall at the ceiling.
* **The return value of every insert is compared**, which is what pins gaps 1 and 2 — the only
  visible signal that an element was dropped.
* **Both backing classes**, `get` indices 0..=11, values to 320.
* **Deliberately excluded:** the same three as `fixed-deque` — `from` (a static), `forEach`'s
  `scope` (D-61), and a negative `get` index (core takes a `usize`; covered by differential probes).

**The grammar was falsified before being trusted.** Sabotage: `push` returning `size + 1` when it
overwrites — reading upstream's `return this.size` as `return ++this.size`, which is what the
non-overwriting branch two lines below actually does. Caught in **497 cases (0.4 s)**, shrunk to
three lines:

```js
var s = new CircularBuffer(Array, 1);
s.push(0); s.push(0);      // port 2, upstream 1
```

Reverted; the seed is committed with provenance in
`crates/difffuzz/proptest-regressions/circular-buffer.txt`. Note what this sabotage would have
survived: the entire original suite, which never asserts an overwriting insert's return value.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`should be possible to wrap buffer around when pushing.` —
`assert.deepStrictEqual(buffer.toArray(), [2, 3, 4])`, at `test/circular-buffer.js:46`. Chosen
because it is the first assertion in the file that reaches an *overwriting* push, which is the only
code this module adds to `FixedDeque`.

**The sabotage:** the overwriting branch of `push` no longer advancing `start` — it still writes the
slot and still returns the unchanged size, so the buffer keeps its capacity and its size, and only
the oldest element is wrong.

**Confirmed red**, and red in precisely the named place: `16 passing, 2 failing`, the first failure
being that assertion with `actual` `[4, 2, 3]` against `expected` `[2, 3, 4]`. Reverted;
**confirmed green again**: `18 passing`.

### Bench

**Not run.** Gate 10 requires an idle machine (DESIGN.md §7.3) and this unit was ported while other
agents were working. `bench/results.json` has no `circular-buffer` entry and `tests/scope.txt` does
not list this unit, which is the honest state rather than an oversight. Gate 10 is batched into the
quiet serial pass; the unit is complete through gates 1–9.
