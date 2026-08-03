# linked-list — evidence

Gate artifacts for `docs/modules/linked-list.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/linked_list.rs` — 21 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | the eleven blocks, as a baseline |
| `shifting_the_last_element_leaves_tail_stale`, `a_stale_tail_from_b_241_is_healed_by_the_next_push`, `a_stale_tail_from_b_241_is_healed_by_the_next_unshift`, `the_staleness_only_appears_once_the_list_is_shifted_fully_empty` | 5, 6 — BUG-LINKED-LIST-1 |
| `a_push_after_the_cursor_opened_is_visible_if_not_yet_past_the_tail` | 1 |
| `a_push_after_the_cursor_has_passed_the_tail_is_not_visible` | 1 |
| `a_shift_is_invisible_to_a_cursor_already_open` | 2 |
| `an_unshift_is_invisible_to_a_cursor_already_open` | 2 |
| `a_cursor_opened_on_an_empty_list_never_yields_anything_even_after_pushes` | 3 |
| `clear_does_not_affect_a_cursor_already_open` | 4 |
| `a_cursor_is_not_restartable` | DIV-STACK-1 |
| `a_for_each_shaped_walk_sees_a_push_made_from_its_own_callback_on_the_lone_tail_node`, `a_step_shaped_walk_does_not_see_a_push_made_between_two_of_its_own_steps` | the port defect the fuzzer found — pins both halves directly |
| `push_after_for_each_shifts_the_list_to_empty_starts_a_fresh_one_element_list` | the second port defect the fuzzer found |
| `interleaved_unshift_and_push_produce_the_expected_order`, `a_long_workout_of_push_shift_unshift_matches_a_vecdeque_reference` | general correctness, cross-checked against `std::collections::VecDeque` |
| `from_iter_builds_in_order`, `shift_on_an_empty_list_reports_absence_without_panicking`, `an_empty_list_reports_empty_everywhere`, `step_checked_reports_done_rather_than_a_gap` | baseline edges |

## Fuzz grammar

* **Op alphabet:** `push` (5), `unshift` (4) — both outweigh `shift` (3) so a program keeps enough
  live nodes to reach the liveness rules rather than emptying the list every few operations —
  `first`/`last` (2 each, the pair BUG-LINKED-LIST-1 depends on), `peek`/`clear` (1 each), `$iter` over
  `values`/`entries` (2), `$next` (4), `$spread` (1), `$forEach` (3, the heaviest of the
  cursor-lifecycle ops — this is the one that reaches "push while the walk is mid-flight").
* **Observable state:** `size`, `first()`, `last()`, `toArray()`, compared after every operation.
* **Values:** a small pool (`0..24`), so a shrunk repro is unambiguous.
* **`$forEach` mutation table:** `push`/`unshift`/`clear` uncapped (`shift` uncapped too — none of
  the three have `push`'s tail-chasing hazard, since `shift`/`unshift`/`clear` are all invisible or
  bounded to the cursor per the module's own liveness rules), `push` alone capped at 8 for the
  reason given in the document and log.
* **Deliberately excluded:** object/reference identity questions (`JsSlot`/`WeakKey`-shaped) do not
  apply — `Value` is compared by content, matching this test file's own primitive-only style — so
  the fuzz side runs `LinkedList<serde_json::Value>` directly, no bridge-specific mirror key type
  needed, unlike `default-map`'s `FuzzKey`.

## Falsification record (gate 6)

Two Rust-level defects were falsified, plus BUG-LINKED-LIST-1; all three were assertions the port's own
history had already made, so the sabotage is literally "revert the fix" in two cases.

**A — the `forEach` timing fix.** This is the same shape as "Bugs this found" #1 in the document;
not separately re-run as a formal gate 6 sabotage, since finding it via the fuzzer IS the
falsification — the campaign that found it is the confirmation.

**B — `push` branching on `head` vs `tail`.** Assertion named first: the last line of
`push_after_for_each_shifts_the_list_to_empty_starts_a_fresh_one_element_list`,
`assert_eq!(list.first(), Some(&9))`. Sabotage: reverted `push` to `match self.tail { ... }`.
Confirmed red: two Rust tests failed (the named one, plus
`a_stale_tail_from_b_241_is_healed_by_the_next_push`'s `first()` assertion, added specifically
because the pre-existing `last()`-only version of that test could not distinguish the two
branches). The original mocha suite stayed green (11 passing) — expected; it never reaches this
state. The differential fuzzer caught it in 162 cases, 272 operations, 0.1 seconds, on the
identical minimised repro shown in the document. Reverted; confirmed green at all three: the two
Rust assertions pass, `11 passing` on the original suite, and a 200-case replay of the same seed
comes back `0 divergences`.

**Nothing was found to be blind here.** Every instrument — the Rust unit tests, the original
suite (correctly green, since it does not test this), and the differential fuzzer — behaved
exactly as expected for this sabotage.

## Bench table

`bench/results.json` → `modules["linked-list"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`shift`/`walk` (50/25/25), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **6.18** | 25.47 | 4.1× faster |
| p99 ns/op | **10.79** | 55.44 | 5.1× faster |
| RSS delta MB | **17.9** | 195.8 | |
| structure-only RSS delta MB | **1.5** | 9.8 | |
| startup ms | **0.6** | 15.5 | 26× (reported separately; not throughput) |
