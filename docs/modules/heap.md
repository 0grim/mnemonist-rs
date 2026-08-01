# heap

Upstream: `heap.js` (576 LOC) + `utils/comparators.js` (79 LOC) · **655 LOC** ·
`test/heap.js` — **232 lines, 14 `it` blocks, 57 assertion statements**.

Port: `crates/mnemonist-core/src/structures/heap.rs`,
`crates/mnemonist-core/src/utils/comparators.rs`.
Bridge: `crates/mnemonist-napi/src/heap.rs`, `crates/mnemonist-napi/src/comparators.rs`,
`crates/mnemonist-napi/src/js_array.rs`.
Shims: `tests/bridge/heap.js`, `tests/bridge/utils/comparators.js`.

`test/heap.js` requires `../heap.js` **and** `../utils/comparators.js`, so under DESIGN.md §1.1 the
two are one unit and neither can land alone.

This is the module that opens **capability tier T2**: a comparator is a JavaScript function called
*from inside* a Rust operation, once per comparison, in the middle of a sift. That re-entrancy — not
the heap algorithm, which is thirty lines — is what the tier is about, and it is the same hazard
shape as B-31, reached through a different door.

---

## What upstream tests

Fourteen `it` blocks. Characterising the shape rather than listing them:

* **Every comparator in the file is total, consistent and side-effect-free.** Three appear: the
  default, one comparing `a.value` against `b.value`, and issue #120's even/odd inversion. None
  mutates, none throws, none returns a non-number.
* **Every heap is fresh and tiny.** The largest holds four elements; the largest `push` sequence is
  four long. No heap is ever grown past a single sift's worth of work.
* **`items` is never read.** It is a public array upstream and the file never touches it, so nothing
  observes the heap's actual layout — only `peek`, `pop` and the sorted output of `toArray` /
  `consume`.
* **`size` is checked, but only where it agrees with the array.** Every assertion on `size` follows
  a successful operation, so the two quantities are never seen to disagree.
* **The raw-array statics get one block.** `Heap.heapify(DEFAULT_COMPARATOR, array)` followed by
  `Heap.consume(DEFAULT_COMPARATOR, array)` on a seven-element literal. `siftUp`, `siftDown`,
  `Heap.push`, `Heap.pop`, `Heap.replace` and `Heap.pushpop` — six of the eight exports — are never
  called at all.
* **`nsmallest` / `nlargest` get three blocks**, over one twelve-element array and a `Set` built
  from it, at `n` ∈ {1, 3, 34}. This is the best-covered part of the file, and issue #120's
  regression test is the only place a custom comparator meets the `n === 1` path.
* **`MaxHeap` gets two blocks**, both behavioural. Its relationship to `Heap` is never inspected.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised.

**The comparator as a callback — the entire regime**

1. **A comparator that mutates the heap.** Three distinct shapes, each answering differently:
   growing the array under an index the sift already chose, shrinking it so the walk reads past its
   own frozen `endIndex`, and **rebinding** it via `clear()` so the sift finishes into a detached
   array. See B-76.
2. **A comparator that throws.** `push` grows the array before it sifts and `++this.size` never
   runs, so the two disagree permanently (B-70). There is no `try`/`finally` anywhere in `heap.js`.
3. **A comparator that throws inside `consume()`**, where `this.size = 0` is the *first* statement,
   so the count leads the array instead of lagging it (B-77).
4. **A comparator returning a non-number.** `'x'` reports "equal" for every pair; `0.5` counts as
   "greater"; a `BigInt` works where `ToNumber` would throw (B-78).
5. **A falsy comparator argument.** `new Heap(0)` and `new Heap('')` take the default silently,
   because the guard is `||` followed by a `typeof` test (B-79). The file asserts the throwing half
   and not this one.
6. **`reverseComparator` swapping rather than negating.** For a comparator that is not
   antisymmetric the two differ, and `MaxHeap` is built on it.

**`utils/comparators.js` — three of its four exports**

7. **`DEFAULT_REVERSE_COMPARATOR` is never used.** Upstream ships it *and*
   `reverseComparator(DEFAULT_COMPARATOR)`; they agree pointwise and are different function
   objects.
8. **`reverseComparator` is never called directly.**
9. **`createTupleComparator` is never called at all** by this test file. Its `size === 2` unrolling
   and its behaviour on a tuple shorter than `size` are both unexercised.

**The raw-array statics**

10. **Six of eight are never called:** `siftUp`, `siftDown`, `push`, `pop`, `replace`, `pushpop`.
11. **`Heap.replace`'s throw is never reached through the static**, only through the method.
12. **That `Heap.push` and `Heap.prototype.push` are different functions** is never observed —
    which turned out to matter, see "Bugs this found".

**`nsmallest` / `nlargest`**

13. **An empty source is never passed.** `n === 1` then answers with the `Infinity` sentinel itself
    (B-71), and through a typed array that narrows to a plausible-looking `0`.
14. **`Infinity` is never an element.** It is the sentinel, so an element equal to it resets the
    "nothing seen yet" test and the next element replaces it unconditionally (B-72). Two adjacent
    `n` values then give contradictory answers on the same input.
15. **A typed array is never passed.** `new iterable.constructor(1)` means the `n === 1` path
    returns the source's class; the `n >= length` path sorts a typed-array clone.
16. **`n === 0` is never passed**, nor a fractional, negative or `NaN` `n`. Upstream accepts all
    of them on the array-like path without complaint, because `n` is a *loop counter* there and not
    an index — see "Bugs this found", defect 3.
17. **That the source is not mutated** is never asserted.
18. **`guessLength`'s `.size` branch is exercised** (via `Set`) **but its `.length` branch is
    not**, because anything with a `.length` is array-like and takes the other path entirely.

**`Heap.from`**

19. **A custom comparator is never passed to `from`.** Both call sites use the default.
20. **`from` on an empty iterable** is never done.
21. **That `Heap.from(array)` copies rather than adopting** is never asserted.

**`MaxHeap`**

22. **`MaxHeap.prototype === Heap.prototype`** is never inspected, so nothing notices that
    `new Heap() instanceof MaxHeap` is `true` and that `new MaxHeap().constructor.name` is
    `'Heap'` (B-75).

**Never called at all**

23. `inspect()` and the `nodejs.util.inspect.custom` symbol.
24. `Heap.MinHeap`, which is `Heap` itself.

## What we test in addition

**Rust native tests** — `crates/mnemonist-core/src/structures/heap.rs` (24) and
`crates/mnemonist-core/src/utils/comparators.rs` (9):

| Test | Closes gap |
|---|---|
| `push_pop_is_ascending`, `to_array_leaves_the_heap_intact`, `consume_empties_the_heap`, `a_max_heap_reverses_the_comparator`, `heapify_then_consume_sorts` | the upstream blocks, as a baseline |
| `a_comparator_that_grows_the_array_mid_sift_does_not_panic` | 1 — and the assertion is that it *completes*, which an algorithm holding `&mut Vec` could not |
| `a_comparator_that_shrinks_the_array_makes_the_walk_read_undefined` | 1 — the frozen `endIndex` half |
| `clear_detaches_an_in_flight_sift` | 1 — the rebinding half (D-41) |
| `a_throwing_comparator_desynchronises_size_from_the_array` | 2 |
| `sort_with_is_stable`, `sort_with_puts_undefined_last_without_comparing_it` | the two `Array.prototype.sort` properties `nsmallest` depends on |
| `nsmallest_over_an_array_like`, `nlargest_over_an_iterable` | 13–18, the paths |
| `pushpop_on_an_empty_heap_returns_its_argument`, `from_items_heapifies_in_place` | 20, 21 |
| `replace_on_an_empty_heap_throws_upstreams_message` | 11 |
| `undefined_compares_equal_to_everything` | the slot semantics gaps 1 and 13 both depend on — and it asserts the *trap*, that Rust's own `Option` ordering says the opposite |
| `nan_compares_equal_to_everything` | 4 |
| `reverse_swaps_arguments_rather_than_negating`, `the_two_reverses_agree_pointwise` | 6, 7 |
| `tuple_comparator_is_lexicographic`, `tuple_comparator_reads_past_a_short_tuple_as_undefined` | 9 |

**JavaScript boundary spec** — `tests/boundary/heap.js`, **51 cases**, covering everything that
needs a real JS comparator, a real array or a real typed array. Its provenance is the important
part: **every expectation was run against the pinned upstream source first** (`bench/upstream/`,
Node 24.18.1) and is what upstream printed. Re-pointed at upstream, 50 of the 51 pass unchanged;
the only failure is the one explicitly about the bridge's own surface. So the file measures
divergence in *either* direction, not merely "the port does what I expected".

Thirteen of the cases are a **re-entrancy matrix**: every method that can be on the stack when a
comparator fires, crossed with twelve re-entrant actions it could take — 145 combinations. What
they guard is the borrow discipline of D-70/D-43, whose failure mode is a `BorrowMutError` that
aborts the process, so "the loop finished" *is* the assertion. The values were checked separately:
the same matrix run against the pinned upstream source and diffed came back **byte identical on
all 145 lines**.

The rest closes gaps 1–6, 10–15, 17, 22, and adds several the Rust side cannot reach: the whole
delegated-`<` regime of D-72 (mixed types, `valueOf`, `toString`-only objects, BigInt heaps,
UTF-16 string order, a comparator returning a `Symbol`), and that a thrown
comparator propagates the caller's own error **object** (not a wrapper), and that the ten statics
coexist with the five prototype methods of the same name.

**Differential fuzzer** — see "Fuzz + bench". Its grammar exists for gap 1 specifically.

**Still untested, stated rather than glossed:** gap 23 (`inspect`, not ported) and gap 9's
`createTupleComparator` beyond its Rust unit tests, since no upstream test file reaches it until
`kd-tree`.

**And one behaviour that is reproduced and cannot be tested:** `Heap.nsmallest(cmp, -Infinity, array)`
**does not terminate**, upstream or here. The scan is `for (i = n; i < l; i++)` and
`-Infinity + 1` is `-Infinity`, so the loop reads `iterable[-Infinity]` — `undefined` — forever.
The port hangs identically, which is bug-for-bug correct and therefore unrunnable in any test or
fuzz grammar. It is a genuine upstream defect and it has **no `B-` number**: this agent's allocated
range (B-70..B-79) was fully spent before it was found, and CLAUDE.md says to say so rather than
spill past the range. Flagged for the orchestrator.

## Bugs this found

Ten upstream defects, **B-70 through B-79**, all verified against Node 24.18.1 and all pinned by
`tests/boundary/heap.js`. Full write-ups in `planning/NOTES.md`; in brief:

| ID | Defect |
|---|---|
| B-70 | a comparator that throws leaves `size` one behind `items.length`, permanently |
| B-71 | `nsmallest(1, [])` answers `[Infinity]` — the sentinel returned as an element |
| B-72 | the sentinel is a real value, so an `Infinity` element resets it and the next element wins |
| B-73 | `FixedReverseHeap`'s capacity guard is `&&` where `||` was meant (see that module's doc) |
| B-74 | `FixedReverseHeap#clear` leaves `items`, so `peek()` is stale (ditto) |
| B-75 | `MaxHeap.prototype = Heap.prototype`, so `instanceof` cannot tell them apart |
| B-76 | nothing stops a comparator from mutating the heap it is comparing |
| B-77 | `#.consume` zeroes `size` first, so a throwing comparator strands the items |
| B-78 | a comparator's return value is coerced, never checked — `'x'` sorts nothing, `-1n` works |
| B-79 | a falsy comparator argument takes the default silently |

**B-72 is the one worth reading twice**, because it is self-contradicting rather than merely odd:

```js
var descending = function (a, b) { return a < b ? 1 : a > b ? -1 : 0; };
Heap.nsmallest(descending, 1, [Infinity, 5])   // [5]
Heap.nsmallest(descending, 2, [Infinity, 5])   // [Infinity, 5]
```

Two adjacent values of `n` disagree about which element is smallest.

**Three defects in the port that every gate missed, found by an independent review.**
`status: all fixed, all pinned by tests/boundary/heap.js`

This is the most important paragraph in the document, because the unit had 21 upstream assertions,
47 boundary cases, three fuzz campaigns and 5 M operations all green when they were found. It is
the sixth entry for `planning/NOTES.md`'s table of confident green signals that were verifying
something other than what was believed — and, like B-31, it was found by *a second, independent
look* rather than by the machinery.

*1 — a `RefCell` borrow held across a call into JavaScript, which aborted the process.*
`Heap::clear` was written as

```rust
let fresh = self.items.borrow().allocate(0)?;   // the Ref lives to the end of the STATEMENT
*self.items.borrow_mut() = fresh;
```

with a comment asserting the opposite. The temporary `Ref` is alive for the whole `allocate` call,
and on the bridge that call read `items.constructor` and invoked it — user JavaScript. Re-entering
`clear()` from that constructor reached the `borrow_mut()` and panicked. A Rust panic across the
FFI boundary is not a catchable `Error`: it aborted Node with `SIGABRT`. `Heap::peek` had the same
shape one method over, reachable through an accessor on index 0.

The fuzzer could not have found it: its `VecStore` never calls JavaScript from `allocate`, so the
class of bug is structurally unreachable there. Fixed by binding the store to a local before using
it — the pattern the other eight methods already used — and, for `clear`, by the fix below, which
removes the JavaScript call entirely.

*2 — `clear()` and `consume()` preserved a class upstream discards.* `Heap.prototype.clear` is
`this.items = []` and `Heap.consume` opens with `var array = new Array(l)`; both are unconditional
literals. Only `nsmallest`'s `n === 1` path is class-preserving (`new iterable.constructor(1)`).
The bridge had one `Store::allocate` doing duty for all three, so
`Heap.from(new Uint8Array(…)).consume()` came back a `Uint8Array` where upstream gives a plain
`Array`. **A port that is more class-faithful than upstream is a defect**, and this one was
invisible to a fuzzer whose only store has a single class. Fixed by splitting `Store::allocate`
(class-preserving) from `Store::plain_array` (always `Array`).

*3 — `n` was validated before upstream would have validated it.* The bridge refused a negative,
fractional or `NaN` `n` up front. Upstream never validates `n` at all: it *compares* it
(`n === 1`, `n >= iterable.length`), *slices* with it, and uses it as a **loop counter** —
`for (i = n; i < l; i++)` with the raw number. So `Heap.nsmallest(cmp, 2.5, array)` reads
`iterable[2.5]`, `iterable[3.5]`, … every one of which is `undefined`, and the scan does nothing;
the answer is the first two elements sorted. Measured against upstream, all of these answer
without throwing where the port threw:

```js
Heap.nsmallest(cmp, -1,  array)   // 11 elements
Heap.nsmallest(cmp, 2.5, array)   // [2, 5]
Heap.nsmallest(cmp, NaN, array)   // []
```

The one place upstream *can* refuse `n` is the `new Array(n)` on the non-array-like path, and it
raises a **`RangeError`**, not mnemonist's own error. Fixed by carrying `n` as the `f64` it is,
reproducing the fractional/negative loop exactly, moving the refusal to where upstream has it, and
raising a real `RangeError` through the environment so napi re-throws the right constructor.

The doc's own gap 16 previously claimed "upstream's `new Array(n)` refuses it too", which was true
for one of three code paths and false for the two the bridge actually intercepted. Corrected above.

**Two more defects in the port, both found by the port's own machinery, both fixed:**

*napi-rs registers a class's statics and its prototype methods through one name table.* Upstream
has five name pairs that exist as both — `push`, `pop`, `replace`, `pushpop`, `consume` — and
declaring both halves made the prototype half silently vanish. **Nine of fourteen cases failed with
`heap.push is not a function`**, which is a loud failure, but the *cause* is not one a reader would
guess: JavaScript has no such conflict, because a constructor and its prototype are different
objects. Fixed by declaring the statics on a `HeapStatics` class the addon copies across and then
deletes from its exports (D-75).

*A `#[napi(factory)]` instantiates with `napi_new_instance(this)`.* `MaxHeap` pulled its factory off
the constructor and called it bare, which died with `Failed to create instance of class`. Fixed by
binding the receiver before deleting the temporary property.

**What the fuzzer found: nothing new**, which is the expected outcome and the same statement D-33
makes. The oracle *is* upstream, so a faithfully reproduced bug is by definition not a divergence.
All ten of B-70…B-79 were found by reading the two files statement by statement and confirming each
against Node. What the fuzzer is for is drift, and it was proven to work in that direction — see
below.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-70 | **The algorithms take a `Store`, not `&mut Vec<T>`.** | An exclusive borrow is exactly what a re-entrant comparator would have to violate, so the natural Rust signature makes upstream's behaviour *inexpressible*. A `RefCell` panic is not a reproduction of "it works and gives this answer". |
| D-71 | **`compare` returns `f64`, not `Ordering`.** | Upstream tests `< 0`, `> 0` and `>= 0` on whatever came back. Three values cannot express a comparator that answers `NaN` or `0.5`, and collapsing would quietly *repair* an inconsistent one. |
| D-72 | **`DEFAULT_COMPARATOR` is ported; `<` and `>` are delegated.** | Number-vs-number and string-vs-string are answered natively and exactly. Anything involving an object, a symbol or a mixed pair goes to a compiled-once `(a, b) => a < b`, because `ToPrimitive` runs user code — re-implementing it would be a port of V8. |
| D-73 | **`items` is a real JavaScript array.** | `Heap.heapify` mutates the caller's array in place and the original suite consumes that array; `FixedReverseHeap` needs a real `ArrayClass`. Also buys typed-array store semantics exactly, and extends D-70's re-entrancy to the array. |
| D-74 | **`MaxHeap` is evaluated JavaScript.** | `MaxHeap.prototype = Heap.prototype` is upstream's, and a second native class would have its own prototype and silently *fix* B-75. |
| D-75 | **The statics live on a `HeapStatics` class, copied across at load.** | napi's one-name-table conflict; see above. **Residual:** `Heap.__max` and `Heap.__maxFrom` survive as non-enumerable properties, because napi defines class properties `configurable: false` and `delete` is a no-op on them. The bridge's only addition to upstream's surface. |
| D-76 | **The `Infinity` sentinel is a value, not an `Option`.** | `Option<Item>` would have fixed B-71 *and* B-72. A slot type that cannot hold `Infinity` answers `is_infinity` false, which is the same statement one level up rather than a papered-over divergence. |
| D-77 | **`#.comparator` is not exposed.** *(divergence: yes)* | The bridge stores a `BridgeComparator`, whose default variant has no JS function behind it at all. Synthesising one to satisfy a getter would be a fabrication — it would not be the object the sift calls. No upstream assertion reads it. |
| — | **`nsmallest(cmp, n, undefined)` is read as the three-argument form.** | Upstream keys off `arguments.length === 2`, which napi's typed signature cannot see. The two forms the original suite uses are exact. |
| — | **`Store::allocate` and `Store::plain_array` are different operations.** | `clear()` and `consume()` allocate a plain `Array` unconditionally, as upstream's `[]` and `new Array(l)` do; only `nsmallest`'s `n === 1` path and `FixedReverseHeap`'s `new ArrayClass(size)` preserve a class. One method doing both made the port *more* class-faithful than upstream. |
| — | **`n` is carried as the `f64` it is, never validated up front.** | Upstream compares `n`, slices with it and uses it as a loop counter; the only construct that can refuse it is the `new Array(n)` on the non-array-like path. A fractional `n` therefore makes the scan read `iterable[2.5]` — `undefined` — every time, and the port reproduces that rather than truncating. |
| — | **A `Store` whose `push` reports zero sifts at index 0, where upstream sifts at `-1`.** | Not reachable from core, whose `VecStore` always reports at least 1; reachable through the bridge, because `push` is a real method lookup on a real JS array (D-73) and can be tampered with. `usize` underflow would panic in debug and wrap in release into an index asking the store to grow to `usize::MAX`, so it is `saturating_sub`. **Measured afterwards: the observable result is identical.** Upstream's `heap[-1] = heap[-1]` writes an expando nothing reads; ours rewrites `heap[0]` with the value it just read. Both leave `items` and `size` exactly as the other does. |
| — | **A missing array method throws an `Error`, not V8's `TypeError`.** | `Heap.from(typedArray).toArray()` reaches `heap.pop()` on a typed array, which has none. Upstream dies with `TypeError: heap.pop is not a function`; the bridge raises `Error: pop is not a function`, because the receiver in V8's message comes from the *source text* of the call site and no Rust code has it. Both throw, at the same point, for the same reason. Measured across ~35 edge cases against the pinned upstream source, this is **the only textual difference**. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |
| — | **`Store::Item` is `Option<T>` in core, where `None` is `undefined`.** | Once a comparator can shrink the array, `heap[childIndex]` reads past the end and `heap[i] = …` writes past it. `Relational` gives `None` JavaScript's rule — compares false against everything — rather than Rust's, which says `None < Some(_)`. |

## Fuzz + bench

### Fuzz

```
module=heap seed=42       cases=25677 ops=2619243 wall=120.0s divergences=0
module=heap seed=20260801 cases=12248 ops=1256277 wall=60.0s  divergences=0
module=heap seed=31337    cases=11060 ops=1147769 wall=60.0s  divergences=0
```

Three campaigns, three seeds, **5.02 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module heap --seed 42 --cases 25677`.

* **Op alphabet:** `push(v)` (weight 6) · `pop()` (3) · `peek()` (2) · `replace(v)` (2) ·
  `pushpop(v)` (2) · `toArray()` (2) · `clear()` (1) · `consume()` (1).
* **Constructor alphabet — the point of the grammar:** six comparator factories, mirrored name for
  name between `fuzz/oracle.js` and `crates/difffuzz/src/modules/heap.rs`. Two are pure
  (`ascending`, `descending`); four are not:

  | factory | what it does from inside the sift |
  |---|---|
  | `pushy` | `items.push(99)` — grows the array under an index the sift already chose |
  | `popper` | `items.pop()` — shrinks it, so the walk reads past its frozen `endIndex` |
  | `clearer` | `heap.clear()` — **rebinds** it, detaching the sift onto the old array |
  | `boom` | throws, leaving `items.length` one ahead of `size` |

* **Observable state, compared after every op:** `size` and `items`. They are separate quantities
  upstream and B-70 makes them genuinely disagree, so comparing both is what pins it.
* **Values:** `0..24`, small enough that duplicates are frequent — a heap's tie-breaking is
  observable through `toArray`, and `sift_up`'s `>= 0` is the only thing that decides it.

**The budget is part of what is compared.** Each mutating factory fires for its first *k*
comparisons and then stops, so the answer depends on the **number and order of comparisons**, not
only on the final ordering. A sift that reaches the right answer by a different route diverges
here, where a black-box push/pop grammar would never notice.

This grammar exists because of a lesson already paid for: B-31 survived 2.94 M operations on
`queue` because the alphabet had no `forEach`, and *an op alphabet that omits a method omits every
bug reachable only through it*. Every comparator in `test/heap.js` is pure; a grammar that
inherited that property would have inherited the same blind spot.

**Falsification of the fuzzer.** Sabotage: `Heap::clear` truncating the backing array in place
instead of rebinding it — the D-41 collapse, and the most plausible way a future cleanup breaks
this port, because `set_length(0)` and `allocate(0)` leave an **identical** observable state for
every program whose comparator has no side effects.

* **All 14 assertions of `test/heap.js` still passed under it**, and all 7 of
  `test/fixed-reverse-heap.js`.
* The fuzzer found it in **0.1 s** and shrank a 200-op program to three operations:

  ```js
  var s = new Heap(clearer);   // clears the heap on its first comparison
  s.push(1);
  s.pushpop(18);               // upstream 1, truncating port undefined
  ```

* `tests/boundary/heap.js` caught it too, by name.

Reverted; the seed is committed with a provenance header in
`crates/difffuzz/proptest-regressions/heap.txt`, and proptest replays it before any novel case.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `should be possible to pop the heap.`
— `assert.strictEqual(heap.pop(), 1)` at `test/heap.js:47`. Chosen because it is the shortest path
from `push` through `sift_down` to an observable value, so a sabotage of the sift cannot miss it by
accident.

**The sabotage:** `sift_down`'s `compare(item, parent) < 0.0` inverted to `> 0.0` — one character,
in the function every other algorithm in the file calls.

**Confirmed red**, and red in the named place: `3 passing, 11 failing`, the named assertion failing
with `34 !== 1`. Reverted; **confirmed green again**: `14 passing`.

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3): three other
agents were working while this unit landed, and a contended run inflated both sides 2–3× when it
was last attempted. `heap` is therefore **complete except gate 10** and is deliberately *not* in
`tests/scope.txt`; `tests/verify.sh` will say so, which is the intended state rather than an
oversight.

One thing worth flagging for whoever runs it: the bridge's comparator crosses the FFI boundary once
per comparison, and its default variant is a native Rust comparison rather than a JS call, so the
port's advantage on a default-comparator workload will be structurally different from its
disadvantage on a user-comparator one. Measure both; do not report either alone.
