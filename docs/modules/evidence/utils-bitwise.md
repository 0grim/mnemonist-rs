# utils/bitwise — evidence

Gate artifacts for `docs/modules/utils-bitwise.md`: test-to-gap table.

## Test-to-gap mapping

`crates/mnemonist-core/src/utils/bitwise.rs` — 14 tests:

| Test | Closes gap |
|---|---|
| `matches_node_24_18_1_on_a_generated_cross_product` | all of them, at 252 points — see the document |
| `both_popcounts_agree_with_the_true_bit_count` | 1 — ~150k words against `u32::count_ones` |
| `table8_is_exactly_popcount_of_every_byte` | 2 — all 256 entries |
| `popcount_of_a_negative_is_the_count_of_its_two_s_complement` | 5 |
| `non_integers_truncate_toward_zero_first` | 5 |
| `values_past_the_32_bit_range_wrap` | 5 — including 2^53 and 1e30, where an `i64` cast would saturate |
| `non_finite_inputs_become_zero` | 5 |
| `msb32_returns_zero_for_every_input_with_the_top_bit_set` | 3 — B-19 |
| `msb32_is_correct_below_the_sign_bit` | 3 — the half that works, over the same word sample |
| `msb8_is_correct_on_bytes_and_unmasked_above_them` | 3, 4 — all 256 bytes, then the overflow cases |
| `test_reads_one_bit_and_wraps_the_shift_count` | 6 |
| `the_byte_wide_critical_bit_helpers_agree_with_each_other` | 7 — the complement property over 4,096 byte pairs |
| `test_critical_bit8_carries_through_number_arithmetic` | 9 |
| `critical_bit32_mask_is_negative_because_the_trailing_mask_re_signs_it` | 8 — B-20 |
