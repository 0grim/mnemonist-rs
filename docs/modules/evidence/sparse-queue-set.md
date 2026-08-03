# sparse-queue-set — evidence

Gate artifacts for `docs/modules/sparse-queue-set.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/sparse_queue_set.rs` — 17 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all seven upstream blocks, including all 13 wrap cycles, as a baseline |
| `the_dequeue_sentinel_truncates_at_the_pointer_width_boundary` | **1, 2, 3** — BUG-SPARSE-QUEUE-SET-1 at `capacity` 256, pinned on `sparse`, on the false-positive `has`, *and* on the enqueue that consequently does nothing |
| `one_below_the_boundary_the_sentinel_fits` | 1, 3 — the control at 255, so the defect is attributed to the boundary and not to the port |
| `the_sentinel_truncates_at_the_second_boundary_too` | 1, 3 — and at 65536, one width up. This is also where the fuzzer's disclosed exclusion is covered |
| `an_out_of_range_enqueue_evicts_a_live_member` | **5, 6, 7, 10** — BUG-SPARSE-QUEUE-SET-2, pinned on `dense` slot by slot and on a walk that yields five members from a four-slot ring |
| `an_out_of_range_member_truncates_into_the_ring` | 4, 10 — the truncating store, and that the member dequeues under its truncated name |
| `a_zero_capacity_queue_counts_phantoms` | **8, 12, 18** — BUG-SPARSE-QUEUE-SET-3: the `NaN` index, the dropped stores, `start` climbing past its own capacity, and every step a gap |
| `dequeuing_an_empty_queue_changes_nothing` | 11 |
| `a_one_slot_ring_wraps_on_every_dequeue` | 9, 14 |
| `clear_resets_the_rotation_as_well_as_the_size` | 13 — clears a *rotated* queue, and asserts the debris stays unreachable afterwards |
| `membership_holds_across_a_wrapped_window` | the `\|\|` in the window test, on the wrapped case, with the non-member checked too |
| `cursors_do_not_restart_but_the_queue_can_be_walked_again` | 16, 17 — both levels of DIV-STACK-2 |
| `a_dequeue_during_iteration_does_not_move_the_walk` | **15** — the frozen-`start` half, which is the opposite of what a live cursor would do |
| `an_enqueue_that_overwrites_an_unread_slot_is_visible` | **15** — the live-`dense` half, driven through an out-of-range enqueue so no duplicate check fires |
| `picks_one_pointer_width_for_both_arrays` | 3 — five capacities across both width boundaries |
| `rejects_a_capacity_no_pointer_array_can_index` | 3 (the throw) |
| `fills_and_drains_a_full_ring` | 5 — 300 members in, 300 out in order, and the queue back at `start == 0` |

## Fuzz grammar

* **Op alphabet:** `enqueue(m)` (weight 4) · `dequeue()` (3) · `has(m)` (3) · `clear()` (1) ·
  `$iter("values")` (2) · `$next()` (3) · `$spread()` (1). `dequeue` is weighted heavily because it
  is the **only** op that moves `start`, and the ring is what this module has that its siblings do
  not; a read-heavy mix would fill the ring once and never rotate it.
* **Observable state, compared after every op:** `size`, `capacity`, **`start`**, `dense`,
  `sparse`. `start` earns its place empirically — falsification B below differs in nothing else.
* **Capacities:** a mixture rather than one range, because the interesting capacities are not
  uniformly distributed. `0..=400` (weight 4, with 0 for BUG-SPARSE-QUEUE-SET-3); `1..=8` (weight 3, so a 200-op
  program wraps many times rather than filling once); and `{255, 256}` (weight 2) — the BUG-SPARSE-QUEUE-SET-1
  boundary drawn as a **point** with its control, since a uniform draw over `0..=400` reaches 256
  about once in 400 programs.
* **Members:** `0..capacity + 64`, so roughly one in eight is out of range.
* **Program length:** 1..200 ops.
* **Deliberately excluded: capacity 65536**, the second BUG-SPARSE-QUEUE-SET-1 boundary. It is the same defect one
  width up and is covered by `the_sentinel_truncates_at_the_second_boundary_too` in the core's
  tests instead. Including it cost about **95% of throughput — measured, 880 op/s against 15,000**
  — because the observable state is two backing arrays, serialised, sent and compared after every
  single operation. A 60-second campaign that executes 5% of its programs is a worse check than a
  native test plus a fast campaign. Nothing else is excluded; every out-of-range member is
  generated.

The BUG-SPARSE-QUEUE-SET-1 sentinel needs no special op to observe: `sparse` is in the observed state, so every
`dequeue` compares the value written into it.

## Falsification record

### Fuzzer falsification

**A — the ring.** Sabotage: `enqueue` "fixed" to refuse a full ring, i.e. BUG-SPARSE-QUEUE-SET-2 repaired. Caught in
**811 cases (0.8 s)**, shrunk to two operations on the smallest possible ring:

```js
var s = new SparseQueueSet(1);
s.enqueue(0);
s.enqueue(1);   // out of range on a capacity-1 queue
                // port dense [0] size 1, upstream dense [1] size 2
```

The repro doubles as the smallest possible demonstration of BUG-SPARSE-QUEUE-SET-2 itself: at capacity 1, the second
enqueue evicts the first.

**B — the degenerate capacity.** Sabotage: `dequeue`'s wrap check tidied from `===` to `>=`.
Reading `if (this.start === this.capacity) this.start = 0;` as a bounds check and "hardening" it is
the most invisible change available in this module — the two are identical for every capacity *but
zero*. Caught in **854 cases (3.9 s)**, shrunk to two operations:

```js
var s = new SparseQueueSet(0);
s.enqueue(0);
s.dequeue();   // port start 0, upstream start 1
```

Nothing else differs: not `size`, not the members, not either backing array. That seed is
simultaneously the justification for observing `start` and the proof that the constructor strategy
really does generate capacity 0.

Both sabotages were reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/sparse-queue-set.txt`.

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications: gate 6 asks that sabotaging the core turns the *original
mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:** the wrap-around block's
`assert.deepStrictEqual(obliterator.take(queue.values()), values)`, at
`test/sparse-queue-set.js:77`. Chosen because it is the only assertion in the file that runs
against a **rotated** ring — it passes trivially on the first of the block's 13 cycles, when
`start` is still `0`, and only bites from the second.

**The sabotage:** `Sequence::slot` computing `start + ordinal` without the modulo, i.e. reading the
ring as linear. That is the plausible mis-port of upstream's `i++; if (i === c) i = 0;`, which does
not look like a modulo when you read it.

**Confirmed red**, and red in precisely the named place: `6 passing, 1 failing`, the failure at
`test/sparse-queue-set.js:77` on the second cycle. Reverted; **confirmed green again**:
`7 passing`.

**And a second falsification that was expected to stay green, and did.** Following `sparse-map`'s
lead, it is worth knowing *which* sabotages this suite cannot catch. The natural repair for BUG-SPARSE-QUEUE-SET-2 —
`if (size >= capacity) return this;` in `enqueue` — leaves the suite at **7 passing, 0 failing**
while turning **three** native tests red (`an_out_of_range_enqueue_evicts_a_live_member`,
`an_enqueue_that_overwrites_an_unread_slot_is_visible`, `a_zero_capacity_queue_counts_phantoms`)
and being caught by the differential fuzzer in 0.8 seconds. The suite cannot see it because the
only member that could enqueue into a full ring is one already present, and the wrap block never
fills the ring anyway.

## Bench table

`bench/results.json` → `modules["sparse-queue-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `enqueue`/`has`/`dequeue` (50/25/25) over capacity 1e6, xorshift32 seed
42, `sparse-set`'s add/has/delete shape with FIFO names — `dequeue` takes no operand, so the
workload's second operand goes unused on that op exactly as `has`'s does on `sparse-set`'s own
workload. Members drawn in range, so this never reaches BUG-SPARSE-QUEUE-SET-2's out-of-range eviction path:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **8.4** | 12.9 | 1.5× faster |
| p99 ns/op | **23.4** | 61.2 | 2.6× faster |
| min ns/op | **5.8** | 10.2 | 1.8× faster |
| RSS delta MB | **11.6** | 41.3 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 16.5 | 27× (reported separately; not throughput) |
