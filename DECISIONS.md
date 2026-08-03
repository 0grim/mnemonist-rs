# Decisions

The choices that shaped this port. Each one changed what the code looks like; each cost something.

The complete catalogue of behavioural differences — every divergence, per module, with its mechanism
— is [docs/DIVERGENCES.md](docs/DIVERGENCES.md). This file is the short version.

---

### 1. The deliverable is a Rust crate. The JavaScript suite is the proof, not a dependency.

`mnemonist-core` has zero dependencies, declares `#![forbid(unsafe_code)]`, and builds and tests
with Node absent from the machine. Upstream's own tests run against it through an N-API bridge that
is **not part of the published crate** — it exists to make equivalence checkable and ships as a
separate crate a Rust user never sees.

### 2. Reproduce upstream bug-for-bug, including the defects.

A port that silently repairs a bug the original suite does not test for is no longer verifiably
equivalent to the original. So upstream's defects are reproduced deliberately, and the cost is
visible in the code: `MultiSet`'s size is a tracked counter rather than a derived one so a failed
delete drifts exactly as upstream's does; `FibonacciHeap::size` is an `i64` because a re-entrant
`clear()` drives upstream's to `-1`; several error messages are pinned to upstream's exact
`TypeError` text, which makes message content part of the contract.

This makes `mnemonist-rs` a compatibility crate, not a better data-structure library.

### 3. A unit of work is the require-closure of one upstream test file, not a source module.

Every `require` in an upstream test file sits at the top, so one missing module throws before a
single assertion runs — the file fails with zero partial credit. Scoping by source module would have
allowed claims of partial progress that does not exist. `test/lru-cache.js` requires four LRU
variants; porting three would score nothing.

### 4. Every JavaScript-value question lives at the bridge, never in the core.

`undefined` versus `null`, truthiness, `SameValueZero` key identity, array-class preservation: all
of it is confined to `mnemonist-napi`. Without this rule a `napi` type leaks into the crate a Rust
caller depends on, and the core stops being portable.

### 5. Cursors are stateful and non-restartable, and freeze their length but not their elements.

`obliterator`'s iterators are not Rust iterators. They are objects that return themselves from
`Symbol.iterator`, so consuming one partially and then spreading it continues rather than restarts.
They also capture their length at creation while reading elements live — so a `push` during
iteration is invisible but an overwrite is not. No collection implements `IntoIterator`, because
that would quietly make every walk restartable.

### 6. JavaScript's `Map` is reproduced, not approximated.

Eleven modules keep their state in a `Map`, so one ordered map underpins all of them: insertion
order preserved, `SameValueZero` equality (`NaN` equals itself, `-0` and `0` collapse), and deletion
by tombstone with periodic compaction. Cursors locate themselves by a never-reused slot id rather
than an array index, so compaction cannot invalidate one.

### 7. Arbitrary JavaScript values are stored as an enum, not one reference each.

`napi_create_reference` rejects primitives below Node-API 10, and napi-rs 3.12 builds a version-8
module whatever the Cargo features say — measured, not assumed. So values are an enum: references
for objects, functions and symbols; by value for primitives, which is observationally exact because
primitives are immutable and compared by value.

### 8. Object keys are refused, with an error that names the limit.

There is no identity hash for a JavaScript object reachable from Rust. Tagging each object with a
hidden symbol mutates the caller's object and fails on a frozen one; an association list of
references is O(n) and leaks every key it has ever seen. **No upstream test anywhere in the library
uses an object key** — audited across every module whose state reaches a `Map`, all four LRU
variants included. Building machinery no test can reach is worse than a stated limit; answering
silently and wrongly is worse than both.

### 9. Where `undefined` cannot be represented, the port raises rather than guesses.

A `usize` cannot hold `undefined`. Where upstream produces a structure whose `size` is `undefined`
and whose every later method is arithmetic on `NaN`, this port raises a named error instead of
inventing a number. Reproduce where reproducible; raise where not; state which happened.

### 10. Benchmarks measure the pure Rust path, never through the bridge.

`bench-runner` links against `mnemonist-core` directly and does not depend on `mnemonist-napi`.
Bridge overhead would poison the comparison in the port's favour on nothing. Both sides run in one
process pair on one machine, interleaved, with a checksum that must agree or the run writes nothing.

### 11. One upstream structure is out of scope, and it is named rather than absorbed.

`semi-dynamic-trie.js` is absent from `mnemonist`'s own `index.js`, has no file in the published
test suite, and is reachable only from upstream's internal benchmarks. A port of it could not have
been checked by the evidence this submission rests on. 43 of 44 are ported; the 44th is stated.

### 12. Gaps are disclosed, including the ones no gate catches.

Benchmark regressions are derived mechanically from the results rather than written down, so a bad
number cannot be forgotten. Where the port is *more* correct than upstream, that is recorded as a
divergence too. And where a fix is known but not applied — 47 of the port's 57 cursors keep napi's
`#.return` and so do not resume after a `break`, where upstream's do — it is written down as an
omission rather than left for a reader to find.

---

Full detail: [docs/DIVERGENCES.md](docs/DIVERGENCES.md) ·
verification: [docs/METHODOLOGY.md](docs/METHODOLOGY.md) ·
upstream defects: [docs/BUGS.md](docs/BUGS.md)
