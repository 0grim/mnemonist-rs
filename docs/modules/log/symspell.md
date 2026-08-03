# symspell — working log

Chronological. See `docs/modules/symspell.md` for the current-state document.

## Bench: vocabulary generator tuned through two rejected designs before reaching the current scramble

A random vocabulary defeats this structure entirely, so the workload's vocabulary generator went
through two designs before the current one, both documented in `bench/runner/src/symspell.rs`'s own
module docs:

* **A 4-letter suffix over a 10-symbol alphabet** was too *dense* — ~413 suggestions per call out
  of a 4,000-word dictionary, and a 200,000-op pass took over a minute.
* **Switching to a 6-letter suffix over the full 26-letter alphabet did not fix it** — ~542
  suggestions per call. The actual cause was encoding the domain value directly in any fixed base,
  which makes consecutive values one-character-apart neighbours by construction, regardless of
  alphabet size.

The fix was a multiplicative scramble (`Math.imul`-matched) before encoding, which spreads the
domain across the suffix space so only the deliberate one-character query perturbation makes a
query findable. Measured after the fix: 98.4% of `search` calls return at least one suggestion,
averaging 1.40 suggestions per call — the figures now stated as current in the document.
