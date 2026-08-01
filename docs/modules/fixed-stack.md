# fixed-stack

Upstream: `fixed-stack.js` (242 LOC) + `utils/iterables.js` (93 LOC, `guessLength` and
`isArrayLike` reachable) + `obliterator/iterator` · `test/fixed-stack.js` — **157 lines, 12 `it`
blocks, 33 assertion statements**.

Port: `crates/mnemonist-core/src/structures/fixed_stack.rs`,
`crates/mnemonist-core/src/structures/backing.rs`, `crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/fixed_stack.rs`, `crates/mnemonist-napi/src/array_class.rs`,
`crates/mnemonist-napi/src/iterables.rs`.

`FixedStack` is `Stack` with a bound, and everything interesting about it is in the bound — and in
the fact that the backing array is a **caller-supplied JavaScript class**, allocated once and never
resized. Two upstream defects fall out of that, one of which the original suite is structurally
unable to see.

---

## What upstream tests

Twelve `it` blocks, all against stacks of capacity 1, 2, 3, 5, 10 or 45:

```js
new FixedStack(Array, 10);                     // push, size, capacity
new FixedStack(Array, 1);  …push twice         // the capacity throw
new FixedStack(Uint8Array, 3); …push 1,2,3     // toArray() deepStrictEqual + instanceof
FixedStack.from([1, 2, 3], Array);             // guessLength path
FixedStack.from([1, 2, 3], Uint8Array, 45);    // values()
FixedStack.from([1, 2, 3], Float64Array, 5);   // entries()
FixedStack.from(Int8Array.from([1,2,3]), Array); // for…of
```

Characterising the shape of that coverage:

* **Four array classes are used** — `Array`, `Uint8Array`, `Float64Array` and (as an *input*)
  `Int8Array`. That is more class coverage than any other module in the repo, and it is the reason
  the bridge had to solve `ArrayClass` properly rather than whitelisting one width.
* **The `forEach` block builds a capacity-3 stack and pushes exactly three items.** This is the
  single shape in which `items.length === size`, and it is the only shape the file's `forEach`
  coverage takes. See B-61.
* **Every `from` call passes an array or a typed array.** The other branch of `from` is never
  reached. See B-60.
* **No `from` call is oversized.** `capacity` is always ≥ the iterable's length, so `size` never
  runs past the array.
* **Iterators are drained immediately.** `iterator.next()` four times in a row, with no mutation in
  between, so no cursor state is ever observed.
* **`peek` on an empty stack is asserted** (`undefined`), which is one of the few edge cases the
  file does cover.

## What upstream does NOT test

This is the section that carries the weight. Everything below is reachable through the public API
and never exercised by the original suite.

**`forEach` — the whole of it, except one degenerate shape**

1. **`forEach` on a stack that is not exactly full is never called.** Its loop bound is
   `this.items.length`, not `this.size`, so an under-full stack invokes the callback *capacity*
   times and the first `capacity - size` calls receive unused slots. See B-61. The one block that
   calls `forEach` builds `size === capacity`, where the defect is invisible.
2. **`forEach` on an empty stack is never called** — which on a capacity-5 stack means five
   callback invocations, all with `undefined`.
3. **`forEach` after a `pop` or a `clear` is never called**, so the debris those leave behind is
   never observed.
4. **The `scope` argument is never passed**, so the `arguments.length > 1 ? scope : this` branch is
   untested.
5. **A callback that mutates the stack is never used**, so nothing pins the fact that `this.items`
   is re-read on every iteration while the bound is not.

**`from` — the branch that cannot work**

6. **`from` is never called with a non-array-like iterable.** A `Set`, a `Map`, a generator or a
   string all reach `iterables.forEach`, which **does not exist**. See B-60. This is a `TypeError`
   on every version of the library that has this file.
7. **`from` is never called with an iterable longer than the capacity**, which is the only way
   `size` runs past the backing array, and it behaves *differently per array class*.
8. **`from` is never called with an unguessable iterable and no capacity** — the
   `could not guess iterable length` throw is untested.

**`clear` and `pop` do not clear**

9. **Nothing asserts that `clear()` leaves the elements in the array.** It sets `size = 0` and
   nothing else, so every element stays reachable through `items` and through `forEach`.
10. **Nothing asserts that a `push` after a `clear` overwrites slot 0** rather than appending.
11. **`pop`'s debris is likewise never observed.**

**The array class**

12. **Element coercion is never asserted.** Every value the suite pushes is a small non-negative
    integer that survives every class unchanged. `push(300)` into a `Uint8Array` stack stores `44`;
    `push(200)` into an `Int8Array` stores `-56`. Neither is tested.
13. **`toArray()`'s class is asserted once** (`instanceof Uint8Array`) but its *contents* for a
    stack whose `size` exceeds the storage are not.
14. **A non-constructor `ArrayClass` is never passed**, so the `this.ArrayClass is not a
    constructor` path is untested.
15. **A fractional, `NaN` or infinite capacity is never passed.** These behave *differently per
    class*: `new FixedStack(Array, 2.5)` throws `RangeError: Invalid array length`, while
    `new FixedStack(Uint8Array, 2.5)` succeeds with `capacity === 2.5` and `items.length === 2`.

**Iteration**

16. **A cursor is never re-drained**, so the non-restartability of D-06 is unobserved.
17. **`[...stack]` is never used.** The suite reaches the cursor through `values()`, `entries()` and
    one `for…of`, so the *factory* half of D-07 has coverage only through the `for…of`.
18. **Mutation during iteration is never performed**, so neither half of the hybrid capture (D-08)
    is tested.
19. **`values()` on an empty stack, or after a `clear`, is never called.**
20. **The `undefined` shrink window is never reached** — it needs (7), which needs (6)'s sibling
    branch.

**Never called at all**

21. `toString()`, `toJSON()`, `inspect()` and the `nodejs.util.inspect.custom` symbol. About 25 LOC
    of the module.

## What we test in addition

`crates/mnemonist-core/src/structures/fixed_stack.rs` — 18 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twelve upstream blocks, as a baseline |
| `for_each_walks_the_capacity_and_not_the_size` | 1, 2 — pinned for both classes, against Node |
| `for_each_agrees_with_values_only_on_a_full_stack` | 1 — asserts the two bounds *disagree* after one `pop`, which is the thing the upstream file cannot see |
| `clear_and_pop_leave_the_elements_in_place` | 3, 9, 11 |
| `a_push_after_clear_reuses_the_array_from_the_bottom` | 10 |
| `a_refused_push_leaves_the_stack_untouched` | — the guard runs before the store |
| `from_an_oversized_array_like_overflows_a_plain_array` | 7 — `Array` grows past its own capacity |
| `from_an_oversized_array_like_is_truncated_by_a_typed_class` | 7, 13 — the same call, opposite outcome |
| `a_truncated_from_makes_the_cursor_yield_undefined` | 20 — the shrink window, reached through public calls |
| `cursors_do_not_restart_but_the_stack_can_be_walked_again` | 16, 17 |
| `a_push_during_iteration_is_not_visible_to_the_cursor` | 18 — the frozen half |
| `a_pop_during_iteration_still_yields_the_popped_element` | 18 — and the contrast with `Stack`, where `pop` opens a gap |
| `an_overwrite_ahead_of_the_cursor_is_visible` | 18 — the live half |
| `a_clear_during_iteration_is_invisible_because_clear_does_nothing_to_the_array` | 18 |
| `a_capacity_of_one_and_an_empty_stack_both_behave` | 19 |
| `from_array_like_accepts_any_iterator` | D-03 |
| `duplicates_are_kept` | — a stack is not a set |
| `error_text_is_upstreams` | — the three message constants, verbatim |

`crates/mnemonist-core/src/structures/backing.rs` — 4 tests pinning the two bits the array class
reduces to, which is what gaps 7 and 13 turn on.

`tests/boundary/iterables.js` — 19 specs for the `utils/iterables` half of the closure, which has
**no upstream test file at all**: `guessLength`'s refusal to validate, `toArray`'s holes (B-2),
`isArrayLike` saying no to `{length: 2}`, and `getPointerArray` throwing before `new Array(l)` does.

**Differential probes against the vendored upstream**, 28 cases, recorded here because they are the
evidence for the bridge half and are not otherwise visible: B-60 for `Set` and for a string; B-61
for both classes; coercion for `Uint8Array`/`Int8Array`/`Float64Array`; `toArray`'s class;
oversized `from` for both classes; all five constructor error paths; `toString`; `toJSON`;
`[...s]` twice; a cursor re-drained; `break` then `next()`; a mutating `forEach`; and
`new FixedStack(Object, 3)` — where upstream produces a `Number` object carrying a `'0'` property,
and so does the port. All 28 agree.

**Still untested, stated rather than glossed:** gap 21 (`inspect`, not ported — a Node display
convenience with no upstream assertion), gap 4 in its `arguments.length` form (see the divergence
table), and gap 15 for typed classes, which is a deliberate divergence rather than a gap.

## Bugs this found

**B-60 — `X.from(iterable, ...)` calls `iterables.forEach`, which does not exist.**
`status: verified against Node 24.18.1`. `utils/iterables.js` exports exactly four functions —
`isArrayLike`, `guessLength`, `toArray`, `toArrayWithIndices` — and no `forEach`. All three
fixed-capacity modules end their `from` static with

```js
iterables.forEach(iterable, function (value) { stack.push(value); });
```

so the branch that would handle a `Set`, a `Map`, a generator or a string is not a slow path, it is
a `TypeError`:

```js
FixedStack.from(new Set([1, 2, 3]), Array, 3)
// TypeError: iterables.forEach is not a function
```

Confirmed for `FixedStack`, `FixedDeque` and `CircularBuffer`. The suite never reaches it because
every `from` call in all three test files passes an array or a typed array, which takes the
array-like fast path and returns before the last line. The fix upstream would be one character —
`iterables.forEach` → the `obliterator/foreach` these files already have a sibling of — which is
what makes the age of the defect notable: the branch has never run.

Reproduced rather than repaired (D-64). A port that quietly made it work would pass every upstream
test and be a different library.

**B-61 — `FixedStack.prototype.forEach` walks `items.length`, not `this.size`.**
`status: verified against Node 24.18.1`. Every other method in the file is written against
`this.size`; `forEach` alone is written against the array's length, which is the capacity:

```js
for (var i = 0, l = this.items.length; i < l; i++)
  callback.call(scope, this.items[l - i - 1], i, this);
```

So an under-full stack hands the callback its unused slots — as `undefined` from an `Array`, as `0`
from a `Uint8Array` — before any real element:

```js
var s = new FixedStack(Array, 5); s.push(1); s.push(2);
s.forEach(function (v, i) { … });
// (undefined, 0) (undefined, 1) (undefined, 2) (2, 3) (1, 4)
```

`FixedDeque.prototype.forEach`, three files away, does it correctly (`l = this.size`), which is
what makes this a slip rather than a design choice. The original suite is *structurally* unable to
see it: its one `forEach` block builds a capacity-3 stack and pushes three items, the single shape
in which the two bounds agree.

This is also the sharpest available illustration of why gate 6 is worth its cost. The most
plausible mis-port of this module — reading `this.items.length` as "the number of items" and
writing `self.size` — **stays green through the entire original suite**. It was caught in 57 fuzz
cases once the grammar had a `forEach` op, and by two native tests written from the source rather
than from the tests.

**What the fuzzer found: nothing new.** 2.81 M operations, zero divergences. That is the expected
outcome (D-33): a faithful port reproduces upstream's bugs, so differential fuzzing structurally
cannot find them. Both bugs above were found by reading the file statement by statement and
confirming each step against Node. What the fuzzer is for is the other direction, and it was proven
to work in that direction twice — see Fuzz, below.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-60 | **`toArray`'s sparse-array behaviour is reproduced, not fixed** (resolves the PROPOSED D-17). | The array is really allocated by calling the realm's own `Array` constructor, so an overstated `guessLength` leaves real holes and an invalid one throws V8's own `RangeError`. `napi_create_array_with_length` would have differed on both. |
| D-61 | **An omitted argument and an explicit `undefined` are the same thing.** | napi generates `CallbackInfo::new(.., None, ..)`, so it does not enforce arity and a missing argument arrives as `undefined`. `new FixedStack(Array, undefined)` therefore raises upstream's *arity* error where upstream raises its *capacity* error. `null` is distinguished correctly — the parameters are `Unknown`, not `Option<Unknown>`, precisely because napi maps `null` to `None` and `new FixedStack(Array, null)` must throw about the number. |
| D-62 | **A fractional, `NaN` or infinite capacity always raises `RangeError: Invalid array length`.** | Upstream passes the raw number to the class and lets it decide, so `new FixedStack(Array, 2.5)` throws while `new FixedStack(Uint8Array, 2.5)` succeeds with `capacity === 2.5` against an `items.length` of 2 — after which the wrap arithmetic compares indices against 2.5. The port requires an integral capacity and raises the `Array` form for every class. The `Array` case is exact; the typed case is the divergence. |
| D-63 | **The array class is probed, not listed.** | Coercion is `scratch[0] = v; scratch[0]` through a real one-element instance of the caller's class, and the backing is decided by `0 in new ArrayClass(1)`. That is exact for every array class, including ones nobody has written yet, where a name whitelist (the `hashed-array-tree` approach) diverges for nine of the twelve built-in typed arrays. **Cost, stated:** two extra one-element constructions of the caller's class per structure, invisible for `Array` and the typed arrays and observable for a constructor with side effects. |
| D-64 | **B-60 is reproduced.** | See above. The `TypeError` is raised with V8's exact wording, from the same point in the sequence — after `guessLength`, after the capacity guards, after `isArrayLike` says no. |
| — | **`items` is not exposed to JS.** | It is a public property upstream and a JS caller can write *through* it; napi can only hand out a copy, which would silently break the write-through. Same call as the `SparseSet` and `HashedArrayTree` bridges. It is exposed in Rust, and the differential fuzzer compares it slot for slot after every operation. |
| — | **`toArray()`'s hole-vs-`undefined` distinction is the fast path's.** | Upstream writes every index of the result explicitly, so a missing slot is an own `undefined` property rather than a hole; the port leaves it unwritten, which is a hole in an `Array` and the class zero in a typed array. The two differ only under `in`/`hasOwnProperty`, and for `FixedStack` the case is unreachable anyway — every index below `size` that a plain `Array` can hold has been written. Reachable for `FixedDeque`, where it is the same call. |
| — | **`capacity` is a `usize` in core.** | Upstream's `capacity` is a JS number and can be non-integral (D-62). The bridge refuses that, so the core type is exact for everything the bridge admits. |
| — | **A huge capacity really allocates.** | `new Array(1e9)` upstream is a cheap hole array; the port allocates `1e9` slots. No test or benchmark reaches it, and a lazy representation would complicate `Backing` for a case upstream's own `items` property makes visible anyway. Stated, not fixed. |
| D-06 | **No collection implements `IntoIterator`.** | It would hand out a fresh iterator per `for` loop and silently restart, where upstream's cursor continues from where it stopped. |
| D-07 | **`Symbol.iterator` is installed from Rust, not from the shim.** | The factory half is the one napi does not provide. `require('@port/addon').FixedStack` is spreadable on its own. |
| D-39 | **`Yield` is `Either<JsSlot, Undefined>`, not `Option<JsSlot>`.** | napi renders `None` as `null`, and the shrink window needs a real `undefined`. |
| D-43 | **`inner` is a `RefCell` from the first commit.** | `&self` on a `Freeze` type is `noalias readonly` to LLVM, which hoisted a read out of exactly this kind of loop once before (B-31). `forEach` here re-reads the array on every step and a callback may mutate, so the hazard is live in this module rather than hypothetical. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion and no Rust equivalent. |
| — | **`forEach(cb, undefined)` binds `this` to the stack.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see. The omitted case — the only one the original suite uses — is exact, and passing a real scope object is exact. |

## Fuzz + bench

### Fuzz

```
module=fixed-stack seed=42       cases=18277 ops=1802802 wall=90.0s divergences=0
module=fixed-stack seed=20260801 cases=10195 ops=1011340 wall=60.0s divergences=0
```

Two campaigns, two seeds, **2.81 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module fixed-stack --seed 42 --cases 18277`.

* **Op alphabet:** `push(v)` (weight 6) · `pop()` (3) · `peek()` (2) · `clear()` (2) ·
  `$iter("values")` (2) · `$next()` (4) · `$spread()` (1) · **`$forEach(mutation, at)` (3)**.
* **Observable state, compared after every op:** `size`, `capacity`, `items`, `toArray()`.
* **Both backing classes** — `Array` and `Uint8Array` — because they are not interchangeable.
  Capacities run 1..=8 so `push` hits the ceiling constantly; values run to 320 so the truncating
  store is exercised.
* **Deliberately excluded:** `from` in all its forms (a static cannot appear in an op sequence; it
  is covered by the original test and by the 28 differential probes above) and `forEach`'s `scope`
  argument (a documented divergence — fuzzing it would only re-report a known decision).

**`$forEach` is new to the harness, and it is the reason this grammar is worth more than its
predecessors.** Every earlier grammar drives iteration through `$iter`/`$next`, which is enough for
an obliterator *cursor* — a cursor freezes its state at creation. It is not enough for
`#.forEach`, which freezes only its loop bound and re-reads the backing array on every step, so a
callback that mutates the collection is visible to the reads after it. **B-31, this port's own
worst bug, was reachable only through a mutating `forEach` and survived 2.94 M operations because
no grammar had one.** The op takes a nullary non-throwing mutation and the callback index to fire
it at, and both sides record `[index, value, this === self]` per invocation.

**The fuzzer was falsified twice, once per half of the grammar.** Both reverted; seed A is
committed with provenance in `crates/difffuzz/proptest-regressions/fixed-stack.txt`.

**A — the `forEach` half.** Sabotage: `items_len()` returning `self.size`, which is the tidy-up a
naive port makes on noticing B-61. Caught in **57 cases (0.0 s)**, shrunk from 200 ops to two
lines:

```js
var s = new FixedStack(Array, 1);
s.forEach(function (v, i) { });     // port [], upstream [[0, undefined, true]]
```

**B — the cursor half.** Sabotage: `Sequence::freeze` capturing `items.len()` instead of
`self.size`, the mirror-image mistake. Caught in **57 cases (0.0 s)** on seed 4242:

```js
var s = new FixedStack(Array, 1);
s.values().next();                  // port {done: false}, upstream {done: true}
```

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications above: gate 6 asks that sabotaging the core turns the
*original mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create a values iterator.` —
`assert.strictEqual(iterator.next().value, 3)`, at `test/fixed-stack.js:128`. Chosen because it is
the first assertion in the file that reaches the cursor, which is the machinery this unit adds.

**The sabotage:** `Sequence::slot` for `FixedStack` reading `items[ordinal]` instead of
`items[l - ordinal - 1]` — dropping the LIFO reversal, which is the single most plausible way to
mis-port a walk whose ordinal is a step counter rather than an index.

**Confirmed red**, and red in precisely the named place: `9 passing, 3 failing`, the first failure
being that assertion with `actual` `1` against `expected` `3`; the other two are the `entries`
iterator and the `for…of` block, which reach the same code. Reverted; **confirmed green again**:
`12 passing`.

**Why not sabotage B-61.** The most plausible mis-port of this module is `items_len()` returning
`self.size`, and it was rejected as a gate-6 sabotage *before being run*, on the grounds that the
suite's only `forEach` block builds `size === capacity`. Confirmed by running it anyway: the
original suite stays fully green. That is the gate-6 lesson in its purest form — the sabotage that
matters most is the one the test file cannot see, which is why the falsification has to be chosen
by naming the assertion it must break rather than by picking the scariest-looking line.

### Bench

**Not run.** Gate 10 requires an idle machine (DESIGN.md §7.3) and this unit was ported while other
agents were working; a contended run inflated both sides 2–3× here once already. `bench/results.json`
has no `fixed-stack` entry and `tests/scope.txt` does not list this unit, which is the honest state
rather than an oversight. Gate 10 is batched into the quiet serial pass; the unit is complete
through gates 1–9.
