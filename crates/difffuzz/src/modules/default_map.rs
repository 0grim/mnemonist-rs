//! [`ModuleSpec`] for `default-map`, and the first grammar over a `Map`.
//!
//! # What this grammar is for
//!
//! `default-map` is thin. What is under test is
//! [`mnemonist_core::map::OrderedMap`] — insertion order, tombstones,
//! compaction and live cursors — plus the four lines of `default-map.js` that
//! are not delegation and that hold B-40. So the grammar is built to reach
//! those and not to admire the wrapper:
//!
//! * **Keys are drawn from a small pool** ([`KEY_POOL`]), so collisions,
//!   overwrites and delete-then-reinsert happen constantly rather than by
//!   luck. A wide key space would spend every program inserting fresh keys and
//!   would never reach the interesting transitions.
//! * **The pool contains `NaN` and `-0`**, the only two places SameValueZero
//!   differs from `===`.
//! * **`undefined` is in the value alphabet**, and the `undefined` factory is
//!   in the constructor alphabet. Between them, B-40 — `size` drifting away
//!   from `items.size` — is reached in two operations, and every subsequent
//!   op is then compared against a *drifted* upstream.
//! * **Deletes outweigh nothing.** A 200-op program over eight keys deletes
//!   enough to force several compactions, which is the only way the cursor's
//!   id-based relocation is exercised at all.
//! * **Cursor lifecycle ops interleave with mutation** (D-21), across all
//!   three iterator flavours, because `Map` iterators are live and that
//!   liveness is invisible to any program that drains without mutating.
//!
//! # Observable state
//!
//! `size` **and** `items`. Both are public upstream. Comparing them
//! *separately* is the point: they disagree by design once B-40 fires, so a
//! port that quietly made `size` return the entry count would agree on `items`
//! and diverge on `size` within a handful of ops. The oracle encodes a JS
//! `Map` as `{"$map": [[k, v], ...]}` — a list, so entry **order** is compared
//! too, not just membership.
//!
//! # What this grammar deliberately does NOT cover
//!
//! **`JsKey` itself.** The real key type lives in `mnemonist-napi`, which is a
//! `cdylib` and cannot be linked into a plain Rust binary, so [`FuzzKey`] here
//! mirrors its normalisation rather than reusing it. What the fuzzer therefore
//! checks is that *the normalisation rule* is right — `NaN` folds, `-0` folds,
//! and nothing else does — against a real `Map`. That the **bridge** applies
//! that rule is checked elsewhere: eight native tests in
//! `mnemonist_napi::js_key`, and the side-by-side probes recorded in
//! `docs/modules/default-map.md`. Stated rather than glossed.
//!
//! **`forEach`.** It takes a callback, and the oracle protocol has no way to
//! send one. Its walk is the same cursor the iterators use, so what is
//! specific to it — the callback arguments and `scope` binding — is covered by
//! the original test file and by the probes instead.

use std::collections::BTreeMap;

use mnemonist_core::map::MapCursor;
use mnemonist_core::structures::default_map::DefaultMap;
use proptest::prelude::*;
use serde_json::{json, Map as JsonMap, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/default-map.txt"
);

/// Every key a generated program can use.
///
/// Eight, so that a 200-op program revisits each one about 25 times. Mixed
/// types on purpose: `0` and `"0"` are different `Map` keys, and a port that
/// stringified its keys would agree on everything else.
const KEY_POOL: usize = 8;

/// The factories a generated `DefaultMap` can be built with.
///
/// Names, matched by a table in `fuzz/oracle.js`. `undefined` is the one that
/// reaches B-40; `autoIncrement` is upstream's own documented factory and is
/// the only stateful one.
const FACTORIES: [&str; 5] = ["undefined", "null", "autoIncrement", "key", "size"];

/// A `Map` key with SameValueZero, mirroring `mnemonist_napi::js_key::JsKey`.
///
/// See the module docs for why this is a mirror rather than the real thing.
/// `Number` holds normalised bits for the same reason `JsKey` does: derived
/// `Hash` and `Eq` are then SameValueZero and cannot disagree with each other.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FuzzKey {
    Number(u64),
    String(String),
}

impl FuzzKey {
    fn number(value: f64) -> Self {
        let normalised = if value == 0.0 {
            0.0
        } else if value.is_nan() {
            f64::NAN
        } else {
            value
        };

        Self::Number(normalised.to_bits())
    }

    /// Decode a key from the wire form the oracle also decodes.
    fn from_json(value: &Value) -> Self {
        match value {
            Value::String(text) => Self::String(text.clone()),
            Value::Number(number) => Self::number(number.as_f64().expect("keys are finite JSON")),
            Value::Object(fields) if fields.contains_key("$nan") => Self::number(f64::NAN),
            Value::Object(fields) if fields.contains_key("$negativeZero") => Self::number(-0.0),
            other => panic!("`{other}` is not a key this grammar generates"),
        }
    }

    /// Encode a key exactly as `fuzz/oracle.js`'s `encode` would.
    fn to_json(&self) -> Value {
        match self {
            Self::String(text) => json!(text),
            Self::Number(bits) => number_json(f64::from_bits(*bits)),
        }
    }
}

/// A JavaScript number, encoded as `JSON.stringify` would encode it.
///
/// JavaScript has one number type and JSON has one number syntax, so `1`
/// serialises as `1` and never as `1.0`. serde_json *does* distinguish the two
/// and compares them unequal, so a Rust side emitting `json!(1.0)` disagrees
/// with the oracle on every integral key — a false divergence that says
/// nothing about the port. Found by this module's very first campaign, which
/// is what a fuzzer is for even when the fault turns out to be in the fuzzer.
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

/// The key pool, as wire values. Index `i` of a generated op selects one.
fn key_at(index: usize) -> Value {
    match index {
        0 => json!("a"),
        1 => json!("b"),
        2 => json!("0"),
        3 => json!(0),
        4 => json!(1),
        5 => json!(-1),
        // The two SameValueZero cases.
        6 => json!({"$nan": true}),
        _ => json!({"$negativeZero": true}),
    }
}

/// A stored value, as the oracle encodes one. `None` is `undefined`.
fn value_slot(value: &Value) -> Option<Value> {
    match value {
        Value::Object(fields) if fields.contains_key("$undefined") => None,
        other => Some(other.clone()),
    }
}

/// Re-encode a stored slot the way the oracle's `encode` would.
fn slot_json(slot: Option<&Value>) -> Value {
    match slot {
        None => json!({"$undefined": true}),
        Some(value) => value.clone(),
    }
}

/// The factory, modelled in Rust. One per instance, so `AutoIncrement`'s
/// counter belongs to the instance and not to the campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Factory {
    Undefined,
    Null,
    AutoIncrement,
    Key,
    Size,
}

impl Factory {
    fn named(name: &str) -> Self {
        match name {
            "undefined" => Self::Undefined,
            "null" => Self::Null,
            "autoIncrement" => Self::AutoIncrement,
            "key" => Self::Key,
            "size" => Self::Size,
            other => panic!("`{other}` is not a factory this grammar generates"),
        }
    }
}

pub struct Instance {
    map: DefaultMap<FuzzKey, Value>,
    factory: Factory,
    /// `autoIncrement`'s counter. Unused by the other four.
    counter: f64,
    /// The one cursor a program can have open, and which of the three
    /// projections it was opened as.
    cursor: Option<(MapCursor, &'static str)>,
}

pub struct DefaultMapSpec;

impl ModuleSpec for DefaultMapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "default-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (0..FACTORIES.len())
            .prop_map(|index| vec![json!({"$factory": FACTORIES[index]})])
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let key = (0..KEY_POOL).prop_map(key_at);
        // A value alphabet narrow enough to collide and wide enough to tell
        // the three JSON scalar shapes apart. `undefined` is weighted in
        // rather than rare: it is the only route to B-40.
        let value = prop_oneof![
            2 => Just(json!({"$undefined": true})),
            1 => Just(Value::Null),
            2 => (0i64..4).prop_map(|n| json!(n)),
            2 => Just(json!("v")),
        ];

        prop_oneof![
            // `get` is the interesting one: it is the mutating read, and the
            // only op that can run the factory.
            5 => key.clone().prop_map(|k| Op::new("get", vec![k])),
            4 => (key.clone(), value).prop_map(|(k, v)| Op::new("set", vec![k, v])),
            3 => key.clone().prop_map(|k| Op::new("delete", vec![k])),
            2 => key.clone().prop_map(|k| Op::new("peek", vec![k])),
            2 => key.prop_map(|k| Op::new("has", vec![k])),
            1 => Just(Op::new("clear", vec![])),
            // Cursor lifecycle, across all three projections.
            2 => prop_oneof![
                Just(Op::new("$iter", vec![json!("entries")])),
                Just(Op::new("$iter", vec![json!("keys")])),
                Just(Op::new("$iter", vec![json!("values")])),
            ],
            4 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let name = args[0]
            .get("$factory")
            .and_then(Value::as_str)
            .expect("ctor arg 0 is a named factory");

        Instance {
            map: DefaultMap::new(),
            factory: Factory::named(name),
            counter: 0.0,
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "get" => {
                // Split the borrow so the factory can touch the counter while
                // the map is being written, which is what the bridge does with
                // the JS callback.
                let Instance {
                    map,
                    factory,
                    counter,
                    ..
                } = instance;
                let key = FuzzKey::from_json(&op.args[0]);

                let value = map.get_or_insert_with(key, |key, size| match factory {
                    Factory::Undefined => None,
                    Factory::Null => Some(Value::Null),
                    Factory::AutoIncrement => {
                        let next = *counter;
                        *counter += 1.0;

                        Some(number_json(next))
                    }
                    Factory::Key => Some(key.to_json()),
                    Factory::Size => Some(json!(size)),
                });

                slot_json(value)
            }
            "peek" => slot_json(instance.map.peek(&FuzzKey::from_json(&op.args[0]))),
            "set" => {
                instance
                    .map
                    .set(FuzzKey::from_json(&op.args[0]), value_slot(&op.args[1]));

                json!({"$self": true})
            }
            "has" => json!(instance.map.has(&FuzzKey::from_json(&op.args[0]))),
            "delete" => json!(instance
                .map
                .delete(&FuzzKey::from_json(&op.args[0]))
                .is_some()),
            "clear" => {
                instance.map.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                let projection = op.args[0]
                    .as_str()
                    .expect("$iter names a projection")
                    .to_owned();
                let projection: &'static str = match projection.as_str() {
                    "entries" => "entries",
                    "keys" => "keys",
                    "values" => "values",
                    other => panic!("`{other}` is not an iterator this module has"),
                };

                instance.cursor = Some((instance.map.cursor(), projection));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some((cursor, projection)) => {
                    let projection = *projection;

                    match cursor.step(instance.map.items()) {
                        None => json!({"done": true, "value": {"$undefined": true}}),
                        Some((key, value)) => json!({
                            "done": false,
                            "value": project(projection, key, value.as_ref()),
                        }),
                    }
                }
            },
            // `Array.from(map)` goes through the COLLECTION's Symbol.iterator,
            // which upstream aliases to `entries` -- not to `values`, as every
            // other module in the port does. A fresh cursor every time, which
            // is what makes the factory half of D-07 observable next to
            // `$next`, which must not restart.
            "$spread" => {
                let mut cursor = instance.map.cursor();
                let mut out = Vec::new();

                while let Some((key, value)) = cursor.step(instance.map.items()) {
                    out.push(project("entries", key, value.as_ref()));
                }

                Value::Array(out)
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let entries: Vec<Value> = instance
            .map
            .items()
            .iter()
            .map(|(key, value)| json!([key.to_json(), slot_json(value.as_ref())]))
            .collect();

        // `size` and `items` are compared as two separate observations
        // because they disagree by design: see B-40.
        let mut state = JsonMap::new();
        state.insert("size".into(), json!(instance.map.size()));
        state.insert("items".into(), json!({"$map": entries}));

        Value::Object(
            state
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}

/// One iterator step, in the shape the projection yields it.
fn project(projection: &str, key: &FuzzKey, value: Option<&Value>) -> Value {
    match projection {
        "keys" => key.to_json(),
        "values" => slot_json(value),
        _ => json!([key.to_json(), slot_json(value)]),
    }
}
