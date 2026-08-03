//! [`ModuleSpec`] for `static-interval-tree`.
//!
//! # A read-only module: every op is a query
//!
//! Unlike most units in this port, nothing in the public API mutates a built
//! tree -- `intervalsContainingPoint` and `intervalsOverlappingInterval` are
//! both pure queries. So `observe()`'s state never actually changes between
//! ops (`size`/`height`/`tree`/`augmentations` are fixed at construction), and
//! the entire signal this campaign carries is in each op's **result**: the
//! matched intervals, compared list for list, in the order each side's
//! traversal produced them. That is deliberate and it is enough -- a wrong
//! traversal order or a wrong prune decision shows up as soon as one op's
//! result differs, which `check_program` already treats as a divergence.
//!
//! `tree` and `augmentations` are still included in the observations, even
//! though they cannot change post-construction: comparing them once, right
//! after `init`, is what pins the BST *shape* (which node holds which
//! interval, and which subtree's max-end pointer is which) rather than only
//! its externally visible query behaviour.
//!
//! # Scope: no getters, and no zero-interval trees
//!
//! Getters are a construction-time JS callback this port resolves once (see
//! `mnemonist_core::structures::static_interval_tree`'s module docs) and are
//! not part of this grammar -- the default `[0]`/`[1]` access is what every
//! generated interval already is. Zero intervals is refused by this port
//! ([`Error::EmptyIntervals`], matching a verified Node crash), and
//! `ModuleSpec::construct` has no `Result` to report that through, so the
//! generator always produces at least one interval; the empty case is pinned
//! separately by `zero_intervals_is_refused_rather_than_silently_accepted` and
//! against real Node in the module docs.
//!
//! # Grammar
//!
//! Intervals are `[start, start + delta]` with `delta >= 0`, so every
//! generated interval is well-formed and closed, matching the upstream file's
//! own stated assumption. Starts repeat often across up to 40 intervals per
//! tree, which is what exercises the stable-sort tie-break the core's own
//! `ties_in_start_are_broken_by_original_insertion_order` test pins in
//! isolation -- here it is exercised by construction alone, on every case.
//! Query points and query intervals range wider than the constructed
//! intervals so that "contains nothing" is a common outcome, not just a hit.

use mnemonist_core::structures::static_interval_tree::{
    Error as CoreError, StaticIntervalTree as CoreTree,
};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Range a generated interval's `start` and query point/interval draw from.
const RANGE: i32 = 150;

pub struct StaticIntervalTreeSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/static-interval-tree.txt"
);

impl ModuleSpec for StaticIntervalTreeSpec {
    type Instance = CoreTree<(f64, f64)>;

    fn module(&self) -> &'static str {
        "static-interval-tree"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "height", "tree", "augmentations"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // At least one interval: zero is a verified upstream crash this port
        // refuses with an `Err` rather than modelling as a constructible
        // (and then queryable) tree -- see the module docs.
        prop::collection::vec((0i32..RANGE, 0i32..RANGE / 3), 1..40)
            .prop_map(|pairs| {
                let intervals: Vec<Value> = pairs
                    .into_iter()
                    .map(|(start, delta)| json!([start, start + delta]))
                    .collect();

                vec![Value::Array(intervals)]
            })
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            3 => (0i32..RANGE).prop_map(|point| {
                Op::new("intervalsContainingPoint", vec![json!(point)])
            }),
            2 => (0i32..RANGE, 0i32..RANGE).prop_map(|(a, b)| {
                let (start, end) = if a <= b { (a, b) } else { (b, a) };

                Op::new("intervalsOverlappingInterval", vec![json!([start, end])])
            }),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let bounds: Vec<(f64, f64)> = args[0]
            .as_array()
            .expect("ctor arg 0 is an array of [start, end] pairs")
            .iter()
            .map(|pair| {
                let pair = pair.as_array().expect("each interval is a 2-element array");

                (
                    pair[0].as_f64().expect("start is a JSON number"),
                    pair[1].as_f64().expect("end is a JSON number"),
                )
            })
            .collect();
        let intervals = bounds.clone();

        CoreTree::new(intervals, bounds)
            .expect("ctor_strategy always generates at least one interval")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "intervalsContainingPoint" => {
                let point = op.args[0].as_f64().expect("a generated point is a number");

                match instance.intervals_containing_point(point) {
                    Ok(matches) => intervals_json(&matches),
                    Err(error) => thrown(&error),
                }
            }
            "intervalsOverlappingInterval" => {
                let pair = op.args[0]
                    .as_array()
                    .expect("a generated query interval is a [start, end] pair");
                let start = pair[0].as_f64().expect("start is a JSON number");
                let end = pair[1].as_f64().expect("end is a JSON number");

                match instance.intervals_overlapping_interval(start, end) {
                    Ok(matches) => intervals_json(&matches),
                    Err(error) => thrown(&error),
                }
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.size(),
            "height": instance.height(),
            "tree": typed_array(instance.tree()),
            "augmentations": typed_array(instance.augmentations()),
        })
    }
}

/// A thrown error, in the shape `fuzz/oracle.js` reports one. Not reached by
/// any tree this campaign's grammar builds (see [`CoreError::StackOverflow`]'s
/// docs); kept for symmetry with every other module's `apply`.
fn thrown(error: &CoreError) -> Value {
    json!({ "$throw": error.to_string() })
}

/// Encode the matched `[start, end]` pairs exactly as `JSON.stringify` would
/// render the same JS numbers: every generated bound in this grammar is a
/// whole number carried in an `f64`, and `json!(f64)`'s default encoding
/// prints `118.0` where the oracle's `Array.from`/`JSON.stringify` prints
/// `118` -- a false divergence, not a real one. Same fix as `vector`'s
/// `number_json`, duplicated per module here to match the existing pattern
/// in this crate.
fn intervals_json(matches: &[(f64, f64)]) -> Value {
    Value::Array(
        matches
            .iter()
            .map(|(start, end)| json!([number_json(*start), number_json(*end)]))
            .collect(),
    )
}

fn number_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}

/// Encode a `PointerVec` exactly as the oracle encodes a JS typed array:
/// `{$typed: value.constructor.name, values: Array.from(value)}`.
fn typed_array(values: &PointerVec) -> Value {
    let name = match values.width() {
        PointerWidth::U8 => "Uint8Array",
        PointerWidth::U16 => "Uint16Array",
        PointerWidth::U32 => "Uint32Array",
    };

    json!({
        "$typed": name,
        "values": (0..values.len()).map(|i| values.get(i)).collect::<Vec<u32>>(),
    })
}
