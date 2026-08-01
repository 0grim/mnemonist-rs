//! [`ModuleSpec`] for `fuzzy-map`.
//!
//! # What this grammar is for
//!
//! `default-map`'s campaign already covers `OrderedMap` — insertion order,
//! tombstones, compaction, live cursors. What is new in `fuzzy-map` is the
//! **hash function** applied before every read or write, so this grammar
//! narrows the value alphabet to what makes hashing meaningful (strings) and
//! spends its budget on the write/read split instead:
//!
//! * **Items are drawn from a small, mixed-case string pool** ([`POOL`]), so
//!   `identity` and a case-insensitive hash disagree on collisions constantly
//!   — `"Hello"` and `"hello"` are one key under one hash and two under the
//!   other.
//! * **`add` and `set` are both in the alphabet.** They hash *different*
//!   arguments (`add` hashes the item itself; `set` hashes the caller's key
//!   and stores the item as-is), which is exactly the distinction
//!   `mnemonist_core::structures::fuzzy_map` leaves to its caller — the
//!   bridge decides which value gets hashed, and this grammar is what checks
//!   both call shapes land on the right one.
//!
//! # The hash function travels as a named factory
//!
//! `new FuzzyMap(descriptor)` takes a function, which JSON cannot carry.
//! Mirroring `default-map`'s `$factory` convention, `fuzz/oracle.js` gained
//! two new entries — `fuzzyIdentity` and `fuzzyLower` — rather than reusing
//! its existing `FACTORIES` table's names, so the two modules' campaigns
//! cannot collide on a name and decode the wrong function.
//!
//! # What this grammar deliberately does not cover
//!
//! **The `[write, read]` array-descriptor form**, where the two directions
//! hash differently. This spec always constructs with a single function
//! (upstream's `writeHashFunction = readHashFunction = descriptor` branch),
//! because the pair form needs two independent named factories per case and
//! the single-function form is what the original suite's `FuzzyMap.from` test
//! exercises for the read/write split instead; the native bridge tests in
//! `mnemonist_napi::fuzzy_map` cover the array form directly. Disclosed rather
//! than silently narrowed.
//!
//! **Object items** (`{title: 'Hello'}`), which the original suite's `add`
//! test uses. This grammar's items are always strings, because a hash
//! function that can fail (`item.title.toLowerCase()` on a bare string throws)
//! would turn every non-title-bearing generated item into an apparatus
//! failure rather than a comparison. `identity`/`lower` both accept a bare
//! string, which keeps every generated program well-defined on both sides.

use mnemonist_core::structures::fuzzy_map::FuzzyMap;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fuzzy-map.txt"
);

/// Mixed-case so `identity` and `lower` disagree on membership constantly.
const POOL: &[&str] = &["Hello", "hello", "World", "WORLD", "Foo", "bar"];

/// The two hash functions this grammar constructs with, matching
/// `fuzz/oracle.js`'s `fuzzyIdentity`/`fuzzyLower`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Identity,
    Lower,
}

impl Hash {
    fn named(name: &str) -> Self {
        match name {
            "identity" => Self::Identity,
            "lower" => Self::Lower,
            other => panic!("`{other}` is not a hash this grammar generates"),
        }
    }

    fn apply(self, text: &str) -> String {
        match self {
            Self::Identity => text.to_owned(),
            Self::Lower => text.to_lowercase(),
        }
    }
}

fn item_at(index: usize) -> Value {
    json!(POOL[index % POOL.len()])
}

pub struct Instance {
    map: FuzzyMap<String, Value>,
    hash: Hash,
}

pub struct FuzzyMapSpec;

impl ModuleSpec for FuzzyMapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "fuzzy-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            Just(vec![json!({"$factory": "fuzzyIdentity"})]),
            Just(vec![json!({"$factory": "fuzzyLower"})]),
        ]
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let item = (0..POOL.len()).prop_map(item_at);
        let key = (0..POOL.len()).prop_map(item_at);

        prop_oneof![
            4 => item.clone().prop_map(|v| Op::new("add", vec![v])),
            4 => (key.clone(), item).prop_map(|(k, v)| Op::new("set", vec![k, v])),
            3 => key.clone().prop_map(|k| Op::new("get", vec![k])),
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
            map: FuzzyMap::new(),
            hash: Hash::named(name),
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        let hash = instance.hash;

        match op.name {
            "add" => {
                let item = op.args[0].as_str().expect("items are strings");
                let key = hash.apply(item);

                instance.map.set(key, Some(op.args[0].clone()));

                json!({"$self": true})
            }
            "set" => {
                let key_text = op.args[0].as_str().expect("keys are strings");
                let key = hash.apply(key_text);

                instance.map.set(key, Some(op.args[1].clone()));

                json!({"$self": true})
            }
            "get" => {
                let key = hash.apply(op.args[0].as_str().expect("keys are strings"));

                match instance.map.get(&key) {
                    Some(value) => value.clone(),
                    None => json!({"$undefined": true}),
                }
            }
            "has" => {
                let key = hash.apply(op.args[0].as_str().expect("keys are strings"));

                json!(instance.map.has(&key))
            }
            "clear" => {
                instance.map.clear();
                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let items: Vec<Value> = instance
            .map
            .items()
            .iter()
            .map(|(key, value)| {
                let slot = match value {
                    Some(value) => value.clone(),
                    None => json!({"$undefined": true}),
                };

                json!([key, slot])
            })
            .collect();

        json!({
            "size": instance.map.size(),
            "items": {"$map": items},
        })
    }
}
