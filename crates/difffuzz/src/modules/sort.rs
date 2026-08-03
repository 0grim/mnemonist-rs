//! [`ModuleSpec`] for `sort` — the first **free-function** module.
//!
//! # How a module with no instance is fuzzed
//!
//! Every spec before this one describes a structure: build it, poke it, read
//! its state. `sort/insertion.js`, `sort/quick.js` and `utils/typed-arrays.js`
//! export bare functions and hold nothing between calls, so all three of those
//! steps are empty. [`ModuleSpec::functions`] declares the shape:
//! `Instance = ()`, `observations()` is `&[]`, `ctor_strategy` is the empty
//! vector, and `observe` is `{}` forever.
//!
//! That would leave the comparison resting entirely on return values, which is
//! not enough for a family of functions whose whole job is to mutate their
//! arguments. So `fuzz/oracle.js` re-encodes an op's **arguments** after the
//! call and compares those too — generically, for every free-function module,
//! rather than through a per-function list of which parameters are
//! out-parameters. A program is then a sequence of independent calls, and each
//! op carries its own subject.
//!
//! # What the grammar reaches that `test/sort.js` does not
//!
//! * **Every window of every array**, rather than the six windows of one
//!   fixed array. `lo` and `hi` are generated against the subject's length, so
//!   empty, single-element and full-width windows all occur.
//! * **Indices that point past the end of `array`.** Upstream reads
//!   `undefined` there and every comparison against it is false; that is the
//!   whole reason `mnemonist_core::sort` compares `Option<&T>`, and this is
//!   the only place the behaviour is checked against the real thing. Index
//!   values are drawn from a range wider than the array.
//! * **`NaN`, and fractional values.** `test/sort.js` sorts integers and
//!   `Math.random()` output. `NaN` loses every comparison in both languages,
//!   so it pins its neighbours in a way an ordering-based port would get
//!   wrong.
//! * **All three pointer widths.** Upstream's suite calls `indices` once, with
//!   11, so it only ever builds a `Uint8Array`.
//! * **`indices`' own coercions.** Its two uses of `length` coerce
//!   differently — see [`mnemonist_core::utils::typed_arrays::indices`] — and
//!   the lengths below sit on both sides of every width boundary, plus the
//!   fractional, negative, `NaN` and too-large cases that throw.
//!
//! # Deliberately excluded
//!
//! * **Non-numeric elements.** Upstream compares them through `valueOf`, which
//!   is bridge tier T2 and a stated divergence (`docs/modules/sort.md`); the
//!   port refuses them, so generating one would produce a divergence the doc
//!   already records rather than a finding.
//! * **Windows past the end.** Same reason: upstream reads `undefined` and
//!   writes into holes, the port refuses, and the divergence is documented.
//! * **The re-entrancy bugs BUG-SORT-1 and BUG-SORT-2.** They need an element whose
//!   comparison calls back into JavaScript, which needs an object element,
//!   which the previous point excludes. Verified by hand against Node instead
//!   — see `docs/modules/sort.md`.
//! * **`getPointerArray` and the rest of `utils/typed-arrays.js`.** Only
//!   `indices` is ported (the file lands helper by helper as modules reach
//!   them), so the others have nothing to compare against.

use mnemonist_core::sort::{insertion, quick};
use mnemonist_core::utils::typed_arrays::{
    self, IndicesError, PointerVec, PointerWidth, INVALID_TYPED_ARRAY_LENGTH,
    POINTER_ARRAY_TOO_LARGE,
};
use proptest::collection;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

pub struct SortSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proptest-regressions/sort.txt");

/// Upstream files whose exports make up this unit — its whole require-closure.
const FILES: &[&str] = &["sort/insertion", "sort/quick", "utils/typed-arrays"];

/// The values a generated array is drawn from, by index.
///
/// Small and repetitive on purpose: duplicates are what separate a stable sort
/// from an unstable one, and the two algorithms here disagree about them.
/// `NaN` is in the pool because it loses every comparison, so it neither sinks
/// nor lets anything sink past it — behaviour an ordering-based port would get
/// wrong and `test/sort.js` never reaches.
const VALUE_POOL: usize = 11;

fn value_at(index: usize) -> Value {
    match index {
        0 => json!(-3),
        1 => json!(-1),
        2 => json!(0),
        3 => json!(1),
        4 => json!(2),
        5 => json!(3),
        6 => json!(7),
        7 => json!(0.5),
        8 => json!(-0.5),
        9 => json!(2.5),
        _ => json!({"$nan": true}),
    }
}

/// Lengths for `indices`, including every one that throws.
///
/// Both sides of each width boundary, both sides of `ToIndex`'s truncation,
/// and the four refusals. Kept as literal `Value`s rather than computed,
/// because `255` and `255.0` are the same JavaScript number but *different*
/// `serde_json::Value`s, and emitting the wrong one is a false divergence that
/// says nothing about the port.
const LENGTH_POOL: usize = 16;

fn length_at(index: usize) -> Value {
    match index {
        0 => json!(0),
        1 => json!(1),
        2 => json!(11),
        3 => json!(255),
        4 => json!(256),
        5 => json!(257),
        6 => json!(65535),
        7 => json!(65536),
        8 => json!(65537),
        9 => json!(3.5),
        10 => json!(255.5),
        11 => json!(256.5),
        12 => json!(-0.5),
        13 => json!(-1),
        14 => json!(-3.5),
        _ => json!({"$nan": true}),
    }
}

impl ModuleSpec for SortSpec {
    type Instance = ();

    fn module(&self) -> &'static str {
        "sort"
    }

    fn functions(&self) -> &'static [&'static str] {
        FILES
    }

    /// None. A module of free functions holds nothing between calls, so there
    /// is nothing to observe; the arguments are compared instead.
    fn observations(&self) -> &'static [&'static str] {
        &[]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            3 => value_sort("inplaceInsertionSort"),
            3 => value_sort("inplaceQuickSort"),
            3 => indices_sort("inplaceInsertionSortIndices"),
            3 => indices_sort("inplaceQuickSortIndices"),
            2 => (0usize..LENGTH_POOL)
                .prop_map(|index| Op::new("indices", vec![length_at(index)])),
        ]
        .boxed()
    }

    /// Shorter than the default, because every op here carries its own subject
    /// rather than accumulating state in one. Two hundred independent calls
    /// buy no more interaction than eighty, and cost four round trips each.
    fn program_len(&self) -> std::ops::Range<usize> {
        1..80
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {}

    fn apply(&self, _instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "inplaceInsertionSort" => apply_value_sort(op, |values, lo, hi| {
                insertion::inplace_insertion_sort(values, lo, hi);
            }),
            "inplaceQuickSort" => apply_value_sort(op, |values, lo, hi| {
                quick::inplace_quick_sort(values, lo, hi);
            }),
            "inplaceInsertionSortIndices" => apply_indices_sort(op, |values, positions, lo, hi| {
                insertion::inplace_insertion_sort_indices(values, positions, lo, hi);
            }),
            "inplaceQuickSortIndices" => apply_indices_sort(op, |values, positions, lo, hi| {
                quick::inplace_quick_sort_indices(values, positions, lo, hi);
            }),
            "indices" => {
                let length = number(&op.args[0]);
                let outcome = match typed_arrays::indices(length) {
                    Ok(array) => typed_json(&array),
                    Err(IndicesError::TooLarge) => json!({"$throw": POINTER_ARRAY_TOO_LARGE}),
                    Err(IndicesError::InvalidLength(value)) => {
                        json!({"$throw": format!("{INVALID_TYPED_ARRAY_LENGTH}: {value}")})
                    }
                };

                returned(outcome, vec![op.args[0].clone()])
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    /// Nothing. See [`ModuleSpec::functions`].
    fn observe(&self, _instance: &mut Self::Instance) -> Value {
        json!({})
    }
}

/// `(array, lo, hi)`, with the window generated against the array's length so
/// every op is in range.
fn value_sort(name: &'static str) -> BoxedStrategy<Op> {
    collection::vec(0usize..VALUE_POOL, 0..25)
        .prop_flat_map(|indices| {
            let length = indices.len();

            (Just(indices), window(length))
        })
        .prop_map(move |(indices, (lo, hi))| {
            let array: Vec<Value> = indices.into_iter().map(value_at).collect();

            Op::new(name, vec![Value::Array(array), json!(lo), json!(hi)])
        })
        .boxed()
}

/// `(array, indices, lo, hi)`.
///
/// The index array's length is independent of the value array's, and its
/// entries are drawn from a range four wider than the values, so a good share
/// of them point past the end of `array`.
fn indices_sort(name: &'static str) -> BoxedStrategy<Op> {
    (
        collection::vec(0usize..VALUE_POOL, 0..25),
        collection::vec(0u32..29, 0..25),
        any::<bool>(),
    )
        .prop_flat_map(|(values, positions, wide)| {
            let length = positions.len();

            (Just(values), Just(positions), Just(wide), window(length))
        })
        .prop_map(move |(values, positions, wide, (lo, hi))| {
            let array: Vec<Value> = values.into_iter().map(value_at).collect();
            let width = if wide { "Uint16Array" } else { "Uint8Array" };

            Op::new(
                name,
                vec![
                    Value::Array(array),
                    json!({"$typed": width, "values": positions}),
                    json!(lo),
                    json!(hi),
                ],
            )
        })
        .boxed()
}

/// `lo <= hi <= length`, shrinking towards the empty window at 0.
fn window(length: usize) -> BoxedStrategy<(usize, usize)> {
    (0..=length)
        .prop_flat_map(move |lo| (Just(lo), lo..=length))
        .boxed()
}

fn apply_value_sort(op: &Op, sort: impl FnOnce(&mut [f64], usize, usize)) -> Value {
    let mut values = numbers(&op.args[0]);
    let lo = number(&op.args[1]) as usize;
    let hi = number(&op.args[2]) as usize;

    sort(&mut values, lo, hi);

    let sorted = Value::Array(values.into_iter().map(number_json).collect());

    // Upstream returns the array it was given, so the return value and the
    // first argument are the same object and must render identically.
    returned(
        sorted.clone(),
        vec![sorted, op.args[1].clone(), op.args[2].clone()],
    )
}

fn apply_indices_sort(op: &Op, sort: impl FnOnce(&[f64], &mut PointerVec, usize, usize)) -> Value {
    let values = numbers(&op.args[0]);
    let mut positions = typed_from_json(&op.args[1]);
    let lo = number(&op.args[2]) as usize;
    let hi = number(&op.args[3]) as usize;

    sort(&values, &mut positions, lo, hi);

    let sorted = typed_json(&positions);

    returned(
        sorted.clone(),
        vec![
            op.args[0].clone(),
            sorted,
            op.args[2].clone(),
            op.args[3].clone(),
        ],
    )
}

/// The envelope `fuzz/oracle.js` wraps every free-function result in.
fn returned(result: Value, args: Vec<Value>) -> Value {
    json!({"$return": result, "$args": args})
}

/// A typed array, in the shape the oracle's `encode` renders one.
fn typed_json(array: &PointerVec) -> Value {
    let name = match array.width() {
        PointerWidth::U8 => "Uint8Array",
        PointerWidth::U16 => "Uint16Array",
        PointerWidth::U32 => "Uint32Array",
    };
    let values: Vec<u32> = (0..array.len()).map(|slot| array.get(slot)).collect();

    json!({"$typed": name, "values": values})
}

/// The inverse: rebuild the `PointerVec` an op argument stands for.
fn typed_from_json(argument: &Value) -> PointerVec {
    let width = match argument["$typed"].as_str() {
        Some("Uint8Array") => PointerWidth::U8,
        Some("Uint16Array") => PointerWidth::U16,
        Some("Uint32Array") => PointerWidth::U32,
        other => panic!("`{other:?}` is not a width this grammar generates"),
    };
    let values = argument["values"]
        .as_array()
        .expect("a typed argument carries its values");

    let mut array = PointerVec::zeroed(width, values.len());

    for (slot, value) in values.iter().enumerate() {
        array.set(
            slot,
            value.as_u64().expect("index values are integers") as u32,
        );
    }

    array
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

/// A JavaScript number, encoded as `JSON.stringify` would encode it.
///
/// JavaScript has one number type and JSON has one number syntax, so `1`
/// serialises as `1` and never as `1.0`. serde_json *does* distinguish the two
/// and compares them unequal, so a Rust side emitting `json!(1.0)` disagrees
/// with the oracle on every integral value — a false divergence that says
/// nothing about the port. Duplicated from `crate::modules::default_map`,
/// which found it the hard way on its first campaign; shared code would be
/// tidier and would put a merge conflict in a file three worktrees edit.
fn number_json(value: f64) -> Value {
    if value.is_nan() {
        return json!({"$nan": true});
    }

    // 2^53 is where a double stops representing consecutive integers, and also
    // where `JSON.stringify` stops printing them plainly.
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}
