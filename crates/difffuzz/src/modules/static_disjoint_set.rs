//! [`ModuleSpec`] for `static-disjoint-set`.
//!
//! # Grammar, and what it deliberately excludes
//!
//! Ops are `union`, `find` and `connected`; `mapping` and `compile` are in the
//! observable state instead, so they run after *every* op rather than only when
//! the generator picks them. That is not a shortcut — both call `find` on every
//! item, so putting them in the observation set means path compression is
//! exercised on every step of every program, which is the part of this module
//! most likely to diverge.
//!
//! **Out-of-range indices are excluded**, and `docs/DECISIONS.md`'s iteration section asks for such
//! exclusions to be stated rather than left implicit. Upstream reads past the
//! end of a typed array, gets `undefined`, and propagates `NaN` through the
//! parent walk; the port raises a `RangeError` at the bridge (see the napi
//! crate's module docs, adaptation 3). That divergence is deliberate and
//! already documented, so fuzzing it would only re-report a known decision.
//!
//! # Sizes
//!
//! `size` spans 1..=400 for two specific reasons:
//!
//! * it straddles 256, where `getPointerArray` switches `parents` from 8-bit to
//!   16-bit while `ranks` — sized from `log2(size)` — stays 8-bit; and
//! * it is large enough for the BUG-STATIC-DISJOINT-SET-1 rank bug to drive one root's rank past 255
//!   and wrap it inside a `Uint8Array`, the compounding-bugs case
//!   that a `Vec<u32>` port would silently get wrong.

use mnemonist_core::structures::static_disjoint_set::StaticDisjointSet;
use mnemonist_core::utils::typed_arrays::PointerWidth;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Largest set the generator builds. See the module docs for why 400.
const MAX_SIZE: u32 = 400;

pub struct StaticDisjointSetSpec;

/// Path proptest writes a minimised failing seed to.
///
/// Absolute, derived at compile time, because a campaign can be launched from
/// anywhere and a relative path would scatter regression files around the tree.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/static-disjoint-set.txt"
);

impl ModuleSpec for StaticDisjointSetSpec {
    type Instance = StaticDisjointSet;

    fn module(&self) -> &'static str {
        "static-disjoint-set"
    }

    fn observations(&self) -> &'static [&'static str] {
        // `size` and `dimension` are properties; `mapping` and `compile` are
        // nullary methods. The oracle tells them apart by `typeof`.
        &["size", "dimension", "mapping", "compile"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (1u32..=MAX_SIZE).prop_map(|size| vec![json!(size)]).boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let size = ctor[0].as_u64().expect("ctor arg 0 is the size") as u32;
        let index = 0u32..size;

        prop_oneof![
            // Weighted towards `union`: it is the only op that changes the
            // partition, so a read-heavy mix would spend the program looking at
            // the same forest.
            3 => (index.clone(), index.clone())
                .prop_map(|(x, y)| Op::new("union", vec![json!(x), json!(y)])),
            1 => index.clone().prop_map(|x| Op::new("find", vec![json!(x)])),
            1 => (index.clone(), index)
                .prop_map(|(x, y)| Op::new("connected", vec![json!(x), json!(y)])),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        // Enough unions to reach the rank wrap at 256 on the larger sizes.
        1..600
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let size = args[0].as_u64().expect("ctor arg 0 is the size") as usize;

        StaticDisjointSet::new(size).expect("generated sizes are well inside the pointer limit")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        let x = arg(op, 0);

        match op.name {
            // Upstream returns `this` for chaining; the oracle encodes that as
            // `{"$self": true}`. The core returns whether a merge happened,
            // which upstream exposes only through `dimension`, so the bool is
            // dropped here exactly as the napi bridge drops it.
            "union" => {
                instance.union(x, arg(op, 1));
                json!({"$self": true})
            }
            "find" => json!(instance.find(x)),
            "connected" => json!(instance.connected(x, arg(op, 1))),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let mapping = instance.mapping();

        // `mapping()` hands back a real typed array in JS, and which one is
        // observable via `constructor.name` — so the *width* is part of the
        // state, not an implementation detail. Encoding it the same way the
        // oracle does is what makes a width divergence visible.
        let typed = json!({
            "$typed": typed_array_name(mapping.width()),
            "values": mapping.values(),
        });

        json!({
            "size": instance.size(),
            "dimension": instance.dimension(),
            "mapping": typed,
            "compile": instance.compile(),
        })
    }
}

fn arg(op: &Op, position: usize) -> usize {
    op.args[position]
        .as_u64()
        .expect("generated args are non-negative integers") as usize
}

fn typed_array_name(width: PointerWidth) -> &'static str {
    match width {
        PointerWidth::U8 => "Uint8Array",
        PointerWidth::U16 => "Uint16Array",
        PointerWidth::U32 => "Uint32Array",
    }
}
