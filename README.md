# mnemonist-rs

A Rust port of [`mnemonist`](https://github.com/Yomguithereal/mnemonist), a library of 44 JavaScript
data structures. Produced for Port Mortem 2026.

The deliverable is a standalone Rust crate. `mnemonist-core` has no dependencies, declares
`#![forbid(unsafe_code)]`, and builds and tests without Node installed. The original JavaScript test
suite serves as the equivalence proof rather than a runtime dependency: those tests execute
unmodified against the Rust build through an N-API bridge that forms no part of the published crate.

```
43 of 44 upstream structures ported        42 of 42 upstream test files ported
42 units through all ten gates (100%)      733 upstream specs passing, unmodified
130.0M differential fuzz operations        zero divergences
71 upstream defects examined               12 documented in full
```

One upstream file is not ported. `semi-dynamic-trie.js` (251 LOC) is absent from `mnemonist`'s own
`index.js`, has no file in the published test suite, is required only from upstream's internal
`benchmark/` directory, and still carries `TODO: rename => ternary search tree` in its header. A
port of it could not have been checked by anything in the original suite, which is the evidence this
submission rests on, so it is out of scope and named here rather than absorbed into a count.

Of the 43 that are ported, 40 are in `mnemonist-core`. The remaining three — `lru-map`,
`lru-cache-with-delete`, `lru-map-with-delete` — are upstream files whose only difference from
`lru-cache` is which key identity they use and which half of an entry they project. Key identity is
a JavaScript-value question, so those three are assembled at the bridge over the same core cache.
Upstream ships no separate test file for any of them: `test/lru-cache.js` requires all four and
exercises them together, which is what makes that closure one unit rather than four.

## Scope of fidelity

The port reproduces `mnemonist`'s observable behaviour, including its defects. This constraint has a
cost, and the affected sites are documented rather than left to be discovered:

- `MultiSet`'s size counter is a tracked value rather than a derived one, so that upstream's drift
  on a failed delete reproduces instead of self-correcting.
- `FibonacciHeap::size` is an `i64`, because JavaScript has no unsigned integers and upstream drives
  the counter to `-1` under a re-entrant `clear()`.
- Certain error messages are pinned to upstream's exact `TypeError` text, making message content
  part of the contract.

`mnemonist-rs` is therefore a compatibility crate rather than a general-purpose data-structure
library. Every deliberate deviation is recorded on the affected item and in that structure's
divergence document.

Where fidelity and Rust idiom did not conflict, the implementation is conventional. `clippy
--all-targets -D warnings` is clean across approximately 72,000 lines, with 25 lint suppressions:
24 concern signature shape or ergonomics (18 `type_complexity`, 5 `too_many_arguments`, 1
`new_without_default`), and one is an `if_same_then_else` in `passjoin-index`, where two branches
share a body but not their guards because only the second calls `levenshtein`. No suppression
conceals a numeric or comparison lint.

## Documentation

| Document | Contents |
|---|---|
| [docs/METHODOLOGY.md](docs/METHODOLOGY.md) | The ten verification gates, what each detected, and the limits of each instrument |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate structure, the role of the bridge, and the six sites where fidelity displaced idiom |
| [docs/BUGS.md](docs/BUGS.md) | Defects identified in the original library, with reproductions |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Deliberate divergences from upstream behaviour |
| [docs/modules/](docs/modules/) | 46 per-unit documents covering upstream coverage, gaps, and additions |

**Bug Catcher submission.** The upstream defects this entry claims are
[docs/BUGS.md](docs/BUGS.md), in full and in one place: 12 written up individually with
reproductions, 48 more in a summary table, and 3 held separately as lower-confidence candidates
because each has a plausible reading as intentional design. Every entry gives the reproduction, what
the original does wrong, and how this port handles it; every row in the table was re-confirmed
against Node 24.18.1 while the document was written rather than carried over from working notes.

Two further candidates are listed as *not* bugs, having proved unreachable through any public
sequence of calls, and one file was read end to end and found to have nothing to file. The document
also records a negative result about its own ranking: a defect whose upstream test asserts the buggy
output as correct would prove the bug survived review, and no case of that kind was found — in every
defect claimed here, the existing suite never reaches the state where the bug lives.

## Verification

```bash
docker build -t port-mortem . && docker run --rm port-mortem
```

The above builds the crate, the bridge and the harness, then executes the unmodified upstream suite.
A second target builds `mnemonist-core` against a base image containing no JavaScript runtime:

```bash
docker build -t pm-core --target core . && docker run --rm pm-core
```

Individual checks:

```bash
cargo test                       # 799 native tests
cargo run --release --example tour -p mnemonist-core   # the crate used from Rust
./tests/run.sh                   # 733 upstream specs, unmodified, via the bridge
./tests/verify.sh                # all ten gates, per unit claimed complete
sha256sum -c tests/SHA256SUMS    # upstream tests byte-identical to published
scripts/status.sh                # derived coverage and per-unit evidence
```

All 42 vendored upstream test files are SHA-256 checked on every commit. Modifying any of them
fails the build.

`tests/scope.txt` records completed units. A unit is listed only after all ten gates pass, or after
an explicit exemption recorded in both `bench/results.json` and the unit's divergence document. Two
units carry a benchmark exemption on that basis: `default-weak-map`, whose entries are reclaimed at
the garbage collector's discretion, so a timing measurement would characterise V8 rather than the
structure; and `_utils`, a require-closure of five unrelated pure-function files sharing no
instance. Both pass gates 1 through 9 in full. `verify.sh` accepts an exemption only when a reason
is given and the unit's divergence document states it; a missing benchmark still fails.

## Performance

Measured against the vendored upstream JavaScript using matched operation streams, interleaved, on
an idle machine. Full methodology in [docs/METHODOLOGY.md](docs/METHODOLOGY.md).

**44 benchmarked workloads across 40 structures. 39 are faster than upstream, 5 are slower.** The
median is 1.46× faster. Two further units carry a benchmark exemption, described below.

**Every workload that is slower**, without exception — this is the complete list, not a selection:

| Structure | Workload | Median, relative to upstream |
|---|---|---|
| `bi-map` | `mixed-1e6` | 1.51× slower |
| `default-map` | `mixed-1e6` | 1.44× slower |
| `multi-array` | `mixed-1e6` | 1.31× slower |
| `heap` | `mixed-1e6` | 1.31× slower |
| `default-map` | `mixed-4e6` | 1.13× slower |

The largest wins, for scale:

| Structure | Workload | Median, relative to upstream |
|---|---|---|
| `fibonacci-heap` | `mixed-2e5` | 24.9× faster |
| `suffix-array` | `build-2e4x50` | 7.5× faster |
| `linked-list` | `mixed-1e6` | 4.2× faster |
| `sparse-set` | `drain-1e5` | 3.3× faster |
| `trie-map` | `mixed-2e5` | 2.8× faster |
| `trie` | `mixed-2e5` | 2.7× faster |

All 44 are in [`bench/results.json`](bench/results.json), keyed per unit and per workload, each with
its own `regressions` array. `scripts/status.sh` reads the same file.

### How stable these figures are

Every workload above was re-measured in one serial pass on an idle machine, because the JavaScript
baseline is not stable across sessions and a table whose rows come from different days cannot be
read down the column. Within a session, back-to-back runs of the same workload agree to 0.9% on both
sides. Across sessions they do not: `kd-tree`'s upstream figure moved 22% and `multi-array`'s 13% on
unchanged code.

That instability is not symmetric, and it cost this table two rows. `multi-set` and `bi-map` were
previously recorded as wins at 0.85× and 1.15×. In this pass both measured as losses, and a further
spot-check of each in isolation on a settled machine measured them **worse still** — 1.29× and 1.56×
against the pass's 1.13× and 1.35×. Both were therefore treated as losses at the worse of the two
figures, on the principle that between two honest measurements the unflattering one is the safer
claim.

Being counted as losses is what got them looked at. `multi-set` and `bi-map` both read a key and
then wrote it back unconditionally, hashing it twice on every call; upstream cannot avoid that,
since a JavaScript `Map` has no look-up-and-update-in-place operation, but `OrderedMap::get_mut`
can. On `multi-set`, where `add` is half the workload's operations and every operation is O(1) map
bookkeeping, that halved the hashing and took the port from 24.80 ns to 16.1–16.4 ns across four
runs — **1.36× faster than upstream**, out of the loss column entirely.

The two regressions nobody had ever looked at turned out to be the cheapest to fix, which is the
pattern this project keeps rediscovering. `bit-set` converted its index to `f64` and ran JavaScript's
full `ToInt32` — truncate, `rem_euclid(2^32)`, sign fixup — on every `set`, `get`, `reset` and
`flip`, although `ToInt32` is the identity for any value already inside `i32`'s range. An
`i32::try_from` fast path, falling back to the float path only for indices that genuinely need it,
took the port from **8.68 ns to 5.91 ns**, and `bit-vector`, which shares the same code, came with
it. `fixed-critbit-tree-map`'s `set` allocated two fresh `Vec`s per call as scratch for its walk;
they are now reused struct fields, cleared on entry, taking the port from **405 ns to 292 ns**.

Both figures come from six runs alternating the old and new code, and in both the upstream side held
steady across all six — 7.85–7.91 ns for `bit-set` — so the port-side change is unambiguous.
`bit-set`, `bit-vector` and `fixed-critbit-tree-map` are all now faster than upstream.

`bi-map` got the same treatment and it made no measurable difference. Six interleaved runs of the
old and new code under identical conditions put the port at 169.9 ns before and 164.6 ns after, a 3%
gap inside a 10% run-to-run spread. The change is kept because one lookup is not worse than two and
the code is no more complex, but no speedup is claimed for it. `bi-map` is also the least stable
measurement in this table: its ratio spanned 1.14× to 1.59× across those six runs, where every other
module reproduces to about 1%. Its published figure should be read as "slower, by somewhere between
a little and a half", not as 1.51.

`kd-tree` was the largest regression at 2.18× slower and is now faster than upstream. Its k-nearest-
neighbour search built a fresh heap-allocated tuple per node visited, and the heap's store clones a
slot on every sift step, so the allocation was closer to once per comparison than once per push.
Fixed-size `[f64; N]` tuples are `Copy`, making that clone a stack copy: the port's own time fell
from 2049 ns to 755 ns, a change far larger than the baseline drift above. It now reads 1.23× faster
than upstream in this pass.

`multi-array` is where measurement disagreed with the reasoning. Narrowing its bookkeeping from
`usize` to `u32` was expected to reduce the cache-miss cost of its bucket walk; measured, it bought
3.9% on the port's own time — kept, but too small to call the width the cause. Removing a zero-fill
found afterwards bought 17%: `get` allocated its result with `vec![0.0; n]`, memsetting bytes that
the next `n` steps immediately overwrote. It now builds by `push` and reverses, and matches the
storage discriminant once per call instead of once per element. The port's own time went 50.2 ns →
48.3 ns → 38.3 ns across the two changes.

What remains is the allocation itself: `get` returns a fresh container per call, and a bare
`Vec::with_capacity(25)` plus fill already accounts for 34.9 ns of it. Both sides allocate, and V8's
nursery is simply better at this shape than a general-purpose allocator. That is the residue, and it
is not addressable without changing what the method returns.

`default-map` is the largest regression left, and it is the one whose cause this port does **not**
claim to know. The obvious explanation — a duplicate hash lookup — was measured and refuted: a probe
separating a peek from a hit put the two 0.7% apart, where a second lookup would have shown up
plainly. What replaced it is an account, not a finding: at a million-key domain `OrderedMap`'s
internal `HashMap` no longer fits in cache and a uniformly random key makes most lookups a real
memory access. That is consistent with the regression *narrowing* from 1.44× to 1.13× as the domain
grows, but it was never isolated against a metric that would have falsified it — the domain-size
sweep with hardware cache-miss counters that would settle it is not available on this host. The
module's own document says so in those words, and this summary does not upgrade it.

The structural fix — storing values inline and tracking order separately — was costed rather than
waved away: roughly 6.5 to 11.5 hours, because `OrderedMap` is used by eight units that are all
complete through every gate, and each would need its falsification redone, its differential campaign
re-run and its benchmark re-measured. It was declined on two grounds. The time is most of what
remained before the freeze, with no slack. And the benchmark's operation mix is three-quarters
writes, which have to touch a second structure under any design that keeps insertion order and the
cursor semantics — so even a successful rewrite would only clearly help the remaining quarter, and
should not be expected to close a 1.44× gap.

The `heap` figures are the clearest measure of what fidelity costs. The delivered implementation
runs at 31.7 ns against upstream's 24.3 ns; a bare `Vec<f64>` heap over the identical workload runs
at 21.7 ns — a separate probe, not part of the table above — so the `RefCell` and comparator
indirection accounts for more than the entire regression.
That indirection is what permits a JavaScript comparator to re-enter and mutate the heap mid-sift,
which upstream behaviour requires.

## Layout

```
crates/mnemonist-core     the crate — 40 structures, no dependencies, no unsafe code
crates/mnemonist-napi     the N-API bridge, used only to run the upstream suite;
                          also the three LRU siblings, which differ from
                          lru-cache only in key identity and projection
crates/difffuzz           differential fuzzer: core against upstream in Node
bench/                    matched benchmark harness, Rust and JavaScript halves
tests/original/           the upstream suite, byte-identical and hashed
docs/modules/             one divergence document per unit
```

## License

MIT, matching upstream. `mnemonist` is © Guillaume Plique and contributors; the vendored test suite
under `tests/original/` is redistributed unmodified under that license.
