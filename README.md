# mnemonist-rs

A Rust port of [`mnemonist`](https://github.com/Yomguithereal/mnemonist), a library of 44 JavaScript
data structures. Produced for Port Mortem 2026.

The deliverable is a standalone Rust crate. `mnemonist-core` has no dependencies, declares
`#![forbid(unsafe_code)]`, and builds and tests without Node installed. The original JavaScript test
suite serves as the equivalence proof rather than a runtime dependency: those tests execute
unmodified against the Rust build through an N-API bridge that forms no part of the published crate.

```
43 of 44 upstream structures ported        42 of 42 upstream test files ported
42 units through all ten gates (100%)      525 upstream specs passing, unmodified
131.2M differential fuzz operations        zero divergences
72 upstream defects examined               12 documented in full
```

**What the 525 counts, and what it does not.** `./tests/run.sh` executes 733 specs in one mocha run:
the 525 above, which are upstream's own files running unmodified, plus 208 written by this port and
kept in `tests/boundary/`. Only the first number is evidence of equivalence, so only the first is
claimed as such — the boundary specs test the bridge, which is this project's code and cannot vouch
for itself. Both numbers are reproducible: `npx mocha test/*.js` and `npx mocha boundary/*.js` from
`tests/.work` after a run.

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

## The code

Where fidelity and Rust idiom did not conflict, the implementation is conventional. `clippy
--all-targets -D warnings` is clean across every crate, with 25 lint suppressions: 24 concern
signature shape or ergonomics, and one is an `if_same_then_else` in `passjoin-index` where two
branches share a body but not their guards. No suppression conceals a numeric or comparison lint.

`mnemonist-core` denies `missing_docs` and `rustdoc::broken_intra_doc_links`, so every public item
is documented and stays that way. `mnemonist-napi` enforces the same on the ten modules with a
genuine Rust surface; its per-structure `#[napi]` methods are exempt, because their only consumer is
JavaScript and their contract is upstream's API. `crates/mnemonist-napi/src/lib.rs` states that
split rather than leaving it to be inferred.

**Size.** Rust only — the Markdown under `docs/` is documentation, not port code. Re-derive with
`scripts/loc.sh`:

| crate | code | tests | rustdoc | comments + blank | total |
|---|---:|---:|---:|---:|---:|
| `mnemonist-core` — the port itself, zero dependencies | 9,973 | 16,281 | 8,168 | 3,306 | 37,728 |
| `mnemonist-napi` — the N-API bridge | 11,451 | 109 | 4,767 | 3,441 | 19,768 |
| `difffuzz` — differential fuzzing harness | 8,718 | 1,103 | 3,206 | 2,373 | 15,400 |
| `bench-runner` — the matched benchmark harness | 2,878 | — | 1,915 | 984 | 5,777 |
| **workspace** | **33,020** | **17,493** | **18,056** | **10,104** | **78,673** |

`mnemonist-core` carries **more test code than implementation code** — 16,281 lines against 9,973 —
which is the shape a compatibility port should have. `mnemonist-napi` has almost no Rust tests by
design: what it must be correct about is what JavaScript sees, so its tests are JavaScript, under
`tests/bridge/` and `tests/boundary/`.

## Documentation

| Document | Contents |
|---|---|
| [docs/METHODOLOGY.md](docs/METHODOLOGY.md) | The ten verification gates, what each detected, and the limits of each instrument |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate structure, the role of the bridge, and the six sites where fidelity displaced idiom |
| [docs/BUGS.md](docs/BUGS.md) | Defects identified in the original library, with reproductions |
| [DECISIONS.md](DECISIONS.md) | The twelve choices that shaped the port, short |
| [docs/DIVERGENCES.md](docs/DIVERGENCES.md) | Every deliberate divergence from upstream behaviour, in full |
| [bench/methodology.md](bench/methodology.md) | How both sides are measured: what is compared, what is excluded, and the checksum that must agree before a figure is written |
| [docs/modules/](docs/modules/) | 46 per-unit documents covering upstream coverage, gaps, and additions |
| [docs/modules/evidence/](docs/modules/evidence/) | per-unit gate artifacts: coverage tables, fuzz grammars, falsification records, benchmark figures |
| [docs/modules/log/](docs/modules/log/) | per-unit working logs — chronological, including superseded figures and refuted hypotheses |

**Bug Catcher submission.** The upstream defects this entry claims are
[docs/BUGS.md](docs/BUGS.md), in full and in one place: 12 written up individually with
reproductions, 49 more in a summary table, and 3 held separately as lower-confidence candidates
because each has a plausible reading as intentional design. Every entry gives the reproduction, what
the original does wrong, and how this port handles it; every row in the table was re-confirmed
against Node 24.18.1 while the document was written rather than carried over from working notes.

Each defect carries a `BUG-<MODULE>-<n>` tag that resolves to the same tag in
[docs/modules/](docs/modules/)`<module>.md`, where the full analysis lives — how the existing suite
missed it, and which test or fuzz seed pins it. Deliberate divergences use `DIV-<MODULE>-<n>` the
same way, and `PORTBUG-n` marks a bug in *this* port rather than upstream's.

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
cargo test                       # 803 native tests
cargo run --release --example tour -p mnemonist-core   # the crate used from Rust
./tests/run.sh                   # 525 upstream specs + 208 of this port's own, via the bridge
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
previously recorded as wins. In this pass both measured as losses, and a spot-check of each in
isolation measured them worse still, so both are published as losses at the worse of the two
figures — between two honest measurements the unflattering one is the safer claim.

`bi-map` is the least stable measurement here: its ratio spanned 1.14× to 1.59× across six runs,
where every other module reproduces to about 1%. Its published figure should be read as "slower, by
somewhere between a little and a half", not as 1.51.

Being counted as a loss is what got a module looked at, and five left the loss column that way —
including `kd-tree`, previously the largest regression in this table at 2.18× slower and now 1.23×
faster, once a per-node heap allocation in its nearest-neighbour search became a stack copy. Each
diagnosis is in that module's own document under *Fuzz + bench*. Two of the five losses that remain
say something the table cannot.

**`default-map` is the largest regression left, and it is the one whose cause this port does not
claim to know.** The obvious explanation — a duplicate hash lookup — was measured and refuted: a
probe separating a peek from a hit put the two 0.7% apart, where a second lookup would have shown up
plainly. What replaced it is an account, not a finding: at a million-key domain `OrderedMap`'s
internal `HashMap` no longer fits in cache and a uniformly random key makes most lookups a real
memory access. That is consistent with the regression *narrowing* from 1.44× to 1.13× as the domain
grows, but it was never isolated against a metric that would have falsified it, and the hardware
cache-miss counters that would settle it are not available on this host. The module's document says
so in those words, and this summary does not upgrade it.

The structural fix — storing values inline and tracking order separately — was costed rather than
waved away, at roughly 6.5 to 11.5 hours: `OrderedMap` backs eight units that are complete through
every gate, and each would need its falsification redone, its campaign re-run and its benchmark
re-measured. It was declined because the benchmark's operation mix is three-quarters writes, which
must touch a second structure under any design preserving insertion order and cursor semantics — so
even a successful rewrite should not be expected to close a 1.44× gap.

**`heap` is the clearest measure of what fidelity costs.** The delivered implementation runs at
31.7 ns against upstream's 24.3 ns; a bare `Vec<f64>` heap over the identical workload runs at
21.7 ns — a separate probe, not part of the table above — so the `RefCell` and comparator
indirection account for more than the entire regression. That indirection is what permits a
JavaScript comparator to re-enter and mutate the heap mid-sift, which upstream behaviour requires.

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

MIT — [LICENSE](LICENSE) for this port, [NOTICE](NOTICE) for what it derives from and
redistributes. Two upstream libraries are involved, both © Guillaume Plique (Yomguithereal), both
MIT:

- **`mnemonist`** ([LICENSE-MNEMONIST](LICENSE-MNEMONIST)) — the library ported here. Its published
  test suite is redistributed **unmodified** under `tests/original/` and hash-verified; that suite
  is the equivalence evidence this port rests on and is not this project's work. A vendored copy of
  the library sits under `bench/upstream/` so both sides of a measurement run the real thing.
- **`obliterator`** ([LICENSE-OBLITERATOR](LICENSE-OBLITERATOR)) — `mnemonist`'s iteration
  primitives, ported too: `obliterator/iterator` became `crates/mnemonist-core/src/cursor/` and
  `obliterator/foreach` became `crates/mnemonist-napi/src/foreach.rs`, both against 2.0.5.
