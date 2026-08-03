# vp-tree — evidence

Gate artifacts for `docs/modules/vp-tree.md`: fuzz grammar detail, self-check figures.

## Fuzz grammar

* **Op alphabet:** `nearestNeighbors(k, query)` (weight 5) · `neighbors(radius, query)` (5).
* **Items and queries are integers in `0..24`**, with `distance(a, b) = |a - b|` — reusing
  `bk-tree`'s own `bkAbsDiff` oracle factory rather than adding a near-duplicate.
* **`neighbors`' radius is drawn across the whole possible span, `0..=24`**, specifically so a
  campaign's radii include both extremes.
* **Observable state: `size`, `nodes`, `lefts`, `rights`, `mus`** — the tree's exact shape,
  compared byte-for-byte on every one of the thousands of randomly generated constructions in a
  campaign, not only the two fixed fixtures the native tests above pin.
* **Deliberately excluded:** a throwing distance function (`|a-b|` cannot throw) and non-integer
  items — both covered by native tests instead, per Deliberate divergences.

## Grammar self-check

`cargo test -p difffuzz --lib grammar_self_check_radius_spans_full_pruning_and_none -- --nocapture`,
an 80-item tree built from the same narrow-range distribution, every `(radius, query)` pair the
grammar's own ranges cover, distance calls counted directly:

```
vp-tree grammar_self_check: 396/600 queries pruned at least one node; 204/600 visited every node
(radius large enough that no pruning was possible)
```
