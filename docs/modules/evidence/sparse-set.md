# sparse-set — evidence

Gate artifacts for `docs/modules/sparse-set.md`: test-to-gap tables, fuzz grammar, full
falsification record, full benchmark tables.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/sparse_set.rs` — 19 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all six upstream blocks, as a baseline |
| `a_duplicate_add_changes_nothing_at_all` | 2 — and asserts both arrays are byte-identical, not just `size` |
| `delete_swaps_the_last_member_into_the_hole` | 1, 3, 4, 5 — deletes a middle member, then the member that moved |
| `deleting_the_last_member_is_a_self_swap` | 3, 5 |
| `clear_leaves_stale_entries_that_stay_unreachable` | 6 — asserts the debris is *there* and unreachable, pinning the O(1) clear against a future "tidy-up" |
| `reads_out_of_range_report_absence` | 11 (the `has`/`delete` half) |
| `an_out_of_range_add_corrupts_the_set_exactly_as_upstream_does` | 10, 11 — the compound defect, pinned value by value |
| `negative_members_arrive_as_their_two_s_complement_and_truncate_alike` | 11 — ToUint32 and a narrowing store compose to the same answer in both languages |
| `size_can_exceed_length_and_then_iteration_hits_the_gap` | 11, 16 — the `undefined` window, reached through public calls only |
| `a_delete_past_capacity_writes_dense_but_not_sparse` | 11 — the expando case, see "Bugs this found" |
| `cursors_do_not_restart_but_the_set_can_be_walked_again` | 13, 14 — both levels of DIV-STACK-2 in one test |
| `a_delete_during_iteration_is_visible_to_the_cursor` | 12 |
| `a_delete_ahead_of_the_cursor_can_yield_a_member_twice` | 12 — the nastier half: the swap makes the last member appear twice and the deleted one never |
| `an_add_during_iteration_is_not_visible_to_the_cursor` | 12 — the frozen-length half |
| `picks_one_pointer_width_for_both_arrays` | 9 — five lengths across both width boundaries |
| `rejects_a_length_no_pointer_array_can_index` | 9 (the throw) |
| `a_zero_length_set_accepts_nothing_and_finds_nothing` | 11, 15, 16 — the degenerate end of the corruption path |
| `a_one_member_set_behaves` | 7 |
| `fills_to_capacity_without_running_off_the_end` | 8 |

`crates/mnemonist-core/src/cursor/mod.rs` — 13 tests against a synthetic source rather than
`SparseSet`: non-restartability, partial consumption, element writes visible, growth invisible,
shrink opening gaps rather than terminating, a fully emptied source, a reversed walk (the
`Stack.values()` shape), and the detached `CursorState` being driven across a mutation that a
borrowing cursor could not permit.

`crates/mnemonist-core/src/utils/typed_arrays.rs` — 10 tests, two of them new for this module:
`try_get` reporting the out-of-range read instead of panicking, and `try_set` dropping the
out-of-range write while still truncating the in-range one.

## Fuzz grammar

* **Op alphabet:** `add(m)` (weight 5) · `delete(m)` (2) · `has(m)` (2) · `clear()` (1) ·
  `$iter("values")` (2) · `$next()` (3) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, **`dense` and `sparse`**. Both
  backing arrays are public upstream, and comparing them slot for slot is what makes the swap in
  `delete` and every truncating store checkable directly rather than only through their eventual
  effect on iteration order.
* **Lengths:** `0..=400`. Zero is included because `new SparseSet(0)` is legal and every member is
  then out of range; 400 straddles 256, where both arrays switch to 16-bit and where a truncating
  `dense` store starts folding distinct members onto the same value.
* **Members:** `0..length + 64`, so roughly **one in eight is out of range**.
* **Program length:** 1..200 ops.
* **`$forEach` mutations:** `delete(a0)`, `clear()` (both uncapped), `add(a0 + 1)` (capped at two
  firings — see the document for why the cap is not tuning).
* **Deliberately excluded: nothing.** This is the contrast with `static-disjoint-set`, which had to
  exclude out-of-range indices. Here they are the most interesting part of the grammar, because
  out-of-range `add` is the only route to `size > length` and therefore the only route to the
  `undefined` window.

## Falsification record

### Fuzzer falsification (two sabotages, one per half of the grammar)

**A — the out-of-range half.** Sabotage: `delete`'s two swap stores treated as if they behaved
alike past capacity, i.e. writing `sparse[0]` where upstream writes an expando. This is the mistake
the port actually made first (BUG-SPARSE-SET-3). Caught in **1,416 cases (6.6 s)**, shrunk from 200 ops to
seven:

```js
var s = new SparseSet(5);
s.add(0); s.add(1); s.add(2); s.add(4);
s.add(5);      // out of range: dense takes it, sparse does not
s.add(5);      // size runs to 6 against a length of 5
s.delete(1);   // port sparse [1,1,2,0,3], upstream [0,1,2,0,3]
```

**B — the cursor half.** Sabotage: one line in `mnemonist-core/src/cursor`, returning `Step::Done`
where the faithful port returns `Step::Gap` — exactly the rejected Option B. Caught in
**352 cases (0.3 s)**, shrunk to two operations:

```js
var s = new SparseSet(0);
s.add(0);        // out of range on a zero-length set: size becomes 1
Array.from(s);   // port [], upstream [undefined]
```

Both sabotages were reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/sparse-set.txt`, where proptest replays them before any novel
case on every subsequent run.

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications above: gate 6 asks that sabotaging the core turns the
*original mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create an iterator over the set's values` —
`assert.deepStrictEqual(obliterator.take(set.values()), [3, 6, 9])`, at `test/sparse-set.js:74`.
It was chosen because it is the only assertion in the file that reaches the new cursor machinery,
which is the code this unit exists to prove out.

**The sabotage:** `Sequence::freeze` for `SparseSet` returning `dense.len()` — the set's
*capacity* — instead of `self.size`, which is the single most plausible way to mis-port
`var size = this.size`.

**Confirmed red**, and red in precisely the named place: `5 passing, 1 failing`, the failure being
that assertion, with `actual` `[3, 6, 9, 0, 0, 0, 0, 0, 0, 0]` against `expected` `[3, 6, 9]`.
Reverted; **confirmed green again**: `6 passing`.

## Bench tables

`bench/results.json` → `modules["sparse-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, 32 MB L3, WSL2, Node 24.18.1, rustc 1.97.1.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, pinned to CPUs 2–3.

**`mixed-1e6`** — 1e6 `add`/`has`/`delete` (50/25/25) over length 1e6, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **8.9** | 11.5 | 1.3× faster |
| p99 ns/op | 24.9 | 25.6 | **a tie** |
| RSS delta MB | **11.4** | 39.0 | |
| structure-only RSS delta MB | **1.4** | 10.2 | |
| startup ms | **0.6** | 15.1 | reported separately; not throughput |

**`mixed-4e6`** — the same op mix at four times the length:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **13.8** | 21.2 | 1.5× faster |
| p99 ns/op | **240.4** | 324.3 | 1.3× |
| min ns/op | **9.0** | 13.8 | |
| structure-only RSS delta MB | **12.9** | 21.6 | |

**`drain-1e5`** — full iteration: a length-1e5 set prefilled by 1e5 random `add`s (leaving 63,061
distinct members), then 100 complete walks, one timed sample per walk:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/element | **0.94** | 3.18 | 3.4× faster |
| p99 ns/element | **1.66** | 4.76 | 2.9× |
| structure-only RSS delta MB | **0.4** | 6.6 | |

`bench/drive.js` derives the `regressions` array mechanically from the published metrics, so one
cannot be quietly dropped from a run — which is exactly why `bench/results.json` currently carries
one entry nobody put there on purpose: `mixed-4e6`'s `p99_ns_per_op` at **330.395 (port) vs 328.803
(original), ratio 1.00**. The table above (an earlier run) shows no such thing; the two disagree
because `bench/results.json` reflects a later re-run and the document's prose was not regenerated
alongside it — the JSON, not prose, is the current source of truth per `bench/methodology.md`. Full
investigation: log.

**RefCell borrow-flag probe** (`bench-runner --refcell-probe`), size 1e6, three repeated probes of
10 measured passes each:

| probe | plain p50 ns/op | RefCell-wrapped p50 ns/op | delta |
|---|---|---|---|
| 1 | 9.278 | 9.317 | +0.4% |
| 2 | 9.218 | 9.368 | +1.6% |
| 3 | 9.248 | 9.147 | −1.1% |

A fourth probe at 30 measured passes (30,000 samples/side) gave 9.157 plain against **8.886**
RefCell-wrapped — the wrapped variant faster. At size 4e6 the sign flips between repeated probes
too (+18%, then −4.8%, then −2.3%).
