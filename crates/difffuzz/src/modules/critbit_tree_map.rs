//! [`ModuleSpec`] for `critbit-tree-map`.
//!
//! # The grammar exists to reach two states: shared prefixes, and deep
//! critical-bit positions
//!
//! DESIGN.md's own warning, restated for this module by the porting brief: a
//! crit-bit tree fuzzed with long random strings proves only that the
//! structure can store unrelated words — every critical bit lands near byte
//! index zero, and the tree never gets deep enough to exercise its own
//! bubble-up rotation. Neither is where this module's bugs live.
//!
//! [`PREFIX_POOL`] is a small, hand-built alphabet chosen so it already
//! contains BOTH shapes before a single op has run:
//!
//! ```text
//! a, ab, abc, abcd, abcda, abcdb, b, ba
//! ```
//!
//! `"a"` is a strict prefix of `"ab"`, `"abc"`, `"abcd"`, `"abcda"` and
//! `"abcdb"`; `"ab"` of the last four of those; `"abc"` of the last three;
//! `"abcd"` of the last two; `"b"` of `"ba"`. **5 of the 8 pool entries are
//! themselves a strict prefix of another** — measured below, not eyeballed,
//! the same threshold `trie`'s own pool was measured against — and
//! `"abcda"`/`"abcdb"` differ **only in their last byte**, which is exactly
//! the deep-critical-bit shape a random-length-distinct-strings grammar
//! cannot produce: their critical bit is at byte index 4, one byte short of
//! the pool's longest entries, and finding it exercises the exact bubble-up
//! comparison this module's own gate 6 falsification targets.
//!
//! # Observable state: `size` and `root`
//!
//! `root` is upstream's own property — a real, argument-free one, unlike
//! every other method here — so it is the one place a full *structural*
//! comparison is possible through the oracle's generic property-read
//! protocol. See `mnemonist_core::structures::critbit_tree_map::RootNode`'s
//! docs for why its `critbit` field is upstream's own packed
//! `(byteIndex << 8) | mask` integer rather than this port's internal
//! `(byte_index, mask)` tuple: reassembling that exact number is what turns
//! a `root` mismatch into a critical-bit-computation bug report rather than
//! a rendering one.
//!
//! # What this grammar deliberately does not cover
//!
//! **`forEach`'s own reentrancy.** No op here drives a callback that mutates
//! the tree mid-walk (`_utils`'s and `sparse-set`'s `$forEach` machinery,
//! B-31's family). `forEach`'s *ordering* is covered instead by
//! `mnemonist_core::structures::critbit_tree_map`'s own native test
//! (`for_each_visits_in_sorted_key_order`) and by gate 4's "should be
//! possible to iterate over the tree." block; nothing here calls it at all,
//! since `root`'s structural comparison already implies the same ordering a
//! correct inorder walk would produce.
//!
//! **Non-Latin-1 keys.** `mnemonist_napi::critbit_tree_map::decode_key`
//! truncates each UTF-16 code unit to its low 8 bits (D-245); fuzzing wider
//! code points would only re-report that already-disclosed divergence.
//! [`PREFIX_POOL`] is plain ASCII so this campaign can never manufacture one
//! by accident.

use std::collections::BTreeMap;

use mnemonist_core::structures::critbit_tree_map::{CritBitTreeMap as CoreMap, RootNode};
use proptest::prelude::*;
use serde_json::{json, Map as JsonMap, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/critbit-tree-map.txt"
);

/// Every key a generated program can use. See the module docs — this pool
/// is the entire mechanism behind both "reach shared prefixes" and "reach a
/// deep critical bit".
pub(crate) const PREFIX_POOL: &[&str] = &["a", "ab", "abc", "abcd", "abcda", "abcdb", "b", "ba"];

/// A stored value slot. `None` is `undefined`, matching
/// `mnemonist_napi::critbit_tree_map::Value` — a stored key can hold it, and
/// `has` (presence) does not care which this is.
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

/// Upstream's own packed `(byteIndex << 8) | mask` integer and object shape,
/// reassembled from [`RootNode`] — see the module docs.
fn root_json(node: &RootNode<'_, Slot>) -> Value {
    match node {
        RootNode::Empty => Value::Null,
        RootNode::External { key, value } => {
            let key_string: String = key.iter().map(|&byte| byte as char).collect();

            json!({"key": key_string, "value": slot_json(value)})
        }
        RootNode::Internal {
            critbit,
            left,
            right,
        } => {
            json!({
                "critbit": critbit,
                "left": root_json(left),
                "right": root_json(right),
            })
        }
    }
}

pub struct CritBitTreeMapSpec;

impl ModuleSpec for CritBitTreeMapSpec {
    type Instance = CoreMap<Slot>;

    fn module(&self) -> &'static str {
        "critbit-tree-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "root"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // `new CritBitTreeMap()` takes no arguments.
        Just(Vec::new()).boxed()
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
            6 => (key.clone(), value).prop_map(|(k, v)| Op::new("set", vec![k, v])),
            4 => key.clone().prop_map(|k| Op::new("get", vec![k])),
            4 => key.clone().prop_map(|k| Op::new("has", vec![k])),
            4 => key.prop_map(|k| Op::new("delete", vec![k])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        CoreMap::new()
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let key = op.args[0].as_str().expect("key is a string");
                let value = slot_from_json(&op.args[1]);

                instance.set(key.as_bytes().to_vec(), value);

                json!({"$self": true})
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
            "delete" => {
                let key = op.args[0].as_str().expect("key is a string");

                json!(instance.delete(key.as_bytes()).is_some())
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
        state.insert("root".into(), root_json(&instance.root()));

        Value::Object(
            state
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}

/// Measures, rather than asserts by eye, that [`PREFIX_POOL`] contains real
/// prefix relationships and a genuinely deep critical-bit pair — the entire
/// mechanism behind this module's two "reach an interesting state" claims.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_self_check_most_entries_are_a_prefix_of_another_entry() {
        let self_prefixing: Vec<&str> = PREFIX_POOL
            .iter()
            .copied()
            .filter(|candidate| {
                PREFIX_POOL
                    .iter()
                    .any(|other| other != candidate && other.starts_with(candidate))
            })
            .collect();

        eprintln!(
            "critbit-tree-map pool self-check: {}/{} entries are a strict prefix of another \
             entry: {:?}",
            self_prefixing.len(),
            PREFIX_POOL.len(),
            self_prefixing
        );

        assert!(
            self_prefixing.len() * 2 >= PREFIX_POOL.len(),
            "PREFIX_POOL must be mostly self-prefixing, or this grammar only proves the tree \
             stores unrelated words; got {self_prefixing:?} out of {PREFIX_POOL:?}"
        );
    }

    /// The deep-critical-bit claim: at least one pair in the pool differs
    /// only in its last byte, which forces the critical bit to the deepest
    /// position that pair's shared prefix allows.
    #[test]
    fn pool_self_check_contains_a_pair_differing_only_in_the_last_byte() {
        let deep_pairs: Vec<(&str, &str)> = PREFIX_POOL
            .iter()
            .flat_map(|&a| PREFIX_POOL.iter().map(move |&b| (a, b)))
            .filter(|(a, b)| a != b && a.len() == b.len() && a[..a.len() - 1] == b[..b.len() - 1])
            .collect();

        eprintln!("critbit-tree-map pool self-check: deep (last-byte-only) pairs: {deep_pairs:?}");

        assert!(
            !deep_pairs.is_empty(),
            "PREFIX_POOL must contain at least one pair differing only in its last byte, or \
             this grammar cannot reach a deep critical-bit position at all: {PREFIX_POOL:?}"
        );
    }

    /// The dynamic half: sample real generated `set` keys (not just the
    /// static pool) and confirm the op stream itself revisits prefix
    /// relationships often.
    #[test]
    fn pool_self_check_generated_programs_revisit_prefix_relationships() {
        let spec = CritBitTreeMapSpec;
        let mut runner =
            proptest::test_runner::TestRunner::new(proptest::test_runner::Config::default());
        let strategy = spec.op_strategy(&[]);

        let mut set_keys = Vec::new();

        for _ in 0..2_000 {
            let tree = strategy
                .new_tree(&mut runner)
                .expect("strategy generates a value");
            let op = tree.current();

            if op.name == "set" {
                set_keys.push(op.args[0].as_str().unwrap().to_string());
            }
        }

        let related = set_keys
            .iter()
            .filter(|candidate| {
                set_keys
                    .iter()
                    .any(|other| *other != **candidate && other.starts_with(candidate.as_str()))
            })
            .count();

        eprintln!(
            "critbit-tree-map grammar self-check: {related}/{} generated `set` keys are a \
             strict prefix of another generated `set` key",
            set_keys.len()
        );

        assert!(
            related * 2 >= set_keys.len(),
            "generated `set` ops must mostly revisit prefix relationships: {related}/{} \
             qualified",
            set_keys.len()
        );
    }
}
