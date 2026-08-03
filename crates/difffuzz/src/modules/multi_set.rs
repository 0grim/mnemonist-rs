//! [`ModuleSpec`] for `multi-set`.
//!
//! # What this grammar is for
//!
//! A **three-item pool** shared by `add`/`remove`/`set`/`edit`, so the same
//! item accumulates a multiplicity greater than one (the "a key genuinely
//! holds several values" state a campaign for this module has to reach) and
//! `remove`/`set`-to-non-positive routinely drive it back to zero and out of
//! `items` entirely.
//!
//! Counts are small positive integers, zero, and small negative integers —
//! `add`/`remove`'s sign-flip delegation (`add(x, -3)` becomes
//! `remove(x, 3)`) is otherwise unreachable. Fractional and `NaN` counts are
//! **not** in this grammar; see `mnemonist_core::structures::multi_set`'s
//! module docs for that permissiveness, and its native tests for direct
//! coverage — reaching it through JSON here would need a `{"$nan": true}`
//! counterpart on both the op-generation and op-consumption sides for a
//! quirk `test/multi-set.js` itself never exercises, and the campaign's
//! actual target (multiplicity collision and drain-to-zero) does not need
//! it.
//!
//! `top` is bounded to a small `n` so it never hits its own `n <= 0` guard —
//! that guard is a JavaScript `typeof`/arity check the bridge owns, already
//! covered by `mnemonist_napi::multi_set`'s native tests, not something this
//! core-level campaign can exercise at all (core's `top` takes an already-
//! validated `usize`).
//!
//! `isSubset`/`isSuperset` are static functions over two instances, which
//! this campaign's single-instance harness has no protocol for; both are
//! covered by native tests instead (`mnemonist_core::structures::multi_set`)
//! against the exact upstream examples in `test/multi-set.js`.

use mnemonist_core::structures::multi_set::MultiSet;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/multi-set.txt"
);

/// Three items: small enough that every op collides constantly.
const ITEM_POOL: usize = 3;

fn item_at(index: usize) -> Value {
    match index {
        0 => json!("a"),
        1 => json!("b"),
        _ => json!("c"),
    }
}

/// A small mixed pool of counts: positive (so multiplicities build up),
/// zero (a documented no-op on `add`/`remove`), and negative (the
/// sign-flip delegation to the other method).
fn count_at(index: usize) -> f64 {
    match index {
        0 => 1.0,
        1 => 2.0,
        2 => 4.0,
        3 => 0.0,
        4 => -1.0,
        _ => -3.0,
    }
}

pub struct MultiSetSpec;

impl ModuleSpec for MultiSetSpec {
    type Instance = MultiSet<String>;

    fn module(&self) -> &'static str {
        "multi-set"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "dimension", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(vec![]).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let item = (0..ITEM_POOL).prop_map(item_at);
        let count = (0..6usize).prop_map(count_at);

        prop_oneof![
            5 => (item.clone(), count.clone())
                .prop_map(|(i, c)| Op::new("add", vec![i, json!(c)])),
            3 => (item.clone(), count.clone())
                .prop_map(|(i, c)| Op::new("remove", vec![i, json!(c)])),
            2 => (item.clone(), count)
                .prop_map(|(i, c)| Op::new("set", vec![i, json!(c)])),
            2 => item.clone().prop_map(|i| Op::new("delete", vec![i])),
            2 => item.clone().prop_map(|i| Op::new("has", vec![i])),
            2 => item.clone().prop_map(|i| Op::new("multiplicity", vec![i])),
            1 => item.clone().prop_map(|i| Op::new("frequency", vec![i])),
            2 => (item.clone(), item.clone())
                .prop_map(|(a, b)| Op::new("edit", vec![a, b])),
            1 => Just(Op::new("clear", vec![])),
            1 => (1..=5usize).prop_map(|n| Op::new("top", vec![json!(n as u64)])),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        MultiSet::new()
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // Upstream's `add`/`remove` return value depends on the SIGN of
            // the raw argument, not on which of the two methods logically
            // ran: `add(x, -3)` returns whatever `remove(x, 3)` returns
            // (`undefined`, unconditionally), and `remove(x, -3)` returns
            // whatever `add(x, 3)` returns (`this`, unconditionally, since
            // the flipped count can never itself be negative again). See
            // `mnemonist_core::structures::multi_set`'s module docs.
            "add" => {
                let item = op.args[0].as_str().expect("item is a string").to_owned();
                let count = op.args[1].as_f64().expect("count is a JSON number");

                instance.add(item, count);

                if count < 0.0 {
                    json!({"$undefined": true})
                } else {
                    json!({"$self": true})
                }
            }
            "remove" => {
                let item = op.args[0].as_str().expect("item is a string").to_owned();
                let count = op.args[1].as_f64().expect("count is a JSON number");

                instance.remove(item, count);

                if count < 0.0 {
                    json!({"$self": true})
                } else {
                    json!({"$undefined": true})
                }
            }
            "set" => {
                let item = op.args[0].as_str().expect("item is a string").to_owned();
                let count = op.args[1].as_f64().expect("count is a JSON number");

                instance.set(item, count);

                json!({"$self": true})
            }
            "delete" => {
                let item = op.args[0].as_str().expect("item is a string");

                json!(instance.delete(&item.to_owned()))
            }
            "has" => {
                let item = op.args[0].as_str().expect("item is a string");

                json!(instance.has(&item.to_owned()))
            }
            "multiplicity" => {
                let item = op.args[0].as_str().expect("item is a string");

                number_json(instance.multiplicity(&item.to_owned()))
            }
            "frequency" => {
                let item = op.args[0].as_str().expect("item is a string");

                number_json(instance.frequency(&item.to_owned()))
            }
            // Upstream returns `undefined` when `a` is absent (its early
            // `if (am === 0) return;`) and `this` only on the path that
            // actually edits something.
            "edit" => {
                let a = op.args[0].as_str().expect("item is a string").to_owned();
                let b = op.args[1].as_str().expect("item is a string").to_owned();
                let a_was_present = instance.multiplicity(&a) != 0.0;

                instance.edit(a, b);

                if a_was_present {
                    json!({"$self": true})
                } else {
                    json!({"$undefined": true})
                }
            }
            "clear" => {
                instance.clear();
                json!({"$undefined": true})
            }
            "top" => {
                let n = op.args[0].as_u64().expect("n is a JSON integer") as usize;
                let survivors = instance.top(n).expect("this grammar only generates n >= 1");

                json!(survivors
                    .into_iter()
                    .map(|(item, count)| json!([item, number_json(count)]))
                    .collect::<Vec<_>>())
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let items: Vec<Value> = instance
            .items()
            .iter()
            .map(|(item, count)| json!([item, number_json(*count)]))
            .collect();

        json!({
            "size": number_json(instance.size()),
            "dimension": instance.dimension(),
            "items": {"$map": items},
        })
    }
}

/// A JavaScript number, encoded as `fuzz/oracle.js`'s `encode()` would: a
/// `NaN` this grammar never generates but `#.delete`'s BUG-MULTI-SET-2 bug (NOTES.md)
/// can still produce as *observed state* is rendered the same way the
/// oracle renders a real one, so a divergence there is comparable rather
/// than a JSON-encoding failure.
fn number_json(value: f64) -> Value {
    if value.is_nan() {
        return json!({"$nan": true});
    }

    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}

/// Direct evidence that this grammar reaches multiplicity greater than one
/// and drains an item back out of `items` entirely -- the two states a
/// campaign for this module has to reach. No oracle involved; this is about
/// the grammar's own reach.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    #[test]
    fn the_grammar_builds_multiplicity_above_one_and_drains_items_to_zero() {
        let spec = MultiSetSpec;
        let mut runner = TestRunner::new(Config::default());
        let mut multiplicity_above_one = 0u64;
        let mut items_drained_to_zero = 0u64;

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
                let target = matches!(op.name, "remove" | "set" | "delete")
                    .then(|| op.args[0].as_str().expect("item is a string").to_owned());
                let was_present = target
                    .as_ref()
                    .map(|item| instance.has(item))
                    .unwrap_or(false);

                spec.apply(&mut instance, op);

                if instance.items().iter().any(|(_, count)| *count > 1.0) {
                    multiplicity_above_one += 1;
                }

                if was_present {
                    let item = target.expect("was_present implies a target");

                    if !instance.has(&item) {
                        items_drained_to_zero += 1;
                    }
                }
            }
        }

        eprintln!(
            "multi-set grammar: {multiplicity_above_one} steps with a \
             multiplicity above one, {items_drained_to_zero} items drained \
             to zero and removed"
        );

        assert!(
            multiplicity_above_one > 100,
            "the grammar should routinely build a multiplicity above one, \
             not rarely: only {multiplicity_above_one} observations over \
             400 programs"
        );
        assert!(
            items_drained_to_zero > 50,
            "the grammar should routinely drain an item to zero, not \
             rarely: only {items_drained_to_zero} over 400 programs"
        );
    }
}
