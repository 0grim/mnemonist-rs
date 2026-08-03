# fuzzy-multi-map — evidence

Gate artifacts for `docs/modules/fuzzy-multi-map.md`: fuzz grammar detail, full benchmark table.

## Fuzz grammar

* **Op alphabet:** `add` (weight 5), `set` (2), `has`/`get` (2 each), `clear` (1).
* **Source pool:** `"Hello"`, `"HELLO"`, `"World"` — `fuzzyLower` (the real factory, shared with
  `fuzzy-map`'s own campaign via `fuzz/oracle.js`'s `FACTORIES` table, so both sides run the
  identical function rather than two hand-written mirrors that could quietly disagree) collapses the
  first two onto one hashed key.
* **Constructor**: `new FuzzyMultiMap(fuzzyLower)` — one hash function shared by both directions,
  `List`-kind container (see D-167 for why `Set`-kind is out of scope for this campaign).
* **Observable state**: `size`, `dimension`, and `items` rendered as the **nested** object upstream's
  own `this.items` actually is — a `MultiMap` *instance*, not a raw `Map`, so `fuzz/oracle.js`'s
  generic `encode()` renders it as `{items: {$map: [...]}, size, dimension}` rather than a bare
  `{$map: ...}`. The very first draft of this spec flattened it, which is indistinguishable from the
  real shape only by accident and diverged on case 0 of every run before the fix — see
  `crates/difffuzz/src/modules/fuzzy_multi_map.rs`'s own `observe` doc comment.

## Bench table

`bench/results.json` → `modules["fuzzy-multi-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`has` (50/25/25), `ContainerKind::List`, over a 200,000
raw-key domain, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **17.3** | 27.4 | 1.6× faster |
| p99 ns/op | **34.2** | 58.5 | 1.7× faster |
| RSS delta MB | **10.8** | 65.3 | |
| structure-only RSS delta MB | **0.1** | 6.5 | |
| startup ms | **0.6** | 16.6 | 28× (reported separately; not throughput) |
