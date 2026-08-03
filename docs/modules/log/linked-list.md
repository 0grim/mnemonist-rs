# linked-list — working log

Chronological. See `docs/modules/linked-list.md` for the current-state document and
`docs/modules/evidence/linked-list.md` for the gate artifacts.

## Fuzz grammar hazard: uncapped `push` inside `$forEach` chased its own tail (found and fixed this series)

`$forEach`'s `push` mutation initially used the same uncapped limit every other mutation table in
this port uses. Because a `push` while the walk sits on the tail relinks that exact tail's `.next`
to the node just pushed — and the walk then advances onto precisely that node, which is now itself
the tail — an uncapped `push` chases its own tail forever. This is not a divergence: a real Node
`forEach` in the identical shape loops identically. It is a program this grammar must not generate,
since a campaign is meant to run thousands of finite cases in its time budget.

An earlier pair of campaign runs at the seeds now recorded in the document measured only ~3-4K
operations in 60-70 seconds because of this hazard. `push` was capped at 8 within `$forEach`;
per-case throughput went from roughly 2 seconds to roughly 5 milliseconds. `fuzz/log.txt` keeps both
the original (slow, but still zero-divergence) entries and the corrected re-runs, annotated, rather
than rewriting history. The document's own campaign numbers (2.37M operations, zero divergences)
are the corrected, fast re-runs.
