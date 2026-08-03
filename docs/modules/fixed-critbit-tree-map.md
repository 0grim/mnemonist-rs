# fixed-critbit-tree-map

Upstream: `fixed-critbit-tree-map.js` (427 LOC) · `test/fixed-critbit-tree-map.js` (154 lines, 4
active `it` blocks — a fifth, "should be possible to delete elements.", is commented out in the
upstream source itself, block and all).

Port: `crates/mnemonist-core/src/structures/fixed_critbit_tree_map.rs` — `FixedCritBitTreeMap<V>`,
the same crit-bit engine as `critbit-tree-map` over pre-allocated, bounded storage instead of
growable arenas. Bridge: `crates/mnemonist-napi/src/fixed_critbit_tree_map.rs`. Shim:
`tests/bridge/fixed-critbit-tree-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/fixed_critbit_tree_map.rs`.

**This is not "critbit-tree-map with a capacity check".** Upstream's own two files share no code at
all — `fixed-critbit-tree-map.js` re-derives the whole algorithm against typed arrays rather than
importing anything — and the port mirrors that independence: two separate files, two separate
`msb8`/`mask_for` copies, with a *different* bitwise convention (direct mask, not inverted) and a
different tie-break direction in `set`'s bubble-up. The thing that actually makes this unit
different is that **there is no capacity guard at all** (see "Bugs this found"), and the fixed
variant's own module docs (`crates/mnemonist-core/src/structures/fixed_critbit_tree_map.rs`) are the
canonical account of the mechanism.

## What upstream tests

* **"should throw if given bad arguments."** — the constructor's own `capacity <= 0` /
  non-numeric guard, nothing more.
* **`set`/`get`/`has`** on a capacity-3 tree with exactly three distinct keys — never exercising
  anything past capacity.
* **Keys differing only in length**, on a capacity-5 tree with exactly five distinct keys — capacity
  reached exactly, never exceeded.
* **`forEach`**, on a capacity-5 tree with exactly five distinct keys, asserting sorted order —
  again exactly at capacity, never past it.

Every active test constructs a tree whose capacity exactly matches the number of distinct keys it
inserts. **No test in the original suite ever exceeds capacity.** That is not an oversight this port
works around; it is the reason the interesting behaviour below is untested by gate 4 at all.

## What upstream does NOT test

**Anything past capacity.** See "Bugs this found" (B-261) for the full, measured mechanism: a
silent corruption on the key that pushes past capacity, then a crash on a later `set` that walks
through it. Untestable by the original suite by construction, reached in essentially every program
this unit's own fuzz grammar generates (see "Fuzz + bench").

**`delete`.** Not merely untested — **absent from upstream entirely**. The commented-out test block
in `test/fixed-critbit-tree-map.js` is the tell; there is no `FixedCritBitTreeMap.prototype.delete`
in the source at all. This port has none either.

**`root` read directly.** No test reads `tree.root`; the only reference in the original suite is
inside a commented-out `printTree` debug helper. See B-260.

**Deep critical-bit positions and non-Latin-1 keys.** Identical reasoning to `critbit-tree-map`'s
own gaps; see that file's docs.

## What we test in addition

**Rust native tests** (`crates/mnemonist-core/src/structures/fixed_critbit_tree_map.rs`, 10):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_set_suite`, `keys_that_differ_only_in_length_do_not_break`, `for_each_visits_in_sorted_key_order_when_exactly_at_capacity`, `rejects_a_zero_capacity` | the upstream blocks, as a baseline |
| `keys_differing_only_in_the_last_byte_route_correctly_within_capacity` | the gate 6 falsification target, within capacity |
| `exceeding_capacity_silently_corrupts_then_crashes_exactly_as_upstream_does`, `a_capacity_of_one_corrupts_on_the_second_key` | B-261, measured at two different capacities |
| `root_is_a_number_fresh_but_null_right_after_a_clear` | B-260 |
| `clear_empties_the_tree_but_does_not_shrink_the_backing_arrays`, `a_set_right_after_a_clear_reuses_index_zero_instead_of_panicking` | a port bug this unit's own differential fuzzer found (see "Bugs this found") |

**Differential fuzzer** — see "Fuzz + bench". Capacity is drawn deliberately small (`2..=5`) against
an 8-key pool specifically so it is exceeded, and B-261's crash reached, in most generated programs
rather than merely capable of it — measured directly, not assumed.

## Bugs this found

### B-261 — no capacity guard at all: exceeding it silently corrupts, then crashes

The constructor's own comment: `// TODO: yell if capacity is already full!`. `lefts`/`rights` are
real, fixed-size typed arrays (`capacity - 1` slots); `critbits` is sized `capacity` — one slot
*larger*; `keys`/`values` are plain, unbounded `Array`s. Measured on a capacity-4 tree: the 5th
distinct key succeeds silently (`size` becomes 5, no error) while corrupting exactly one node's
children (a typed-array write past its end is a silent no-op, not a throw); `get` on the two keys
reachable only through that node then silently returns `undefined`. The 6th distinct key walks
*through* the corrupted node and throws upstream's own
`TypeError: Cannot read properties of undefined (reading 'length')`. Full transcript and mechanism
in this port's own module doc comment, part 1.

Reproduced as `Error::Corrupted`, with upstream's own message text (D-246, below).
Measured, not assumed, that this campaign actually reaches it:
`pool_self_check_capacity_is_actually_exceeded_and_hits_the_crash` samples 500 real constructed
instances driven by real generated programs and asserts both "size exceeded capacity" and
"`Error::Corrupted` was reached" hold for a majority — consistently ~60% and 100% respectively
across three separate runs of the check.

A second, narrower defect in the same neighbourhood: `set`'s "attach to an existing internal node"
branch writes to literal slot `lefts[0]`/`rights[0]` rather than to the node actually being visited
— measured unreachable in practice (every internal node's children are always both written at
creation, so the branch's own guard never fires), and reproduced verbatim anyway. See the module doc
comment, part 2.

### B-260 — `root` is `0` fresh off the constructor, but `null` right after a `clear`

Verified against Node 24.18.1, confirmed in the source (`fixed-critbit-tree-map.js:99` vs `:120`):
the constructor sets `this.root = 0`; `clear` sets `this.root = null`. No method's *behaviour*
depends on which — every internal read of `root` treats `null` exactly as `0` would, through the
same fallthrough that resolves any non-numeric pointer to "not found" — so this is observable only
by reading `root` directly, which no original test does but this unit's own fuzz spec's `root`
observation does. Reproduced via a `root_is_null` flag, as detailed above.

### Two port defects, not upstream's

Found by this unit's own differential fuzzer and fixed before any campaign was logged. Summary:
the very first smoke run crashed the whole process (not merely reported a divergence) because `set`
checked `self.keys.is_empty()` instead of `self.size == 0` to detect an empty tree — after a `clear`
(which resets `size` but never truncates `keys`/`values`, matching upstream exactly), the next `set`
fell through to the walk loop with `pointer == EMPTY` unguarded and panicked computing an
out-of-bounds index. Fixed by adding a real `size` field and a `store_external` helper that
overwrites low indices post-`clear` instead of always pushing.

## Deliberate divergences

* **D-245**, inherited from `critbit-tree-map`: keys truncated to bytes at the bridge. Identical
  reasoning; see that file's docs.
* **D-246**: B-261's crash is a Rust `Result::Err` carrying upstream's own
  message text, not a panic modelling JavaScript's `NaN`-as-array-index cascade. `mnemonist-core`
  forbids `unsafe_code` and has no analogue of a typed-array read past its end silently returning
  `undefined`; a `panic!` would abort the whole Node process at the FFI boundary where upstream
  merely throws a catchable exception. Observationally identical text; the mechanism producing it
  differs.

## Fuzz + bench

### Fuzz

```
module=fixed-critbit-tree-map   seed=42       cases=9987   ops=1020404  wall=60.0s  divergences=0
module=fixed-critbit-tree-map   seed=20260801 cases=10061  ops=1025762  wall=60.0s  divergences=0
```

**Grammar:** `crates/difffuzz/src/modules/fixed_critbit_tree_map.rs`, sharing
`crate::modules::critbit_tree_map::PREFIX_POOL` directly. `capacity` is drawn from `2..=5` against
the same 8-key pool specifically so B-261's overflow is reached in most generated programs, not
merely capable of it — see "Bugs this found" for the measured evidence. Ops: `set`, `get`, `has`,
`clear`. No `delete` (upstream has none).

**Observed state: `size` and `root`.** Unlike the unbounded variant's `root`, this one is a bare
number — `fixed-critbit-tree-map.js` never builds `InternalNode`/`ExternalNode` objects at all — so
comparing it checks that this port's internal-node allocation order matches upstream's
`this.offset++`/`this.size++` counters exactly, including across a capacity overflow, but it does
**not** expose any node's critical-bit value the way the unbounded variant's `root` does. That
distinction matters directly below.

**What this grammar deliberately does not cover:** `delete` (does not exist upstream); `forEach`
(no op drives it — see the falsification section for why that specifically matters here, unlike in
the unbounded variant); non-Latin-1 keys (D-245).

### Falsification (gate 6)

**Target named before running:** the identical sabotage as `critbit-tree-map`'s own falsification —
`msb8`'s `!` dropped, `x & !(x >> 1)` → `x & (x >> 1)` — predicted to break
`reproduces_the_upstream_set_suite`,
`keys_differing_only_in_the_last_byte_route_correctly_within_capacity`, and
`keys_that_differ_only_in_length_do_not_break`.

**Confirmed red** in `cargo test`: 4 of 10 native tests failed, including all three named targets
(plus `for_each_visits_in_sorted_key_order_when_exactly_at_capacity`, unnamed but also broken).

**The differential fuzzer stayed green** — `divergences=0` at 5s, 15s and 20s (3304 cases, 341,855
ops) against the identical sabotage that the unbounded variant's fuzzer caught on its *second*
generated operation. Investigated rather than accepted at face value, because "the fuzzer proves the
falsification means something" is exactly what this gate exists to check, not assume:

1. **Every byte-difference this pool can actually produce was checked directly** (a small script
   comparing correct vs. sabotaged `msb8` over every pairwise prefix comparison the 8-key pool can
   generate, including the tail-extension case): the sabotage changes the computed mask in **every
   single case** (`xor=3 → mask 2 vs 1`; every tail-extension case → `mask 64 vs 63`). The bug is
   real and triggered on essentially every internal-node creation, not merely capable of being
   triggered.
2. **But the change is a *consistent* one, not a random one.** `get_direction = (byte & mask) != 0`
   is used identically for both insertion routing and lookup routing. Changing the mask changes
   *which* bit is tested, which — for these specific byte pairs — flips left and right at that node,
   but flips them the *same way* every time a key is inserted or looked up. The tree ends up an
   exact left-right mirror of what upstream would build, not a broken one: every key is still
   reachable, `get`/`has`/`set` all return upstream's exact values, and `size` is identical.
3. **`root` cannot see a mirror**, because it is a bare index for this variant (see "Observed
   state", above) — swapping which pointer field (`lefts[i]` vs `rights[i]`) holds which value never
   changes *which* internal/external index ends up allocated where, since allocation order comes
   from `next_internal`/`size` counters that do not depend on direction at all. A mirrored tree and
   the correct one report the identical `root` integer.
4. **The one upstream operation that *would* reveal a mirror is `forEach`** — an inorder walk visits
   left before right, so a swap at any node changes the *visitation order*, which is exactly what
   `for_each_visits_in_sorted_key_order_when_exactly_at_capacity` (a native Rust test, gate 7) caught
   above. It is **not** what the fuzz grammar reaches, because no op in it drives `forEach` — a
   deliberate exclusion (see "What this grammar deliberately does not cover"), made for a reason that
   turns out to matter here specifically: neither `critbit-tree-map.js` nor
   `fixed-critbit-tree-map.js` implements `Symbol.iterator`, so the oracle's generic `$spread` op
   (`Array.from(instance)`, the mechanism `trie`/`trie-map`'s campaigns use for exactly this kind of
   structural snapshot) has nothing to iterate — hooking an equivalent up would mean extending
   `fuzz/oracle.js`'s protocol non-generically for this one module, not reusing existing machinery.

So: **the falsification stayed green, and correctly so, for a specific and now-understood reason** —
this variant's only observable structural check (`root`) is topologically blind to a self-consistent
left-right mirror, and the one channel that is not blind to it (`forEach`'s visitation order) is
covered by gate 7, not gate 9, for this module. Both instruments are telling the truth about what
they each cover; neither is broken. Filed as the sharpest illustration this pair of units produced of
"passing your own verification is not the same as being correct" — here, *two* green instruments
(fuzzer clean, and it would remain clean forever against this exact class of bug) sit next to one
red one (the native `forEach`-order test) checking the same underlying defect from different angles.

**Reverted; confirmed green** on both instruments: `cargo test` 10/10, and the differential fuzzer
`divergences=0` again at the same seed/duration.

### Bench

`bench/results.json` → `modules["fixed-critbit-tree-map"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-2e5`** — 1e6 mixed `set`/`get`/`has` (50/25/25) — `fuzzy-map`'s shape, not `sparse-map`'s:
upstream has no `delete` at all (see this module's own docs above). `size` 200,000 is **both** the
capacity and the full key domain, which is load-bearing rather than a style choice: upstream's
`set` has no capacity guard whatsoever, and a distinct key past capacity silently corrupts the tree
— the next operation to walk through that corrupted node THROWS. Capping the domain at capacity
means the tree fills to capacity (every key is drawn from `0..size`, so at most `size` distinct keys
are ever possible) but can never overflow it — "capacity actually filled" without ever reaching the
crash this module's own docs describe. Same zero-padded, deep-critical-bit key shape as
`critbit-tree-map`, reused verbatim. xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 325.78 | **306.50** | upstream 1.06× faster |

**Fixed 2026-08-03 — two allocations per `set`.** `set` declared `let mut ancestors: Vec<usize> =
Vec::new()` and `let mut path: Vec<bool> = Vec::new()` fresh on every call and dropped both at the
end. Any walk that descends through even one internal node pushes into them, so once the tree is
non-trivial that is two heap allocations on essentially every insert — and `set` is half this
workload's operation mix.

Both are now struct fields, cleared on entry rather than reallocated. Neither is observable outside
a single `set` call: cleared on entry, filled to that call's own traversal depth, read only within
the same call. The struct derives `Debug` and `Clone` but not `PartialEq`, and nothing formats it,
so carrying a stale scratch buffer into a clone changes nothing.

Six runs alternating the old and new code put the port's p50 at **393–424 ns before and 280–307 ns
after**, about 28%. This module now reads **1.18× faster** than upstream where it read 1.11× slower.
| p99 ns/op | **527.09** | 822.52 | 1.6× faster |
| RSS delta MB | **27.0** | 192.5 | |
| structure-only RSS delta MB | 0.1 | **1.0** | |
| startup ms | **0.6** | 15.4 | 26× (reported separately; not throughput) |

**One real, reproducible loss: p50, ~1.06–1.08× across two independent runs.** Re-run twice rather
than published from a single pass — a clean-looking result invites the question of what was left
out just as much as a *loss* that might be noise, and
this one held in both passes rather than appearing once. **Cause: unconfirmed.** A plausible but
unverified explanation is `BoundedSlots`' `Option`-returning bounds check on every internal-node
read (`lefts`/`rights`), which upstream's raw typed-array indexing does not pay for — but this has
not been checked against a metric that would falsify it (e.g. isolating that one accessor), so it
is labelled unconfirmed rather than asserted, per this port's rule against overclaiming performance
causation. p99 and every RSS/startup figure still favour the port; the structure-only RSS row is
the one place upstream's fixed typed arrays are smaller than the port's own pre-allocated arenas at
this size. Checksum `15858409098`, identical on both sides — same ops, same answers, including
reaching capacity without the corruption path ever firing.
