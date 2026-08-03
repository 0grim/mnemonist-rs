# bk-tree — working log

Chronological. See `docs/modules/bk-tree.md` for the current-state document.

## Bench: domain size for `mixed-3e5` chosen by ruling out two failure modes

Domain size was chosen by ruling out two failure modes first:

* **2,000 — too small.** Duplicate `add`s chain onto each other and both `add`/`search` degrade
  superlinearly; a 200,000-op run took 21 seconds.
* **1,000,000 — too large.** The tree collapses to depth 1, so `search`'s recursive descent never
  actually runs.

300,000 was settled on as the workable middle, and is what the current document states as the
workload's domain size.
