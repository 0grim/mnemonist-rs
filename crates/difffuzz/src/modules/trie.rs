//! [`ModuleSpec`] for `trie`.
//!
//! Shares [`crate::modules::trie_map`]'s [`PREFIX_POOL`](super::trie_map::PREFIX_POOL)
//! and tokenisation rather than re-deriving them: `trie.js` is upstream's own
//! `TrieMap.prototype` copy-and-delete (`mnemonist_core::structures::trie`'s
//! module docs), so the two engines share every prefix-relationship concern
//! this grammar exists to reach. See that module's docs for why the pool is
//! shaped the way it is, and for what this grammar deliberately does not
//! cover (array mode, digit tokens, a starting sub-prefix on `keys`).
//!
//! The one thing that differs is the value: a `Trie` node's own value is a
//! bare `bool` (never `undefined` — there is no way to store one; `add`
//! always writes `true`, and `update`'s inherited callback is declared
//! `Option<bool> -> bool` in core), where `trie-map`'s is a full JSON slot.
//! `has` is presence-based regardless of that value — `SENTINEL in node`,
//! not a truthiness check — so a `trie.update(prefix, () => false)` still
//! reports `has(prefix) === true` afterwards; the fuzz grammar's own
//! `trieToggle` factory (`fuzz/oracle.js`) exists specifically to reach that
//! state on both sides.
//!
//! Also shared with `trie-map`: the D-201 regime split. `delete` and `clear`
//! never share a program with a persistent `$iter`/`$next` cursor, for the
//! identical reason — `Trie` walks the same `mnemonist_core::structures::
//! trie_map::Walk` engine `TrieMap` does, so it inherits the same
//! path-based-re-navigation divergence from upstream's live object
//! references. See `crate::modules::trie_map`'s module docs for the confirmed
//! repro; this module's own first campaign (before the split existed)
//! reproduced it independently, over `add`/`keys` rather than `set`/`entries`.

use std::collections::BTreeMap;

use mnemonist_core::structures::trie::Trie as CoreTrie;
use mnemonist_core::structures::trie_map::{Entry, NodeView, Walk};
use proptest::prelude::*;
use serde_json::{json, Map as JsonMap, Value};

use crate::modules::trie_map::PREFIX_POOL;
use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proptest-regressions/trie.txt");

fn tokens(word: &str) -> Vec<String> {
    word.chars().map(|c| c.to_string()).collect()
}

fn root_json(node: NodeView<'_, String, bool>) -> Value {
    let mut object = JsonMap::new();

    for entry in node.entries() {
        match entry {
            Entry::Word(value) => {
                object.insert("\u{0}".to_string(), json!(*value));
            }
            Entry::Child(token, child) => {
                object.insert(token.clone(), root_json(child));
            }
        }
    }

    Value::Object(object)
}

pub struct Instance {
    trie: CoreTrie<String>,
    cursor: Option<Walk<String>>,
}

pub struct TrieSpec;

impl ModuleSpec for TrieSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "trie"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "root"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // `new Trie()` either way -- array mode is out of scope; see
        // `crate::modules::trie_map`'s docs. `ctor[0]` is the same D-201
        // regime flag that module's docs describe, not a real `Token`.
        any::<bool>().prop_map(|prunes| vec![json!(prunes)]).boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let prunes = ctor[0].as_bool().expect("ctor[0] is the regime flag");
        let prefix = (0..PREFIX_POOL.len()).prop_map(|index| json!(PREFIX_POOL[index]));

        let common = prop_oneof![
            6 => prefix.clone().prop_map(|p| Op::new("add", vec![p])),
            3 => prefix.clone().prop_map(|p| Op::new("has", vec![p])),
            2 => prefix
                .clone()
                .prop_map(|p| Op::new("update", vec![p, json!({"$factory": "trieToggle"})])),
            3 => prefix.clone().prop_map(|p| Op::new("find", vec![p])),
            1 => Just(Op::new("$spread", vec![])),
        ];

        // D-201, shared with `trie-map`: `delete`/`clear` never share a
        // program with a persistent `$iter`/`$next` cursor. See the module
        // docs.
        if prunes {
            prop_oneof![
                15 => common,
                4 => prefix.prop_map(|p| Op::new("delete", vec![p])),
                1 => Just(Op::new("clear", vec![])),
            ]
            .boxed()
        } else {
            prop_oneof![
                15 => common,
                2 => Just(Op::new("$iter", vec![json!("keys")])),
                4 => Just(Op::new("$next", vec![])),
            ]
            .boxed()
        }
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        Instance {
            trie: CoreTrie::new(),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "add" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                instance.trie.add(tokens(prefix));

                json!({"$self": true})
            }
            "has" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                json!(instance.trie.has(tokens(prefix)))
            }
            "delete" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                json!(instance.trie.delete(tokens(prefix)))
            }
            "update" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                instance
                    .trie
                    .update(tokens(prefix), |old| !old.unwrap_or(false));

                json!({"$self": true})
            }
            "find" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                let matches: Vec<Value> = instance
                    .trie
                    .find(tokens(prefix))
                    .into_iter()
                    .map(|suffix| json!(format!("{prefix}{}", suffix.join(""))))
                    .collect();

                Value::Array(matches)
            }
            "clear" => {
                instance.trie.clear();

                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(instance.trie.walk(std::iter::empty()));

                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(walk) => match instance.trie.step(walk) {
                    None => json!({"done": true, "value": {"$undefined": true}}),
                    Some(suffix) => json!({"done": false, "value": suffix.join("")}),
                },
            },
            "$spread" => {
                let mut walk = instance.trie.walk(std::iter::empty());
                let mut out = Vec::new();

                while let Some(suffix) = instance.trie.step(&mut walk) {
                    out.push(json!(suffix.join("")));
                }

                Value::Array(out)
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let mut state = JsonMap::new();
        state.insert("size".into(), json!(instance.trie.size()));
        state.insert("root".into(), root_json(instance.trie.root()));

        Value::Object(
            state
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}
