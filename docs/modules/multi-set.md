# multi-set

Upstream: `multi-set.js` (445 LOC) — depends on `fixed-reverse-heap.js` (209 LOC, already ported)
· `test/multi-set.js` — **361 lines, 26 `it` blocks, 83 assertion statements**.

Port: `crates/mnemonist-core/src/structures/multi_set.rs` — `MultiSet<K>`, a `Map` from item to
multiplicity (a JavaScript-numeric `f64` count, on purpose — see below). Bridge:
`crates/mnemonist-napi/src/multi_set.rs`. Shim: `tests/bridge/multi-set.js`. Fuzz spec:
`crates/difffuzz/src/modules/multi_set.rs`.

`test/multi-set.js` also `require`s `multi-map.js`, for its one `MultiSet.from(map)` case — already
ported as its own unit, so nothing extra was needed here beyond the sibling being present.

---

## What upstream tests

* **`add`**, including `count === 0` (no-op), a negative count (delegates to `remove`), and the
  default (`count` omitted, meaning `1`).
* **`remove`**, symmetrically: zero, negative (delegates to `add`), removing more than present
  (floors at zero and deletes), and removing zero items (a no-op).
* **`set`**, including that a non-positive count deletes the item entirely.
* **`has`**, **`multiplicity`/`get`/`count`** (all one method upstream), **`frequency`**, **`clear`**,
  **`delete`**.
* **`edit`**, three cases: `a` absent (no-op), `a` present and `b` new, `a` present and `b` already
  existing (multiplicities combine).
* **`forEach`** (each item repeated `multiplicity` times, `(value, value)` per call) and
  **`forEachMultiplicity`** (one call per distinct item, `(multiplicity, item)`).
* **Three iterator factories** — `keys()` (distinct items), `values()` (flattened, repeated),
  `multiplicities()` (`[item, count]` pairs) — plus `for...of` (`Symbol.iterator`, aliased to
  `values`).
* **`MultiSet.from(iterable)`**, from a plain array of repeated values and from a `MultiMap`.
* **Argument-type validation**: `add`/`set`/`remove` all throw `/number/` for a non-numeric count
  (`'56'`, a string that looks numeric but is not the `number` type).
* **`top(n)`**, against a real letter-frequency example, for two different `n`.
* **Issue #197's regression**: `size`/`dimension` stay at exactly `0` after redundant `add`/`remove`
  pairs followed by a `remove` on an item no longer present.
* **`isSubset`/`isSuperset`**, four multisets, six assertions each, including the `A === B` identity
  shortcut (implicitly, since `letters` is never compared against itself in the file — see "What
  upstream does NOT test").

## What upstream does NOT test

**`#.delete` on an item that was never in the set at all** — and this is the one that matters most.
Upstream's own guard is `if (count === 0) return false;`, where `count` is `this.items.get(item)` —
`undefined` for a missing item, and `undefined === 0` is `false`, so the guard **never actually
fires**: no live entry's multiplicity is ever exactly `0` (every method here deletes an item outright
rather than leaving a zero multiplicity behind). The fall-through does `this.size -= undefined`
(`NaN`), decrements `dimension` unconditionally, and still returns `true` — see "Bugs this found",
B-161.

**`#.edit(a, b)` where `b` already exists as a distinct key.** The file's own third `edit` case
(`set.add('c'); set.edit('b', 'c');`) exercises exactly this shape but never reads `.dimension`
afterwards — only `multiplicities()`. `edit` never touches `dimension` at all, even though a real key
(`b`, absorbed into... no, `a`, deleted) disappears from `items`. See "Bugs this found", B-162.

**`#.set` called twice on the same key with two *positive* counts.** The file's own double-`.set`
case (`set.set('hello', 4); set.set('hello', -34);`) follows a positive `set` with a
**non-positive** one, which takes the early delete branch — never two positive calls in a row. See
"Bugs this found", B-160.

**A fractional or `NaN` count**, from a real JS `number`. `typeof count !== 'number'` is upstream's
only type guard; it never checks integrality. `add('a', 1.5)` is legal upstream and leaves
`size`/multiplicity fractional. See `MultiSet`'s module docs and "What we test in addition".

**`isSubset`/`isSuperset` with `A === B`** (the identical object passed twice). `isSubset`'s own
short-circuit (`if (A === B) return true;`) is real code with no assertion exercising it in the
original suite.

## What we test in addition

`crates/mnemonist-core/src/structures/multi_set.rs` — 16 tests, covering every upstream block as a
baseline plus B-161 (deleting an absent item corrupting `size`/`dimension` while reporting `true`),
B-162 (`edit` into an existing key not adjusting `dimension`), B-160 (`set` on an existing item
adding rather than replacing), and the `A === B` identity shortcut for `isSubset`/`isSuperset`.

`fold_falsy`'s `NaN`-folds-to-`1` behaviour and the fractional-count permissiveness are exercised
indirectly by every `add`/`remove`/`set` test's own `f64` counts (all are already whole numbers by
construction, so this is a documented rather than a directly-pinned gap — see "Deliberate
divergences").

**Differential fuzzer** — a three-item pool over `add`/`remove`/`set`/`edit`/`delete`, small counts
including zero and negative; see "Fuzz + bench" for the measured numbers and a `grammar_self_check`
that counts multiplicity-above-one and drain-to-zero states directly.

## Bugs this found

Three upstream defects, all confirmed by reading (each is a straightforward consequence of the
source, not a runtime ambiguity), none reachable through gate 4 alone.

### B-160 — `#.set` on an existing item **adds**, it does not replace

`multi-set.js`'s `set`:

```js
MultiSet.prototype.set = function(item, count) {
  ...
  currentCount = this.items.get(item);
  if (typeof currentCount === 'number') {
    this.items.set(item, currentCount + count);   // added, not replaced
  } else {
    this.dimension++;
    this.items.set(item, count);
  }
  this.size += count;
  return this;
};
```

A method named `set` reads as "make the multiplicity exactly `count`" — `set.set('hello', 4)` then
`set.set('hello', 3)` gives `multiplicity('hello') === 7`, not `3`. `test/multi-set.js`'s own two
`.set` cases never call it twice on the same key with two positive counts (its double-call case
follows a positive `set` with a non-positive one, which takes the early delete branch instead), so
gate 4 cannot see this. Reproduced faithfully in `MultiSet::set` — a "corrected" replace-semantics
version would be *more correct than upstream* and therefore wrong per this port's bug-for-bug
fidelity rule. Pinned by `set_replaces_a_missing_item_but_adds_to_an_existing_one`.

### B-161 — `#.delete` on an absent item corrupts `size` to `NaN`, decrements `dimension`, and reports `true`

`multi-set.js`'s `delete`:

```js
MultiSet.prototype.delete = function(item) {
  var count = this.items.get(item);
  if (count === 0) return false;
  this.size -= count;
  this.dimension--;
  this.items.delete(item);
  return true;
};
```

`count === 0` is meant to guard "item not present", but `this.items.get(item)` on a missing item is
`undefined`, and `undefined === 0` is `false` — the guard is dead code, because no live entry's
multiplicity is ever actually `0` (every write method here deletes an item outright rather than
leaving a zero behind). So deleting an item that was **never in the set at all** falls through:
`this.size -= undefined` is `NaN` (poisoning `size` from then on — the very field
`size_stays_consistent_across_redundant_removes_issue_197` exists to keep sane), `dimension`
decrements even though nothing was removed, `this.items.delete` is a harmless no-op, and the method
still reports `true`, indistinguishable from a real deletion. Reproduced bug-for-bug in
`MultiSet::delete`, which is why `dimension` is a **tracked counter** here rather than derived from
`items.len()` the way `multi-map`'s is (see `MultiSet`'s module docs) — a derived counter would
silently *fix* this defect instead of reproducing it. Pinned by
`b_161_deleting_an_absent_item_corrupts_size_and_dimension_but_reports_true`.

### B-162 — `#.edit` never adjusts `dimension`, even when it removes a real key

`multi-set.js`'s `edit`:

```js
MultiSet.prototype.edit = function(a, b) {
  var am = this.multiplicity(a);
  if (am === 0) return;
  var bm = this.multiplicity(b);
  this.items.set(b, am + bm);
  this.items.delete(a);
  return this;
};
```

When `b` already exists as a distinct key, `edit(a, b)` merges `a`'s multiplicity into `b` and
deletes `a` — the real distinct-key count drops by one — but `dimension` is never touched by this
method at all. `test/multi-set.js`'s own third `edit` case (`set.add('c'); set.edit('b', 'c');`)
exercises exactly this shape but only reads `multiplicities()` afterwards, never `.dimension`, so
gate 4 cannot see the drift. Reproduced bug-for-bug: `MultiSet::edit` does not touch the tracked
`dimension` counter either. Pinned by `b_162_edit_into_an_existing_key_does_not_adjust_dimension`.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-163 | **`dimension` is a tracked `i64` counter, not derived from `items.len()`.** | The one place `multi-map`'s equivalent simplification (derive it) would be *wrong*: B-161 and B-162 both make upstream's own counter diverge from the real distinct-key count, and a derived counter cannot reproduce either divergence. `i64` rather than `usize` because B-161 can drive it negative. |
| D-164 | **`add`'s/`remove`'s return-value inconsistency (`this` vs. `undefined`, depending on which branch of the sign-flip delegation ran) is not modelled at the bridge.** | Untested by `test/multi-set.js`, which never checks either method's return value; the bridge always returns `this` for chaining. The differential fuzzer's own spec *does* model this exactly (see "Fuzz + bench" — comparing raw return values against upstream needed it), which is where the asymmetry was actually confirmed empirically rather than only by reading. |
| D-165 | **Counts are `f64` throughout, including a fractional one repeating a `values()`/`forEach` item `ceil(multiplicity)` times via `i < multiplicity`.** | Not a divergence from upstream — this *is* upstream's own behaviour, faithfully reproduced rather than rounded away — but stated because it is easy to assume a "count" is an integer. See `MultiSet`'s module docs. |
| D-166 | **`edit`'s execution order (`set` on `b` before `delete` of `a`) is preserved even when `a === b`**, which doubles the multiplicity and then deletes the (now sole) entry outright. | Untested upstream; reproduced because nothing here should special-case a shape the source itself does not guard against. |

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **1.69M operations, zero divergences**:

```
module=multi-set  seed=42       cases=10229 ops=1016185 wall=90.0s divergences=0
module=multi-set  seed=20260801 cases=6787  ops=674368  wall=60.0s divergences=0
```

Reproduce with e.g. `target/release/difffuzz --module multi-set --seed 42 --cases 10229`.

The op alphabet covers `add`/`remove`/`set`/`edit`/`delete`/`has`/`multiplicity`/`frequency`/`clear`
plus a bounded `top(n)` (`n` in `1..=5`, so it never hits its own arity guard — out of scope for a
core-level campaign). The item pool is three items; the count pool mixes positive (so multiplicities
build up), zero (a documented no-op) and negative (the sign-flip delegation between `add` and
`remove`) values — fractional and `NaN` counts are deliberately not in this grammar, see D-165.
Observable state is `size`, `dimension`, `items` (`[item, count]` pairs, in insertion order). Full
grammar: evidence file.

Two harness bugs in the fuzz spec's own comparison logic (not port or upstream defects) were found
and fixed while getting this campaign clean; full account: log.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check` (400
generated programs, up to 300 ops each, no oracle): 36,424 steps with a multiplicity above one,
3,795 items drained to zero and removed, across those 400 programs. Both floors are asserted in the
test itself.

**Falsification of the port (gate 6):** the assertion named first was `test/multi-set.js:307` —
`assert.deepStrictEqual(top5, [['i', 7], [' ', 7], ['r', 4], ['e', 4], ['s', 4]]);`, `top`'s
descending-by-count ordering against the file's own letter-frequency example. The sabotage,
`MultiSet::top`'s comparator swapped to the ascending, "keep-the-smallest" direction, is confirmed
red — the named assertion reports the five *least* frequent characters in ascending order instead of
the five most frequent in descending order; reverted, the full 26-block, 83-assertion suite passes
again. Nothing was found to be blind — the sabotage broke exactly the mechanism it targeted.

### Bench

`bench/results.json` → `modules["multi-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `add`/`multiplicity`/`remove` (50/25/25) over a 20,000-item domain
(`add`/`remove` deliberately used rather than `delete`/`set`, which carry reproduced-bug-for-bug
corruption — B-160/B-161 — on paths this workload would otherwise hit constantly), ~12.5 net
multiplicity per item on average by the run's end: the port is 1.36× faster at p50 (16.2 vs 22.3
ns/op), 1.2× faster at p99, about 1.13× slower at min. Full table and the p50 fix history: evidence
file and log.

`add` reaches `OrderedMap::get_mut` instead of doing a lookup followed by an unconditional insert —
two hash lookups of the same key on every call, on the operation that is half this workload's mix.
Upstream has no choice: a JS `Map` cannot look up and hand back a handle to update in place.
`OrderedMap::get_mut` can, and `set` on an existing key is already an in-place `mem::replace` into
the same slot, so bumping the multiplicity through the `&mut f64` preserves insertion order
identically. `remove`'s "still positive afterwards" path is the same shape; its "drops to zero" path
is unchanged, since deleting is not something `get_mut` can do.

**One regression, on `min_ns_per_op` only** — p50 and p99 both win clearly, and a single-metric
1.13× gap on the *minimum* (the single fastest batch out of 10,000) is the shape a noise floor takes
rather than a structural cost: `min_ns_per_op` is the least statistically stable of the three latency
figures (one sample, not a percentile over many), and nothing about this module's `add`/
`multiplicity`/`remove` path does asymptotically more work than upstream's identical three calls.
Reported rather than omitted regardless — a regression is stated even when the likelier
explanation is measurement noise, not silently dropped because it looks small.
