//! [`ModuleSpec`] for `sparse-set`.
//!
//! # Grammar, and what it deliberately includes
//!
//! This is the first module whose grammar exercises **iteration interleaved
//! with mutation** (D-21), which is what DESIGN.md 3.7 was written for and
//! what `static-disjoint-set` had no surface to reach. Three ops beyond the
//! plain methods:
//!
//! * `$iter("values")` opens a cursor and keeps it. Both sides hold exactly
//!   one.
//! * `$next()` steps it, against whatever the set has become since — which is
//!   the hybrid capture (D-08) under test.
//! * `$spread()` is `Array.from(set)`, going through the *collection's*
//!   `Symbol.iterator` and therefore constructing a fresh cursor each time.
//!   Separate from `$next` on purpose: the factory half of D-07 is only
//!   observable by comparing an op that must restart against one that must
//!   not.
//!
//! # What is NOT excluded, and why that is the point
//!
//! `static-disjoint-set` had to exclude out-of-range indices, because upstream
//! reads past a typed array and propagates `NaN` where the port raises. This
//! module excludes **nothing**. Every out-of-range member is generated, and
//! reproducing what upstream does with it is the port's job:
//!
//! * `has`/`delete` past the end return `false`;
//! * `add` past the end truncates into `dense`, drops the `sparse` write and
//!   increments `size` anyway;
//! * so `size` can exceed `length`, and iteration then runs off the end of
//!   `dense` and yields `undefined`.
//!
//! That last chain is why members are drawn from `0..length + 64` rather than
//! `0..length`: roughly one member in eight is out of range, which is frequent
//! enough that a 200-op program reliably reaches the corrupted regime and
//! iterates inside it.
//!
//! # Observable state
//!
//! `size`, `length`, **and both backing arrays**. Comparing `dense` and
//! `sparse` slot for slot after every op is what makes the swap-in-`delete`
//! and the truncating stores checkable directly, rather than only through
//! their eventual effect on iteration order. Both are public properties
//! upstream, so this observes the same surface a JS caller sees. The
//! `{"$typed": ...}` envelope carries the width, so an 8-vs-16-bit divergence
//! shows up as a type difference rather than silently agreeing on values.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::sparse_set::SparseSet;
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Largest set the generator builds.
///
/// Straddles 256, where `getPointerArray` switches both arrays from 8-bit to
/// 16-bit — and 256 is also where a truncating `dense` store starts to fold
/// distinct members onto the same value.
const MAX_LENGTH: u32 = 400;

/// How far past `length` a generated member may reach.
///
/// Small enough that out-of-range members stay a minority of the traffic,
/// large enough that they are not rare.
const OVERSHOOT: u32 = 64;

pub struct SparseSetSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/sparse-set.txt"
);

/// The set plus the one cursor a program can have open over it.
///
/// This is the shape that forced [`CursorState`] to exist separately from
/// `Cursor`: a borrowing cursor here would make `Instance` self-referential.
pub struct Instance {
    set: SparseSet,
    cursor: Option<CursorState<SparseSet>>,
}

impl ModuleSpec for SparseSetSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "sparse-set"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "length", "dense", "sparse"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // 0 included: `new SparseSet(0)` is legal upstream, and every member
        // is then out of range, which is the degenerate end of the corruption
        // path rather than a separate case.
        (0u32..=MAX_LENGTH)
            .prop_map(|length| vec![json!(length)])
            .boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let length = ctor[0].as_u64().expect("ctor arg 0 is the length") as u32;
        let member = 0u32..(length + OVERSHOOT);

        prop_oneof![
            // Weighted towards `add`: it is the only op that grows the set, so
            // a read-heavy mix would spend most of a program on an empty one.
            5 => member.clone().prop_map(|m| Op::new("add", vec![json!(m)])),
            2 => member.clone().prop_map(|m| Op::new("delete", vec![json!(m)])),
            2 => member.prop_map(|m| Op::new("has", vec![json!(m)])),
            1 => Just(Op::new("clear", vec![])),
            // Cursor lifecycle. `$next` outweighs `$iter` so a cursor is
            // usually stepped several times, with mutations landing between
            // the steps.
            2 => Just(Op::new("$iter", vec![json!("values")])),
            3 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        // Long enough that `add`s outrun `length` on the smaller sets and the
        // corrupted regime is reached with a cursor still open.
        1..200
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let length = args[0].as_u64().expect("ctor arg 0 is the length") as usize;

        Instance {
            set: SparseSet::new(length).expect("generated lengths are inside the pointer limit"),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // Upstream returns `this` for chaining; the oracle encodes that as
            // `{"$self": true}`. The core returns whether the member was new,
            // which the napi bridge drops for the same reason.
            "add" => {
                instance.set.add(member(op));
                json!({"$self": true})
            }
            "delete" => json!(instance.set.delete(member(op))),
            "has" => json!(instance.set.has(member(op))),
            // `clear()` returns `undefined` upstream.
            "clear" => {
                instance.set.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(CursorState::open(&instance.set));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(cursor) => step_value(cursor.step(&instance.set)),
            },
            // A *fresh* cursor every time, which is what the collection-level
            // `Symbol.iterator` does upstream. Reusing `instance.cursor` here
            // would quietly turn the factory into the identity and the test
            // would still pass on every non-interleaved program.
            "$spread" => {
                let mut cursor = CursorState::open(&instance.set);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.set) {
                        Step::Item(item) => items.push(json!(item)),
                        Step::Gap => items.push(json!({"$undefined": true})),
                        Step::Done => break,
                    }
                }

                Value::Array(items)
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.set.size(),
            "length": instance.set.length(),
            "dense": typed_array(instance.set.dense()),
            "sparse": typed_array(instance.set.sparse()),
        })
    }
}

fn member(op: &Op) -> usize {
    op.args[0]
        .as_u64()
        .expect("generated members are non-negative integers") as usize
}

/// A step, in the shape `fuzz/oracle.js` normalises both sides to.
///
/// The `Gap`/`Done` distinction is the whole of DESIGN.md 3.7: both carry
/// `undefined` as their value, and only `done` tells them apart.
fn step_value(step: Step<u32>) -> Value {
    match step {
        Step::Item(item) => json!({"done": false, "value": item}),
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

/// Encode a backing array exactly as the oracle encodes a JS typed array.
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
