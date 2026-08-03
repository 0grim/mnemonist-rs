# fuzzy-map — working log

Chronological. See `docs/modules/fuzzy-map.md` for the current-state document.

## Fuzz spec harness bug: factory name mismatch, spec had never actually run (found and fixed this series)

The difffuzz spec's `Hash::named` matched the literal names `"identity"`/`"lower"`, but
`fuzz/oracle.js`'s `FACTORIES` table and this spec's own constructor strategy both use the prefixed
`fuzzyIdentity`/`fuzzyLower` (chosen precisely so this module's factory names cannot collide with
`default-map`'s, which the oracle also serves). Every generated program panicked at construction,
before a single comparison ran — which is why this spec had never actually executed: it was not yet
wired into `tests/differential.rs`, and the one earlier manual campaign attempt persisted a
regression seed that, on inspection, was the harness panic rather than a finding. That spurious seed
was deleted rather than kept. Fixed by matching the prefixed names; see
`crates/difffuzz/src/modules/fuzzy_map.rs`'s history.
