# utils/bitwise

Upstream: `utils/bitwise.js` (109 LOC) · **no test file exists.**

Port: `crates/mnemonist-core/src/utils/bitwise.rs`. No bridge: nothing in the upstream test corpus
reaches these functions directly, and nothing outside the library calls them.

---

## Scope note: this is not a "unit"

A unit is the require-closure of one upstream *test file*, and `utils/bitwise.js` has none — so
gates 3, 4, 6 and 10 have no target here and this file will never appear in `tests/scope.txt` on its
own. It is a **member** of the `bit-set` and `bit-vector` units, and the gates that apply to it are
1 (ported), 2 (`forbid(unsafe_code)`, zero deps), 7 (native tests) and 8 (this document). Its
falsification and its fuzz coverage are recorded here but performed through its hosts; see
"Fuzz + bench" below.

Written up separately anyway, because "the upstream suite does not test this file at all" is a
coverage fact worth stating in its own right rather than burying in two other documents.

## What upstream tests

**Nothing.** There is no `test/bitwise.js`, and no other test file requires the module.

The repo's own suite reaches exactly **one** of the nine exported functions, and only indirectly:
`table8Popcount`, through `BitSet.prototype.rank` and `BitVector.prototype.rank`. Both `rank` tests
use a handful of evenly spaced bits, so even that one function is exercised over a narrow slice of
its input domain and never against a reference count.

The remaining eight — `msb32`, `msb8`, `test`, `criticalBit8`, `criticalBit8Mask`,
`testCriticalBit8`, `criticalBit32Mask`, `popcount` — have **zero** coverage from `mocha`. Three of
them (`criticalBit8`, `criticalBit8Mask`, `testCriticalBit8`) are load-bearing for
`critbit-tree-map.js` and `fixed-critbit-tree-map.js`, both of which have their own test files that
exercise them transitively; `criticalBit32Mask` appears to have no caller in the shipped library at
all.

## What upstream does NOT test

Everything, so the useful form of this section is *which properties were never checked*:

1. **That either population count is correct.** `popcount` and `table8Popcount` are two independent
   implementations of the same function and are never compared with each other, let alone with a
   reference.
2. **That `TABLE8` is what it claims to be.** It is filled at module load by calling `popcount` on
   each byte; if `popcount` were wrong for a byte, the table would be wrong in exactly the same way
   and the two would still agree.
3. **`msb32` on any input at all.** Including the entire top half of its domain, where it is broken.
4. **`msb8` outside `0..=255`**, which the JSDoc calls "a byte" and the code never checks.
5. **The ToInt32 boundaries** — negative inputs, non-integers, values at or beyond 2^32, `NaN`,
   `Infinity`. Every function in the file begins with an implicit conversion and none of it is
   pinned.
6. **Shift-count wrapping in `test`.** `test(1, 32)` is `test(1, 0)`, which is `1`, not `0`.
7. **That `criticalBit8Mask` is the complement of `criticalBit8`**, which is the property it exists
   to provide.
8. **`criticalBit32Mask`'s sign**, which is where its defect is.
9. **`testCriticalBit8` at the Number/int32 boundary**, where `1 + (x | mask)` can reach 2^31 and
   the following `>> 8` reinterprets it as negative.

## What we test in addition

`crates/mnemonist-core/src/utils/bitwise.rs` — 14 tests:

| Test | Closes gap |
|---|---|
| `matches_node_24_18_1_on_a_generated_cross_product` | all of them, at 252 points — see below |
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

**The cross-product test is the one that matters.** 28 inputs × 9 second arguments, every one of the
nine functions evaluated on each, run through the **upstream file on real Node 24.18.1** and pasted
in as literals. So "this port matches upstream" is an executed comparison rather than an assertion,
while `mnemonist-core` still builds and tests with no JavaScript runtime present (gate 2). The
inputs deliberately include `0x80000000`, `0xffffffff`, `-1` and `256` — the values where upstream
misbehaves — so any later "tidying" of `msb32` or `criticalBit32Mask` fails the test.

`popcount` is additionally checked against `u32::count_ones` over ~150,000 words: every single-bit
and adjacent-pair pattern, all 65,536 values in each half of the range, and 20,000 xorshift32 draws.
It is exact everywhere, which makes it the one function in the file with no defect — worth pinning
precisely because it is the one a reader would be least surprised to find broken.

## Bugs this found

**B-19 — `msb32` returns `0` for every input whose bit 31 is set.**
`status: VERIFIED against Node 24.18.1`. `x |= (x >> 1)` is an **arithmetic** shift. An input with
the top bit set smears to `-1` at the first step, and the closing `x & ~(x >> 1)` is then
`-1 & ~(-1)`, which is `-1 & 0`, which is `0`. Measured: `msb32(0xFFFFFFFF) === 0`,
`msb32(2**31) === 0`, `msb32(-1) === 0`, while `msb32(0x40000000) === 1073741824`. The function is
correct across the entire lower half of its domain, which is why it has survived: the failure is
confined to the half where "which is the highest set bit?" has the most obvious answer.

`msb8` has the same shape, but its three-step smear stops well short of bit 31, so it only misfires
on input that is not a byte — `msb8(256) === 256`. Upstream's JSDoc says "a byte" and the code never
checks.

**B-20 — `criticalBit32Mask`'s trailing `& 0xffffffff` undoes its own `>>> 0`.**
`status: VERIFIED against Node 24.18.1`.

```js
exports.criticalBit32Mask = function (a, b) {
  return (~msb32(a ^ b) >>> 0) & 0xffffffff;
};
```

The `>>> 0` produces the intended unsigned mask. `& 0xffffffff` then converts *both* operands back
to signed 32-bit — `0xffffffff` is `-1` there — so the mask is an identity that re-signs the result.
Measured: `criticalBit32Mask(1, 2) === -3`, `criticalBit32Mask(0, 0) === -1`. The byte-wide sibling
`criticalBit8Mask` ends in `& 0xff` and is correct, which makes the pair a compact illustration of
the same idiom being right at one width and wrong at another. Low severity — `criticalBit32Mask`
appears to have no caller in the shipped library.

Both are reproduced bug-for-bug and pinned by tests, so a future cleanup fails rather than silently
diverging.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-319 | **Every function takes `f64` and returns `i32`.** | Not an aesthetic choice. Each is written in terms of JS bitwise operators, and every JS bitwise operator begins with ToInt32; taking `u32` would delete the conversion, and the conversion is where three of the four defects live. `to_int32` and `to_uint32` are exposed so a caller sees the coercion rather than inferring it. |
| D-320 | **`to_int32` is not upstream code.** | It is the *implicit* first step of every operator in the file, written out once. Implemented with an exact `fmod`, so it is right for magnitudes past 2^53 where an `i64` cast saturates and would silently disagree. |
| D-321 | **`TABLE8` is built from `u8::count_ones`, not from `popcount`.** | Upstream fills it by calling its own `popcount` at module load, which cannot be done in a `const fn`. The substitution is only legitimate if the two agree everywhere, so `table8_is_exactly_popcount_of_every_byte` checks all 256 entries against `popcount` rather than assuming it. |
| D-322 | **`popcount`'s intermediates are `f64`.** | Upstream's first statement is `x -= x >> 1 & 0x55555555`, where the subtraction happens on the *Number* and only the right-hand side is converted — so an input at or above 2^31 stays a float across the first step. Doing the whole thing in `i32` gives the same answer for every input tested, but by a different route, and the point of a bug-for-bug port is to transcribe the route. |
| D-323 | **No napi bridge.** | Nothing in the upstream test corpus calls these functions from JavaScript, and a bridge with no caller is scaffolding for its own sake. |

## Fuzz + bench

### Fuzz

**No campaign of its own**, and that is a scope statement rather than an omission: the differential
fuzzer drives *instances*, and this module exports free functions with no state to observe. What it
gets instead is continuous indirect coverage — `table8Popcount` is called once per word by
`BitSet.rank` and `BitVector.rank`, both of which are in their modules' op alphabets with weight 2,
and `rank` is compared against upstream after every operation of every generated program. Across the
four campaigns recorded in `fuzz/log.txt` for `bit-set` and `bit-vector` that is several million
`table8Popcount` calls, each of them differentially checked.

The other eight functions are **not** reachable from any fuzz grammar in this repo, because nothing
that is fuzzed calls them. They are covered by the 252-point Node cross-product and the exhaustive
sweeps above instead. Stated explicitly, because "we fuzzed the bit modules" would otherwise read as
covering this file, and it does not.

### Falsification (gate 6)

Gate 6 proper needs an original test file, and there is none. The nearest equivalent available is
recorded here for completeness: **`table8Popcount` is the only function the mocha suite reaches, and
sabotaging it does turn `bit-set` red** — `BitSet.rank` is asserted five times in
`test/bit-set.js:113`. That is the same evidence gate 6 asks for, borrowed from a host module.

The stronger check for this file is the cross-product test, and it is falsifiable by construction:
"fixing" `msb32` to return `-2147483648` for `0xFFFFFFFF`, or `criticalBit32Mask` to return an
unsigned mask, fails it immediately, because both wrong-looking answers are recorded there as
literals taken from Node.

### Bench

**Not applicable.** `bench/results.json` is keyed per unit, and this file is not one — its cost is
measured as part of `bit-set` and `bit-vector`, whose `rank` workloads are dominated by
`table8Popcount`. Those benchmarks are batched into the quiet pass along with everything else in
this series; see the two host documents.
