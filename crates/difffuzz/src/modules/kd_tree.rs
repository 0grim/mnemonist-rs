//! [`ModuleSpec`] for `kd-tree`.
//!
//! # The real entry point is a static factory, not `new`
//!
//! `function KDTree(dimensions, build)`'s own second argument is an
//! already-built internal shape only `.from`/`.fromAxes` produce -- there is
//! no directly usable `new KDTree(...)` the way every other fuzzed module
//! has one. [`KdTreeSpec::static_factory`] names `"from"`, so `ctor` here is
//! `.from`'s own argument order (`[data, dimensions]`), and `fuzz/oracle.js`'s
//! `init` calls `Ctor.from(...)` instead of `new Ctor(...)`. See that file's
//! `staticFactory` handling and `crate::spec::ModuleSpec::static_factory`.
//!
//! # A read-only module: every op is a query
//!
//! As with `vp-tree`, there is no mutation after construction, so `axes`,
//! `labels`, `pivots`, `lefts` and `rights` are fixed and observed directly,
//! comparing the tree's exact shape on every generated program rather than
//! only the fixed fixture the core module's native tests pin.
//!
//! # Grammar: a dense integer grid, specifically for the splitting plane
//!
//! The sharp risk for this module is a query whose nearest neighbor lies
//! across a splitting plane from the query point -- precisely the case a
//! naive implementation gets wrong. Points are drawn
//! from a small 2D integer grid (`0..RANGE` per axis) with an *index* label,
//! so:
//!
//! * many points share the same coordinate on whichever axis the tree
//!   happens to split on, which is what makes a query land close to a
//!   splitting plane in the first place, rather than deep inside one half;
//! * many points sit at genuinely equal squared distance from a query
//!   (a dense grid guarantees repeated exact ties), forcing
//!   `kNearestNeighbors`' `[dist, visited++, pivot]` tie-break to actually
//!   run;
//! * query points are drawn from a *wider* window than the grid
//!   (`-RANGE/2..RANGE + RANGE/2`), so some queries land outside the point
//!   cloud entirely (favouring one clean subtree) and some land in the
//!   thick of it (forcing the "go the other way too" branch on nearly every
//!   visited node).
//!
//! `grammar_self_check` below measures both: how often the nearest point
//! found is NOT the point closest on the split axis alone (a naive
//! "just follow the split" implementation would get these wrong), and how
//! often ties actually occur.

use mnemonist_core::structures::kd_tree::KdTree as CoreTree;
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/kd-tree.txt"
);

/// Coordinates are drawn from `0..RANGE` per axis. Small and dense on
/// purpose -- see the module docs.
const RANGE: i64 = 12;
const DIMENSIONS: usize = 2;

pub struct KdTreeSpec;

impl ModuleSpec for KdTreeSpec {
    type Instance = CoreTree<i64>;

    fn module(&self) -> &'static str {
        "kd-tree"
    }

    fn static_factory(&self) -> Option<&'static str> {
        Some("from")
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "dimensions", "pivots", "lefts", "rights"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // `.from(iterable, dimensions)`'s own argument order.
        //
        // One point in eight is generated *shorter* than `DIMENSIONS`.
        // Upstream reads `row[1][d]` past the end, gets `undefined`, and
        // stores it into a `Float64Array` as `NaN` -- it does not throw, so
        // this is a shape a caller can really hand `.from`. Generating it is
        // what makes the grammar able to find a port that panics on it
        // instead: a panic at the bridge aborts the host process rather than
        // surfacing as a divergence, so an ungenerated short row is a hole
        // the campaign cannot report.
        //
        // NaN coordinates are worth reaching in their own right: every
        // comparison against NaN is false in both languages, so a sort or a
        // distance test that treats "not less than" as "greater or equal"
        // parts company from upstream exactly here.
        let point = (0..RANGE, 0..RANGE, 0..8u8).prop_map(|(x, y, short)| {
            if short == 0 {
                json!([x])
            } else {
                json!([x, y])
            }
        });

        prop::collection::vec(point, 2..60)
            .prop_map(|points| {
                let rows: Vec<Value> = points
                    .into_iter()
                    .enumerate()
                    .map(|(label, point)| json!([label as i64, point]))
                    .collect();

                vec![Value::Array(rows), json!(DIMENSIONS)]
            })
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let k = 1u32..8;
        // Wider than the grid on both ends: some queries land well outside
        // the point cloud, some deep inside it.
        let coordinate = -(RANGE / 2)..(RANGE + RANGE / 2);
        let query = (coordinate.clone(), coordinate);

        prop_oneof![
            3 => query.clone().prop_map(|(x, y)| Op::new("nearestNeighbor", vec![json!([x, y])])),
            3 => (k.clone(), query.clone()).prop_map(|(k, (x, y))| {
                Op::new("kNearestNeighbors", vec![json!(k), json!([x, y])])
            }),
            3 => (k, query).prop_map(|(k, (x, y))| {
                Op::new("linearKNearestNeighbors", vec![json!(k), json!([x, y])])
            }),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let rows: Vec<(i64, Vec<f64>)> = args[0]
            .as_array()
            .expect("ctor arg 0 is the rows array")
            .iter()
            .map(|row| {
                let row = row.as_array().expect("each row is a [label, point] pair");
                let label = row[0].as_i64().expect("label is a JSON integer");
                let point: Vec<f64> = row[1]
                    .as_array()
                    .expect("point is a coordinate array")
                    .iter()
                    .map(|c| c.as_f64().expect("each coordinate is a JSON number"))
                    .collect();

                (label, point)
            })
            .collect();

        CoreTree::from_rows(rows, DIMENSIONS).expect("a fixed positive DIMENSIONS never raises")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "nearestNeighbor" => {
                let query = query_point(&op.args[0]);

                match instance.nearest_neighbor(&query) {
                    Some(label) => json!(*label),
                    None => json!({"$undefined": true}),
                }
            }
            "kNearestNeighbors" => {
                let k = op.args[0].as_u64().expect("k is a JSON integer") as usize;
                let query = query_point(&op.args[1]);

                match instance.k_nearest_neighbors(k, &query) {
                    // A `None` is upstream's `undefined` *element*, which the
                    // oracle encodes the same way -- not a JSON `null`, and
                    // not an absence to be dropped.
                    Ok(labels) => Value::Array(
                        labels
                            .into_iter()
                            .map(|l| match l {
                                Some(label) => json!(label),
                                None => json!({"$undefined": true}),
                            })
                            .collect(),
                    ),
                    Err(message) => json!({"$throw": message}),
                }
            }
            "linearKNearestNeighbors" => {
                let k = op.args[0].as_u64().expect("k is a JSON integer") as usize;
                let query = query_point(&op.args[1]);

                match instance.linear_k_nearest_neighbors(k, &query) {
                    Ok(labels) => Value::Array(labels.into_iter().map(|l| json!(l)).collect()),
                    Err(message) => json!({"$throw": message}),
                }
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.size(),
            "dimensions": instance.dimensions(),
            "pivots": typed_array(instance.pivots()),
            "lefts": typed_array(instance.lefts()),
            "rights": typed_array(instance.rights()),
        })
    }
}

fn query_point(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .expect("a generated query point is a coordinate array")
        .iter()
        .map(|c| c.as_f64().expect("each coordinate is a JSON number"))
        .collect()
}

/// Encode a `PointerVec` exactly as the oracle encodes a JS typed array.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn squared(a: &[f64], b: &[f64]) -> f64 {
        a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
    }

    /// The measurement that makes the grammar above worth its complexity: how
    /// often does the
    /// point closest on the split axis ALONE differ from the true nearest
    /// neighbor? If it never does, the "go the other way too" branch is
    /// dead weight in this grammar.
    #[test]
    fn grammar_self_check_queries_land_across_the_splitting_plane() {
        let mut rng_state = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            rng_state
        };

        let points: Vec<(i64, Vec<f64>)> = (0..60)
            .map(|label| {
                let x = (next() % RANGE as u64) as f64;
                let y = (next() % RANGE as u64) as f64;
                (label, vec![x, y])
            })
            .collect();

        let tree = CoreTree::from_rows(points.clone(), DIMENSIONS)
            .expect("a fixed positive DIMENSIONS never raises");

        let mut across_plane = 0u32;
        let mut ties = 0u32;
        let mut total = 0u32;

        for _ in 0..500 {
            let qx = (next() % (RANGE * 2) as u64) as f64 - (RANGE / 2) as f64;
            let qy = (next() % (RANGE * 2) as u64) as f64 - (RANGE / 2) as f64;
            let query = vec![qx, qy];

            let true_best = points
                .iter()
                .map(|(label, p)| (squared(&query, p), *label))
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
                .unwrap();

            let count_at_best = points
                .iter()
                .filter(|(_, p)| squared(&query, p) == true_best.0)
                .count();
            if count_at_best > 1 {
                ties += 1;
            }

            // A naive "trust the first split" answer: whichever point is
            // closest purely along axis 0 (the root's split axis).
            let naive_best = points
                .iter()
                .min_by(|a, b| {
                    (a.1[0] - qx)
                        .abs()
                        .partial_cmp(&(b.1[0] - qx).abs())
                        .unwrap()
                })
                .unwrap();

            if (naive_best.1[0] - query[0]).abs() > 0.0
                && squared(&query, &naive_best.1) != true_best.0
            {
                across_plane += 1;
            }

            // Compared by DISTANCE, not by which point: a genuine tie (two
            // points equally close, both counted in `ties` below) means more
            // than one label is a correct nearest neighbor, and the tree's
            // deterministic traversal can legitimately land on a different
            // tied point than this brute-force scan's first-encountered one.
            let found = tree.nearest_neighbor(&query).copied();
            let found_distance = found.map(|label| squared(&query, &points[label as usize].1));

            assert_eq!(
                found_distance,
                Some(true_best.0),
                "tree's nearest neighbor must be exactly as close as the \
                 brute-force minimum for {query:?}"
            );

            total += 1;
        }

        eprintln!(
            "kd-tree grammar_self_check: {across_plane}/{total} queries had their true nearest \
             neighbor across a naive single-axis split; {ties}/{total} had a genuine distance tie"
        );

        assert!(
            across_plane > 0,
            "at least one of {total} queries must have its true nearest neighbor \
             on the far side of a naive single-axis guess"
        );
        assert!(
            ties > 0,
            "at least one of {total} queries must have a genuine distance tie \
             among the point grid"
        );
    }
}
