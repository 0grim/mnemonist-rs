# default-weak-map — evidence

Gate artifacts for `docs/modules/default-weak-map.md`: test-to-gap table, fuzz grammar, full
falsification record.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/default_weak_map.rs` — 12 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | the four blocks, as a baseline |
| `b_242_the_factory_re_runs_on_every_get_of_a_stored_undefined_value` | 1 — BUG-DEFAULT-WEAK-MAP-1, pinned call-by-call |
| `a_defined_value_written_by_the_factory_ends_the_b_242_re_run` | 1 |
| `a_re_triggered_factory_overwrites_in_place_rather_than_duplicating_the_key` | 1 — no duplicate identity leaks out of a re-triggered factory |
| `has_and_peek_disagree_on_a_stored_undefined` | 2 |
| `set_overwrites_an_existing_key_in_place` | — |
| `delete_distinguishes_a_missing_key_from_a_stored_undefined` | 2 |
| `clear_drops_every_entry` | 5 |
| `an_empty_map_reports_nothing` | — |
| `values_mut_reaches_every_stored_slot_including_the_undefined_ones` | — |
| `identity_not_content_decides_a_match_two_equal_but_distinct_keys` | 3 — pins that the matcher, not core, decides identity, and that two predicates that never consider each other equal never collapse to one entry |

## Fuzz grammar

* **Op alphabet:** `get` (5, the mutating read and the only route to a factory call),
  `set` (4), `delete` (3), `peek`/`has` (2 each), `clear` (1). No cursor ops — this module has no
  iteration surface at all (see "What is and is not observable" in the document).
* **Key pool:** eight slots, mirrored on the Rust side as `FuzzKey(u8)` — an index, not an object,
  since `mnemonist-napi` is a `cdylib` and cannot be linked into this binary (identical reasoning
  to `default-map`'s own `FuzzKey`). `fuzz/oracle.js`'s `WEAK_KEY_POOL` is the real-object side:
  eight plain objects, created once at oracle start-up, held by a module-level array for the
  process's entire life, so none of them is ever eligible for collection during any campaign.
* **Values:** `undefined` (weight 2, the only route to BUG-DEFAULT-WEAK-MAP-1), `null`, small integers, `'v'`.
* **Constructors:** `undefined`/`null` factories, both already in `fuzz/oracle.js`'s shared
  `FACTORIES` table (added for `default-map`) and reused verbatim: both accept upstream's
  one-argument `(key) -> value` signature unchanged, since they ignore every argument they are
  called with regardless of arity.
* **Observable state, compared after every op:** none (`observations()` is empty). Every
  comparison is a return value; this is the entire observable surface, not a narrowed one.
* **Deliberately excluded:** any observation of key collection/reclamation (impossible to fuzz
  honestly); object keys with distinguishable identity but coincidental *structural*
  equality (every pool slot is a bare `{}`, so this grammar alone cannot distinguish "compares by
  identity" from "compares by deep equality" the way a real adversarial case would — that
  distinction is instead pinned by the core module's own
  `identity_not_content_decides_a_match_two_equal_but_distinct_keys` Rust test, which controls the
  matcher directly); a non-object argument to any method (bridge-specific, and this grammar only
  ever generates pool-object keys by construction).

## Falsification record (gate 6)

**The assertion named first:** the probe script's `calls === 3` (mirroring the core Rust test
`b_242_the_factory_re_runs_on_every_get_of_a_stored_undefined_value`'s own `assert_eq!(calls, 3, ...)`),
run against the real compiled addon rather than against core directly — because the bridge is
where BUG-DEFAULT-WEAK-MAP-1's *composition* (peek-miss triggers the factory) actually lives; core's `peek`/
`write_from_factory` are neutral primitives a caller composes, the same way upstream's own bug is
a composition of two lines, neither wrong on its own.

**The sabotage:** `crates/mnemonist-napi/src/default_weak_map.rs`'s `get` changed to check
`has()` (key presence) before running the factory, instead of `peek()` (value definedness) — the
"fix" a careful porter would reach for.

**Confirmed red:** a direct script against the rebuilt addon (`set(key, undefined); get(key) x3`)
reported `calls === 0`, not `3` — even sharper than expected, since with this sabotage `has()` is
already `true` from the `set()` call, so the factory never runs even once.

**Confirmed green where expected, for a stated reason.** The original mocha suite stayed green
(`4 passing`): it never counts factory invocations, so it cannot see this class of bug either way.
**The differential fuzzer *also* stayed green** (`500 cases, 49416 ops, 0 divergences`) — and this
is not a miss, it is a structural fact stated up front in this document's own module docs and
`default_map.rs`'s: *the differential fuzzer compares `mnemonist-core` against upstream JS; the
napi bridge is not in that loop at all.* A bridge-only composition bug is invisible to it by
construction, the same way PORTBUG-1 was before this port started holding core state in a `RefCell`.

**Reverted; confirmed green again:** the same script reports `calls === 3`, and the original suite
still passes (`4 passing`).
