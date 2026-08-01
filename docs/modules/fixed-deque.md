# fixed-deque

Upstream: `fixed-deque.js` (357 LOC) + `utils/iterables.js` (93 LOC, `guessLength` and `isArrayLike`
reachable) + `obliterator/iterator` · `test/fixed-deque.js` — **281 lines, 16 `it` blocks, 77
assertion statements**.

Port: `crates/mnemonist-core/src/structures/fixed_deque.rs`,
`crates/mnemonist-core/src/structures/backing.rs`, `crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/fixed_deque.rs`, `crates/mnemonist-napi/src/array_class.rs`,
`crates/mnemonist-napi/src/iterables.rs`.

A ring over a pre-allocated array. `start` is where the first element lives and logical position
`j` is at `items[(start + j) mod capacity]` — except that upstream does not write it that way, and
the difference is observable. See "Deliberate divergences".

---

## What upstream tests

Sixteen `it` blocks, the most thorough test file of the three in this wave. It genuinely covers the
ring: `should handle tricky situations.` interleaves `push`, `unshift`, `pop` and `shift` on a
capacity-6 deque and asserts `start` directly, and `should be consistent over time.` walks a
capacity-3 deque through eleven operations checking `toArray()` after each phase.

Characterising the shape of that coverage:

* **`start` is asserted twice**, at lines 151 and 274 — the only module in the wave whose internal
  geometry the suite inspects.
* **Both capacity throws are covered**, `push` and `unshift`, in the same block.
* **`get` is called four times**, all on a *full* capacity-3 deque: `get(0..2)` and `get(3)`. That
  is the whole of its coverage, and it is exactly the shape in which B-62 is invisible.
* **`forEach` is called once**, on a full capacity-3 deque, with no mutation and no `scope`.
* **Three array classes** — `Array`, `Uint8Array`, `Float64Array`, plus `Int8Array` as an input.
* **Every `from` call passes an array or typed array**, and none is oversized.
* **Iterators are drained immediately**, with no mutation in between.

## What upstream does NOT test

**`#.get` — the guard**

1. **`get(index)` for `size <= index < capacity` is never called.** The guard is
   `index >= this.capacity`, not `index >= this.size`, so it returns whatever is in the slot —
   debris a `pop` or `shift` left behind. See B-62.
2. **`get` with a negative index is never called.** There is no lower bound at all, so `get(-1)` on
   a deque with `start === 2` returns the element at physical slot 1 — an element that was shifted
   out. See B-62.
3. **`get` with a fractional, `NaN` or non-numeric index is never called.**
4. **`get` on a wrapped deque is never called** — every `get` in the file is on a deque with
   `start === 0`.

**`forEach`**

5. **`forEach` on a non-full, empty, or wrapped deque is never called.**
6. **A callback that mutates the deque is never used**, so nothing pins the fact that `capacity`,
   `size` and `start` are frozen at entry while `this.items` is read live — a `shift` inside the
   callback moves the deque's start and *not* the walk's.
7. **The `scope` argument is never passed.**

**Removals leave debris**

8. **Nothing asserts that `pop`, `shift` and `clear` write nothing to the array.** All three move
   only the geometry, so every removed element stays reachable — through `items`, and through
   `get` (gap 1).

**`from`**

9. **`from` is never called with a non-array-like iterable** — that branch is a `TypeError`. See
   B-60.
10. **`from` is never called with an iterable longer than the capacity**, which leaves
    `size > capacity` and makes the walk go round the ring *more than once*, repeating elements.
11. **`from` is never called with an unguessable iterable and no capacity.**

**The array class**

12. **Element coercion is never asserted.** `push(300)` into a `Uint8Array` deque stores `44`.
13. **A non-constructor `ArrayClass` is never passed.**
14. **A fractional, `NaN` or infinite capacity is never passed**, and the three classes behave
    differently there.

**Iteration**

15. **A cursor is never re-drained** (D-06), and **`[...deque]` is never used** except through one
    `for…of` (D-07).
16. **Mutation during iteration is never performed** (D-08), in either direction — neither an
    element overwritten ahead of the cursor nor a `shift` moving the deque's start under it.
17. **`values()` on an empty or cleared deque is never called.**
18. **`entries()` is called once, on a fresh deque**, so its own `j` counter is never observed
    diverging from the physical index — which it does on any wrapped deque.

**Never called at all**

19. `inspect()` and the `nodejs.util.inspect.custom` symbol.

## What we test in addition

`crates/mnemonist-core/src/structures/fixed_deque.rs` — **17 tests**; the sixteen substantive
ones are below, plus a `Debug` smoke test:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all sixteen upstream blocks |
| `get_is_bounded_by_the_capacity_and_returns_debris_below_it` | 1 — for both backing classes, so the `undefined` and the class-zero halves are both pinned |
| `get_past_the_size_wraps_around_to_the_shifted_element` | 1, 4 — the debris is the *wrapped* slot, not a stale tail |
| `removals_leave_the_elements_in_place` | 8 |
| `a_refused_insert_leaves_the_deque_untouched_and_names_its_method` | — the two messages differ by method name |
| `an_oversized_from_walks_the_ring_more_than_once` | 10 — `[1,2,1,2]`, and the single conditional subtraction in `pop` |
| `an_oversized_from_is_truncated_by_a_typed_class` | 10 |
| `cursors_do_not_restart_but_the_deque_can_be_walked_again` | 15 |
| `a_push_during_iteration_is_not_visible_to_the_cursor` | 16 |
| `a_shift_during_iteration_does_not_move_the_cursor` | 6, 16 — the frozen-`start` half, which is this module's sharpest cursor behaviour |
| `an_overwrite_ahead_of_the_cursor_is_visible` | 16 |
| `a_wrapped_deque_walks_front_to_back` | 4, 18 |
| `unshift_from_the_zero_start_wraps_to_the_last_slot` | — the `start === 0` wrap, reached here from an *empty* deque rather than a full one |
| `a_capacity_of_one_and_an_empty_deque_both_behave` | 17 |
| `from_array_like_accepts_any_iterator` | D-03 |
| `error_text_is_upstreams` | — the message constants, verbatim |

**Differential probes against the vendored upstream**, 23 cases for this class (50 across both this
and `circular-buffer`), recorded here because they are the evidence for the bridge half: B-60 for a
`Set` and a generator; B-62 in both its forms including `get(-1)` and `get(-2)`; `get` with `1.5`,
`NaN`, `Infinity` and `undefined`; coercion for `Uint8Array` and `Int8Array`; `toArray` on a wrapped
deque; `forEach`'s three arguments and its `this`; `forEach` on an empty deque; a `forEach` whose
callback shifts twice; oversized `from` for both backings; all five constructor error paths;
`new FixedDeque(Array, 2.5)`; `[...d]` twice; a cursor re-drained; `break` then `next()`;
`entries()`; and a cursor stepped across a `shift`. All agree.

**Still untested, stated rather than glossed:** gap 19 (`inspect`, not ported), gap 3 in its
non-numeric form (a deliberate divergence, D-65), gap 7 in its `arguments.length` form (D-61), and
gap 14 for typed classes (D-62).

## Bugs this found

**B-62 — `#.get` is bounded by the capacity, and has no lower bound at all.**
`status: verified against Node 24.18.1`. Every reader in the file guards on `this.size`. `get`
guards on the capacity:

```js
FixedDeque.prototype.get = function (index) {
  if (this.size === 0 || index >= this.capacity) return;
  index = this.start + index;
  if (index >= this.capacity) index -= this.capacity;
  return this.items[index];
};
```

Two consequences, both measured:

```js
var d = new FixedDeque(Array, 3); d.push(1); d.push(2); d.pop();
d.size;    // 1
d.get(1);  // 2      <- popped, still returned
d.get(3);  // undefined, because 3 >= capacity — the one guard that fires

var e = new FixedDeque(Array, 4);
[1,2,3,4].forEach(function (v) { e.push(v); });
e.shift(); e.shift();     // start === 2, holding [3, 4]
e.get(-1);  // 2          <- shifted out, still returned
e.get(-2);  // 1
```

The original suite's four `get` calls are all on a *full* capacity-3 deque, which is the single
shape in which "bounded by the capacity" and "bounded by the size" are the same statement.

`CircularBuffer` inherits this literally — upstream pastes the same function object onto its
prototype — so the defect is one bug in two classes.

**B-60 — `from` on a non-array-like iterable throws.** Shared with `fixed-stack` and
`circular-buffer`; see `docs/modules/fixed-stack.md` for the full write-up. Confirmed for this
class: `FixedDeque.from(new Set([1,2,3]), Array, 3)` is
`TypeError: iterables.forEach is not a function`.

**What the fuzzer found: nothing new.** Two campaigns, zero divergences — the expected outcome
(D-33). Both bugs were found by reading the file statement by statement against Node.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-65 | **`get` with a non-numeric index returns `undefined`.** | Upstream reaches *string concatenation*: `this.start + "1"` is `"21"`, which the next comparison coerces back to a number, so on a large enough deque `get("1")` can return a real element at physical slot 21. The port refuses at the type check. Everything numeric — negative, fractional, `NaN`, infinite — is reproduced exactly. |
| — | **`toArray()` produces the fast path's answer for a missing slot.** | Upstream has two paths: `items.slice(start, offset)` when `start + size < capacity`, which preserves a hole, and a slow path whose `array[j] = undefined` creates an own property. The port always produces the hole. Observable only through `in`/`hasOwnProperty`, and the elements are identical either way. |
| — | **The wrap is one conditional subtraction, not `%`.** | Not a divergence — a *choice to reproduce* one. The two agree while `start + size < 2 * capacity`, which every path but an oversized `from` maintains; where they disagree, upstream keeps the out-of-range index and reads whatever is there. Writing `%` would have been the tidier code and the wrong answer. `values()` genuinely *is* `%`, because its loop steps and wraps on equality. |
| — | **`items` is not exposed to JS.** | A public property upstream that a JS caller can write *through*; napi can only hand out a copy. Same call as `SparseSet`, `HashedArrayTree` and `FixedStack`. Exposed in Rust and compared slot for slot by the fuzzer. |
| D-60, D-61, D-62, D-63, D-64, D-66 | See `docs/modules/fixed-stack.md`. | The `iterables`, arity, capacity, `ArrayClass`, B-60 and B-63 decisions are shared by all three fixed-capacity modules -- they live in one `from_parts`. |
| D-06, D-07, D-39, D-43 | See `docs/modules/fixed-stack.md`. | Cursor and bridge decisions, shared repo-wide. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |

## Fuzz + bench

### Fuzz

```
module=fixed-deque seed=42       cases=16921 ops=1730822 wall=90.0s divergences=0
module=fixed-deque seed=20260801 cases=11127 ops=1149893 wall=60.0s divergences=0
```

Two campaigns, two seeds, **2.88 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module fixed-deque --seed 42 --cases 16921`.

* **Op alphabet:** `push(v)` (weight 5) · `unshift(v)` (3) · `pop()` (2) · `shift()` (2) ·
  `peekFirst()` (1) · `peekLast()` (1) · **`get(i)` (2)** · `clear()` (1) · `$iter("values")` (2) ·
  `$next()` (4) · `$spread()` (1) · `$forEach(mutation, at)` (3).
* **Observable state, compared after every op:** `size`, `capacity`, **`start`**, `items`,
  `toArray()`. `start` is in the set because the upstream file asserts on it and because it is the
  one number a wrong wrap moves first.
* **`get` indices run 0..=11 against capacities of 1..=8**, so both clauses of B-62's guard are
  exercised constantly: past the size (debris) and past the capacity (the guard that fires).
* **Both backing classes**, capacities 1..=8, values to 320.
* **Deliberately excluded:** `from` (a static cannot appear in an op sequence; covered by the
  original test and the differential probes), `forEach`'s `scope` (D-61), and a **negative** `get`
  index — the fuzzer drives `mnemonist-core`, whose `get` takes a `usize`, and the negative path is
  the bridge's. It is covered by four differential probes instead, and this exclusion is the reason
  they are recorded above rather than left as scratch work.

**The grammar was falsified before being trusted.** Sabotage: `get`'s guard changed from
`index >= self.capacity` to `index >= self.size` — the "obvious correction" of B-62, and the change
any reader who has not checked upstream would make. Caught in **823 cases (1.2 s)**, shrunk to five
lines:

```js
var s = new FixedDeque(Array, 2);
s.push(0); s.push(0);
s.forEach(function (v, i) { if (i === 0) s.pop(); });
s.get(1);        // port undefined, upstream 0
```

Reverted; the seed is committed with provenance in
`crates/difffuzz/proptest-regressions/fixed-deque.txt`.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`should be possible to unshift the deque.` — `assert.strictEqual(deque.start, 3)`, at
`test/fixed-deque.js:151`. Chosen because `start` is the one piece of internal geometry the file
inspects directly, so it is the assertion that most specifically exercises the ring rather than its
results.

**The sabotage:** `previous_start()` returning `self.start.saturating_sub(1)` instead of wrapping to
`capacity - 1` when `start` is zero — dropping the one line that makes `unshift` a ring operation
rather than a bounded one.

**Confirmed red**, and red in precisely the named place: `13 passing, 3 failing`, and the second
failure is that assertion, at `test/fixed-deque.js:151`, with `actual` `0` against `expected` `3`.
The other two are `should be possible to pop the deque.` and `should handle tricky situations.`,
which both reach `unshift` on a deque whose `start` is zero. Reverted; **confirmed green again**:
`16 passing`.

### Bench

**Not run.** Gate 10 requires an idle machine (DESIGN.md §7.3) and this unit was ported while other
agents were working. `bench/results.json` has no `fixed-deque` entry and `tests/scope.txt` does not
list this unit, which is the honest state rather than an oversight. Gate 10 is batched into the
quiet serial pass; the unit is complete through gates 1–9.
