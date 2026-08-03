//! [`ModuleSpec`] for `multi-map`.
//!
//! # What this grammar is for
//!
//! A **three-key pool** shared by every `set`/`remove` call, so the same key
//! is hit repeatedly and its bucket accumulates several values before
//! anything empties it back out — the two states a campaign for this unit
//! has to reach: a key genuinely holding several values, and a container
//! emptying to zero via `remove`/`delete`.
//! `remove`/`delete` are weighted in for the same reason.
//!
//! The constructor alternates between the default (`List`-kind) and `Set`
//! (via `{"$global": "Set"}`, resolved by `fuzz/oracle.js`'s
//! `decodeCtorArg`), so both bucket kinds get their own campaign share.
//!
//! # Deliberately excluded
//!
//! **Cursor lifecycle ops (`$iter`/`$next`/`$spread`)**, for the same reason
//! `bi-map`'s spec excludes them: expressing them here needs the module's own
//! stored cursor state (`sparse_map`'s `Instance` wrapper is the pattern), and
//! `keys()`/`values()`/`entries()`/`containers()`/`associations()` are all
//! already covered by `mnemonist_napi::multi_map`'s own Rust tests and by
//! `test/multi-map.js` itself, which asserts every one of them. What this
//! campaign is for is the *bucket* bookkeeping — `size`/`dimension`/the
//! per-key contents — which every op below still observes in full through
//! `observe()`.

use mnemonist_core::structures::multi_map::{Bucket, ContainerKind, MultiMap};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/multi-map.txt"
);

/// Three keys: small enough that `set`/`remove` collide constantly.
const KEY_POOL: usize = 3;

/// Four values, mixed types (`FuzzKey::from_json` mirrors `mnemonist_napi::
/// js_key::JsKey`'s SameValueZero), wide enough that a `Set`-kind bucket sees
/// genuine duplicates and genuine distinct members both.
const VALUE_POOL: usize = 4;

/// A `Map`/bucket-member key with SameValueZero — a mirror of
/// `mnemonist_napi::js_key::JsKey`, kept local for the same reason `bi_map`'s
/// own `FuzzKey` is: the real type lives in the `cdylib` bridge crate.
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
        _ => json!("c"),
    }
}

fn value_at(index: usize) -> Value {
    match index {
        0 => json!("hello"),
        1 => json!("world"),
        2 => json!(0),
        _ => json!(1),
    }
}

/// Render a bucket exactly as `fuzz/oracle.js`'s `encode()` renders the real
/// `Array`/`Set` it stands for.
fn render_bucket(bucket: &Bucket<FuzzKey>) -> Value {
    let values: Vec<Value> = bucket.values().iter().map(FuzzKey::to_json).collect();

    match bucket.kind() {
        ContainerKind::List => json!(values),
        ContainerKind::Set => json!({"$set": values}),
    }
}

pub struct MultiMapSpec;

impl ModuleSpec for MultiMapSpec {
    type Instance = MultiMap<FuzzKey, FuzzKey>;

    fn module(&self) -> &'static str {
        "multi-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "dimension", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            3 => Just(vec![]),
            2 => Just(vec![json!({"$global": "Set"})]),
        ]
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let key = (0..KEY_POOL).prop_map(key_at);
        let value = (0..VALUE_POOL).prop_map(value_at);

        prop_oneof![
            // Weighted towards `set`: it is the only op that grows a bucket.
            5 => (key.clone(), value.clone())
                .prop_map(|(k, v)| Op::new("set", vec![k, v])),
            3 => (key.clone(), value.clone())
                .prop_map(|(k, v)| Op::new("remove", vec![k, v])),
            2 => key.clone().prop_map(|k| Op::new("delete", vec![k])),
            2 => key.clone().prop_map(|k| Op::new("has", vec![k])),
            2 => key.clone().prop_map(|k| Op::new("get", vec![k])),
            2 => key.prop_map(|k| Op::new("multiplicity", vec![k])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let kind = match args.first().and_then(|arg| arg.get("$global")) {
            Some(_) => ContainerKind::Set,
            None => ContainerKind::List,
        };

        MultiMap::new(kind)
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let key = FuzzKey::from_json(&op.args[0]);
                let value = FuzzKey::from_json(&op.args[1]);

                instance.set(key, value);

                json!({"$self": true})
            }
            "remove" => {
                let key = FuzzKey::from_json(&op.args[0]);
                let value = FuzzKey::from_json(&op.args[1]);

                json!(instance.remove(key, &value))
            }
            "delete" => json!(instance.delete(&FuzzKey::from_json(&op.args[0]))),
            "has" => json!(instance.has(&FuzzKey::from_json(&op.args[0]))),
            "get" => match instance.get(&FuzzKey::from_json(&op.args[0])) {
                Some(bucket) => render_bucket(bucket),
                None => json!({"$undefined": true}),
            },
            "multiplicity" => {
                json!(instance.multiplicity(&FuzzKey::from_json(&op.args[0])) as u64)
            }
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
            .map(|(key, bucket)| json!([key.to_json(), render_bucket(bucket)]))
            .collect();

        json!({
            "size": instance.size(),
            "dimension": instance.dimension(),
            "items": {"$map": items},
        })
    }
}

/// Direct evidence that this grammar reaches the two states a campaign for
/// this unit has to reach: a key genuinely holding several values,
/// and a bucket emptying back to zero (and leaving `items`) via `remove`/
/// `delete`. Runs the strategies directly, with no oracle involved — this is
/// about the grammar's own reach, not about port-vs-upstream agreement.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    #[test]
    fn the_grammar_builds_multi_value_keys_and_drains_them_to_zero() {
        let spec = MultiMapSpec;
        let mut runner = TestRunner::new(Config::default());
        let mut multi_value_observations = 0u64;
        let mut keys_emptied_via_remove_or_delete = 0u64;

        for _ in 0..400 {
            let ctor = spec
                .ctor_strategy()
                .new_tree(&mut runner)
                .expect("ctor_strategy never rejects")
                .current();
            let ops = proptest::collection::vec(spec.op_strategy(&ctor), 1..300)
                .new_tree(&mut runner)
                .expect("op_strategy never rejects")
                .current();
            let mut instance = spec.construct(&ctor);

            for op in &ops {
                let had_key = matches!(op.name, "remove" | "delete")
                    .then(|| instance.has(&FuzzKey::from_json(&op.args[0])))
                    .unwrap_or(false);

                spec.apply(&mut instance, op);

                if instance.items().iter().any(|(_, bucket)| bucket.len() > 1) {
                    multi_value_observations += 1;
                }

                if had_key
                    && matches!(op.name, "remove" | "delete")
                    && !instance.has(&FuzzKey::from_json(&op.args[0]))
                {
                    keys_emptied_via_remove_or_delete += 1;
                }
            }
        }

        eprintln!(
            "multi-map grammar: {multi_value_observations} steps with a \
             multi-value bucket, {keys_emptied_via_remove_or_delete} keys \
             drained to zero and removed from items"
        );

        assert!(
            multi_value_observations > 100,
            "the grammar should routinely build multi-value buckets, not \
             rarely: only {multi_value_observations} observations over 400 \
             programs"
        );
        assert!(
            keys_emptied_via_remove_or_delete > 100,
            "the grammar should routinely drain a bucket to zero, not \
             rarely: only {keys_emptied_via_remove_or_delete} over 400 \
             programs"
        );
    }
}
