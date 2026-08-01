//! [`ModuleSpec`] for `vp-tree`.
//!
//! # A read-only module: every op is a query
//!
//! Like `static-interval-tree`, nothing in the public API mutates a built
//! tree -- there is no `add`. So `nodes`/`lefts`/`rights`/`mus` are fixed at
//! construction, and this campaign observes them directly on every generated
//! program: a byte-for-byte comparison of the tree's own *shape*, not just
//! its query answers, on every random input rather than only the two fixed
//! fixtures the core module's native tests pin. `nearestNeighbors`/
//! `neighbors` are the only ops, and their **order** is part of what is
//! compared (a `deepStrictEqual`-shaped JSON array, not a set), which is
//! what a wrong tie-break or a wrong traversal order shows up as.
//!
//! # Grammar: a narrow integer range with a real metric
//!
//! Items are small integers in `0..RANGE` and `distance(a, b) = |a - b|` --
//! the same real, trivially-mirrored metric `bk-tree`'s own campaign uses
//! (`bkAbsDiff` in `fuzz/oracle.js`, reused rather than re-added). The
//! narrowness is deliberate and is the answer to the risk CLAUDE.md names for
//! this module by name: a wide item range would make every distance from a
//! vantage point distinct, so the median split (`mus`) would never have to
//! choose between two *equal* distances and the "genuine near-ties" this
//! module's brief asks for would never occur. With `RANGE = 24` and up to 80
//! items, repeated collisions on the same distance from any given node are
//! constant, which is the only way the quicksort's tie-break (and therefore
//! the tree's exact shape) is ever exercised at all.
//!
//! `neighbors`' radius is drawn across the **whole** possible distance span,
//! `0..=RANGE`, specifically so that a run's radii include both extremes:
//! zero (prunes hardest -- only the query's own value, if present, can
//! match) and larger than every possible distance (no pruning is possible at
//! all, everything is a hit). See `grammar_self_check` below for the
//! measured split.

use mnemonist_core::structures::vp_tree::{Neighbor, VpTree as CoreTree};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/vp-tree.txt"
);

/// Items and queries are drawn from `0..RANGE`. See the module docs for why
/// this must stay narrow.
const RANGE: i64 = 24;

fn dist(a: &i64, b: &i64) -> f64 {
    (a - b).abs() as f64
}

pub struct VpTreeSpec;

impl ModuleSpec for VpTreeSpec {
    type Instance = CoreTree<i64>;

    fn module(&self) -> &'static str {
        "vp-tree"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "nodes", "lefts", "rights", "mus"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // Upstream's own constructor order: `new VPTree(distance, items)`.
        prop::collection::vec(0..RANGE, 1..80)
            .prop_map(|items| {
                vec![
                    json!({"$factory": "bkAbsDiff"}),
                    Value::Array(items.into_iter().map(Value::from).collect()),
                ]
            })
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let k = 1u32..8;
        let radius = 0i64..=RANGE;

        prop_oneof![
            5 => (k, 0..RANGE).prop_map(|(k, q)| Op::new("nearestNeighbors", vec![json!(k), json!(q)])),
            5 => (radius, 0..RANGE).prop_map(|(r, q)| Op::new("neighbors", vec![json!(r), json!(q)])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let items: Vec<i64> = args[1]
            .as_array()
            .expect("ctor arg 1 is the items array")
            .iter()
            .map(|v| v.as_i64().expect("every item is a JSON integer"))
            .collect();

        CoreTree::new(items, dist)
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "nearestNeighbors" => {
                let k = op.args[0].as_u64().expect("k is a JSON integer") as usize;
                let query = op.args[1].as_i64().expect("query is a JSON integer");

                neighbors_json(&instance.nearest_neighbors(k, &query, dist))
            }
            "neighbors" => {
                let radius = op.args[0].as_i64().expect("radius is a JSON integer") as f64;
                let query = op.args[1].as_i64().expect("query is a JSON integer");

                neighbors_json(&instance.neighbors(radius, &query, dist))
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.size(),
            "nodes": typed_array(instance.nodes()),
            "lefts": typed_array(instance.lefts()),
            "rights": typed_array(instance.rights()),
            "mus": typed_f64_array(instance.mus()),
        })
    }
}

/// `[{distance, item}, ...]`, in the exact order the tree produced -- this
/// grammar's whole reason to fuzz at all is that order, not membership.
fn neighbors_json(neighbors: &[Neighbor<i64>]) -> Value {
    Value::Array(
        neighbors
            .iter()
            .map(|n| json!({"distance": number_json(n.distance), "item": n.item}))
            .collect(),
    )
}

/// Encode a whole-valued `f64` as a JSON integer, exactly as `JSON.stringify`
/// renders the same JS number: every distance in this grammar is
/// `|a - b|` over two JSON integers, so it is always whole, and `json!(f64)`'s
/// default encoding would otherwise print `5.0` where the oracle's raw
/// number and `JSON.stringify` print `5` -- a false divergence, not a real
/// one. Same fix as `vector`'s `number_json`/`static-interval-tree`'s own
/// (CLAUDE.md: grep before inventing shared machinery; duplicated per-module
/// here to match the existing pattern in this crate).
fn number_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}

/// Encode a `PointerVec` (`nodes`/`lefts`/`rights`) exactly as the oracle
/// encodes a JS typed array: `{$typed: value.constructor.name, values:
/// Array.from(value)}`.
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

/// As [`typed_array`], for `mus` (a real `Float64Array` upstream, whose
/// values are not always whole -- unlike `nodes`/`lefts`/`rights`'s integer
/// pointers, `mus` can average two distances into a `.5`).
fn typed_f64_array(values: &[f64]) -> Value {
    json!({
        "$typed": "Float64Array",
        "values": values.iter().copied().map(number_json).collect::<Vec<Value>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement the module doc's `grammar_self_check` reports:
    /// `neighbors`' radius spans the whole distance range on purpose, so a
    /// campaign's radii really do include both "prune almost everything" and
    /// "prune nothing at all" -- not merely a spread that happens to average
    /// out to something in between. Measured directly by counting distance
    /// calls (one per node the traversal visits) over a batch of queries at
    /// every radius in the grammar's range, against an 80-item tree.
    #[test]
    fn grammar_self_check_radius_spans_full_pruning_and_none() {
        let items: Vec<i64> = (0..RANGE).cycle().take(80).collect();
        let tree = CoreTree::new(items.clone(), dist);
        let size = items.len();

        let mut pruned_queries = 0u32;
        let mut full_scan_queries = 0u32;
        let mut total_queries = 0u32;

        for radius in 0..=RANGE {
            for query in 0..RANGE {
                let visited = std::cell::Cell::new(0usize);
                let counted = |a: &i64, b: &i64| {
                    visited.set(visited.get() + 1);
                    dist(a, b)
                };

                tree.neighbors(radius as f64, &query, counted);

                total_queries += 1;
                if visited.get() < size {
                    pruned_queries += 1;
                } else {
                    full_scan_queries += 1;
                }
            }
        }

        eprintln!(
            "vp-tree grammar_self_check: {pruned_queries}/{total_queries} queries pruned at \
             least one node; {full_scan_queries}/{total_queries} visited every node (radius \
             large enough that no pruning was possible)"
        );

        assert!(
            pruned_queries > 0,
            "at least one (radius, query) pair must prune at least one node"
        );
        assert!(
            full_scan_queries > 0,
            "at least one (radius, query) pair must visit every node (the pruning bound never \
             excludes anything)"
        );
    }
}
