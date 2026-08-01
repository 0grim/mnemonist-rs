//! [`ModuleSpec`] for `sparse-map`.
//!
//! # What this grammar reaches that `sparse-set`'s could not
//!
//! Two things, and both are specific to the payload.
//!
//! **The value store is generated, not fixed.** `new SparseMap(Values, length)`
//! takes an array constructor, and the four this port supports behave
//! differently enough that fuzzing only one would be fuzzing half the module: a
//! plain `Array` **grows** past the map's length where a typed array drops the
//! write, and a typed array narrows `700` to `188` where an `Array` keeps it.
//! The constructor travels as `{"$global": "Uint8Array"}` and `fuzz/oracle.js`
//! resolves it against the real global.
//!
//! **Three cursors, not one.** `$iter` takes the factory name, so a program can
//! open a `keys`, `values` or `entries` walk, and the three disagree in exactly
//! the state the port is most likely to get wrong: once `size` has run past
//! `length`, `keys` gaps on `dense` while `values` may still have real data
//! from a grown `Array` store, and `entries` never gaps at all because upstream
//! yields the pair rather than its halves.
//!
//! # Observable state
//!
//! `size`, `length`, `dense`, `sparse` **and `vals`**. All three arrays are
//! public properties upstream. `vals` is the one that matters most here:
//! `delete` moves the key and deliberately leaves the value behind (B-11), so a
//! port that "tidied that up" would still agree on `size`, on `dense`, on
//! `sparse` and on every `has` — and disagree only on `get` and on `vals`
//! itself. Comparing the array directly is what makes the defect checkable
//! rather than inferable.
//!
//! # Deliberately excluded
//!
//! Nothing in the op alphabet. The values are integers in `0..=1000`, which is
//! narrower than every JS number — the bridge takes `f64` and this spec takes
//! `u32` — because the oracle compares JSON and a float that renders as `13.0`
//! on one side and `13` on the other is a false divergence, not a finding. The
//! narrowing is disclosed in `fuzz/log.txt` rather than left implicit.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::sparse_map::{Projected, Projection, SparseMap, Values};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{
    for_each, for_each_args, for_each_index, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY,
};

/// Largest map the generator builds.
///
/// Straddles 256, where `getPointerArray` switches `dense` and `sparse` from
/// 8-bit to 16-bit — and 256 is also where a truncating `dense` store starts to
/// fold distinct members onto the same value.
const MAX_LENGTH: u32 = 400;

/// How far past `length` a generated member may reach.
const OVERSHOOT: u32 = 64;

/// Largest value stored. Comfortably past 255, so an 8-bit value store
/// truncates on a good fraction of its writes.
const MAX_VALUE: u32 = 1_000;

/// The value array constructors a generated program may ask for.
///
/// `None` is `Array` — passed as a one-argument constructor call, which is also
/// how the *other* upstream signature (`new SparseMap(length)`) gets exercised.
const STORES: [Option<(&str, PointerWidth)>; 4] = [
    None,
    Some(("Uint8Array", PointerWidth::U8)),
    Some(("Uint16Array", PointerWidth::U16)),
    Some(("Uint32Array", PointerWidth::U32)),
];

pub struct SparseMapSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/sparse-map.txt"
);

/// The map plus the one cursor a program can have open over it.
pub struct Instance {
    map: SparseMap<u32>,
    cursor: Option<(Projection, CursorState<SparseMap<u32>>)>,
}

impl ModuleSpec for SparseMapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "sparse-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "length", "dense", "sparse", "vals"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // 0 included: `new SparseMap(0)` is legal upstream, every member is
        // then out of range, and an `Array` value store still grows one slot
        // per `set` while `dense` never takes one.
        (0usize..STORES.len(), 0u32..=MAX_LENGTH)
            .prop_map(|(store, length)| match STORES[store] {
                None => vec![json!(length)],
                Some((name, _)) => vec![json!({ "$global": name }), json!(length)],
            })
            .boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let member = 0u32..(length_of(ctor) + OVERSHOOT);
        let value = 0u32..=MAX_VALUE;

        prop_oneof![
            // Weighted towards `set`: it is the only op that grows the map, so
            // a read-heavy mix would spend most of a program on an empty one.
            5 => (member.clone(), value).prop_map(|(m, v)| Op::new("set", vec![json!(m), json!(v)])),
            2 => member.clone().prop_map(|m| Op::new("delete", vec![json!(m)])),
            2 => member.clone().prop_map(|m| Op::new("has", vec![json!(m)])),
            2 => member.prop_map(|m| Op::new("get", vec![json!(m)])),
            1 => Just(Op::new("clear", vec![])),
            // Cursor lifecycle. All three factories, because the three
            // projections disagree precisely where the port is most likely to
            // be wrong.
            1 => Just(Op::new("$iter", vec![json!("keys")])),
            1 => Just(Op::new("$iter", vec![json!("values")])),
            1 => Just(Op::new("$iter", vec![json!("entries")])),
            3 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            // `forEach` re-reads `this.size` every iteration, exactly as
            // `sparse-set` does and exactly as `sparse-queue-set` does NOT.
            // See `crate::spec::ForEach`.
            2 => for_each_strategy(MUTATIONS),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..200
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let length = length_of(args) as usize;

        let map = match args.first().and_then(|arg| arg.get("$global")) {
            None => SparseMap::array(length),
            Some(name) => SparseMap::typed(length, width_of(name)),
        }
        .expect("generated lengths are inside the pointer limit");

        Instance { map, cursor: None }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // Upstream returns `this` for chaining; the oracle encodes that as
            // `{"$self": true}`.
            "set" => {
                instance.map.set(member(op), value(op));
                json!({"$self": true})
            }
            "delete" => json!(instance.map.delete(member(op))),
            "has" => json!(instance.map.has(member(op))),
            "get" => optional(instance.map.get(member(op))),
            // `clear()` returns `undefined` upstream.
            "clear" => {
                instance.map.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                let projection = projection_of(op);

                instance.cursor = Some((
                    projection,
                    CursorState::open_projected(&instance.map, projection),
                ));

                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some((projection, cursor)) => step_value(*projection, cursor.step(&instance.map)),
            },
            // A *fresh* cursor every time, through the collection-level
            // `Symbol.iterator` — which upstream aliases to `entries`, not to
            // `values`. Reusing `instance.cursor` here would quietly turn the
            // factory into the identity.
            // Upstream's own loop, re-read bound and all:
            //
            // ```js
            // for (var i = 0; i < this.size; i++)
            //   callback.call(scope, this.vals[i], this.dense[i]);
            // ```
            //
            // Note the argument ORDER: the value is first and the key second,
            // which is why the `set` mutation's rule is `arg1,arg0`.
            "$forEach" => {
                let spec = for_each(op);
                let mut seen = Vec::new();
                let mut fired = 0usize;
                let mut index = 0usize;

                while index < instance.map.size() {
                    let value = slot(instance.map.vals().slot(index));
                    let key = slot(instance.map.dense().try_get(index));
                    let received = vec![value, key];

                    seen.push(Value::Array(received.clone()));

                    if fired < spec.limit {
                        if let Some(args) = for_each_args(&spec, &received) {
                            fired += 1;

                            match spec.method.expect("for_each_args returned Some") {
                                "set" => {
                                    let value =
                                        args[1].as_u64().expect("a stored value is a JSON integer")
                                            as u32;

                                    instance.map.set(for_each_index(&spec, args[0]), value);
                                }
                                "delete" => {
                                    instance.map.delete(for_each_index(&spec, args[0]));
                                }
                                "clear" => instance.map.clear(),
                                other => {
                                    panic!("`{other}` is not a $forEach mutation for this module")
                                }
                            }
                        }
                    }

                    index += 1;
                }

                json!({ "seen": seen })
            }
            "$spread" => {
                let mut cursor = CursorState::open_projected(&instance.map, Projection::Entries);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.map) {
                        Step::Done => break,
                        step => items.push(step_item(Projection::Entries, step)),
                    }
                }

                Value::Array(items)
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.map.size(),
            "length": instance.map.length(),
            "dense": typed_array(instance.map.dense()),
            "sparse": typed_array(instance.map.sparse()),
            "vals": value_store(instance.map.vals()),
        })
    }
}

fn length_of(ctor: &[Value]) -> u32 {
    ctor.last()
        .and_then(Value::as_u64)
        .expect("the last ctor arg is always the length") as u32
}

fn width_of(name: &Value) -> PointerWidth {
    let name = name.as_str().expect("a $global argument names a global");

    STORES
        .iter()
        .flatten()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, width)| *width)
        .expect("only the constructors in STORES are ever generated")
}

fn projection_of(op: &Op) -> Projection {
    match op.args[0].as_str() {
        Some("keys") => Projection::Keys,
        Some("values") => Projection::Values,
        Some("entries") => Projection::Entries,
        other => panic!("`{other:?}` is not an iterator factory on this module"),
    }
}

/// What the callback may do to the map, and how often.
///
/// `set` writes back the pair it was just handed, so it overwrites rather than
/// grows and is safe uncapped even though this module's bound is live.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("delete", "arg1", FOR_EACH_MANY),
    ("set", "arg1,arg0", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

/// A `vals` or `dense` slot as the oracle encodes it.
fn slot(value: Option<u32>) -> Value {
    match value {
        Some(item) => json!(item),
        None => json!({"$undefined": true}),
    }
}

fn member(op: &Op) -> usize {
    op.args[0]
        .as_u64()
        .expect("generated members are non-negative integers") as usize
}

fn value(op: &Op) -> u32 {
    op.args[1]
        .as_u64()
        .expect("generated values are non-negative integers") as u32
}

/// `Some(v)` as itself, `None` as JS `undefined` — never as `null`, which is a
/// different value to `assert.deepStrictEqual` and to the oracle's encoder.
fn optional<T: Into<Value>>(value: Option<T>) -> Value {
    match value {
        Some(value) => value.into(),
        None => json!({"$undefined": true}),
    }
}

/// A step, in the shape `fuzz/oracle.js` normalises both sides to.
fn step_value(projection: Projection, step: Step<Projected<u32>>) -> Value {
    json!({
        "done": step.is_done(),
        "value": step_item(projection, step),
    })
}

/// What a single step *yields*, before the `{done, value}` envelope.
///
/// The `Entries` projection is the odd one: upstream builds `[dense[i],
/// vals[i]]` and yields **the array**, so a missing half is `undefined` inside
/// a yielded value and the step itself never gaps. `Keys` and `Values` read one
/// slot and yield a bare `undefined` when it is not there.
fn step_item(projection: Projection, step: Step<Projected<u32>>) -> Value {
    match step {
        Step::Item(Projected::Key(key)) => json!(key),
        Step::Item(Projected::Value(value)) => json!(value),
        Step::Item(Projected::Entry(key, value)) => json!([optional(key), optional(value)]),
        // A gap on an entries walk is unreachable, but encoding it as an empty
        // pair rather than as a bare `undefined` keeps the shape honest if it
        // ever becomes reachable.
        Step::Gap if matches!(projection, Projection::Entries) => {
            json!([optional::<u32>(None), optional::<u32>(None)])
        }
        Step::Gap | Step::Done => json!({"$undefined": true}),
    }
}

/// Encode an index array exactly as the oracle encodes a JS typed array.
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

/// Encode `vals`, which is *not* always a typed array.
///
/// The `Array` form is a plain JS array with holes in it. `Array.prototype.map`
/// preserves holes and `JSON.stringify` renders one as `null`, so a hole must
/// be [`Value::Null`] here — not `{"$undefined": true}`, which is what an
/// element explicitly set to `undefined` would encode to. Nothing in this
/// grammar can store `undefined`, so the two never have to be told apart, but
/// getting the hole wrong would fail every `Array`-backed program immediately.
fn value_store(values: &Values<u32>) -> Value {
    match values {
        Values::Typed(slots) => typed_array(slots),
        Values::Array(slots) => Value::Array(
            slots
                .iter()
                .map(|slot| match slot {
                    Some(value) => json!(value),
                    None => Value::Null,
                })
                .collect(),
        ),
    }
}
