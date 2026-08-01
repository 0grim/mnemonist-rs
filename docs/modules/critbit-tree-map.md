# critbit-tree-map

Upstream: `critbit-tree-map.js` (515 LOC) · `test/critbit-tree-map.js` (140 lines, 4 `it` blocks).

Port: `crates/mnemonist-core/src/structures/critbit_tree_map.rs` — `CritBitTreeMap<V>`, an
arena-indexed crit-bit tree over byte-string keys. Bridge:
`crates/mnemonist-napi/src/critbit_tree_map.rs`. Shim: `tests/bridge/critbit-tree-map.js`. Fuzz
spec: `crates/difffuzz/src/modules/critbit_tree_map.rs`.

A crit-bit tree (a.k.a. PATRICIA trie) branches on the position of the first bit at which two keys
differ, rather than on a shared token alphabet the way `trie-map` does — every internal node holds
only "which byte, which bit", and every stored key lives in a leaf. `fixed-critbit-tree-map.js` is
the same idea over pre-allocated typed arrays, not code-shared with this file upstream and not
code-shared in the port either; see `docs/modules/fixed-critbit-tree-map.md` for what is genuinely
different about it, not merely bounded.

## What upstream tests

* **`set`/`get`/`has`**, including overwriting an existing key without growing `size`, and a get on
  a key that shares a prefix with a stored one but was never itself stored.
* **`delete`**, including deleting a key twice (the second call returns `false`) and deleting five
  keys back to an empty tree in reverse insertion order, then confirming every one of them is gone.
* **Keys differing only in length** — `'meta'`/`'metastasis'`/`'metastases'` alongside unrelated
  keys — the prefix-relationship shape this port's own fuzz grammar was built to reach routinely
  rather than by chance.
* **`forEach`**, asserting the exact sorted-by-key order against `lodash.sortBy`, over five keys
  including one whose case (`'Abc'` vs `'abc'`) exercises byte-value ordering directly.

## What upstream does NOT test

**Deep critical-bit positions.** Every test key differs from its neighbours within the first two or
three bytes; nothing in the original suite stores two keys that agree on a long shared prefix and
diverge only in their last byte, which is exactly the shape that exercises the bubble-up rotation
(`ancestors`/`path`/`best` in `set`) at more than one level. This port's own fuzz pool and native
tests both add it explicitly — see "What we test in addition" and "Fuzz + bench".

**A key that is a byte-for-byte extension of another by exactly one zero byte.**
`findCriticalBit`'s tail branch compares the longer key's next byte against an *implicit* `0`
(`bitwise.criticalBit8Mask(b.charCodeAt(i))`, a one-argument call whose missing second parameter
becomes `0` through XOR coercion); when that next byte happens to be a literal `0x00`, the packed
mask degenerates to `0xff`, and every present byte routes right at that node regardless of its own
value. No original test ever embeds a NUL byte, so this degenerate case is untested there; this
port's own native test exercises it directly (see below).

**Non-Latin-1 keys.** `charCodeAt` returns a full UTF-16 code unit, but every bitwise helper
upstream feeds it into masks with `0xff` internally, so the "critical bit" computed for a code point
≥ 256 is not the true first differing bit at all — a latent bug neither original suite can reach,
since every test key is plain ASCII. See "Deliberate divergences".

## What we test in addition

**Rust native tests** (`crates/mnemonist-core/src/structures/critbit_tree_map.rs`, 9):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_set_suite`, `reproduces_the_upstream_delete_suite`, `keys_that_differ_only_in_length_do_not_break`, `for_each_visits_in_sorted_key_order`, `clear_resets_size_and_removes_everything` | the upstream blocks, as a baseline |
| `keys_differing_only_in_the_last_byte_route_correctly`, `a_deep_prefix_chain_is_fully_reachable` | the gate 6 falsification target: deep critical-bit positions and multi-level bubble-up |
| `a_shared_prefix_followed_by_a_nul_byte_still_routes_correctly` | the `0xff`-mask degenerate case above |
| `setting_again_after_deleting_back_to_empty_does_not_point_root_at_a_stale_slot` | a port bug this unit's own differential fuzzer found (see "Bugs this found") |

**Differential fuzzer** — see "Fuzz + bench". Reaches deep critical-bit positions and heavy
prefix-sharing on every campaign, measured directly rather than assumed.

## Bugs this found

**Two port defects, not upstream's**, both found by this unit's own differential fuzzer and fixed
before any campaign was logged — full account in `planning/NOTES.md`'s "Two port defects found by
fuzzing" entry for this pair of units, and in this file's own module doc comment. Summary: `set`'s
"tree is empty" fast path hardcoded `root = external_ptr(0)`, correct only the very first time it
runs; after a `delete` back to an empty tree, this port's append-only arena already holds a stale
entry at index `0` (unlike upstream, which links real, garbage-collected object references and has
no equivalent index to get wrong), and the next `set` pushed its key at index `1` while `root` kept
pointing at the stale slot. `CritBitTreeMap::root`'s own "a reachable external node always holds a
value" panic caught the mismatch, minimised by proptest to
`set("a", undefined); delete("a"); set("a", undefined)`. Fixed by capturing the real pushed index.

No upstream bug was found in this unit specifically — both of upstream's genuine defects in this
engine are in the **fixed** variant's typed-array bookkeeping (B-260, B-261; see
`docs/modules/fixed-critbit-tree-map.md`), which has no equivalent in the unbounded, garbage-
collected version.

## Deliberate divergences

* **D-245** (DECISIONS-CANDIDATES.md): keys are truncated to their low 8 bits at the bridge
  (`mnemonist_napi::critbit_tree_map::decode_key`), rather than reproducing upstream's own masked
  critical-bit arithmetic for UTF-16 code points ≥ 256. A no-op for every key either original suite
  supplies (all Latin-1/ASCII); sidesteps re-deriving which of several interacting masked bitwise
  operations upstream's own bug would produce, for zero test-suite benefit. Same judgement call as
  trie's D-200.
* This port uses an **arena of indices** (`Ptr`, `keys`/`values`/`internals` vectors) rather than
  `Box`-linked nodes the way a naive translation of upstream's object graph would suggest. An
  implementation-technique difference with no observable consequence — see the module's own doc
  comment for why `Box` would fight the borrow checker on `set`'s bubble-up specifically, and why
  the arena shape is shared in spirit (not in code) with the fixed variant, which genuinely cannot
  use `Box` at all.

## Fuzz + bench

### Fuzz

```
module=critbit-tree-map   seed=42       cases=10987  ops=1096914  wall=60.0s  divergences=0
module=critbit-tree-map   seed=20260801 cases=10634  ops=1054262  wall=60.0s  divergences=0
```

**Grammar:** `crates/difffuzz/src/modules/critbit_tree_map.rs`. `PREFIX_POOL` is
`["a", "ab", "abc", "abcd", "abcda", "abcdb", "b", "ba"]` — eight entries, of which **5/8 are
themselves a strict prefix of another entry**, measured (not eyeballed) by
`pool_self_check_most_entries_are_a_prefix_of_another_entry`, the same threshold `trie`'s own pool
was measured against. `"abcda"`/`"abcdb"` differ **only in their last byte** (byte index 4), forcing
a critical bit at the deepest position this pool's shared prefix allows — measured by
`pool_self_check_contains_a_pair_differing_only_in_the_last_byte` — and a 2,000-sample draw from the
real `set` op strategy shows generated keys revisiting the pool's prefix relationships in practice
(`pool_self_check_generated_programs_revisit_prefix_relationships`; typically ~65% of generated `set`
keys are a strict prefix of another generated `set` key across both regimes sampled).

Ops: `set`, `get`, `has`, `delete`, `clear`. No cursor lifecycle ops — `critbit-tree-map.js` has no
iterator surface at all (no `values`/`keys`/`entries`, no `Symbol.iterator`), unlike `trie-map`.

**Observed state: `size` and `root`.** `root` is upstream's own property, rebuilt to match its exact
shape (`{critbit, left, right}` for an internal node, `{key, value}` for a leaf, `null` for empty) —
see `RootNode`'s doc comment for why `critbit` is reassembled into upstream's own packed
`(byteIndex << 8) | mask` integer rather than this port's internal `(byte_index, mask)` tuple: it is
what turns a critical-bit computation bug into a `root` mismatch rather than only a rendering one,
and it is exactly what caught gate 6's sabotage below.

**What this grammar deliberately does not cover:** `forEach` (no op drives it; its ordering is
covered instead by the native test and by gate 4), and non-Latin-1 keys (D-245, above).

### Falsification (gate 6)

**Target named before running:** `mnemonist_core::structures::critbit_tree_map::msb8`, the literal
port of `utils/bitwise.js#msb8` that isolates a byte's single highest set bit. Predicted to break
`keys_differing_only_in_the_last_byte_route_correctly` and `a_deep_prefix_chain_is_fully_reachable`
— both depend on correctly identifying which bit two keys first differ on.

**Sabotage:** `x & !(x >> 1)` → `x & (x >> 1)` (dropped the `!`, so the function returns everything
*but* the top bit of the pre-OR'd run, rather than isolating the top bit alone).

**Confirmed red**, three independent instruments:
* `cargo test`: 6 of 9 native tests failed, including both named targets and
  `reproduces_the_upstream_set_suite`.
* The differential fuzzer (`--seed 99 --duration 5`): diverged on the **second operation** of a
  2-op program (`set("abcda", ...)`, `set("a", ...)`), reporting
  `root.critbit: port 448 vs upstream 447` — the packed mask off by exactly the dropped bit,
  confirming the `root` observation's `critbit` field is precisely what this falsification exercises.
* (Gate 4 was not separately re-run under this sabotage; the native suite already reproduces the
  same assertions the original suite's `it` blocks check, and both failed identically.)

**Reverted; confirmed green** on both instruments: `cargo test` 19/19, and the same seed/duration
differential run reported `divergences=0` again.

Nothing stayed green here — unlike the fixed variant (see that file's own falsification section),
where the identical sabotage is invisible to the differential fuzzer for a structural reason worth
reading about.

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3); this unit is
therefore complete except gate 10 and deliberately not added to `tests/scope.txt`.
