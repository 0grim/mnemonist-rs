# fixed-stack — evidence

Gate artifacts for `docs/modules/fixed-stack.md`: test-to-gap table, differential probe list, fuzz
grammar, full falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/fixed_stack.rs` — 19 tests (18 substantive plus a `Debug`
smoke test):

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

## `tests/boundary/iterables.js` — 19 specs

For the `utils/iterables` half of the closure, which has no upstream test file at all:
`guessLength`'s refusal to validate, `toArray`'s holes (B-2), `isArrayLike` saying no to
`{length: 2}`, and `getPointerArray` throwing before `new Array(l)` does.

## Differential probes against the vendored upstream — 28 cases

B-60 for `Set` and for a string; B-61 for both classes; coercion for
`Uint8Array`/`Int8Array`/`Float64Array`; `toArray`'s class; oversized `from` for both classes; all
five constructor error paths; `toString`; `toJSON`; `[...s]` twice; a cursor re-drained; `break`
then `next()`; a mutating `forEach`; `from` on a `DataView` (the one disagreement, D-66/B-63); and
`new FixedStack(Object, 3)` — where upstream produces a `Number` object carrying a `'0'` property,
and so does the port. All agree except the `DataView` case, which is D-66.

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 6) · `pop()` (3) · `peek()` (2) · `clear()` (2) ·
  `$iter("values")` (2) · `$next()` (4) · `$spread()` (1) · **`$forEach(mutation, at)` (3)**.
* **Observable state, compared after every op:** `size`, `capacity`, `items`, `toArray()`.
* **Both backing classes** — `Array` and `Uint8Array` — because they are not interchangeable.
  Capacities run 1..=8 so `push` hits the ceiling constantly; values run to 320 so the truncating
  store is exercised.
* **Deliberately excluded:** `from` in all its forms (a static cannot appear in an op sequence; it
  is covered by the original test and by the 28 differential probes above) and `forEach`'s `scope`
  argument (a documented divergence — fuzzing it would only re-report a known decision).

## Falsification record

### Fuzzer falsification

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

Both reverted; seed A is committed with provenance in
`crates/difffuzz/proptest-regressions/fixed-stack.txt`.

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
original suite stays fully green.

## Bench table

`bench/results.json` → `modules["fixed-stack"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peek`/`pop` (50/25/25), capacity 10,000 against 1e6 ops,
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4.2** | 7.1 | 1.7× faster |
| p99 ns/op | **5.5** | 11.7 | 2.1× faster |
| min ns/op | **3.8** | 6.7 | 1.8× faster |
| RSS delta MB | **6.2** | 19.1 | |
| structure-only RSS delta MB | **0.1** | 0.4 | |
| startup ms | **0.6** | 16.3 | 28× (reported separately; not throughput) |
