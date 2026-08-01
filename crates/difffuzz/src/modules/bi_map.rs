//! [`ModuleSpec`] for `bi-map`.
//!
//! # What this grammar is for
//!
//! Every op is a **forward**-direction call (`set`, `delete`, `get`, `has`,
//! `clear`) — there is no oracle protocol for reaching `instance.inverse.*`
//! (see "Deliberately excluded" below) — but every observation reads **both**
//! sides, so the constraint-resolution logic in `set`/`delete` (the four
//! branches documented in `mnemonist_core::structures::bi_map`) is checked
//! from the one direction the grammar can drive and would show up on the
//! other side if it broke the bijection.
//!
//! * **Keys and values share one small pool** ([`POOL`]), so `set` collides
//!   with an existing key, an existing value, or both, far more often than a
//!   wide space would produce by chance — that collision handling is the
//!   entire point of the module.
//! * **`clear` and `delete` are weighted in**, because the "reinsert after
//!   delete moves to the end" behaviour (inherited from `OrderedMap`) and the
//!   bijection's `size`/`inverse.size` agreement are both easiest to break
//!   right after a removal.
//!
//! # Observable state
//!
//! `size`, `items` (upstream's own `this.items`, a real `Map`) and `inverse` —
//! upstream's whole `InverseMap` object, encoded generically by
//! `fuzz/oracle.js`'s `encode()`: `{size, items: {$map: [...]}, inverse:
//! {$self: true}}`, because `instance.inverse.inverse === instance` and
//! `encode` special-cases exactly that circular reference. No oracle change
//! was needed to reach it — `bi-map.js` makes `items` and `inverse` ordinary
//! enumerable instance properties, the same as `default-map.js` does for
//! `items`.
//!
//! # Deliberately excluded
//!
//! **`instance.inverse.*` is never called directly.** The oracle's `op`
//! command dispatches `instance[name](...)`, which has no way to reach a
//! nested `instance.inverse.set(...)`. Every forward op still mutates both
//! maps, and every observation still reads both, so the bijection invariant is
//! fully checked; what is not separately fuzzed is `InverseMap`'s own
//! generic-method delegation (`has`/`get`/`forEach`/`keys`/`values`/`entries`
//! *called through* `.inverse`), which the original suite and
//! `mnemonist_napi::bi_map`'s tests both exercise instead.
//!
//! **Cursor lifecycle ops (`$iter`/`$next`/`$spread`).** `bi-map`'s cursor is
//! `mnemonist_core::map::MapCursor` over `OrderedMap`, already fuzzed by
//! `default-map`'s campaign against the same `Map` semantics; adding a second
//! copy here would multiply campaign time without reaching new code. `forEach`
//! is likewise not in this alphabet yet — see `fuzz/log.txt`.

use mnemonist_core::structures::bi_map::BiMap;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/bi-map.txt"
);

/// Shared by keys and values, so `set` collides constantly. Mixed types for
/// the same reason `default-map`'s pool is: `0` and `"0"` are different `Map`
/// keys (SameValueZero, not `==`), and a port that coerced would agree on
/// everything else.
const POOL: usize = 6;

/// A `Map` key with SameValueZero, mirroring `mnemonist_napi::js_key::JsKey`.
/// A mirror rather than the real thing for the same reason `default-map`'s
/// `FuzzKey` is one: the real type lives in the `cdylib` bridge crate.
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

    fn from_json(value: &Value) -> Self {
        match value {
            Value::String(text) => Self::String(text.clone()),
            Value::Number(number) => Self::number(number.as_f64().expect("keys are finite JSON")),
            other => panic!("`{other}` is not a key this grammar generates"),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::String(text) => json!(text),
            Self::Number(bits) => number_json(f64::from_bits(*bits)),
        }
    }
}

/// A JavaScript number, encoded as `JSON.stringify` would encode it — see
/// `default_map`'s identical helper for why this cannot be `json!(value)`.
fn number_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}

fn key_at(index: usize) -> Value {
    match index {
        0 => json!("a"),
        1 => json!("b"),
        2 => json!("c"),
        3 => json!(0),
        4 => json!(1),
        _ => json!(2),
    }
}

pub struct BiMapSpec;

impl ModuleSpec for BiMapSpec {
    type Instance = BiMap<FuzzKey>;

    fn module(&self) -> &'static str {
        "bi-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "items", "inverse"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(vec![]).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let key = (0..POOL).prop_map(key_at);
        let value = (0..POOL).prop_map(key_at);

        prop_oneof![
            5 => (key.clone(), value).prop_map(|(k, v)| Op::new("set", vec![k, v])),
            3 => key.clone().prop_map(|k| Op::new("delete", vec![k])),
            3 => key.clone().prop_map(|k| Op::new("get", vec![k])),
            2 => key.prop_map(|k| Op::new("has", vec![k])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        BiMap::new()
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let key = FuzzKey::from_json(&op.args[0]);
                let value = FuzzKey::from_json(&op.args[1]);

                instance.set(key, value);

                json!({"$self": true})
            }
            "delete" => json!(instance.delete(&FuzzKey::from_json(&op.args[0])).is_some()),
            "get" => match instance.get(&FuzzKey::from_json(&op.args[0])) {
                Some(value) => value.to_json(),
                None => json!({"$undefined": true}),
            },
            "has" => json!(instance.has(&FuzzKey::from_json(&op.args[0]))),
            "clear" => {
                instance.clear();
                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let items: Vec<Value> = instance
            .items()
            .iter()
            .map(|(key, value)| json!([key.to_json(), value.to_json()]))
            .collect();
        let inverse_items: Vec<Value> = instance
            .inverse()
            .iter()
            .map(|(key, value)| json!([key.to_json(), value.to_json()]))
            .collect();

        json!({
            "size": instance.size(),
            "items": {"$map": items},
            "inverse": {
                "size": instance.inverse_size(),
                "items": {"$map": inverse_items},
                "inverse": {"$self": true},
            },
        })
    }
}
