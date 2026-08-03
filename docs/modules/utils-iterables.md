# utils/iterables

Upstream: `utils/iterables.js` (93 LOC), on `obliterator/foreach` and `utils/typed-arrays.js` ·
**no test file of its own.**

Port: `crates/mnemonist-napi/src/iterables.rs` — at the boundary, not in core.
Specs: `tests/boundary/iterables.js`, 19 of them.

---

## Scope note: this is not a "unit"

A unit is the require-closure of one upstream *test file*, and `utils/iterables.js` has none — so
gates 3, 4, 6 and 10 have no target here and this file will never appear in `tests/scope.txt` on
its own. It is a **member** of the `fixed-stack`, `fixed-deque` and `circular-buffer` units (and,
eventually, of `_utils`, `vector`, `kd-tree`, `vp-tree` and every other `.from()` caller), and the
gates that apply to it are 1 (ported), 2, 7 (its specs) and 8 (this document).

It is *reachable* from `test/_utils.js`, whose require-closure is `typed-arrays` +
`binary-search` + `hash-tables` + `iterables` + `merge` — about 1,166 LOC, all five of which must
exist before one assertion in that file runs. That closure is out of scope, so **every assertion in
`tests/boundary/iterables.js` is coverage upstream does not have.**

## Why it is in `mnemonist-napi` and not in `mnemonist-core`

All four functions are JavaScript-value questions. `isArrayLike` asks `Array.isArray ||
ArrayBuffer.isView`; `guessLength` reads two properties and checks their `typeof`; `toArray`
preallocates a JS array and drives `obliterator/foreach`; `toArrayWithIndices` picks a typed-array
constructor. None of that has a Rust meaning, and the same grep — every call site is inside
a `.from()` static or an iterable-accepting constructor, operating on the *user-supplied argument*
— applies to this file as directly as it does to `forEach`.

It is built **on** `crate::foreach::for_each`, not on a second copy of the five-branch dispatch.
The collector it passes is a real JS function, so branch 2 hands a host `forEach` exactly the kind
of callback it expects and nothing about the delegation is simulated.

## What upstream tests

**Nothing directly.** The four functions are exercised only incidentally, through the `.from()`
statics of the modules that import them — and in this wave's three modules, only `guessLength` and
`isArrayLike` are ever reached, because the `from` branch that would call `toArray` does not exist
(B-60).

## What upstream does NOT test

Everything. The list below is what the boundary specs cover, organised by function.

**`isArrayLike`**

1. That it is **false for `{length: 2}`** — the thing "array like" normally means.
2. That it is **true for a `DataView`**, which holds no elements at all: `isTypedArray` is
   `ArrayBuffer.isView`, which a `DataView` satisfies. A `from` taking the array-like branch on one
   reads `.length` (undefined) and copies nothing.
3. That it is **false for a string**, which is why `FixedStack.from('abc', Array)` takes the other
   branch and dies in B-60.

**`guessLength`**

4. That `.length` wins over `.size` when both are present and disagree.
5. That a **non-numeric `.length` is ignored** and `.size` is then consulted.
6. That it **validates nothing**: `-1`, `3.5` and `NaN` are all returned as they are, because all
   three are `typeof 'number'`.
7. That `null` and `undefined` throw from the property read, with V8's wording, rather than from
   any guard of the function's own.

**`toArray` — B-2**

8. That an **overstated length leaves real holes**, not `undefined`. `{length: 5}` yielding two
   values gives `[1, 2, <3 empty items>]`: `length === 5`, `2 in array === false`, and
   `Array.prototype.map` skips them.
9. That an **understated length is silently exceeded**, because `array[i++] = v` grows the array.
10. That an **invalid length throws `RangeError: Invalid array length`** — from the allocation, not
    from a guard.
11. The sharpest form of B-2: `toArray({length: 5})` reaches `forEach`'s **plain-object** branch,
    which enumerates own properties *including `length` itself*, so the array's first element is
    the number 5.
12. That `forEach`'s falsy guard still applies, so `toArray('')` throws.

**`toArrayWithIndices`**

13. That the index width comes from the **guess**, not from the yield count: 3 → `Uint8Array`,
    300 → `Uint16Array`, 70000 → `Uint32Array`.
14. That the indices array is the identity over the filled prefix and the class zero after it.
15. That `getPointerArray` throws **before** `new Array(l)` does, which is decided purely by the
    order of two statements upstream.
16. That an unguessable target gets two plain, growing arrays — and that a `Map` is *not*
    unguessable, because it has `.size`.

## What we test in addition

`tests/boundary/iterables.js` — 19 specs, closing every gap above.

They are **differential**: each case is run through the port and through a verbatim inline copy of
the four upstream bodies, built on the genuine `obliterator/foreach` devDependency, and the two
outcomes are compared with a description that distinguishes a hole from an `undefined`, a
`Uint8Array` from an `Array`, and a length from a filled prefix. The inline copy exists because the
vendored `bench/upstream/utils/iterables.js` cannot resolve its own `require('obliterator/foreach')`
from inside the assembled work tree; a further spec checks the copy against the vendored source
line by line, so it cannot silently drift. Full spec list: evidence file.

`crates/mnemonist-core/src/utils/typed_arrays.rs` supplies `get_pointer_array`, which is the one
part of this file that *is* pure computation and which already has ten native tests of its own.

**Still untested, stated rather than glossed:** `toArray` on a target whose `forEach` throws
mid-iteration (the partially-filled array is observable, and nothing pins it), and `guessLength` on
a `Proxy` with a throwing `get` trap. Neither is reachable from any module in scope.

## Bugs this found

**B-2 — `toArray` produces sparse arrays when `guessLength` lies.** Verified against Node
24.18.1. Recorded before kickoff as the strongest of the pre-port bug candidates; this is its
confirmation and its reproduction. The three measured forms are gaps 8, 9 and 10 above, and the
sharpest is gap 11 — `toArray({length: 5})` returning `[5, <4 empty items>]`, where the `5` is the
`length` property itself, enumerated by `forEach`'s plain-object branch.

**B-60 — `iterables.forEach` does not exist**, and three modules call it. Found while porting
`fixed-stack`; it belongs to this file as much as to them, because the missing export is here. See
`docs/modules/fixed-stack.md` and NOTES B-60.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-60 | **B-2 is reproduced, not repaired**. | The array is really allocated by calling the running realm's `Array` constructor rather than by `napi_create_array_with_length`, so the holes are real holes and the `RangeError` is V8's own. The two calls differ exactly where this module is interesting: `napi_create_array_with_length(-1)` does not throw. |
| D-18 | **`guessLength` trusts `.length` then `.size` without validating.** | Confirmed rather than changed; it is what feeds D-60. |
| D-39 | **`guessLength` returns `Either<f64, Undefined>`, not `Option<f64>`.** | napi renders `None` as `null`, and upstream returns a bare `undefined`. |
| — | **`toArrayWithIndices` returns a real JS array**, built with `napi_create_array_with_length`, not a plain object with `"0"`/`"1"` keys. | Callers destructure it. |
| — | **The index-array width comes from `mnemonist-core`'s `get_pointer_array`.** | The one part of this file that is pure computation, so it lives in core and is shared with `sparse-set`, `sparse-map` and `static-disjoint-set` rather than reimplemented at the boundary. |

## Fuzz + bench

### Fuzz

**Not fuzzed directly, and that is a real gap rather than an omission.** The differential fuzzer
compares `mnemonist-core` against upstream; this file has no core half to drive, exactly as
`crate::foreach` has none. Its coverage is the 19 boundary specs above, which *are* differential —
against a verbatim copy of the upstream bodies rather than against a generated program — plus the
six campaigns of the three modules that reach `guessLength` and `isArrayLike` through their `from`
statics (`fixed-stack`, `fixed-deque`, `circular-buffer`; 8.79 M operations, zero divergences).

Stating the shape of that gap precisely: a generated program can reach `guessLength` and
`isArrayLike` only through construction, never as an op, and `toArray`/`toArrayWithIndices` are not
reached at all by any module currently in the port. Their coverage until a module reaches them is
the 19 hand-written cases.

### Falsification (gate 6)

Gate 6 asks that sabotaging the port turns the *original mocha suite* red. This file has no
original suite, so the gate has no target — the same position as `utils/bitwise`.

What was performed instead, on `tests/boundary/iterables.js`, and named before it was run: the
sabotage had to break `getPointerArray throws BEFORE new Array(l) does.`, which is the one spec
whose subject is the *order of two statements* rather than the result of either — the sabotage,
allocating the value array before choosing the index width, is the tidier reading a port written
from the function's description rather than from its source would produce. Confirmed red, and red
only there (18 passing, 1 failing, that spec, with the port raising a different error class than
upstream); reverted, confirmed green again (19 passing). The spec that checks the inline reference
against the vendored upstream source is a second guard of a different kind: it pins seven
distinguishing lines, so any edit to those four upstream bodies moves one of them and the copy
cannot silently drift. Full record: evidence file.

### Bench

**Not benchmarked, and not planned.** These are four helper functions on a `.from()` path, called
once per construction; the modules that use them are benchmarked as wholes. Gate 10 does not apply
to a non-unit.
