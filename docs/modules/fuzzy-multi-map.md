# fuzzy-multi-map

Upstream: `fuzzy-multi-map.js` (196 LOC) — depends on `multi-map.js` (408 LOC, already ported as its
own unit) · `test/fuzzy-multi-map.js` — **189 lines, 11 `it` blocks, 27 assertion statements**.

Port: `crates/mnemonist-core/src/structures/fuzzy_multi_map.rs` — `FuzzyMultiMap<K, V>`, a thin
wrapper over `mnemonist_core::structures::multi_map::MultiMap`, adding no behaviour beyond the
wrapping (hashing is entirely the bridge's concern). Bridge:
`crates/mnemonist-napi/src/fuzzy_multi_map.rs`. Shim: `tests/bridge/fuzzy-multi-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/fuzzy_multi_map.rs`.

"Same as the fuzzy map but relying on a `MultiMap` rather than a `Map`" — upstream's own header
comment, verbatim, and the port follows it exactly: every method here delegates straight to a wrapped
`MultiMap`, one level up from `crate::structures::fuzzy_map::FuzzyMap`, which does the identical thing
one level up from a plain `OrderedMap`.

---

## What upstream tests

* **Constructor validation**: an invalid hash function (a plain object, or an array containing one)
  throws `/hash/`, for all three shapes upstream accepts (`descriptor` bare, `[write, read]`, one
  slot invalid).
* **`add`** — hashes the *item itself*.
* **`set`** — hashes the *given key*; the item is stored separately from what was hashed.
* **`clear`**.
* **`get`**, **`has`** — hashes the query with `readHashFunction`, case-insensitively in every test
  (`'HELLO'`/`'Hello'`/`'hello'` all resolve to the one bucket).
* **`forEach`** and **`values()`** (`for...of` too, `Symbol.iterator` aliased to `values`) — both
  flattened over the wrapped `MultiMap`'s own buckets, discarding the hashed key entirely.
* **`FuzzyMultiMap.from(iterable, descriptor, Container, useSet)`**, twice: once with an
  `[writeHash, readHash]` pair from a plain array, and once with **exactly three arguments** where
  the third is a boolean — upstream's `arguments.length === 3` special case reinterprets that
  boolean as `useSet` rather than `Container`, which is the *only* way this test's second `.from`
  call reaches `useSet` at all.
* **A `Set` container**, storing plain **objects** (`{title: 'Hello1'}`, etc.) and asserting that a
  duplicate *object reference* does not grow the bucket, while three distinct objects (even with
  colliding hashed keys) do.

## What upstream does NOT test

**Every one of `MultiMap`'s own gaps**, inherited wholesale since this module adds no behaviour of
its own — see `docs/modules/multi-map.md`. In particular, a container mutated mid-walk is untested
here for the identical reason.

**`readHashFunction` and `writeHashFunction` differing** in any test beyond the two `.from` calls'
`[writeHash, readHash]` pairs — every `add`/`set`/`get`/`has` call in the file's own instance-level
tests uses a *single* hash function for both directions (the constructor's one-argument
falsy-substitution path).

**A hash function that throws**, or that returns a value `JsKey`/`FuzzKey` cannot represent (an
object, say). Every test's hash functions return plain lowercased strings.

**`Set`-kind membership for two objects that are SameValueZero-but-not-identical** — impossible to
construct for two distinct object literals in JavaScript anyway (object equality is always by
reference), so this is not a gap so much as a statement that the one test case (`three` added twice,
literally the same reference) is the *only* shape `Set`-kind object dedup can take at all.

## What we test in addition

**Rust native tests** — `crates/mnemonist-core/src/structures/fuzzy_multi_map.rs` (5) — the wrapping
itself, since there is no new algorithm to pin beyond what `multi_map`'s own tests already cover:
`reproduces_the_upstream_walkthrough`, `clear_resets_size_and_dimension`,
`get_returns_every_item_hashed_to_the_same_key`, `has_matches_the_hashed_key`,
`set_kind_deduplicates_by_the_supplied_equality` (over a plain `PartialEq` item, standing in for
`multi_map`'s own `Set`-kind coverage one layer down).

**Differential fuzzer** — `fuzzyLower` (a real factory shared with `fuzzy-map`'s own campaign)
collapsing `'Hello'`/`'HELLO'`/`'World'` onto two hashed keys, so the campaign hits "one key, several
values" through hash collision specifically, rather than through a literal repeated key — see "Fuzz +
bench".

**Still untested, stated rather than glossed:** `Set`-kind object-identity dedup is not fuzzable
through the differential protocol at all (see "Deliberate divergences") — it is covered only by
`test/fuzzy-multi-map.js` itself and by `mnemonist_napi::fuzzy_multi_map`'s own bridge-level test,
which is why gate 6's falsification for this unit specifically targets that path (see "Fuzz +
bench").

## Bugs this found

No upstream defect. Two resource-management defects in this port's own bridge, found and fixed
before any differential campaign ran clean — both are exactly the kind of thing `test/
fuzzy-multi-map.js` itself caught immediately (as leak warnings on stderr, not as failing
assertions), which is worth being honest about: gate 4 passing does not mean gate 4 *ever ran clean
on the first attempt*.

### Bridge defect 1 — `.from`'s collector re-retained an already-retained value

`FuzzyMultiMap.from`'s dispatch collects every `(value, key)` pair the underlying iteration visits
**before** any hash function runs (`collect_pairs`, mirroring `crate::fuzzy_map`'s own reason: the
collector closure must be `'static`, and the hash functions are not). The first version of
`.from` then called `resolve(&env, &value)` — producing a live view of the *already-retained* slot —
and passed that live view to `store()`, which called `Retained::new(&item)` **again**, creating a
second, independent `napi_ref` for the same JS object while the first one (still held by the
now-discarded `value: Option<Retained>` local) was simply dropped without
[`Retained::release`]. Found immediately: `test/fuzzy-multi-map.js`'s own "should be possible to
create an index from arbitrary iterables" test printed `mnemonist-rs: a retained JavaScript value
was dropped without being released` to stderr four times (once per stored object across its two
`.from` calls) while still reporting every assertion green — a leak invisible to gate 4 itself, only
visible by watching the process's own diagnostic output. Fixed by `store_retained`, which takes the
already-retained value directly rather than re-deriving it from a resolved view; `store` (used by
`add`/`set`, whose `item` argument is always a fresh, never-before-retained live value) is unchanged.

### Bridge defect 2 — a rejected `Set`-kind duplicate had nowhere to go and leaked

`MultiMap::set_with`'s original signature returned `Result<(), E>`: when a `Set`-kind bucket already
had an equal member, the candidate value was simply dropped at the end of the function, silently. For
`mnemonist_napi::multi_map` (values are `JsKey`, plain data) that is harmless. For
`fuzzy_multi_map`'s `Rc<RefCell<Retained>>` items it is a leak: "should work with a Set container"'s
fourth `.set` call (`map.set('hello', three)`, the literal-same-object duplicate) retained `three`
fresh (as every `add`/`set` call must, since it arrives as a live argument), found it already present
by `same_value_zero`, and the freshly-retained handle — never stored, never released — printed the
identical leak warning. Fixed at the source: `MultiMap::set_with` now returns `Result<Option<V>, E>`,
handing the rejected candidate **back** rather than dropping it, and
`JsFuzzyMultiMap::store_retained` releases exactly what comes back
(`Rc::try_unwrap`+`Retained::release`). This is a core-level API change with no behavioural
consequence for `mnemonist_napi::multi_map` (which still discards the `None`/`Some` via the
infallible `MultiMap::set` convenience wrapper) — see `docs/modules/multi-map.md`'s own native test,
`set_with_hands_a_rejected_duplicate_back_instead_of_dropping_it`, which pins the contract this fix
depends on.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-167 | **`Set`-kind object-identity dedup is not fuzzable through the differential protocol.** | The core-level campaign drives `FuzzyMultiMap<String, String>` through the infallible `set_with` convenience path (plain `PartialEq`), which has no notion of JS object identity at all — the whole reason `same_value_zero` exists is JavaScript-specific (`napi_strict_equals`) and lives entirely in the bridge, one layer outside what the differential fuzzer compares (core vs. upstream JS, not bridge vs. upstream JS). Covered instead by `test/fuzzy-multi-map.js` itself and by a bridge-level native test; see "What we test in addition". |
| D-168 | **`FuzzyMultiMap.from`'s three-argument boolean-shift (`arguments.length === 3` reinterpreting `Container` as `useSet`) is reproduced by shape, not by counting real napi arguments.** | napi has no `arguments.length` equivalent; the bridge instead checks "the third parameter is present, the fourth is absent, and the third is a JS boolean" — indistinguishable from upstream's own check for every call `test/fuzzy-multi-map.js` makes, and the only case this project could construct where the two rules would disagree (a caller passing an explicit `undefined` as a fourth argument *and* a boolean third) is not exercised by any test. |
| D-169 | **Values are `Rc<RefCell<Retained>>`, not a bare `Retained`.** | `MultiMap`'s flattened cursor snapshots a bucket by cloning its contents, and a bare `Retained` (owning exactly one `napi_ref`) cannot be cloned at all without either failing to compile or double-freeing. `Rc` clones cheaply (a refcount bump, never a second `napi_ref`); `RefCell` gives `release` (which needs `&mut self`) a way in through a shared handle. See `mnemonist_napi::fuzzy_multi_map`'s own module docs for the one stated consequence: a `values()`/`entries()`-style iterator kept open across a `clear()` observes the now-released, inert value if read afterwards — untested by `test/fuzzy-multi-map.js`, and the same class of gap `multi-map`'s own flattened cursor states for a same-bucket mutation mid-walk. |

## Fuzz + bench

### Fuzz

```
module=fuzzy-multi-map  seed=42       cases=9562 ops=964655 wall=90.0s divergences=0
module=fuzzy-multi-map  seed=20260801 cases=6276 ops=629178 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1.59M operations, zero divergences**. Reproduce with e.g.
`target/release/difffuzz --module fuzzy-multi-map --seed 42 --cases 9562`.

* **Op alphabet:** `add` (weight 5), `set` (2), `has`/`get` (2 each), `clear` (1).
* **Source pool:** `"Hello"`, `"HELLO"`, `"World"` — `fuzzyLower` (the real factory, shared with
  `fuzzy-map`'s own campaign via `fuzz/oracle.js`'s `FACTORIES` table, so both sides run the
  identical function rather than two hand-written mirrors that could quietly disagree) collapses the
  first two onto one hashed key.
* **Constructor**: `new FuzzyMultiMap(fuzzyLower)` — one hash function shared by both directions,
  `List`-kind container (see D-167 for why `Set`-kind is out of scope for this campaign).
* **Observable state**: `size`, `dimension`, and `items` rendered as the **nested** object upstream's
  own `this.items` actually is — a `MultiMap` *instance*, not a raw `Map`, so `fuzz/oracle.js`'s
  generic `encode()` renders it as `{items: {$map: [...]}, size, dimension}` rather than a bare
  `{$map: ...}`. The very first draft of this spec flattened it, which is indistinguishable from the
  real shape only by accident and diverged on case 0 of every run before the fix — see
  `crates/difffuzz/src/modules/fuzzy_multi_map.rs`'s own `observe` doc comment.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check` (400
generated programs, up to 300 ops each, no oracle):

```
fuzzy-multi-map grammar: 41,673 steps with a multi-value bucket, 4,193 clears of a nonempty map
```

Both floors are asserted in the test itself.

### Falsification of the port (gate 6)

Because this unit's own differential fuzzer cannot reach `Set`-kind object-identity dedup (D-167),
the sharpest target is that exact path, at the bridge — not core, which is untouched here.

**The assertion the sabotage had to break was named first:** `test/fuzzy-multi-map.js:179` —
`assert.strictEqual(map.size, 3);` — in "should work with a Set container", which only holds if the
fourth `.set` call's literal-same-object duplicate (`three`, added twice) is correctly recognised as
already present.

**The sabotage:** `mnemonist_napi::fuzzy_multi_map::same_value_zero`'s object-reference branch had
its `napi_strict_equals` result discarded and replaced with a hardcoded `Ok(false)` — every pair of
objects, including two handles to the literal same one, now compares as never equal.

**Confirmed red:** the named assertion failed, `map.size` reporting `4`, not `3` — the exact
off-by-one a broken dedup produces (`AssertionError: 4 !== 3`).

**Reverted; confirmed green again**: the full 11-block, 27-assertion suite passes.

**Nothing was found to be blind.** The sabotage broke exactly the mechanism it targeted, and only
that mechanism — every other test in the file (all of which use string-hashed keys, never
`Set`-kind object dedup) stayed green throughout, which is itself confirmation that the sabotage was
narrow enough to be a real test of this one path rather than of the whole bridge.

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3).
`fuzzy-multi-map` is therefore complete except gate 10, and deliberately not in `tests/scope.txt`.
