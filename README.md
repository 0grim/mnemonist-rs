# mnemonist-rs

A Rust port of [`mnemonist`](https://github.com/Yomguithereal/mnemonist) — 44 JavaScript data
structures — built for **Port Mortem 2026**.

**The deliverable is a standalone Rust crate.** `mnemonist-core` has zero dependencies, declares
`#![forbid(unsafe_code)]`, and builds and tests with Node absent from the machine. The original
JavaScript test suite is the *proof of equivalence*, not a runtime dependency: those tests run
**unmodified** against the Rust build through a thin N-API bridge that ships to nobody.

```
42 of 42 upstream test files ported        733 upstream specs passing, unmodified
40 units through all ten gates (94%)       799 native Rust tests
129.5M differential fuzz operations        zero divergences
74 upstream bug candidates, 66 verified    against Node 24.18.1
```

## What this crate is, and what it is not

It reproduces `mnemonist`'s behaviour **including its bugs**. That is the contract, and it is worth
being explicit about the price:

`MultiSet`'s size counter is a tracked value rather than a derived one, so that upstream's drift on
a failed delete reproduces instead of silently healing. `FibonacciHeap::size` is an `i64` because
JavaScript has no unsigned integers and upstream drives it to `-1` under a re-entrant `clear()`.
Some error text is pinned to upstream's exact `TypeError` string.

**So this is a compatibility crate**, not "the best data structures one could write in Rust". Where
a behaviour is deliberately wrong it is documented on the item itself and in that structure's
divergence document. Nothing is left to be discovered by surprise.

Everywhere fidelity and idiom did not conflict, the code is ordinary Rust. `clippy --all-targets
-D warnings` is clean across ~71,000 lines, with **25 lint suppressions in total**: 24 about
signature shape or ergonomics (18 `type_complexity`, 5 `too_many_arguments`, 1
`new_without_default`), and one `if_same_then_else` where two differently-guarded branches share a
body deliberately, because only the second calls `levenshtein` — documented at the site. Whatever
fidelity cost here, it was not paid by silencing the linter.

## Start here

| Document | What it answers |
|---|---|
| **[docs/METHODOLOGY.md](docs/METHODOLOGY.md)** | The ten gates every unit passes, what each one caught, and **what the instruments cannot see** |
| **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** | The crate split, why the bridge exists, and the six places fidelity cost idiom |
| **[docs/BUGS.md](docs/BUGS.md)** | Defects found in the original library, with reproductions |
| **[docs/DECISIONS.md](docs/DECISIONS.md)** | Every deliberate divergence from upstream, and why |
| **[docs/modules/](docs/modules/)** | 46 per-unit documents: what upstream tests, what it does *not* test, what we added |

## Verifying the claims above

```bash
docker build -t port-mortem . && docker run --rm port-mortem   # one command, everything
```

Or directly:

```bash
cargo test                       # 799 native tests
./tests/run.sh                   # 733 upstream specs, unmodified, through the bridge
./tests/verify.sh                # the ten gates, per unit claimed complete
sha256sum -c tests/SHA256SUMS    # the upstream tests are byte-identical to published
scripts/status.sh                # derived status: coverage and per-unit evidence
```

**The originals are hashed on purpose.** The easiest way to pass someone else's tests is to edit
them; all 42 upstream test files are SHA-256 checked on every commit, so that option is closed —
including to us.

`tests/scope.txt` is the done marker, and a unit enters it only when **all ten gates** pass.
Two units are deliberately excluded, with reasons recorded rather than left pending:
`default-weak-map`, because its entries vanish at the garbage collector's discretion and a timing
figure would measure V8 rather than the structure; and `_utils`, a require-closure of five unrelated
pure-function files with no shared instance.

## Performance

Benchmarked against the real upstream JavaScript, matched op streams, interleaved, on an idle
machine. **We are not faster everywhere, and the losses are the interesting part.**

| Structure | vs upstream (p50) |
|---|---|
| `fibonacci-heap` | **25× faster** |
| `linked-list` | **4.1× faster** |
| `trie` | **2.6× faster** |
| `lru-cache` | **1.8× faster** |
| `heap` | **1.32× slower** |
| `default-map` | **1.42× slower** |
| `multi-array` | **1.90× slower** |
| `kd-tree` | **2.18× slower** |

Every loss has a **confirmed** cause, tested against a measurement that would have come out
differently had the explanation been wrong. One recorded explanation was refuted that way:
`default-map`'s loss was blamed on a double hash lookup that does not exist, and is actually
cache-miss bound.

The most interesting number is `heap`. A bare `Vec<f64>` heap over the identical workload runs
21.7ns against upstream's 24.3ns — **our algorithm beats JavaScript outright.** The wrapped version
runs 31.8ns, so the `RefCell` and comparator indirection costs more than the entire regression. That
indirection is what lets a JavaScript comparator re-enter and mutate the heap mid-sift, which is
behaviour we are contracted to reproduce. **That number is the price of fidelity, measured.**

## Layout

```
crates/mnemonist-core     the product — 40 structures, zero dependencies, no unsafe
crates/mnemonist-napi     the proof harness — lets the unmodified JS tests reach Rust
crates/difffuzz           differential fuzzer: our core vs real upstream in Node
bench/                    matched benchmark harness, Rust and JS halves
tests/original/           the upstream suite, byte-identical and hashed
docs/modules/             one divergence document per unit
```

## License

MIT, matching upstream. `mnemonist` is © Guillaume Plique and its contributors; the vendored test
suite under `tests/original/` is redistributed unmodified under that license.
