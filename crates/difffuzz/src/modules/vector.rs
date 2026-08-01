//! [`ModuleSpec`] for `vector`.
//!
//! # Scope: the four widths the bridge resolves, not `PointerVector`
//!
//! `Vector.PointerVector` has no real `ArrayClass` value a caller passes to
//! the base constructor (see `crates/mnemonist-napi/src/vector.rs`'s module
//! docs) -- it is reached through a JS-installed subclass wrapper, which the
//! oracle's generic `new Ctor(...request.ctor)` dispatch has no way to invoke.
//! This campaign therefore constructs the base `Vector` with one of
//! `Uint8Array`/`Uint16Array`/`Uint32Array`/`Float64Array`, which is the same
//! scope `mnemonist-core`'s own `Vector::fixed`/`Vector::f64` model. The
//! width-transition behaviour `PointerVector` adds is pinned instead by the
//! core native tests (`shrinking_a_pointer_vector_keeps_its_current_width`
//! and friends) and directly against Node in the module docs.
//!
//! # Grammar
//!
//! `set`/`get` indices run well past any generated length, so the
//! `index == length` admission (get/set's off-by-one -- see the core module
//! docs) and the ordinary out-of-bounds throw are both common. Values run past
//! 255 so a `Uint8Array`/`Uint16Array` vector's truncating store is exercised;
//! `Float64Array` never truncates, which the shared observation
//! (`array`, encoded as the real typed array) checks either way. `push`/`pop`
//! pairs drive the stale-slot-after-growth behaviour: a `pop` leaves a slot
//! unerased, and a subsequent `grow`/`reallocate`/`resize` carries it forward
//! -- comparing `array` after every op is what makes that checkable directly.
//!
//! # Observable state
//!
//! `length`, `capacity`, and `array` (the whole backing store, capacity
//! region included). `array` is the point: without it, the stale-slot and
//! bulk-copy behaviours are only checkable through `get(length)`, which the
//! grammar reaches often but not certainly.

use mnemonist_core::structures::vector::{Error as CoreError, Storage, Vector as CoreVector};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Largest index a generated `set`/`get` reaches. Well past any generated
/// length or capacity, so the `length < index` throw/`undefined` and the
/// `index == length` admission are both common.
const MAX_INDEX: u32 = 64;

/// Largest `capacity`/`length` a generated `grow`/`resize`/`reallocate` asks
/// for, and the largest `initialCapacity`/`initialLength`.
const MAX_EXTENT: u32 = 48;

/// Largest value pushed or stored. Past 255 so a `Uint8Array` truncates and
/// past 65,535 so a `Uint16Array` does too.
const MAX_VALUE: f64 = 70_000.0;

pub struct VectorSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proptest-regressions/vector.txt");

impl ModuleSpec for VectorSpec {
    type Instance = CoreVector;

    fn module(&self) -> &'static str {
        "vector"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["length", "capacity", "array"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (
            // `{"$global": …}` is resolved to the real constructor by
            // fuzz/oracle.js; JSON has no way to carry one directly.
            prop::sample::select(&["Uint8Array", "Uint16Array", "Uint32Array", "Float64Array"][..]),
            0u32..MAX_EXTENT,
            0u32..MAX_EXTENT,
        )
            .prop_map(|(class, initial_capacity, initial_length)| {
                vec![
                    json!({ "$global": class }),
                    json!({
                        "initialCapacity": initial_capacity,
                        "initialLength": initial_length,
                    }),
                ]
            })
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            5 => (0.0..MAX_VALUE).prop_map(|v| Op::new("push", vec![json!(v)])),
            3 => Just(Op::new("pop", vec![])),
            3 => (0u32..MAX_INDEX, 0.0..MAX_VALUE)
                    .prop_map(|(i, v)| Op::new("set", vec![json!(i), json!(v)])),
            3 => (0u32..MAX_INDEX).prop_map(|i| Op::new("get", vec![json!(i)])),
            1 => (0u32..MAX_EXTENT).prop_map(|c| Op::new("grow", vec![json!(c)])),
            1 => Just(Op::new("grow", vec![])),
            2 => (0u32..MAX_EXTENT).prop_map(|l| Op::new("resize", vec![json!(l)])),
            1 => (0u32..MAX_EXTENT).prop_map(|c| Op::new("reallocate", vec![json!(c)])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let class = args[0]["$global"].as_str();
        let options = &args[1];
        let capacity = number(&options["initialCapacity"]);
        let length = number(&options["initialLength"]);

        match class {
            Some("Uint8Array") => CoreVector::fixed(PointerWidth::U8, capacity, length),
            Some("Uint16Array") => CoreVector::fixed(PointerWidth::U16, capacity, length),
            Some("Uint32Array") => CoreVector::fixed(PointerWidth::U32, capacity, length),
            Some("Float64Array") => CoreVector::f64(capacity, length),
            other => panic!("ctor arg 0 is a supported array class, got {other:?}"),
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // `push` returns the new length; upstream never throws from it
            // with the default policy this campaign uses (`applyPolicy`'s two
            // throws both need a custom one), so this arm never reaches
            // `thrown` in practice -- kept for symmetry with the others.
            "push" => match instance.push(number_f64(&op.args[0])) {
                Ok(length) => json!(length),
                Err(error) => thrown(&error),
            },
            "pop" => match instance.pop() {
                Some(value) => json!(value),
                None => json!({"$undefined": true}),
            },
            // `set` returns `this` upstream, which the oracle encodes as
            // `{"$self": true}`.
            "set" => match instance.set(number(&op.args[0]), number_f64(&op.args[1])) {
                Ok(()) => json!({"$self": true}),
                Err(error) => thrown(&error),
            },
            "get" => match instance.get(number(&op.args[0])) {
                Some(value) => json!(value),
                None => json!({"$undefined": true}),
            },
            "grow" => match instance.grow(op.args.first().map(number)) {
                Ok(()) => json!({"$self": true}),
                Err(error) => thrown(&error),
            },
            "resize" => match instance.resize(number(&op.args[0])) {
                Ok(()) => json!({"$self": true}),
                Err(error) => thrown(&error),
            },
            "reallocate" => match instance.reallocate(number(&op.args[0])) {
                Ok(()) => json!({"$self": true}),
                Err(error) => thrown(&error),
            },
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "length": instance.length(),
            "capacity": instance.capacity(),
            "array": typed_array(instance.storage()),
        })
    }
}

fn number(value: &Value) -> usize {
    value
        .as_u64()
        .expect("generated arguments are non-negative integers") as usize
}

fn number_f64(value: &Value) -> f64 {
    value.as_f64().expect("generated values are JSON numbers")
}

/// A thrown error, in the shape `fuzz/oracle.js` reports one.
fn thrown(error: &CoreError) -> Value {
    json!({ "$throw": error.to_string() })
}

/// Encode the backing store exactly as the oracle encodes a JS typed array:
/// `{$typed: value.constructor.name, values: Array.from(value)}`.
fn typed_array(storage: &Storage) -> Value {
    match storage {
        Storage::Fixed(values) | Storage::Pointer(values) => {
            let name = match values.width() {
                PointerWidth::U8 => "Uint8Array",
                PointerWidth::U16 => "Uint16Array",
                PointerWidth::U32 => "Uint32Array",
            };

            json!({
                "$typed": name,
                "values": pointer_values(values),
            })
        }
        Storage::F64(values) => json!({
            "$typed": "Float64Array",
            "values": values.clone(),
        }),
    }
}

fn pointer_values(values: &PointerVec) -> Vec<u32> {
    (0..values.len()).map(|i| values.get(i)).collect()
}
