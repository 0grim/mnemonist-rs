# Architecture

The deliverable of this project is **a standalone Rust crate**. The JavaScript test suite exists to
prove that crate behaves like the library it replaces; it is not a dependency of it, and nothing in
the shipped crate knows JavaScript exists.

That single constraint determines almost every structural decision below.

---

## The four crates

| Crate | Lines | Role | Ships to a Rust user? |
|---|---|---|---|
| `mnemonist-core` | 34,909 | **The product.** 40 data structures plus supporting utilities | **Yes** |
| `mnemonist-napi` | 18,785 | The proof harness — a Node addon letting the original JS tests call in | No |
| `difffuzz` | 13,805 | Differential fuzzer comparing the core against real upstream JavaScript | No |
| `bench/runner` | — | Matched benchmark driver | No |

Only the first is the deliverable. The other three exist to produce evidence about it.

---

## `mnemonist-core` — the product

**Three properties are enforced by the build**, not by convention:

- **`#![forbid(unsafe_code)]`.** Not `deny` — `forbid`, which cannot be locally overridden.
- **A zero-dependency tree.** `cargo tree -p mnemonist-core` emits exactly one line: the crate
  itself. No serialisation library, no runtime, nothing transitive.
- **It builds and tests with Node absent from the machine.** A Rust user needs no JavaScript
  toolchain, and the check is run with Node removed from `PATH` rather than assumed.

Internally it is organised by domain:

```
crates/mnemonist-core/src/
├── structures/   40 data structures — the public surface
├── utils/         8 shared modules: comparators, typed arrays,
│                  binary search, hash tables, merge, bitwise
├── cursor/        iteration primitives shared across structures
├── map/           key/ordering machinery for the map-backed family
└── sort/          the sorting routines upstream ships with
```

---

## `mnemonist-napi` — the proof harness

A Node addon built with napi-rs. Its job is to let **unmodified** upstream test files resolve
`require('../lru-map.js')` and reach Rust.

It is also, deliberately, **where all JavaScript weirdness is quarantined.** These modules exist
solely to model semantics that have no Rust equivalent:

| Module | What it models |
|---|---|
| `js_value.rs`, `js_slot.rs` | a value that may be absent, `undefined`, or `null` — three distinct states |
| `js_key.rs` | `SameValueZero` key identity, including `NaN` as a key and `-0` colliding with `0` |
| `foreach.rs` | the five-branch `forEach` dispatch, whose second callback argument varies by input type |
| `iterables.rs` | `isArrayLike` and friends, which accept a different set of inputs than `forEach` does |
| `array_class.rs` | preserving which typed-array constructor a structure was built with |
| `js_array.rs` | arrays whose holes are distinguishable from stored `undefined` |
| `cursor.rs`, `map_cursor.rs` | iterators that survive being called back into from JavaScript |
| `statics.rs` | static factory methods (`.from`) as JavaScript exposes them |

**None of this appears in the crate a Rust user depends on.** That is the point of the split.

### The boundary rule

When a behaviour is JavaScript-shaped, the bridge absorbs it and the core stays clean. Two worked
examples:

**`fuzzy-map`'s hash functions are JavaScript callbacks.** The core therefore accepts an
*already-hashed* key and never holds a callback; the bridge owns the function reference and calls
it. A Rust user gets a structure parameterised over a plain Rust hashing closure.

**`bk-tree`'s distance function can throw.** The core exposes `try_add`/`try_search` taking
`FnMut(&I, &I) -> Result<i64, E>`, so a thrown JavaScript exception becomes a real `Err` and leaves
the tree untouched — and infallible `add`/`search` for a Rust caller whose distance cannot fail.
The fallibility is expressed in Rust's own vocabulary rather than as a special case for JS.

---

## `difffuzz` — the fuzzer, and what its position implies

`difffuzz` generates operation sequences, replays them against both `mnemonist-core` and **real
upstream JavaScript in Node**, and compares state after every operation.

Note what it connects: **core against upstream. The bridge is not in that loop.**

That is a deliberate choice — fuzzing through the bridge would test the harness rather than the
product — but it has a consequence worth stating plainly, because it took three separate
discoveries to name: **every defect that lives in `mnemonist-napi` is invisible to the fuzzer by
construction.** Reference retention, borrow discipline, argument marshalling, factory composition.
Those are found by reading, by boundary tests, and by review, and this project found real ones in
each of those ways. `docs/METHODOLOGY.md` covers the evidence.

---

## Recurring implementation decisions

Five choices recur across many structures. Each was made once, deliberately, and reused.

**Arena allocation with typed indices.** Linked structures — `linked-list`, `fibonacci-heap`, the
crit-bit trees — store nodes in a `Vec` addressed by a `NodeId` newtype rather than through
`Rc<RefCell<Node>>`. A literal translation of upstream's circular lists would be a reference cycle
that never drops. Arena indices are both safer and closer to what the original actually does.

**Comparators return `f64`, not `Ordering`.** Upstream tests `< 0`, `> 0` and `>= 0` on whatever a
comparator returns. `Ordering` has three values and cannot represent a comparator answering `NaN` or
`0.5`; collapsing to it would silently *repair* an inconsistent comparator instead of reproducing
its behaviour.

**`i64` where JavaScript uses a number.** Indices in `bit-set`, `static-interval-tree` and
`suffix-array` are `i64`, not `usize`, because a JavaScript number is an `f64` and negative or
fractional indices are reachable. `usize` would make unrepresentable a state the original reaches.

**Interior mutability at the bridge, not in the core.** Core algorithms take a store rather than
`&mut Vec<T>`, because a JavaScript comparator invoked mid-sift can call back in and mutate the
structure. An exclusive borrow would make that inexpressible rather than merely awkward. The bridge
holds a `RefCell` and takes only `borrow()`.

**Fallibility is explicit.** Any operation that can invoke user code has a `try_` form returning
`Result`, with the infallible variant as a convenience for Rust callers.

---

## Where fidelity cost idiom

This crate reproduces upstream's behaviour, **including its bugs**. That is the contract, and it has
a price. Six places where the Rust is worse than it would otherwise be:

| Site | What idiom would prefer | Why it is not that |
|---|---|---|
| `MultiSet::dimension` | derive it from the collection's length | upstream's counter drifts on a failed delete; a derived value would silently heal a bug we must reproduce |
| `FibonacciHeap::size` | `usize` | JavaScript has no unsigned integers, and upstream drives this counter to `-1` under a re-entrant `clear()` |
| Arena slot reuse | recycle freed slots | never recycling is the analogue of a garbage collector keeping a referenced object alive; by Rust standards it is unbounded growth under churn |
| `Set`-kind membership | `HashSet` with `Hash + Eq` | JavaScript `Set` semantics are `SameValueZero` over arbitrary values, which has no Rust-expressible hash — so it is a linear scan |
| Error types | idiomatic messages | some error text is pinned to upstream's exact `TypeError` string, making message content part of the contract |
| Trie node layout | value field plus a child map | one insertion-ordered list is what reproduces `for (k in node)` enumeration order |

**What this means for a user.** `mnemonist-core` is a *compatibility* crate: its contract is
"behaves like `mnemonist`, bugs included", not "the best data structures one could write in Rust".
Where a behaviour is deliberately wrong it is documented on the item itself and in that structure's
divergence document under `docs/modules/`. Nothing is left to be discovered by surprise.

Everywhere the two did not conflict, the code is ordinary Rust. `clippy --all-targets -D warnings`
is clean across the workspace, and there are **22 lint suppressions in roughly 67,000 lines**: 17
`type_complexity`, 4 `too_many_arguments`, 1 `new_without_default`. All three are complaints about
signature shape or ergonomics. **Not one suppresses a correctness or semantic lint** — no
`float_cmp`, no `comparison_chain`, no `manual_*`. Whatever fidelity cost here, it was not paid by
silencing the linter.

---

## Testing topology

```
   upstream JS test files ──(unmodified)──► mnemonist-napi ──► mnemonist-core
              │                                                      ▲
              │                                                      │
              └── proof of equivalence                    difffuzz ──┘
                                                              │
                                                              ▼
                                                   real upstream JS in Node
```

Two independent paths reach the core. The upstream suite arrives through the bridge and answers
"does it behave like the original on the cases its authors wrote". The fuzzer bypasses the bridge
entirely and answers "does it behave like the original on cases nobody wrote". Neither subsumes the
other, and the gap between them — the bridge itself — is covered by reading and targeted tests
rather than pretended away.
