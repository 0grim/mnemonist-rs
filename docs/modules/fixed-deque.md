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

Sixteen `it` blocks, the most thorough test file of the three fixed-capacity modules (`fixed-stack`,
`fixed-deque`, `circular-buffer`). It genuinely covers the
ring: `should handle tricky situations.` interleaves `push`, `unshift`, `pop` and `shift` on a
capacity-6 deque and asserts `start` directly, and `should be consistent over time.` walks a
capacity-3 deque through eleven operations checking `toArray()` after each phase.

Characterising the shape of that coverage:

* **`start` is asserted twice**, at lines 151 and 274 — the only one of the three whose internal
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
16. **Mutation during iteration is never performed**, in either direction — neither an
    element overwritten ahead of the cursor nor a `shift` moving the deque's start under it.
17. **`values()` on an empty or cleared deque is never called.**
18. **`entries()` is called once, on a fresh deque**, so its own `j` counter is never observed
    diverging from the physical index — which it does on any wrapped deque.

**Never called at all**

19. `inspect()` and the `nodejs.util.inspect.custom` symbol.

## What we test in addition

`crates/mnemonist-core/src/structures/fixed_deque.rs` — **17 tests** (16 substantive plus a `Debug`
smoke test), closing every gap above except 3, 7, 9, 11, 12, 13, 14 and 19: a 1:1 reproduction of
all sixteen upstream blocks as a baseline, `get` bounded by the capacity and returning debris below
it (for both backing classes), the debris shown to be the *wrapped* slot rather than a stale tail,
removals leaving elements in place, an oversized `from` walking the ring more than once (and being
truncated by a typed class), cursor non-restartability, a push during iteration staying invisible,
a shift during iteration not moving the cursor (the frozen-`start` half, this module's sharpest
cursor behaviour), an overwrite ahead of the cursor staying visible, a wrapped deque walking front
to back, the `start === 0` wrap reached from an empty deque, and a capacity-1/empty deque both
behaving. Full test-to-gap mapping: evidence file.

**Differential probes against the vendored upstream**, 23 cases for this class (50 across both this
and `circular-buffer`), recorded because they are the evidence for the bridge half: B-60 for a
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
Verified against Node 24.18.1. Every reader in the file guards on `this.size`. `get`
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

**What the fuzzer found: nothing new.** Two campaigns, zero divergences — the expected outcome.
Both bugs were found by reading the file statement by statement against Node.

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

Two campaigns, two seeds, **2.88 M operations, zero divergences**:

```
module=fixed-deque seed=42       cases=16921 ops=1730822 wall=90.0s divergences=0
module=fixed-deque seed=20260801 cases=11127 ops=1149893 wall=60.0s divergences=0
```

Reproduce with `target/release/difffuzz --module fixed-deque --seed 42 --cases 16921`.

The op alphabet covers `push`/`unshift`/`pop`/`shift`/`peekFirst`/`peekLast`/`get`/`clear` plus the
cursor ops and `$forEach`. Observable state is `size`, `capacity`, **`start`**, `items`, `toArray()`
— `start` is in the set because the upstream file asserts on it and because it is the one number a
wrong wrap moves first. `get` indices run 0..=11 against capacities of 1..=8, so both clauses of
B-62's guard are exercised constantly: past the size (debris) and past the capacity (the guard that
fires). Both backing classes are generated, capacities 1..=8, values to 320. Deliberately excluded:
`from` (a static cannot appear in an op sequence; covered by the original test and the differential
probes), `forEach`'s `scope` (D-61), and a **negative** `get` index — the fuzzer drives
`mnemonist-core`, whose `get` takes a `usize`, and the negative path is the bridge's, covered by
four differential probes instead. Full grammar: evidence file.

**The grammar was falsified before being trusted.** Sabotage: `get`'s guard changed from
`index >= self.capacity` to `index >= self.size` — the "obvious correction" of B-62, and the change
any reader who has not checked upstream would make. Caught in 823 cases (1.2 s), shrunk to five
lines. Reverted; the seed is committed with provenance in
`crates/difffuzz/proptest-regressions/fixed-deque.txt`. Full repro: evidence file.

**Falsification of the port (gate 6):** the assertion named first was
`should be possible to unshift the deque.` — `assert.strictEqual(deque.start, 3)` at
`test/fixed-deque.js:151` — chosen because `start` is the one piece of internal geometry the file
inspects directly, so it is the assertion that most specifically exercises the ring rather than its
results. The sabotage, `previous_start()` returning `self.start.saturating_sub(1)` instead of
wrapping to `capacity - 1` when `start` is zero (dropping the one line that makes `unshift` a ring
operation rather than a bounded one), is confirmed red in precisely the named place (13 passing, 3
failing, the second failure being the named assertion with `actual` `0` against `expected` `3`; the
other two also reach `unshift` on a deque whose `start` is zero). Reverted; confirmed green again
(16 passing). Full record: evidence file.

### Bench

`bench/results.json` → `modules["fixed-deque"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peekLast`/`pop` (50/25/25), back-end operations only (mirroring
`fixed-stack`'s shape rather than adding `unshift`/`shift`, which exercise the same ring arithmetic
from the other end), capacity 10,000 against 1e6 ops, guarded the same way `fixed-stack`'s `push`
is — see that module's bench doc for why an unguarded push into a full structure would benchmark
V8's `Error` construction rather than the ring: the port is 1.6× faster at p50 (4.6 vs 7.5 ns/op),
2.3× faster at p99 (5.9 vs 13.4), 1.7× faster at min. No regressions. Full table: evidence file.

The numbers track `fixed-stack`'s closely — expected, since the timed op mix touches the same three
primitives at the same shape and the ring's extra geometry (`start`, wrap-once arithmetic) is a few
integer operations per call, not a different asymptotic cost.
