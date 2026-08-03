//! [`ModuleSpec`] for `default-weak-map`.
//!
//! # What this grammar can and cannot check — read before adding an observation
//!
//! `mnemonist_core::structures::default_weak_map`'s module docs say it
//! plainly: a real `WeakMap` exposes no `size`, no iteration, nothing that
//! reads as "the whole map." So [`ModuleSpec::observations`] is **empty**,
//! deliberately — not an oversight, and not the same thing as an untested
//! module. Every comparison this campaign makes is a **return value**:
//! `get`, `peek`, `has`, `delete`, `set`'s `{"$self": true}` — which is the
//! entire observable surface upstream has, faithfully covered rather than
//! narrowed.
//!
//! # Identity over the wire: a fixed key pool, never collected
//!
//! JSON cannot carry object identity, so a key travels as
//! `{"$weakKey": n}`, and `fuzz/oracle.js` resolves it against a small pool
//! of real objects created **once**, at oracle start-up, and held by a
//! module-level array for the process's entire life. That is what makes the
//! whole campaign well-defined: those objects are never eligible for
//! collection, so `WeakKey::matches` on the port side and `===` on the
//! oracle side are comparing the same fixed set of identities throughout —
//! GC timing, which neither side can observe anyway (see the core module's
//! docs), never enters the picture. [`FuzzKey`] here is `u8`, an index into
//! that same pool, mirroring the oracle's resolution the same way
//! `default_map`'s own `FuzzKey` mirrors `JsKey` rather than reusing it
//! (`mnemonist-napi` is a `cdylib` and cannot be linked into this binary).
//!
//! # Deliberately excluded
//!
//! **Object keys with distinguishable identity but coincidental structural
//! equality.** Every key in the pool is a bare `{}`; two pool slots are
//! never equal in *content* either, so this grammar cannot by itself
//! distinguish "compares by identity" from "compares by deep equality" the
//! way a real adversarial case would. That distinction is instead pinned by
//! `mnemonist_core::structures::default_weak_map`'s own
//! `identity_not_content_decides_a_match…` Rust test, which controls the
//! matcher directly rather than through a `u8` mirror. **`peek`/`get`/`has`/
//! `delete` called with a non-object argument** (the ordering divergence
//! `crates/mnemonist-napi/src/default_weak_map.rs` documents for `get`) is
//! bridge-specific and not reachable through `mnemonist-core` at all — this
//! grammar only ever generates pool-object keys, by construction.

use mnemonist_core::structures::default_weak_map::DefaultWeakMap;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/default-weak-map.txt"
);

/// Number of distinct pool objects `fuzz/oracle.js` holds — see the module
/// docs. Small, so the same handful of identities are set, deleted and
/// re-read constantly rather than each op minting a fresh one.
const KEY_POOL: usize = 8;

/// A mirror of the oracle's key pool: which slot, nothing else. See the
/// module docs on why this is `u8` and not `JsKey`/an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FuzzKey(u8);

impl FuzzKey {
    fn from_json(value: &Value) -> Self {
        let index = value
            .get("$weakKey")
            .and_then(Value::as_u64)
            .expect("a key argument in this grammar is always `{\"$weakKey\": n}`");

        Self(index as u8)
    }

    fn matches(self) -> impl FnMut(&FuzzKey) -> bool {
        move |candidate: &FuzzKey| *candidate == self
    }
}

fn key_at(index: usize) -> Value {
    json!({"$weakKey": index as u64})
}

/// A stored value, as the oracle encodes one. `None` is `undefined`.
fn value_slot(value: &Value) -> Option<Value> {
    match value {
        Value::Object(fields) if fields.contains_key("$undefined") => None,
        other => Some(other.clone()),
    }
}

fn slot_json(slot: Option<&Value>) -> Value {
    match slot {
        None => json!({"$undefined": true}),
        Some(value) => value.clone(),
    }
}

/// The two factories this grammar exercises. Both are already in
/// `fuzz/oracle.js`'s shared `FACTORIES` table (`default-map` put them
/// there), and both accept upstream's one-argument
/// `DefaultWeakMap` factory signature unchanged — `undefined`/`null` ignore
/// every argument they are called with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Factory {
    Undefined,
    Null,
}

impl Factory {
    fn named(name: &str) -> Self {
        match name {
            "undefined" => Self::Undefined,
            "null" => Self::Null,
            other => panic!("`{other}` is not a factory this grammar generates"),
        }
    }
}

pub struct Instance {
    map: DefaultWeakMap<FuzzKey, Value>,
    factory: Factory,
}

pub struct DefaultWeakMapSpec;

impl ModuleSpec for DefaultWeakMapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "default-weak-map"
    }

    /// Empty, deliberately. See the module docs: a real `WeakMap` exposes no
    /// "whole state" to compare, so every check here is a return value.
    fn observations(&self) -> &'static [&'static str] {
        &[]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            Just(vec![json!({"$factory": "undefined"})]),
            Just(vec![json!({"$factory": "null"})]),
        ]
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let key = (0..KEY_POOL).prop_map(key_at);
        // `undefined` weighted in rather than rare: it is the only route to
        // BUG-DEFAULT-WEAK-MAP-1, and once stored, every following `get` on that key re-runs
        // the factory.
        let value = prop_oneof![
            2 => Just(json!({"$undefined": true})),
            1 => Just(Value::Null),
            2 => (0i64..4).prop_map(|n| json!(n)),
            2 => Just(json!("v")),
        ];

        prop_oneof![
            5 => key.clone().prop_map(|k| Op::new("get", vec![k])),
            4 => (key.clone(), value).prop_map(|(k, v)| Op::new("set", vec![k, v])),
            3 => key.clone().prop_map(|k| Op::new("delete", vec![k])),
            2 => key.clone().prop_map(|k| Op::new("peek", vec![k])),
            2 => key.prop_map(|k| Op::new("has", vec![k])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let name = args[0]
            .get("$factory")
            .and_then(Value::as_str)
            .expect("ctor arg 0 is a named factory");

        Instance {
            map: DefaultWeakMap::new(),
            factory: Factory::named(name),
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "get" => {
                let key = FuzzKey::from_json(&op.args[0]);

                if let Some(value) = instance.map.peek(key.matches()) {
                    return slot_json(Some(value));
                }

                let manufactured = match instance.factory {
                    Factory::Undefined => None,
                    Factory::Null => Some(Value::Null),
                };

                slot_json(
                    instance
                        .map
                        .write_from_factory(key.matches(), || key, manufactured),
                )
            }
            "peek" => slot_json(instance.map.peek(FuzzKey::from_json(&op.args[0]).matches())),
            "set" => {
                let key = FuzzKey::from_json(&op.args[0]);

                instance
                    .map
                    .set(key.matches(), || key, value_slot(&op.args[1]));

                json!({"$self": true})
            }
            "has" => json!(instance.map.has(FuzzKey::from_json(&op.args[0]).matches())),
            "delete" => json!(instance
                .map
                .delete(FuzzKey::from_json(&op.args[0]).matches())
                .is_some()),
            "clear" => {
                instance.map.clear();
                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, _instance: &mut Self::Instance) -> Value {
        // Empty, matching `observations()`. See the module docs.
        json!({})
    }
}
