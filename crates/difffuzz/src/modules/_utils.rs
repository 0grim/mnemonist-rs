//! [`ModuleSpec`] for `_utils` — the fifth **free-function** module, and the
//! only one whose require-closure is five upstream files at once
//! (`typed-arrays`, `binary-search`, `hash-tables`, `iterables`, `merge`; see
//! `mnemonist_core::utils::merge`'s module docs and DESIGN.md §1.1's table).
//! Same mode `crate::modules::sort`/`crate::modules::set` introduced:
//! `Instance = ()`, no observable state, every op's arguments are echoed back
//! after the call because several of these functions mutate them (or, for
//! `merge`/`unionUnique`, because the whole comparison is a return value).
//!
//! # What the grammar reaches that `test/_utils.js` does not
//!
//! * **B-180.** `merge`/`unionUnique` with three-or-more arrays where at
//!   least one is empty and two-or-more are not — `test/_utils.js`'s own
//!   k-array cases never mix an empty array in, so gate 4 cannot see this.
//!   The array-count strategy below (2 to 5 arrays, each independently 0 to 5
//!   elements) produces that combination often, deliberately, rather than by
//!   luck.
//! * **Unsorted, duplicate-heavy and mixed-sign inputs to the two-array
//!   `merge`/`unionUnique`/`intersectionUnique`.** Upstream's own suite only
//!   ever feeds already-sorted arrays; the algorithms have no validation and
//!   run deterministically on anything, so an unsorted array is exactly the
//!   kind of "awkward value" this task exists to reach.
//! * **A real port defect, found on this campaign's first run (seed 42):**
//!   `unionUnique`'s two-array prefix loop had no dedup check upstream, and a
//!   first draft of this port added one anyway — *more correct* than
//!   upstream, and so a bug per CLAUDE.md. `unionUnique([-5, -5, 0], [-0.5])`
//!   caught it inside the first 300 generated cases. Fixed in
//!   `mnemonist_core::utils::merge::union_unique_two`; see that function's
//!   own test, `the_prefix_loop_does_not_deduplicate_an_already_non_unique_input`.
//! * **`lowerBoundIndices`'s own quirk** — `hi` defaults from `array.length`,
//!   not `indices.length` — generated directly by drawing indices whose
//!   length differs from the array's.
//! * **`concat` at more than one width and more than the two/three arrays the
//!   original test uses.**
//!
//! # Deliberately excluded
//!
//! * **`getPointerArray`/`getMinimalRepresentation`.** Both return a real JS
//!   *constructor*, and `fuzz/oracle.js`'s `encode` has no case for a
//!   function — it falls through unmodified, and `JSON.stringify` then drops
//!   the property outright. There is nothing to compare through this
//!   protocol. Both are covered instead by native Rust tests pinned against
//!   Node 24.18.1 output (`crates/mnemonist-core/src/utils/typed_arrays.rs`)
//!   and by the real bridge integration run (`tests/run.sh test/_utils.js`).
//! * **The three `WithComparator` variants.** The comparator is a JavaScript
//!   function called from inside the search loop; comparing against it here
//!   would mean rendering an equivalent comparator as JS source for every
//!   generated case, which is real re-entrant-callback machinery this unit
//!   was scoped to avoid. Covered instead by
//!   `crate::utils::binary_search`'s own exhaustive native tests (an
//!   exhaustive agreement-with-a-linear-scan check over every short sorted
//!   array, plus the two comparator-argument-order cases).
//! * **`iterables`.** No core-side pure function exists to fuzz — see
//!   `docs/modules/utils-iterables.md`, "Not fuzzed directly, and that is a
//!   real gap rather than an omission."
//! * **All of `hash-tables.js`.** Its two exports, `hashes` and
//!   `linearProbing`, are both plain objects (`exports.hashes = {jenkinsInt32:
//!   ...}`, `exports.linearProbing = {get, has, set}`) — upstream never
//!   exports a bare, top-level function at all. `fuzz/oracle.js`'s
//!   free-function protocol dispatches with one property lookup,
//!   `instance[request.name](...)` (see its own docs on the `functions`
//!   mode); there is no `instance.linearProbingSet`, only
//!   `instance.linearProbing.set`, so any op name here would resolve to
//!   "not a function" on the JS side — confirmed by this campaign's own
//!   first attempt, before the ops were removed. Extending the protocol to
//!   walk a dotted path would be a structural change to `fuzz/oracle.js`,
//!   which CLAUDE.md's shared-file rule reserves for additive edits, not a
//!   change one module's fuzz spec should make unilaterally. Covered instead
//!   by `crate::utils::hash_tables`'s own extensive native tests (pinned
//!   against Node for `jenkinsInt32`, and covering every edge — the key `0`,
//!   a full table, every power-of-two size, a non-power-of-two size — the
//!   original suite's one fixed example does not reach) and by the real
//!   bridge integration run.

use mnemonist_core::utils::merge as core_merge;
use mnemonist_core::utils::{binary_search as core_search, typed_arrays as core_typed};
use proptest::collection;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

pub struct UtilsSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/_utils.txt"
);

/// The whole require-closure of `test/_utils.js` — see DESIGN.md §1.1.
const FILES: &[&str] = &[
    "utils/typed-arrays",
    "utils/binary-search",
    "utils/hash-tables",
    "utils/iterables",
    "utils/merge",
];

/// The values a generated `merge`/`unionUnique`/`intersectionUnique` array
/// draws from. Small and repetitive so duplicates and overlaps between
/// arrays are frequent rather than incidental — that is what separates
/// `merge` from `unionUnique` and what makes `intersectionUnique` return
/// something other than `[]`.
const NUMBER_POOL: usize = 8;

fn number_at(index: usize) -> Value {
    match index {
        0 => json!(-5),
        1 => json!(-1),
        2 => json!(0),
        3 => json!(1),
        4 => json!(2),
        5 => json!(3),
        6 => json!(-0.5),
        _ => json!({"$nan": true}),
    }
}

impl ModuleSpec for UtilsSpec {
    type Instance = ();

    fn module(&self) -> &'static str {
        "_utils"
    }

    fn functions(&self) -> &'static [&'static str] {
        FILES
    }

    /// None. See [`ModuleSpec::functions`].
    fn observations(&self) -> &'static [&'static str] {
        &[]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            4 => arrays_op("merge"),
            4 => arrays_op("unionUnique"),
            3 => arrays_op("intersectionUnique"),
            3 => search_op("search"),
            3 => search_op("lowerBound"),
            3 => search_op("upperBound"),
            2 => lower_bound_indices_op(),
            2 => concat_op(),
        ]
        .boxed()
    }

    /// Shorter than the default: every op carries its own subject, so a long
    /// program buys interaction cost (one oracle round trip each) without
    /// buying interaction value.
    fn program_len(&self) -> std::ops::Range<usize> {
        1..80
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {}

    fn apply(&self, _instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "merge" => apply_arrays(op, |arrays| match arrays.len() {
                2 => Ok(core_merge::merge_two(&arrays[0], &arrays[1])),
                _ => core_merge::merge_k(&as_slices(&arrays)),
            }),
            "unionUnique" => apply_arrays(op, |arrays| match arrays.len() {
                2 => Ok(core_merge::union_unique_two(&arrays[0], &arrays[1])),
                _ => core_merge::union_unique_k(&as_slices(&arrays)),
            }),
            "intersectionUnique" => apply_arrays(op, |arrays| {
                Ok(match arrays.len() {
                    2 => core_merge::intersection_unique_two(&arrays[0], &arrays[1]),
                    _ => core_merge::intersection_unique_k(&as_slices(&arrays)),
                })
            }),

            "search" => apply_search(op, core_search::search),
            "lowerBound" => apply_search(op, |a, v, lo, hi| {
                core_search::lower_bound(a, v, lo, hi) as isize
            }),
            "upperBound" => apply_search(op, |a, v, lo, hi| {
                core_search::upper_bound(a, v, lo, hi) as isize
            }),
            "lowerBoundIndices" => apply_lower_bound_indices(op),

            "concat" => apply_concat(op),

            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    /// Nothing. See [`ModuleSpec::functions`].
    fn observe(&self, _instance: &mut Self::Instance) -> Value {
        json!({})
    }
}

// ------------------------------------------------------------- merge/union/intersection

/// 2 to 5 number arrays, each independently 0 to 5 elements. The 0-length
/// case is not filtered out — it is the whole point (B-180).
///
/// # `NaN` is excluded from three-or-more-array groups
///
/// Found by this very campaign's first run, seed 42: `merge([-5], [NaN],
/// [-1])` diverges (`port: [-5, NaN, -1]`, `upstream: [-1, NaN, -5]`) even
/// though `filtered.len() == original_len == 3` and B-180 does not apply.
/// The cause is [`k_way_scan`](mnemonist_core::utils::merge)'s "current
/// minimum across live arrays" pick, which this file's docs already flagged
/// as a linear scan standing in for upstream's `FibonacciHeap` — every
/// comparison against `NaN` is `false` in both directions, so "which array
/// has the smaller head" stops being well-defined the moment `NaN`
/// participates in a three-or-more-way pick, and the two implementations'
/// arbitrary tie-breaks disagree about *which real value* comes out first.
/// This is not B-180 and not a stale-length bug: it reproduces on plain,
/// distinctly-sized arrays with no empties at all.
///
/// The two-array path (`merge_two`/`union_unique_two`) has no such pick — it
/// is a direct pointer-driven merge with the exact `<=`/`<` comparisons
/// upstream's own two-array functions use, verified against Node to still
/// agree with `NaN` present (`merge([-5, NaN], [-1, 3])` matches on both
/// sides) — so `NaN` stays in the pool there. See
/// `docs/modules/_utils.md`'s "Deliberate divergences" for the D-entry this
/// exclusion documents rather than silently works around.
fn arrays_op(name: &'static str) -> BoxedStrategy<Op> {
    prop_oneof![2 => two_arrays_op(name), 3 => k_way_arrays_op(name)].boxed()
}

/// Exactly two arrays, small and repetitive pool, `NaN` included. Neither
/// `merge_two` nor `union_unique_two` picks a "minimum among several ties" —
/// both are a direct two-pointer walk with the exact `<=`/`<` upstream uses,
/// verified to still agree with upstream on `NaN` — so there is no tie-break
/// ambiguity here at all, and duplicates/`NaN` are safe to generate freely.
fn two_arrays_op(name: &'static str) -> BoxedStrategy<Op> {
    collection::vec(collection::vec(0usize..NUMBER_POOL, 0..6), 2..=2)
        .prop_map(move |arrays| {
            let args: Vec<Value> = arrays
                .into_iter()
                .map(|indices| Value::Array(indices.into_iter().map(number_at).collect()))
                .collect();

            Op::new(name, args)
        })
        .boxed()
}

/// Three to five arrays, values **globally distinct across the whole group**
/// (never mind repetitive) — a deliberate restriction, and the reason is a
/// second real finding from this campaign's own first runs, not a
/// convenience:
///
/// `unionUnique([3], [2, -5], [2])` disagreed (`port: [2,-5,2,3]`, `upstream:
/// [2,-5,3]`), and — sharper still — `merge([3], [2, -5], [2])` disagrees on
/// **order alone**: upstream is `[2, 2, -5, 3]`, this port `[2, -5, 2, 3]`.
/// The cause is [`k_way_scan`](mnemonist_core::utils::merge)'s tie-break: it
/// keeps the earliest array on a value tie, where upstream's `FibonacciHeap`
/// updates its `min` pointer with `<=`, favouring the **most recently
/// pushed** node — and after consolidation restructures the tree across
/// pops, which node that ends up being depends on the heap's internal
/// degree-bucket merging (`consolidate`, `fibonacci-heap.js`), not on
/// insertion order alone. Reproducing that exactly means porting
/// `fibonacci-heap.js` itself (T2 tier, ~115 LOC, not part of this unit's
/// require-closure — DESIGN.md's own tier table). Recorded as a stated
/// divergence rather than chased: `docs/modules/_utils.md`, "Deliberate
/// divergences".
///
/// What stays true regardless of tie-break policy: when no two currently-live
/// array heads are ever equal, "the global minimum" is unambiguous and every
/// correct implementation — heap or linear scan — extracts the identical
/// sequence. So distinct values sidestep the gap structurally rather than
/// avoiding the input shape that would reveal it; B-180 does not depend on
/// value content at all (it is a pure index-count bug) and stays fully
/// reachable through this strategy's own 0-length arrays.
fn k_way_arrays_op(name: &'static str) -> BoxedStrategy<Op> {
    collection::vec(0usize..6, 3..6)
        .prop_flat_map(|lengths| {
            let total: usize = lengths.iter().sum();

            (Just(lengths), distinct_values(total))
        })
        .prop_map(move |(lengths, mut values)| {
            let args: Vec<Value> = lengths
                .into_iter()
                .map(|length| {
                    Value::Array(values.drain(0..length).map(|value| json!(value)).collect())
                })
                .collect();

            Op::new(name, args)
        })
        .boxed()
}

/// `total` pairwise-distinct `i32`s. The pool (20,000 values) is wide enough
/// against the small counts this grammar ever asks for (at most 25) that
/// `prop_filter`'s rejection sampling essentially never has to retry.
fn distinct_values(total: usize) -> BoxedStrategy<Vec<i32>> {
    collection::vec(-10_000i32..10_000, total..=total)
        .prop_filter("values must be pairwise distinct", |values| {
            let unique: std::collections::HashSet<i32> = values.iter().copied().collect();

            unique.len() == values.len()
        })
        .boxed()
}

fn apply_arrays(
    op: &Op,
    run: impl FnOnce(Vec<Vec<f64>>) -> Result<Vec<f64>, core_merge::KWayError>,
) -> Value {
    let arrays: Vec<Vec<f64>> = op.args.iter().map(numbers).collect();
    let echoed: Vec<Value> = op.args.clone();

    let result = match run(arrays) {
        Ok(values) => Value::Array(values.into_iter().map(number_json).collect()),
        Err(core_merge::KWayError::StaleLengthMismatch) => {
            json!({"$throw": core_merge::STALE_LENGTH_TYPE_ERROR})
        }
    };

    returned(result, echoed)
}

fn as_slices(arrays: &[Vec<f64>]) -> Vec<&[f64]> {
    arrays.iter().map(Vec::as_slice).collect()
}

// ------------------------------------------------------------------- binary-search

/// `(array, value, lo, hi)`. `lo`/`hi` are independently a small integer or
/// `undefined`, and neither is clamped against the other or against
/// `array.length` — an out-of-range or inverted window is exactly the
/// "awkward value" this module's own docs describe as unpinned by upstream's
/// suite.
fn search_op(name: &'static str) -> BoxedStrategy<Op> {
    collection::vec(0usize..NUMBER_POOL, 0..10)
        .prop_flat_map(|indices| {
            let bound = indices.len() as u32 + 3;

            (
                Just(indices),
                0usize..NUMBER_POOL,
                optional_bound(bound),
                optional_bound(bound),
            )
        })
        .prop_map(move |(indices, value, lo, hi)| {
            let array: Vec<Value> = indices.into_iter().map(number_at).collect();

            Op::new(name, vec![Value::Array(array), number_at(value), lo, hi])
        })
        .boxed()
}

fn optional_bound(bound: u32) -> BoxedStrategy<Value> {
    prop_oneof![
        1 => Just(json!({"$undefined": true})),
        3 => (0..bound).prop_map(|value| json!(value)),
    ]
    .boxed()
}

/// `search` returns `-1` for absent; `lowerBound`/`upperBound` always return
/// a non-negative index. Both encode the same way — a plain JSON integer —
/// so one function serves all three callers.
fn apply_search(
    op: &Op,
    search: impl FnOnce(&[f64], &f64, Option<usize>, Option<usize>) -> isize,
) -> Value {
    let array = numbers(&op.args[0]);
    let value = number(&op.args[1]);
    let lo = optional_usize(&op.args[2]);
    let hi = optional_usize(&op.args[3]);

    let result = search(&array, &value, lo, hi);

    returned(json!(result), op.args.clone())
}

/// `(array, indices, value, lo, hi)`. `indices` is drawn independently of
/// `array`'s length, including past the end of it — the shape that reaches
/// `lowerBoundIndices`'s own `hi`-defaults-from-`array` quirk
/// (`crate::utils::binary_search`'s module docs).
fn lower_bound_indices_op() -> BoxedStrategy<Op> {
    (
        collection::vec(0usize..NUMBER_POOL, 0..10),
        collection::vec(0usize..14, 0..10),
    )
        .prop_flat_map(|(values, positions)| {
            let bound = values.len().max(positions.len()) as u32 + 3;

            (
                Just(values),
                Just(positions),
                0usize..NUMBER_POOL,
                optional_bound(bound),
                optional_bound(bound),
            )
        })
        .prop_map(|(values, positions, value, lo, hi)| {
            let array: Vec<Value> = values.into_iter().map(number_at).collect();
            let indices: Vec<Value> = positions.into_iter().map(|index| json!(index)).collect();

            Op::new(
                "lowerBoundIndices",
                vec![
                    Value::Array(array),
                    Value::Array(indices),
                    number_at(value),
                    lo,
                    hi,
                ],
            )
        })
        .boxed()
}

fn apply_lower_bound_indices(op: &Op) -> Value {
    let array = numbers(&op.args[0]);
    let indices: Vec<usize> = op.args[1]
        .as_array()
        .expect("indices argument is a JSON array")
        .iter()
        .map(|value| value.as_u64().expect("indices are non-negative integers") as usize)
        .collect();
    let value = number(&op.args[2]);
    let lo = optional_usize(&op.args[3]);
    let hi = optional_usize(&op.args[4]);

    let result = core_search::lower_bound_indices(&array, &indices, &value, lo, hi);

    returned(json!(result), op.args.clone())
}

// ------------------------------------------------------------------------- typed-arrays

/// 1 to 5 `Uint8Array`s, each 0 to 8 bytes. `$typed`-wrapped because
/// `concat`'s bridge (and upstream itself, via `arguments[0].constructor`)
/// needs a real typed array — a plain `Array` has no `.set`, which a plain
/// JSON array would decode to.
fn concat_op() -> BoxedStrategy<Op> {
    collection::vec(collection::vec(0u8..=255, 0..9), 1..6)
        .prop_map(|arrays| {
            let args: Vec<Value> = arrays
                .into_iter()
                .map(|bytes| json!({"$typed": "Uint8Array", "values": bytes}))
                .collect();

            Op::new("concat", args)
        })
        .boxed()
}

fn apply_concat(op: &Op) -> Value {
    let arrays: Vec<Vec<u8>> = op
        .args
        .iter()
        .map(|argument| {
            argument["values"]
                .as_array()
                .expect("a $typed argument carries its values")
                .iter()
                .map(|value| value.as_u64().expect("bytes are small integers") as u8)
                .collect()
        })
        .collect();
    let slices: Vec<&[u8]> = arrays.iter().map(Vec::as_slice).collect();

    let result = core_typed::concat(&slices);

    returned(
        json!({"$typed": "Uint8Array", "values": result}),
        op.args.clone(),
    )
}

// ------------------------------------------------------------------------------ shared

/// The envelope `fuzz/oracle.js` wraps every free-function result in.
fn returned(result: Value, args: Vec<Value>) -> Value {
    json!({"$return": result, "$args": args})
}

fn numbers(argument: &Value) -> Vec<f64> {
    argument
        .as_array()
        .expect("an array argument is a JSON array")
        .iter()
        .map(number)
        .collect()
}

/// One wire value as the JavaScript number it stands for.
fn number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().expect("JSON numbers are doubles"),
        Value::Object(fields) if fields.contains_key("$nan") => f64::NAN,
        other => panic!("`{other}` is not a number this grammar generates"),
    }
}

/// `undefined` (an omitted bound) as `None`, an integer as `Some`.
fn optional_usize(value: &Value) -> Option<usize> {
    if value.get("$undefined").and_then(Value::as_bool) == Some(true) {
        return None;
    }

    Some(value.as_u64().expect("a bound is an integer or undefined") as usize)
}

/// A JavaScript number, encoded as `JSON.stringify` would encode it.
///
/// Duplicated from `crate::modules::sort`, which duplicated it from
/// `crate::modules::default_map` — see either's module docs for why this is
/// copied rather than shared (CLAUDE.md: a shared helper is a merge conflict
/// three worktrees would fight over).
fn number_json(value: f64) -> Value {
    if value.is_nan() {
        return json!({"$nan": true});
    }

    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}
