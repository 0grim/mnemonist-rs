//! [`ModuleSpec`] for `fuzzy-multi-map`.
//!
//! # What this grammar is for
//!
//! `add`/`set` hash their argument through a **named factory** before
//! storing — the same device `fuzzy-map`'s campaign uses, and for the same
//! reason: the hash function travels as `{"$factory": "fuzzyLower"}` and
//! `fuzz/oracle.js` resolves it against its own `FACTORIES` table, so both
//! sides run the identical function rather than two hand-written mirrors of
//! it that could quietly disagree.
//!
//! This module wraps `multi-map`, so the same collision-heavy shape applies
//! one level up: a **three-string item pool**, lowercased by the hash
//! function, so `'Hello'`/`'HELLO'`/`'hello'` all land in the one bucket —
//! several distinct-looking `add`s becoming one key holding several values,
//! and `.clear()` weighted in to empty it back out.
//!
//! # Deliberately excluded
//!
//! **`Set`-kind membership by object identity is not fuzzable through this
//! protocol at all.** `mnemonist_napi::fuzzy_multi_map`'s own `same_value_zero`
//! exists because upstream's `Set`-container test stores plain *objects*
//! (`{title: 'Hello1'}`) and dedups by reference identity — a JavaScript
//! concern with no core-level counterpart, since `FuzzyMultiMap<K, V>` is
//! generic over `V` and this campaign, like `multi-map`'s, drives it with a
//! plain `PartialEq` item (`String`) through the infallible `set_with`
//! convenience path. That path is exactly what `multi-map`'s own campaign
//! already exercises for `Set`-kind dedup; what is unique to this module —
//! the hash-before-store step — is what this grammar actually targets.
//! Object-identity dedup itself is covered by `mnemonist_napi::
//! fuzzy_multi_map`'s own native test and by `test/fuzzy-multi-map.js`'s
//! `Set`-container case.

use mnemonist_core::structures::fuzzy_multi_map::FuzzyMultiMap;
use mnemonist_core::structures::multi_map::{Bucket, ContainerKind};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fuzzy-multi-map.txt"
);

/// Three source strings, chosen so `fuzzyLower` collapses several of them
/// onto the same hashed key.
fn item_at(index: usize) -> &'static str {
    match index {
        0 => "Hello",
        1 => "HELLO",
        _ => "World",
    }
}

fn render_bucket(bucket: &Bucket<String>) -> Value {
    let values: Vec<Value> = bucket.values().iter().map(|value| json!(value)).collect();

    match bucket.kind() {
        ContainerKind::List => json!(values),
        ContainerKind::Set => json!({"$set": values}),
    }
}

pub struct FuzzyMultiMapSpec;

impl ModuleSpec for FuzzyMultiMapSpec {
    type Instance = FuzzyMultiMap<String, String>;

    fn module(&self) -> &'static str {
        "fuzzy-multi-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "dimension", "items"]
    }

    /// `new FuzzyMultiMap(fuzzyLower)` — a single hash function shared by
    /// both directions, exactly as `upstream`'s falsy-substitution collapses
    /// `[descriptor, descriptor]` to one function when a bare function is
    /// given.
    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(vec![json!({"$factory": "fuzzyLower"})]).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let item = (0..3usize).prop_map(|index| json!(item_at(index)));

        prop_oneof![
            5 => item.clone().prop_map(|i| Op::new("add", vec![i])),
            2 => (item.clone(), item.clone())
                .prop_map(|(k, v)| Op::new("set", vec![k, v])),
            2 => item.clone().prop_map(|i| Op::new("has", vec![i])),
            2 => item.prop_map(|i| Op::new("get", vec![i])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        // The hash function itself is a JavaScript concern the bridge owns
        // (see `mnemonist_napi::fuzzy_multi_map`); core's constructor takes
        // only the resolved `ContainerKind`, and this campaign -- like
        // `multi-map`'s -- fuzzes the `List`-kind path (see the module docs
        // for why `Set`-kind identity dedup is out of scope here).
        FuzzyMultiMap::new(ContainerKind::List)
    }

    /// Mirrors `fuzz/oracle.js`'s own hashing: lowercase the argument, the
    /// same transform `fuzzyLower` performs, so the key this campaign hashes
    /// to agrees with what upstream's actual factory call produces.
    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        let infallible =
            |a: &String, b: &String| -> Result<bool, std::convert::Infallible> { Ok(a == b) };

        match op.name {
            "add" => {
                let item = op.args[0].as_str().expect("item is a string").to_owned();
                let key = item.to_lowercase();

                instance
                    .set_with(key, item, infallible)
                    .expect("a String comparison cannot fail");

                json!({"$self": true})
            }
            "set" => {
                let key = op.args[0].as_str().expect("key is a string").to_lowercase();
                let item = op.args[1].as_str().expect("item is a string").to_owned();

                instance
                    .set_with(key, item, infallible)
                    .expect("a String comparison cannot fail");

                json!({"$self": true})
            }
            "has" => {
                let key = op.args[0].as_str().expect("key is a string").to_lowercase();

                json!(instance.has(&key))
            }
            "get" => {
                let key = op.args[0].as_str().expect("key is a string").to_lowercase();

                match instance.get(&key) {
                    Some(bucket) => render_bucket(bucket),
                    None => json!({"$undefined": true}),
                }
            }
            "clear" => {
                instance.clear();
                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    /// `this.items` upstream is not a raw `Map` — it is a `MultiMap`
    /// **instance** (`this.items = new MultiMap(Container)`), so
    /// `fuzz/oracle.js`'s generic `encode()` renders it as the nested object
    /// its own enumerable properties are: `{items: {$map: [...]}, size,
    /// dimension}` (`Container`, a function, is silently dropped by
    /// `JSON.stringify`). So the observation below reproduces that nesting.
    /// Flattening it to a bare `$map` diverges on case 0 of every campaign,
    /// which is the cheapest possible way to be told about it.
    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let items: Vec<Value> = instance
            .items()
            .items()
            .iter()
            .map(|(key, bucket)| json!([key, render_bucket(bucket)]))
            .collect();

        json!({
            "size": instance.size(),
            "dimension": instance.dimension(),
            "items": {
                "items": {"$map": items},
                "size": instance.items().size(),
                "dimension": instance.items().dimension(),
            },
        })
    }
}

/// Direct evidence that `fuzzyLower` collapsing `'Hello'`/`'HELLO'`/`'World'`
/// onto two hashed keys routinely gives a bucket more than one value, and
/// that `.clear()` routinely drains the whole map back to zero -- the two
/// states a campaign for this unit has to reach. No oracle involved.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    #[test]
    fn the_grammar_builds_multi_value_hashed_keys_and_clear_drains_them() {
        let spec = FuzzyMultiMapSpec;
        let mut runner = TestRunner::new(Config::default());
        let mut multi_value_observations = 0u64;
        let mut clears_of_a_nonempty_map = 0u64;

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
                if op.name == "clear" && instance.dimension() > 0 {
                    clears_of_a_nonempty_map += 1;
                }

                spec.apply(&mut instance, op);

                if instance
                    .items()
                    .items()
                    .iter()
                    .any(|(_, bucket)| bucket.len() > 1)
                {
                    multi_value_observations += 1;
                }
            }
        }

        eprintln!(
            "fuzzy-multi-map grammar: {multi_value_observations} steps with \
             a multi-value bucket, {clears_of_a_nonempty_map} clears of a \
             nonempty map"
        );

        assert!(
            multi_value_observations > 100,
            "the grammar should routinely build multi-value buckets via \
             hash collisions, not rarely: only {multi_value_observations} \
             over 400 programs"
        );
        assert!(
            clears_of_a_nonempty_map > 20,
            "the grammar should routinely drain the map to zero via clear, \
             not rarely: only {clears_of_a_nonempty_map} over 400 programs"
        );
    }
}
