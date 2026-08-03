# stack

Upstream: `stack.js` (210 LOC) + `obliterator/foreach.js` (70 LOC) +
`obliterator/iterator.js` (95 LOC) · `test/stack.js` — **126 lines, 11 `it` blocks, 22 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/stack.rs`, `crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/stack.rs`, `foreach.rs`, `js_slot.rs`, `statics.rs`, `cursor.rs`.

`stack` is the simplest structure in the library and was chosen for exactly that reason:
it is the **host for the `obliterator/forEach` boundary coercion**, the primitive
that gates six further modules and roughly 65% of the remaining test weight. If the five-branch
dispatch is wrong, it shows up here on 210 lines rather than inside `vector`, which needs four
primitives at once.

The dispatch itself is documented alongside this file rather than inside it, because it is not a
`Stack` behaviour — see **"The `forEach` dispatch"** below and `tests/boundary/foreach.js`.

---

## What upstream tests

Eleven `it` blocks, none longer than eight lines:

```js
stack.push('test');                        assert.strictEqual(stack.size, 1);
stack.clear();                             assert.deepStrictEqual(stack.toArray(), []);
assert.strictEqual(stack.peek(), undefined);
assert.strictEqual(stack.pop(), 3);        // …down to undefined
stack.forEach(function (item, i, l) { assert.strictEqual(item, 3 - i); assert.strictEqual(stack, l); });
assert.deepStrictEqual(Stack.from([1, 2, 3]).toArray(), [3, 2, 1]);
assert.deepStrictEqual(Stack.of(1, 2, 3).toArray(), [3, 2, 1]);
var iterator = stack.values();             assert.strictEqual(iterator.next().value, 3);
var iterator = stack.entries();            assert.deepStrictEqual(iterator.next().value, [0, 3]);
for (var item of stack) assert.strictEqual(item, --i);
```

Characterising the shape of that coverage:

* **Every stack holds at most three elements**, and every element is a small integer or the string
  `'test'`.
* **Every stack is fresh.** No block reuses one across two operations that could interact.
* **`from` is called with exactly two things: an array literal, and `arguments`.** Both reach
  branch 1 of the dispatch. The other four branches are never touched.
* **Iteration is always immediate.** Every cursor is created and drained in the same block, with no
  mutation in between — already established for all 24 stored-iterator
  sites in the suite.
* **`forEach` is called once**, on a fresh three-element stack, with a callback that does not
  mutate.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by the original suite.

**Return values**

1. **`push`'s return value is never used.** Upstream returns `++this.size`; nothing asserts it.
2. **`pop`'s return value on an empty stack is asserted, but `size` afterwards is not** — so the
   early `return` that keeps `size` from going negative is only half-covered.
3. **`toString`, `toJSON` and `inspect` are never called.** ~25 LOC of the module, including the
   `Array.prototype.join` semantics that render `null` and `undefined` as the empty string.

**The two ways `items` can change**

4. **`clear()` is never called while a cursor is open.** Upstream's `clear` is `this.items = []` —
   a **new array** — and the cursor captured the old one, so it is completely unaffected. This is
   the single most consequential untested behaviour in the module.
5. **`pop()` is never called while a cursor is open.** It shortens the *same* array, so the cursor
   reads past its new end and yields `undefined`. Same-shaped mutation, opposite result.
6. **`push()` is never called while a cursor is open.** The cursor's length is frozen, so it is
   invisible.
7. **A `forEach` callback never mutates.** Upstream freezes the loop bound but re-reads
   `this.items` on every iteration, so a callback that clears changes what the remaining
   iterations read — a *third* behaviour, different again from (4) and (5).

**Cursors**

8. **No cursor is ever re-drained**, so D-06's non-restartability is unobserved.
9. **No cursor is ever partially consumed and then spread.**
10. **`break`-ing out of a `for…of` and then calling `next()` is never done.** Upstream cursors
    have no `return` method, so the walk resumes.
11. **`entries()` is drained but its index is never checked against a mutated stack.**

**`from`, i.e. the dispatch**

12. **Four of the five branches are never reached.** No `Map`, no `Set`, no generator, no plain
    object, no string, no bare iterator, no falsy value. See the separate table below.

**Values**

13. **Only numbers and one string are ever stored.** Object identity, `undefined`, `null`, `NaN`,
    `-0`, lone surrogates and BigInts are all untested, and every one of them is a value a real
    caller can push.

## What we test in addition

`crates/mnemonist-core/src/structures/stack.rs` — 14 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all eleven upstream blocks, as a baseline |
| `push_returns_the_new_size` | 1 |
| `popping_an_empty_stack_does_not_move_the_size` | 2 |
| `size_and_the_backing_length_track_each_other` | the two quantities upstream keeps separate |
| `cursors_do_not_restart_but_the_stack_can_be_walked_again` | 8, 9 — both levels of D-07 in one test |
| `a_push_during_iteration_is_not_visible_to_the_cursor` | 6 |
| `a_pop_during_iteration_opens_a_gap_at_the_top_of_the_walk` | 5 — the `undefined` window |
| `clear_rebinds_the_array_and_leaves_an_open_cursor_untouched` | 4 — the one a `Vec<T>` cannot express |
| `a_cursor_detached_by_clear_never_sees_the_new_array` | 4, extended past the rebinding |
| `for_each_reads_the_live_array_where_the_cursor_reads_the_capture` | 7 — the third behaviour |
| `peek_is_a_pure_read` | — |
| `an_empty_stack_iterates_zero_times` | — |
| `from_iter_accepts_any_iterator` | D-03: core takes any `IntoIterator` |
| `duplicates_are_kept` | — |

`crates/mnemonist-core/src/cursor/mod.rs` — 3 new tests for `Sequence::limit`
(`a_live_limit_sees_growth_that_a_frozen_one_does_not`,
`a_live_limit_resumes_after_reporting_done`,
`a_live_limit_that_shrinks_ends_the_walk_without_a_gap`), bringing that file to 16.

`tests/boundary/stack-queue.js` — **37 specs**, each asserted **both** differentially against the
vendored upstream source in `bench/upstream/` **and** explicitly. The differential half catches
what we did not think to assert; the explicit half makes a failure say which behaviour broke. This
file is where gaps 3, 7, 10, 12 and 13 are closed, because they need JavaScript: mutation from
inside a callback is a compile error in Rust, which is the whole point of the exercise.

`tests/boundary/foreach.js` — **37 specs** for the dispatch itself; see below.

**Still untested, stated rather than glossed:** `inspect()` and the
`nodejs.util.inspect.custom` symbol (gap 3, not ported — a Node display convenience with no
upstream assertion and no Rust equivalent); and `forEach`'s `scope` in its `arguments.length` form
(see the divergence table).

## The `forEach` dispatch

`obliterator/foreach` is imported by 30 of the 44 upstream modules and has **no test file
anywhere** — mnemonist exercises it only incidentally, through `Stack.from([1, 2, 3])`. It is
ported into `mnemonist-napi`, not core, because a grep of all 30 importing modules shows every call
site is `forEach(iterable, cb)` inside a `.from()` static or an iterable-accepting constructor,
operating on the user-supplied argument (D-03).

`tests/boundary/foreach.js` covers all five branches, differentially against the real
`obliterator/foreach` (a harness devDependency). What that found:

| Branch | Behaviour | Reached by the original suite? |
|---|---|---|
| 1 — indexed sequence | index is a **number**; strings walk by UTF-16 code unit, so a surrogate pair yields two halves; a sparse array's holes are visited as `undefined`; `i < l` is a numeric comparison, so `length: 2.5` admits three iterations | array and `arguments` only |
| 2 — has own `.forEach` | **delegates**, and preempts 3 and 4 — a `Map` therefore yields `(value, key)` with a **string** key, and its `Symbol.iterator` is never touched | no |
| 3 — `Symbol.iterator`, no `.next` | coerced by calling it with the target as `this` | no |
| 4 — has `.next` | drained with its **own** counter; `s.done !== true` strictly, so `done: 0` keeps going | no |
| 5 — plain object | key is a **string**, in the engine's own enumeration order: integer-like ascending, then insertion order | no |

Three traps, all reproduced and all pinned:

* **The falsy guard is `if (!iterable) throw`.** `forEach('', cb)` throws while `forEach('a', cb)`
  iterates; likewise `0`, `false`, `NaN`, `0n`, and the error text is verbatim.
* **`toString()` is invoked on an arbitrary user value mid-dispatch.** It can throw, it can be
  absent (`Object.create(null)` dies with `TypeError: iterable.toString is not a function`), and it
  can return `'[object Arguments]'` and hijack branch 1.
* **A truthy primitive reaches the `in` operator and dies there** — see B-30 below.

## Bugs this found

**B-30 — `forEach` on a truthy primitive dies in the `in` operator, not in its own guard.**
`status: verified against Node 24.18.1`. A number, boolean, symbol or bigint survives
`if (!iterable) throw`, is not an indexed sequence and has no `.forEach`, and then meets
`Symbol.iterator in iterable`. `in` requires an object:

```js
forEach(5, cb)   // TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in 5
```

Confirmed for `5`, `true`, `10n` (which stringifies as `10`) and `Symbol(x)`. The library's own
"not iterable" guard never fires, and the caller gets a message that reads like a bug in
obliterator's internals. Reproduced verbatim. Low severity, but it is a real gap in a guard that
exists — two lines would close it. Cross-ref B-30, above, and two further blind spots in the same
guard.

**Three defects in this port, all found by differentially probing the *bridge*, and two of them in
code that was already green.** None is an upstream bug; they are recorded because the way they were
found is the point.

1. **`&self` on a `Freeze` type is `noalias readonly`, and LLVM used it.** napi hands the same
   object to JS as `&self` and `&mut self`, and JS re-enters from a callback:

   ```js
   q.forEach(function (value, i) { if (i === 0) { q.dequeue(); q.dequeue(); } });
   ```

   Upstream: `1, 4, undefined, undefined`. The port: `1, 2, 3, 4` — the read was hoisted out of the
   loop, while the *same object* reported its new `offset` correctly one line later. Fixed by typing
   it honestly: the bridges hold `RefCell<Core*>`, which is not `Freeze`. **The `sparse-set` bridge
   has the same defect and is not fixed here** — it is already in `tests/scope.txt`, and a
   `forEach` callback that deletes yields `[1, 4, 3, 4]` there against upstream's `[1, 4]`.

2. **napi's `#[napi(iterator)]` installs a `#.return` that latches.** `obliterator/iterator` has
   none, so `break` leaves a cursor resumable; napi's writes `[[GeneratorState]] = true` *before*
   the Rust `complete()` runs, so `complete` cannot prevent it. The addon deletes the method at
   load. **This corrects a claim in the `sparse-set` bridge's docs** that napi's default `complete`
   is observably equivalent — it was reasoned about, not measured.

3. **`napi_create_reference` rejects primitives** below Node-API 10, and napi-rs 3.12 does not
   export `node_api_module_get_api_version_v1`, so the addon is a version-8 module however its
   Cargo features are set (moving `napi9` → `napi10` changes nothing — measured). `JsSlot` is
   therefore an enum: references for object/function/symbol, by value for primitives.

**A claim this module's own falsification withdrew.** An earlier comment said routing `Stack.of`
through `Stack.from(arguments)` makes the original suite exercise branch 1's `[object Arguments]`
clause. Deleting the clause leaves all 22 assertions green: a modern `arguments` object carries
`Symbol.iterator` and falls through to branches 3/4 with the same numeric second argument. The
clause is observable only for something claiming the tag *without* being iterable. Corrected in
both places rather than quietly dropped.

**What the fuzzer found: nothing new.** Two campaigns, 4.40 M operations, zero divergences — the
expected outcome, since a faithful port reproduces upstream's bugs and differential fuzzing
structurally cannot find them. What it is for is the other direction, and it was proven in
that direction below.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-41 | **The backing store is `Rc<RefCell<Vec<T>>>`, not `Vec<T>`.** | A JS array is a reference, and `clear()` **rebinds** it while `pop()` mutates it in place. A `Vec` makes those two indistinguishable and shortens an open walk for both. Mutators still take `&mut self`; this is not interior mutability for convenience. |
| D-42 | **`Sequence` gained a `limit` method.** | Defaulting to the frozen length, so every existing source is unchanged. `Queue` overrides it; `Stack` does not. Normalising the two would have been an unforced assumption. |
| D-43 | **The bridge holds `RefCell<CoreStack<JsSlot>>`.** | Because `&self` is `noalias readonly` for a `Freeze` type and JS mutates through the same pointer. The fix is the type, not a barrier — see "Bugs this found" (1). |
| D-44 | **Values are a `JsSlot` enum, not one `napi_ref` each.** | `napi_create_reference` rejects primitives for a version-8 module. Observationally exact, because primitives are immutable and compared by value: `Object.is` cannot tell a rebuilt `-0` or `NaN` from the original. |
| D-45 | **`Stack.of` is installed as evaluated JavaScript.** | napi-rs has no variadic parameter and `arguments` has no Rust representation. A fixed literal, evaluated once at load; it is upstream's own line, and it keeps the addon self-contained. Behaviourally identical to a native implementation — measured. |
| D-46 | **napi's generator `#.return` is deleted from every cursor.** | Upstream cursors have no `return`, so `IteratorClose` finds nothing and a `break` leaves the walk resumable. |
| D-06 | **No collection implements `IntoIterator`.** | It would hand out a fresh iterator per `for` loop and silently restart. Collections expose `values()`; the `Cursor` is the stateful thing. |
| D-07 | **`Symbol.iterator` is installed from Rust, not from the shim.** | The factory half is the one napi does not provide. A shim that added semantics would mean the addon was incomplete without the test harness. |
| D-03 | **`forEach` lives in `mnemonist-napi`, not core.** | Every one of the 30 call sites operates on a user-supplied JS value inside `.from()`. Core takes `IntoIterator`; a Rust caller never meets the dispatch. |
| — | **`size` and `items.length` are kept as separate quantities.** | They coincide on every public path, but upstream tracks them separately and `values()` is defined against the second. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion and no Rust equivalent. |
| — | **`forEach(cb, undefined)` binds `this` to the stack.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see. The omitted-argument case — the only one the original suite uses — is exact, and passing a real scope object is exact. |

## Fuzz + bench

### Fuzz

```
module=stack seed=42       cases=28240 ops=2823415 wall=120.0s divergences=0
module=stack seed=20260801 cases=15752 ops=1579917 wall=60.0s  divergences=0
```

Two campaigns, two seeds, **4.40 M operations, zero divergences**.

Reproduce with `target/release/difffuzz --module stack --seed 42 --cases 28240`.

* **Op alphabet:** `push(v)` (weight 6) · `pop()` (3) · `peek()` (2) · `clear()` (2) ·
  `$iter("values")` (2) · `$next()` (4) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, **`items`** and `toArray()`. `items` is a
  public property upstream, and observing it directly is what makes the array-rebinding checkable
  without waiting for a cursor to notice it. Comparing `size` *and* `items` separately is how a port
  that silently unified the two would be caught.
* **Values:** `0..48`, small enough that duplicates are frequent — a stack is not a set.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.** Every method `stack.js` exposes is in the alphabet or the
  observation set, except `inspect`, which is not ported.

The grammar's point is the **pair**: `clear()` rebinds the array and `pop()` shortens it, and a
cursor open across either must react differently. That is why `clear` carries real weight rather
than being a token op.

**The fuzzer was falsified before it was trusted.** Sabotage: `clear()` emptying the backing array
in place instead of rebinding it — which is the only thing a `Vec<T>` can do, and which makes
`clear()` indistinguishable from popping everything. Caught in **101 cases (0.1 s)**, shrunk from
200 ops to four:

```js
var s = new Stack();
s.push(0);
var it = s.values();
s.clear();
it.next();      // port {value: undefined}, upstream {value: 0}
```

Reverted; the seed is committed with a provenance header in
`crates/difffuzz/proptest-regressions/stack.txt`, where proptest replays it before any novel case
on every subsequent run.

### Falsification of the port (gate 6)

Gate 6 asks that sabotaging the core turns the **original mocha suite** red, proving it exercises
Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create a values iterator` —
`assert.strictEqual(iterator.next().value, 3)`, at `test/stack.js:102`. Chosen because it is the
first assertion in the file that reaches the reversed cursor, which is the arithmetic most likely
to be mis-ported.

**The sabotage:** `Sequence::slot` for `Stack` walking **forward** (`items[ordinal]`) instead of
newest-first (`items[l - ordinal - 1]`).

**Confirmed red**, and red in the named place: `8 passing, 3 failing`, the failures being the
values iterator, the entries iterator and the `for…of` block. Note what stayed green — `toArray`,
which does its own reversal — so the sabotage isolated the cursor rather than the module.
Reverted; **confirmed green again**: `11 passing`.

**A second, separate falsification, of the dispatch.** Sabotage: an off-by-one in branch 1
(`i + 1 < l`, dropping the last element of every indexed sequence). Named assertion:
`should be possible to create a stack from an arbitrary iterable`,
`assert.deepStrictEqual(stack.toArray(), [3, 2, 1])` at `test/stack.js:88`. **Confirmed red**,
`13 passing, 9 failing` across both stack and queue. Reverted, green again.

**A third attempt that stayed green, and what it proved.** Deleting branch 1's `[object Arguments]`
clause left all 22 assertions passing — see "Bugs this found". A falsification that cannot fail is
just a second green light; this one was informative precisely because it failed to fail, and the
claim it disproved was withdrawn from two source comments.

### Bench

**Not run.** Benchmarks need an idle machine, and this machine was under contention at the time;
gate 10 is batched into a quiet pass. `stack` is ready for it: the workloads it needs are
push/pop churn and a full drain, both of which the existing `bench/runner` shape already supports.
Until that pass lands, this unit is **not** in `tests/scope.txt` and does not claim to be done.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`stack`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 4.40 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `pop()`, `push(a0)` and `clear()`, all uncapped.

Safe uncapped because `l = this.items.length` is captured before the first step. The interesting
program is a callback that pops: `l - i - 1` is then computed from the **old** length against the
**new** array, which opens an `undefined` hole mid-walk — the behaviour `tests/boundary/stack-queue.js`
pins by hand and this op now generates by the thousand.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["stack"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peek`/`pop` (50/25/25), value magnitude 1e6, xorshift32 seed 42.
`peek` stands in for `vector`'s `get`, since a stack exposes no random access — otherwise the same
shape `vector`'s own bench uses, chosen for the same reason: the throughput floor other members of
this group can be read against.

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4.6** | 7.3 | 1.6× faster |
| p99 ns/op | **6.8** | 30.3 | 4.5× faster |
| min ns/op | **4.2** | 5.2 | 1.2× faster |
| RSS delta MB | **7.9** | 44.9 | |
| structure-only RSS delta MB | **1.3** | 9.9 | |
| startup ms | **0.6** | 15.7 | 26× (reported separately; not throughput) |

**No regressions on any metric.** `Stack`'s backing is `Rc<RefCell<Vec<f64>>>` (D-06/D-07's
array-rebinding requirement — see the module docs), so every `push`/`pop` pays a refcount bump and a
borrow-flag check, the same mechanism `heap.md`'s bench found a regression from. It does not show up
here: unlike `heap`, there is no `Comparator` trait call riding alongside it, and V8's own `Array`
push/pop is not free either — it must handle the JS `Array`'s own bookkeeping (length, potential
hidden-class transitions) on every call. The `RefCell` check is real but small enough, relative to
what V8 pays for the same operation, that it does not flip the result. Unconfirmed which side of that
comparison dominates by how much: not isolated by profiling here, only that the direction is
consistent with "a `RefCell` alone is a small cost" — see the dedicated `--refcell-probe` measurement
in `docs/modules/sparse-set.md`, which measured that mechanism in isolation and found it
indistinguishable from run-to-run noise at this workload's size.
