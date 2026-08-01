//! [`ModuleSpec`] for `multi-array`.
//!
//! # What this grammar is for
//!
//! A **ten-index pool** shared by `set`/`push`/`get`/`values`/`entries`, so
//! the same index is hit repeatedly and its bucket accumulates several
//! values (the state CLAUDE.md's fuzz-campaign guidance asks for directly:
//! measured below by `grammar_self_check`). The constructor alternates
//! between the default dynamic container and a fixed-capacity
//! `Uint8Array`/`Uint16Array`/`Uint32Array` one (small capacities, so a
//! `push`/`set` past it is common and the capacity throw is exercised, not
//! just the growable path).
//!
//! `get` renders a container exactly as `fuzz/oracle.js`'s `encode()`
//! renders the real JS value: a plain array in dynamic mode,
//! `{"$typed": ..., "values": [...]}` in fixed mode (`ArrayBuffer.isView`),
//! matching what a real typed-array *view* upstream hands back.
//!
//! # Deliberately excluded: `containers`/`associations`/`values`/`entries`/`keys`
//!
//! All five build a genuine `obliterator`-shaped iterator upstream (see
//! `crates/mnemonist-napi/src/multi_array.rs`'s module docs for the bridge
//! bug this fact itself caught), which `fuzz/oracle.js`'s `encode()` has no
//! special case for and reduces to `{}` on both sides — a comparison that
//! can only ever agree trivially and would add cases without adding
//! signal, the same reasoning `multi-map`'s spec gives for excluding its own
//! cursor-lifecycle ops. `get`/`has`/`multiplicity` already exercise the
//! same underlying bucket-walk logic these methods share (`get` performs an
//! identical tail-to-head walk to `containers`' per-bucket read), and
//! `test/multi-array.js` itself, run unmodified by gate 4, is what actually
//! pins these five methods' own behaviour.

use mnemonist_core::structures::multi_array::{CapacityExceeded, MultiArray as CoreMultiArray};
use mnemonist_core::utils::typed_arrays::PointerWidth;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/multi-array.txt"
);

/// Ten indices: small enough that `set`/`push` collide on the same bucket
/// constantly, wide enough that `push`'s own dimension-appending path (which
/// never revisits an index) still has room to run several times.
const INDEX_POOL: u32 = 10;

/// Values run past 255/65535 so `Uint8Array`/`Uint16Array` truncation is
/// exercised in fixed mode.
const MAX_VALUE: f64 = 70_000.0;

/// Small on purpose: a fixed-capacity instance should run out of room
/// routinely, not rarely.
const MAX_CAPACITY: u32 = 12;

pub struct MultiArraySpec;

/// The Rust instance under test, plus the width the constructor resolved --
/// needed only to render `get`/`containers`/`associations` the way the
/// bridge would (see the module docs).
pub struct Instance {
    inner: CoreMultiArray,
    width: Option<PointerWidth>,
}

impl ModuleSpec for MultiArraySpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "multi-array"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "dimension"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            3 => Just(vec![]),
            2 => (
                prop::sample::select(&["Uint8Array", "Uint16Array", "Uint32Array"][..]),
                1u32..MAX_CAPACITY,
            )
                .prop_map(|(class, capacity)| {
                    vec![json!({ "$global": class }), json!(capacity)]
                }),
        ]
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let index = 0u32..INDEX_POOL;

        prop_oneof![
            5 => (index.clone(), 0.0..MAX_VALUE)
                .prop_map(|(i, v)| Op::new("set", vec![json!(i), json!(v)])),
            4 => (0.0..MAX_VALUE).prop_map(|v| Op::new("push", vec![json!(v)])),
            3 => index.clone().prop_map(|i| Op::new("get", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("has", vec![json!(i)])),
            2 => index.prop_map(|i| Op::new("multiplicity", vec![json!(i)])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        if args.is_empty() {
            return Instance {
                inner: CoreMultiArray::new(),
                width: None,
            };
        }

        let width = match args[0]["$global"].as_str() {
            Some("Uint8Array") => PointerWidth::U8,
            Some("Uint16Array") => PointerWidth::U16,
            Some("Uint32Array") => PointerWidth::U32,
            other => panic!("ctor arg 0 is a supported array class, got {other:?}"),
        };
        let capacity = number(&args[1]);

        Instance {
            inner: CoreMultiArray::fixed(width, capacity),
            width: Some(width),
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let index = number(&op.args[0]);
                let value = number_f64(&op.args[1]);

                match instance.inner.set(index, value) {
                    Ok(()) => json!({"$self": true}),
                    Err(error) => thrown(&error),
                }
            }
            "push" => match instance.inner.push(number_f64(&op.args[0])) {
                Ok(()) => json!({"$self": true}),
                Err(error) => thrown(&error),
            },
            "get" => match instance.inner.get(number(&op.args[0])) {
                Some(values) => render_bucket(&values, instance.width),
                None => json!({"$undefined": true}),
            },
            "has" => json!(instance.inner.has(number(&op.args[0]))),
            "multiplicity" => json!(instance.inner.multiplicity(number(&op.args[0])) as u64),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.inner.size(),
            "dimension": instance.inner.dimension(),
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

/// Same rendering `sort`/`default_map`/`bloom_filter`/`set`/`vector` all use
/// (CLAUDE.md: grep before inventing shared machinery -- duplicated here
/// rather than factored out, matching the existing pattern in this crate):
/// a whole number in the JS safe-integer range prints without a decimal
/// point, matching `JSON.stringify`.
fn number_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}

/// A thrown error, in the shape `fuzz/oracle.js` reports one.
fn thrown(error: &CapacityExceeded) -> Value {
    json!({ "$throw": error.to_string() })
}

/// A container exactly as `fuzz/oracle.js`'s `encode()` renders the real
/// value the bridge hands back: a plain array of numbers in dynamic mode, or
/// `{"$typed": "<class>", "values": [...]}` in fixed mode -- the
/// `ArrayBuffer.isView` branch, since `get`/`containers`/`associations`
/// build a real typed-array *view*, unlike `values`/`entries`.
fn render_bucket(values: &[f64], width: Option<PointerWidth>) -> Value {
    match width {
        None => json!(values.iter().copied().map(number_json).collect::<Vec<_>>()),
        Some(width) => {
            let class = match width {
                PointerWidth::U8 => "Uint8Array",
                PointerWidth::U16 => "Uint16Array",
                PointerWidth::U32 => "Uint32Array",
            };
            let narrowed: Vec<u64> = values.iter().map(|&v| v as u64).collect();

            json!({ "$typed": class, "values": narrowed })
        }
    }
}

/// Direct evidence that this grammar reaches the states CLAUDE.md's
/// fuzz-campaign guidance asks for: a bucket genuinely holding several
/// values, and a fixed-capacity instance actually running out of room (not
/// just accepting every `push`/`set` up to some huge, never-hit ceiling).
/// Runs the strategies directly, no oracle, no `node` -- this is about the
/// grammar's own reach, not port-vs-upstream agreement.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    #[test]
    fn the_grammar_builds_multi_value_buckets_and_exhausts_fixed_capacity() {
        let spec = MultiArraySpec;
        let mut runner = TestRunner::new(Config::default());
        let mut multi_value_observations = 0u64;
        let mut capacity_exceeded_hits = 0u64;

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
                let result = spec.apply(&mut instance, op);

                if result
                    .get("$throw")
                    .and_then(|v| v.as_str())
                    .map(|m| m.contains("capacity"))
                    .unwrap_or(false)
                {
                    capacity_exceeded_hits += 1;
                }

                if instance.inner.containers().iter().any(|c| c.len() > 1) {
                    multi_value_observations += 1;
                }
            }
        }

        eprintln!(
            "multi-array grammar: {multi_value_observations} steps with a \
             multi-value bucket, {capacity_exceeded_hits} capacity-exceeded throws"
        );

        assert!(
            multi_value_observations > 100,
            "the grammar should routinely build multi-value buckets, not \
             rarely: only {multi_value_observations} observations over 400 \
             programs"
        );
        assert!(
            capacity_exceeded_hits > 20,
            "the grammar should routinely exhaust a fixed instance's \
             capacity, not rarely: only {capacity_exceeded_hits} over 400 \
             programs"
        );
    }
}
