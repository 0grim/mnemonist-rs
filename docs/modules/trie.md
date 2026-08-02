# trie

Upstream: `trie.js` (167 LOC) · `test/trie.js` (254 lines, 10 `it` blocks, 49 assertion
statements).

Port: `crates/mnemonist-core/src/structures/trie.rs` — `Trie<T>`, composing over
`crates/mnemonist-core/src/structures/trie_map.rs`'s `TrieMap<T, bool>` rather than copying and
deleting methods the way upstream's own `trie.js` does (Rust has no equivalent of "copy every
property off `TrieMap.prototype`, then delete four of them"). Bridge:
`crates/mnemonist-napi/src/trie.rs`. Shim: `tests/bridge/trie.js`. Fuzz spec:
`crates/difffuzz/src/modules/trie.rs`.

**This is one engine, two units, by upstream's own design** — its header comment says so directly:
*"the Trie is based upon the TrieMap since the underlying machine is the very same. The Trie just
does not let you set values and only considers the existence of the given prefixes."* Nearly
everything in `docs/modules/trie-map.md` — the node representation, the enumeration-order
discipline, the lazy walk, both upstream bugs, all three divergences — applies here unchanged. This
file states only what is genuinely specific to `Trie`: `add`/`find`'s own shape, which methods
`trie.js` actually deletes (and the one it does not), and the fuzz grammar's value alphabet
(there isn't one — a `Trie` node's value is a bare `true`).

## What upstream tests

* **`add`/`has`/`delete`**, including the empty sequence (`''`/`[]`) as a valid member, adding the
  same member twice without growing `size`, and the exact `root` shape after each mutation.
* **`add`'s own override**: sets the sentinel to `true` unconditionally (there is no value to
  overwrite), and `delete`'s pruning, checked against the exact resulting `root` — the same
  algorithm `trie-map` uses, since `delete` is one of the methods `trie.js` does **not** override.
* **`find`'s own override**: returns bare matched sequences, not `[sequence, value]` pairs — the one
  place `Trie` genuinely diverges from copying `TrieMap.prototype` wholesale, at three depths (exact
  prefix, shorter prefix, no match).
* **Custom tokens** (`new Trie(Array)`): sequences of whole strings, re-asserting `add`, `root`,
  `has`, `delete`, `find` in that mode.
* **`prefixes`/`keys`** (the same function, aliased) as a lazy iterator, and `for...of` over the
  trie itself (`Symbol.iterator`, aliased to `keys` — **not** to `entries`, since a `Trie` has no
  value to pair a key with).
* **`Trie.from`**, from a plain array of strings.

## What upstream does NOT test

Everything `docs/modules/trie-map.md`'s own "What upstream does NOT test" lists applies here too
(a resolvable-but-empty sub-prefix, `delete`/`clear` under an open iterator, a `SENTINEL`-shaped
token, digit tokens, the raw-vs-coerced echo split) — same engine, same untested paths. Specific to
this file:

**`update`, inherited from `TrieMap` completely unmodified.** `trie.js`'s delete list is `set`,
`get`, `values`, `entries` — **not** `update`. Confirmed against real Node 24.18.1:
`new Trie().update` is a real, callable function, running `TrieMap.prototype.update` against the
boolean sentinel. `test/trie.js` never calls it and `trie.d.ts` does not declare it, but it is
genuinely reachable through the public API, so it is bridged (`crates/mnemonist-napi/src/trie.rs`)
and fuzzed (the `trieToggle` factory) rather than silently dropped.

**A node's own value ever being anything but the literal `true`** — `has` asks about presence
(`SENTINEL in node`), not truthiness, so `trie.update(prefix, () => false)` leaves `has(prefix)`
`true`. No test reaches `update` at all, so this combination is untested twice over.

## What we test in addition

**Rust native tests** (`crates/mnemonist-core/src/structures/trie.rs`, 8):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite`, `adding_the_same_item_again_does_not_increase_size`, `the_null_sequence_is_a_valid_member`, `delete_removes_and_prunes_singleton_chains`, `find_returns_the_suffix_beyond_the_given_prefix`, `walk_visits_every_word`, `clear_resets_size_and_removes_everything` | the upstream blocks, as a baseline |
| `has_distinguishes_a_stored_word_from_a_mere_prefix_of_one` | the gate 6 falsification target |
| `update_is_inherited_from_trie_map_and_still_works` | the "not tested, but reachable" gap above — pins that `update` genuinely still works through the `Trie` wrapper, including that a second `update` on the same prefix does not grow `size` |

Everything `trie-map`'s own native tests pin about `Node`/`Walk`/the sentinel-collision/the
delete-and-open-walk interaction is exercised through the shared engine and is not duplicated here;
see that file.

**Differential fuzzer** — see "Fuzz + bench". Reaches the same shared-prefix and prefix-of-a-word
states `trie-map`'s campaign does, over `add` instead of `set`, plus the untested
`update`-then-`has` combination via `trieToggle`.

## Bugs this found

Both upstream defects this unit's engine has — B-200 (a `SENTINEL`-shaped token corrupts the trie)
and B-201 (a `delete`/`clear` under an open cursor leaves it yielding stale content) — are
documented in full in `docs/modules/trie-map.md`, since they are properties of the shared node
representation and the shared lazy walk, not of anything `trie.js` adds. This unit's own fuzz
campaign rediscovered B-201 independently, over `add`/`clear`/`keys` rather than `set`/`delete`/
`entries` — see "Fuzz + bench" for that repro specifically.

## Deliberate divergences

The three structural divergences — D-200 (sentinel collision not reproduced), D-201 (path-based
walk, not a live reference), D-202 (no integer-key enumeration rule) — are inherited from the shared
`TrieMap` engine and documented in full in `docs/modules/trie-map.md`. Nothing here adds a new one;
`Trie`'s own composition-over-copy-and-delete is an implementation-technique difference from
upstream with no observable consequence (see that file's header note), not a behavioural divergence.

## Fuzz + bench

### Fuzz

```
module=trie   seed=42       cases=6498  ops=652034  wall=60.0s  divergences=0
module=trie   seed=20260801 cases=6141  ops=612873  wall=60.0s  divergences=0
```

**Grammar:** `crates/difffuzz/src/modules/trie.rs`, sharing `crate::modules::trie_map::PREFIX_POOL`
and its tokenisation directly rather than re-deriving them — see that module's docs for why the pool
is shaped `a, ab, abc, abcd, b, ba, bc, bad` and for the measured evidence (`pool_self_check_*`)
that it produces shared prefixes in practice, not merely in principle. There is no value alphabet
here: a `Trie` node's own value is always the bare `true` `add` writes (or whatever `update`'s
`trieToggle` factory flips it to — `(old) => !old`), so the only thing to compare per-node is
presence.

**The regime split (D-201), inherited.** Exactly `trie-map`'s split — `ctor[0]` is an internal flag
choosing whether a program exercises `delete`/`clear` or a persistent `$iter`/`$next` cursor over
`keys()`, never both. This unit's *own* first campaign, run before the split existed, independently
reproduced the divergence, minimised to:

```
var s = new Trie();
var it = s.keys();
s.clear();
s.add("a");
it.next();
// port: {done: false, value: "a"}   -- sees the post-clear addition
// upstream: {done: true}            -- the stale, pre-clear root has nothing on it
```

the mirror image of `trie-map`'s own repro: there, pruning left upstream *ahead* of the port
(stale content still yielded); here, `clear` leaves upstream *behind* it (the stale root can never
see a later addition). Both are the identical root cause — a live reference against a re-navigated
path — confirmed independently in each unit rather than assumed to transfer.

**Observed state:** `size` and `root` (a bare `bool` at each `Word` position rather than a JSON
value). Same order-independent-object / order-sensitive-array split as `trie-map`; see that file.

**What this grammar deliberately does not cover:** identical to `trie-map`'s list — array mode,
digit tokens, a starting sub-prefix on `keys`, and `delete`/`clear` under an open cursor (D-201,
excluded by construction after being found reachable in practice). See that file for the reasoning
behind each; it is not repeated here since the mechanism is the shared engine, not anything specific
to `Trie`.

### Falsification (gate 6)

Run as **one falsification covering both units** — `TrieMap::has` backs `Trie::has` directly, so a
single sabotage exercises both. Named target, sabotage, and the confirmed-red/confirmed-green
results for every instrument (including this unit's own original-suite failures at
`test/trie.js:143`/`:92`/`:198` and its own minimised fuzz repro, `s.add("ba"); s.has("b")` — port
`true`, upstream `false`) are recorded in full in `docs/modules/trie-map.md`'s "Falsification (gate
6)" section, to avoid describing the same sabotage twice.

### Bench

`bench/results.json` → `modules["trie"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-2e5`** — 1e6 mixed `add`/`has`/`delete` (50/25/25) over a 200,000-value domain, keys the
lowercase hex encoding of each drawn `u32` (`bench/runner/src/trie.rs`), xorshift32 seed 42. The
domain is an order of magnitude below the other modules' 1e6 on purpose: every distinct key here is
a multi-node walk through a per-node hash map, not a flat array index, and an equal domain would
have made this module by far the slowest wall-clock component of the pass for no representativeness
gained — 200,000 keys already exercises deep prefix sharing (values under `0x1000` alone number in
the thousands).

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **172.5** | 454.8 | 2.6× faster |
| p99 ns/op | **265.6** | 792.5 | 3.0× faster |
| RSS delta MB | **30.7** | 220.5 | |
| structure-only RSS delta MB | **0.1** | 6.6 | |
| startup ms | **0.6** | 16.2 | 27× (reported separately; not throughput) |

**A clean win on every metric — no regressions.** This is the allocation-heavy, string-keyed
profile flagged as genuinely different from the array/typed-array modules, and the
port's per-node `HashMap<char, Node>` fan-out costs V8 noticeably more than the equivalent plain-
object node upstream uses, both in time and in the RSS delta (upstream's is 7× the port's here). The
0.1 MB structure-only delta versus 6.6 MB is the widest such gap measured across all seven modules
in this pass.
