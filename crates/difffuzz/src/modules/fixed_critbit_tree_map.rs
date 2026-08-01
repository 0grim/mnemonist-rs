//! [`ModuleSpec`] for `fixed-critbit-tree-map`.
//!
//! Shares [`crate::modules::critbit_tree_map::PREFIX_POOL`] directly rather
//! than re-deriving it — same reasoning as `trie` sharing `trie_map`'s pool:
//! one measured prefix pool, not two copies that could quietly drift apart.
//!
//! # Capacity is chosen small enough that it is always reached
//!
//! [`ModuleSpec::ctor_strategy`] draws `capacity` from `2..=5` against an
//! 8-entry key pool, and [`ModuleSpec::program_len`] runs up to 200 ops —
//! so a program that ever calls `set` with more than `capacity` *distinct*
//! keys (overwhelmingly likely; the pool alone has eight) drives this
//! module straight into the state
//! `mnemonist_core::structures::fixed_critbit_tree_map`'s own module docs
//! describe: a silent corruption on the key that pushes past capacity, then
//! [`Error::Corrupted`] — upstream's own crash — on whichever later `set`
//! call walks back through it. The `pool_self_check_capacity_is_actually_
//! exceeded` test below measures this directly over real constructed
//! instances and real generated programs, not just the op arguments: both
//! "size grew past capacity at least once" and "a `set` call actually hit
//! `Error::Corrupted` at least once" are asserted over a real sample, the
//! same discipline `trie`'s and `trie-map`'s own pool self-checks use.
//!
//! # `Error::Corrupted` is compared as a thrown error, not specially
//!
//! [`ModuleSpec::apply`] encodes it as `{"$throw": <message>}` — exactly the
//! shape `fuzz/oracle.js` already produces for any op whose call throws
//! (`catch (error) { result = {$throw: ...}; }`), so no oracle-side change
//! is needed at all: upstream's own crash and this port's `Err` compare as
//! the same kind of event automatically, and the *text* comparing equal is
//! what proves the message itself was reproduced, not just "both sides
//! failed somehow".
//!
//! # Observable state: `size` and `root`
//!
//! `root` here is a raw pointer (a plain number), not a nested object —
//! `fixed-critbit-tree-map.js` never builds `InternalNode`/`ExternalNode`
//! objects at all (see the core module's own `root` doc comment) — so
//! comparing it is simultaneously a check that this port's internal node
//! indices are allocated in the same order upstream's `this.offset++`/
//! `this.size++` counters would be, including through a capacity overflow.
//!
//! # What this grammar deliberately does not cover
//!
//! **`delete`.** `fixed-critbit-tree-map.js` has no such method; there is
//! nothing to fuzz.
//!
//! **Non-Latin-1 keys and `forEach` reentrancy.** Identical reasoning to
//! `critbit_tree_map`'s own grammar; see that module's docs.

use std::collections::BTreeMap;

use mnemonist_core::structures::fixed_critbit_tree_map::{
    Error as CoreError, FixedCritBitTreeMap as CoreMap,
};
use proptest::prelude::*;
use serde_json::{json, Map as JsonMap, Value};

use crate::modules::critbit_tree_map::PREFIX_POOL;
use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fixed-critbit-tree-map.txt"
);

/// Capacities small enough, against an 8-entry pool, that overflow is all
/// but guaranteed in any program of a useful length. See the module docs.
const MIN_CAPACITY: usize = 2;
const MAX_CAPACITY: usize = 5;

type Slot = Option<Value>;

fn slot_json(slot: &Slot) -> Value {
    match slot {
        None => json!({"$undefined": true}),
        Some(value) => value.clone(),
    }
}

fn slot_from_json(value: &Value) -> Slot {
    match value {
        Value::Object(fields) if fields.contains_key("$undefined") => None,
        other => Some(other.clone()),
    }
}

pub struct FixedCritBitTreeMapSpec;

impl ModuleSpec for FixedCritBitTreeMapSpec {
    type Instance = CoreMap<Slot>;

    fn module(&self) -> &'static str {
        "fixed-critbit-tree-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "root"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (MIN_CAPACITY..=MAX_CAPACITY)
            .prop_map(|capacity| vec![json!(capacity)])
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let key = (0..PREFIX_POOL.len()).prop_map(|index| json!(PREFIX_POOL[index]));
        let value = prop_oneof![
            2 => Just(json!({"$undefined": true})),
            1 => Just(Value::Null),
            3 => (0i64..5).prop_map(|n| json!(n)),
            2 => Just(json!("v")),
        ];

        prop_oneof![
            7 => (key.clone(), value).prop_map(|(k, v)| Op::new("set", vec![k, v])),
            4 => key.clone().prop_map(|k| Op::new("get", vec![k])),
            4 => key.prop_map(|k| Op::new("has", vec![k])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        // Long enough that, against a capacity as low as 2, overflow is
        // reached almost every run rather than merely possible.
        1..200
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let capacity = args[0].as_u64().expect("ctor arg 0 is the capacity") as usize;

        CoreMap::new(capacity).expect("generated capacities are always positive")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let key = op.args[0].as_str().expect("key is a string");
                let value = slot_from_json(&op.args[1]);

                match instance.set(key.as_bytes().to_vec(), value) {
                    Ok(_displaced) => json!({"$self": true}),
                    // Same shape `fuzz/oracle.js` already produces for a
                    // thrown call -- see the module docs.
                    Err(CoreError::Corrupted) => {
                        json!({"$throw": CoreError::Corrupted.to_string()})
                    }
                    Err(other) => panic!("set() cannot fail this way once constructed: {other}"),
                }
            }
            "get" => {
                let key = op.args[0].as_str().expect("key is a string");

                match instance.get(key.as_bytes()) {
                    Some(slot) => slot_json(slot),
                    None => json!({"$undefined": true}),
                }
            }
            "has" => {
                let key = op.args[0].as_str().expect("key is a string");

                json!(instance.has(key.as_bytes()))
            }
            "clear" => {
                instance.clear();

                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let mut state = JsonMap::new();
        state.insert("size".into(), json!(instance.size()));
        state.insert("root".into(), json!(instance.root()));

        Value::Object(
            state
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}

/// Measures, rather than assumes, that capacity is actually exceeded by
/// real constructed instances driven by real generated programs — the
/// entire mechanism behind this module's "the interesting part is capacity"
/// claim.
#[cfg(test)]
mod tests {
    use proptest::test_runner::TestRunner;

    use super::*;

    #[test]
    fn pool_self_check_capacity_is_actually_exceeded_and_hits_the_crash() {
        let spec = FixedCritBitTreeMapSpec;
        let mut runner = TestRunner::new(proptest::test_runner::Config::default());

        let ctor_strategy = spec.ctor_strategy();
        let op_strategy_source = spec.op_strategy(&[]);

        let mut programs_run = 0;
        let mut programs_exceeding_capacity = 0;
        let mut programs_hitting_corrupted = 0;

        for _ in 0..500 {
            let ctor_tree = ctor_strategy
                .new_tree(&mut runner)
                .expect("ctor strategy generates a value");
            let ctor = ctor_tree.current();
            let mut instance = spec.construct(&ctor);
            let capacity = ctor[0].as_u64().unwrap() as usize;

            let mut exceeded = false;
            let mut corrupted = false;

            for _ in 0..80 {
                let op_tree = op_strategy_source
                    .new_tree(&mut runner)
                    .expect("op strategy generates a value");
                let op = op_tree.current();

                let result = spec.apply(&mut instance, &op);

                if instance.size() > capacity {
                    exceeded = true;
                }
                if result.get("$throw").is_some() {
                    corrupted = true;
                }
            }

            programs_run += 1;
            if exceeded {
                programs_exceeding_capacity += 1;
            }
            if corrupted {
                programs_hitting_corrupted += 1;
            }
        }

        eprintln!(
            "fixed-critbit-tree-map pool self-check: {programs_exceeding_capacity}/{programs_run} \
             programs exceeded capacity; {programs_hitting_corrupted}/{programs_run} hit \
             Error::Corrupted at least once"
        );

        assert!(
            programs_exceeding_capacity * 2 >= programs_run,
            "capacity must be exceeded in most generated programs, or this grammar only proves \
             the tree works within capacity: {programs_exceeding_capacity}/{programs_run}"
        );
        assert!(
            programs_hitting_corrupted * 2 >= programs_run,
            "the capacity-overflow crash must actually be reached in most generated programs, \
             not merely in principle: {programs_hitting_corrupted}/{programs_run}"
        );
    }
}
