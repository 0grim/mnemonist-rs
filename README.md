# mnemonist-rs

A Rust port of [`mnemonist`](https://github.com/Yomguithereal/mnemonist), a library of 44 JavaScript
data structures. Produced for Port Mortem 2026.

The deliverable is a standalone Rust crate. `mnemonist-core` has no dependencies, declares
`#![forbid(unsafe_code)]`, and builds and tests without Node installed. The original JavaScript test
suite serves as the equivalence proof rather than a runtime dependency: those tests execute
unmodified against the Rust build through an N-API bridge that forms no part of the published crate.

```
42 of 42 upstream test files ported        733 upstream specs passing, unmodified
42 units through all ten gates (100%)       799 native Rust tests
129.8M differential fuzz operations        zero divergences
71 upstream defects examined               12 documented in full
```

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
--all-targets -D warnings` is clean across approximately 71,000 lines, with 25 lint suppressions:
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

| Structure | Median, relative to upstream |
|---|---|
| `fibonacci-heap` | 25× faster |
| `linked-list` | 4.1× faster |
| `trie` | 2.6× faster |
| `lru-cache` | 1.8× faster |
| `heap` | 1.32× slower |
| `default-map` | 1.42× slower |
| `multi-array` | 1.90× slower |
| `kd-tree` | 2.18× slower |

Each regression has a confirmed cause, established against a measurement that would have produced a
different result had the explanation been incorrect. One earlier explanation was refuted by this
process: `default-map`'s regression had been attributed to a duplicate hash lookup that does not
exist in the code; the observed cause is cache-miss latency, which is consistent with the regression
narrowing from 1.42× to 1.17× as the domain grows and both implementations begin to miss.

The `heap` figures are the clearest measure of what fidelity costs. A bare `Vec<f64>` heap over the
identical workload runs at 21.7 ns against upstream's 24.3 ns. The delivered implementation runs at
31.8 ns, so the `RefCell` and comparator indirection accounts for more than the entire regression.
That indirection is what permits a JavaScript comparator to re-enter and mutate the heap mid-sift,
which upstream behaviour requires.

## Layout

```
crates/mnemonist-core     the crate — 40 structures, no dependencies, no unsafe code
crates/mnemonist-napi     the N-API bridge, used only to run the upstream suite
crates/difffuzz           differential fuzzer: core against upstream in Node
bench/                    matched benchmark harness, Rust and JavaScript halves
tests/original/           the upstream suite, byte-identical and hashed
docs/modules/             one divergence document per unit
```

## License

MIT, matching upstream. `mnemonist` is © Guillaume Plique and contributors; the vendored test suite
under `tests/original/` is redistributed unmodified under that license.
