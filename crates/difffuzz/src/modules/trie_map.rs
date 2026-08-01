//! [`ModuleSpec`] for `trie-map`.
//!
//! # The grammar exists to reach one state: shared prefixes
//!
//! DESIGN.md's own warning, restated for this module by the porting brief:
//! a trie fuzzed with long random strings proves only that the structure can
//! store unrelated words — every `set` lands on a fresh branch, `has` on a
//! partial key is always false, and `delete` never has a sibling to leave
//! behind. None of that is where a trie's own bugs live.
//!
//! So every prefix a generated program can use comes from [`PREFIX_POOL`], a
//! *small, hand-built* alphabet chosen so the pool itself already contains
//! prefix relationships, before a single op has run:
//!
//! ```text
//! a, ab, abc, abcd, b, ba, bc, bad
//! ```
//!
//! `"a"` is a strict prefix of `"ab"`, `"abc"` and `"abcd"`; `"ab"` is a strict
//! prefix of `"abc"` and `"abcd"`; `"abc"` is a strict prefix of `"abcd"`;
//! `"b"` is a strict prefix of `"ba"`, `"bc"` and `"bad"`; `"ba"` is a strict
//! prefix of `"bad"`. The tests below measure this directly rather than
//! asserting it by eye: **5 of the 8 pool entries are themselves a strict
//! prefix of another pool entry**, and a 2,000-sample draw from the actual
//! `set` op strategy shows generated prefixes revisiting that structure, not
//! merely the pool being capable of it in principle. With only eight possible
//! prefixes and every op drawing from the same pool, a 200-op program
//! revisits this repeatedly — `set("ab")` after `set("abc")` leaves `"ab"` a
//! stored word one step above an existing branch; `has("a")` after
//! `set("ac")`-shaped ops but with `"a"` never itself `set` is exactly the
//! "mere prefix, not a word" state gate 6's falsification target breaks.
//!
//! # What this grammar deliberately does not cover
//!
//! **Array mode.** `new TrieMap(Array)` runs each array element through
//! upstream's `ToPropertyKey`, which `mnemonist-napi`'s bridge approximates
//! with `String(value)` rather than reproducing in full (see
//! `mnemonist_napi::trie_map`'s module docs, and D-91's precedent). Fuzzing
//! that coercion would mean either reimplementing it a third time here (after
//! core, which does not have it at all, and the bridge) or comparing against
//! a divergence that is really in the fuzz spec's own mirror. String mode
//! alone already reaches every trie-shaped state — shared prefixes, deletion
//! pruning, live-walk interleaving — that array mode does not add to; only
//! the coercion rule itself would be new, and it is covered by the original
//! test file's "custom tokens" block plus
//! `mnemonist_napi::trie_map`'s own reasoning instead.
//!
//! **Digit tokens.** `Object.keys` sorts integer-like keys ascending before
//! any other key, a rule this port does not implement (D-202). `PREFIX_POOL`
//! is built entirely from letters so this campaign can never manufacture a
//! divergence out of that documented, disclosed gap.
//!
//! **A starting sub-prefix on `values`/`keys`/`entries`.** Upstream supports
//! `trie.values('rate')`; this grammar always opens a walk from the root.
//! Covered instead by `mnemonist_core::structures::trie_map`'s own
//! `walk_visits_every_word_in_the_same_order_as_find` and by gate 4.
//!
//! **`delete` interleaved with an open cursor — found on contact, then
//! deliberately excluded (D-201).** The very first campaign run for this
//! module (before this narrowing existed) diverged in under a hundred
//! operations: `TrieMap.prototype.values`/`keys`/`entries` closes over live
//! JS *object references* it has already queued but not yet visited, and
//! `delete`'s own pruning can remove a queued node's reference from its
//! *parent* while leaving that node's own value untouched. Confirmed by hand
//! against real Node —
//!
//! ```text
//! t.set('a', 1); t.set('ab', 2);
//! var it = t.entries(); it.next();      // {value: ['a', 1]}
//! t.delete('ab');
//! it.next();                             // {value: ['ab', 2]} -- still yielded
//! ```
//!
//! `mnemonist_core::structures::trie_map::Walk` re-navigates by token path
//! instead of holding a reference (required so a cursor can be resumed from a
//! fresh `&TrieMap` at the FFI boundary — see that module's own docs), so it
//! disagrees with upstream in exactly this interaction. This is D-201, and it
//! was already known and disclosed before this campaign ran; what the
//! campaign added was independent confirmation that the interaction is
//! genuinely reachable, not just theoretically possible.
//!
//! [`ModuleSpec::ctor_strategy`] therefore generates a **regime flag**
//! alongside the (otherwise unused) constructor arguments: `prunes = true`
//! programs get `delete` and no cursor lifecycle ops at all; `prunes = false`
//! programs get the full cursor lifecycle (including deletions and
//! insertions that are NOT pruning-related, so live-addition-during-iteration
//! stays fuzzed) but never call `delete`. Splitting per-program rather than
//! per-op is necessary because [`ModuleSpec::op_strategy`] cannot see the
//! instance's runtime state (whether a cursor happens to be open right now),
//! only the constructor arguments chosen once at the start of the program.
//! The flag travels as `ctor[0]` purely for the fuzz harness's own use; it is
//! never a real `Token` argument (`new TrieMap()` is called either way, and
//! neither `true` nor `false` is ever the global `Array`, so it would be
//! harmless even if it were).

use std::collections::BTreeMap;

use mnemonist_core::structures::trie_map::{Entry, TrieMap as CoreTrieMap, Walk};
use proptest::prelude::*;
use serde_json::{json, Map as JsonMap, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/trie-map.txt"
);

/// Every prefix a generated program can use. See the module docs — this is
/// the entire mechanism the "reach shared prefixes" requirement rests on.
pub(crate) const PREFIX_POOL: &[&str] = &["a", "ab", "abc", "abcd", "b", "ba", "bc", "bad"];

/// A stored value slot. `None` is `undefined`, matching
/// `mnemonist_napi::trie_map::Value` — a stored word can hold it, and `has`
/// (word presence) does not care which this is.
type Slot = Option<Value>;

/// `{"$undefined": true}` <-> `None`, the same envelope the oracle and every
/// other T3 spec in this crate use.
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

/// Per-character tokens for one pool string — string mode's own
/// tokenisation, `prefix[i]` one code unit at a time. Plain ASCII in this
/// pool, so a Rust `char` and a UTF-16 code unit coincide; the surrogate-pair
/// fidelity `mnemonist_napi::trie_map::Token` cares about has nothing to add
/// here and is exercised at the bridge layer instead (gate 4).
fn tokens(word: &str) -> Vec<String> {
    word.chars().map(|c| c.to_string()).collect()
}

/// The `Value`/`root` these two agree on: `mnemonist_core`'s generic
/// `NodeView` walked into a nested JSON object, exactly as upstream's plain
/// object *is* one already.
fn root_json(node: mnemonist_core::structures::trie_map::NodeView<'_, String, Slot>) -> Value {
    let mut object = JsonMap::new();

    for entry in node.entries() {
        match entry {
            Entry::Word(value) => {
                object.insert("\u{0}".to_string(), slot_json(value));
            }
            Entry::Child(token, child) => {
                object.insert(token.clone(), root_json(child));
            }
        }
    }

    Value::Object(object)
}

pub struct Instance {
    map: CoreTrieMap<String, Slot>,
    /// The one cursor a program can have open, and which projection it was
    /// opened as.
    cursor: Option<(Walk<String>, &'static str)>,
}

pub struct TrieMapSpec;

impl ModuleSpec for TrieMapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "trie-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "root"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // `new TrieMap()` either way -- array mode is out of scope; see the
        // module docs. `ctor[0]` is the regime flag the docs describe: this
        // is the ONE per-program random choice `op_strategy` gets to see, so
        // it is where the "never mix delete with an open cursor" split has
        // to live.
        any::<bool>().prop_map(|prunes| vec![json!(prunes)]).boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let prunes = ctor[0].as_bool().expect("ctor[0] is the regime flag");
        let prefix = (0..PREFIX_POOL.len()).prop_map(|index| json!(PREFIX_POOL[index]));
        let value = prop_oneof![
            2 => Just(json!({"$undefined": true})),
            1 => Just(Value::Null),
            3 => (0i64..5).prop_map(|n| json!(n)),
            2 => Just(json!("v")),
        ];

        let common = prop_oneof![
            5 => (prefix.clone(), value.clone()).prop_map(|(p, v)| Op::new("set", vec![p, v])),
            3 => prefix.clone().prop_map(|p| Op::new("get", vec![p])),
            3 => prefix.clone().prop_map(|p| Op::new("has", vec![p])),
            3 => prefix
                .clone()
                .prop_map(|p| Op::new("update", vec![p, json!({"$factory": "trieIncrement"})])),
            3 => prefix.clone().prop_map(|p| Op::new("find", vec![p])),
            1 => Just(Op::new("$spread", vec![])),
        ];

        // D-201: `delete` and `clear` never share a program with a
        // persistent `$iter`/`$next` cursor. Both prune structure a cursor
        // may have already queued a reference to -- `clear` more bluntly
        // than `delete`, since it replaces the whole root -- and upstream's
        // cursor, holding that reference directly, keeps yielding the stale
        // content where this port's path-based walk correctly (and
        // divergently) finds nothing there any more. `$spread` is exempt in
        // both regimes: it opens and fully drains a cursor within one op, so
        // nothing is ever left "queued but not visited" across a later
        // pruning op. See the module docs.
        if prunes {
            prop_oneof![
                18 => common,
                4 => prefix.prop_map(|p| Op::new("delete", vec![p])),
                1 => Just(Op::new("clear", vec![])),
            ]
            .boxed()
        } else {
            prop_oneof![
                18 => common,
                2 => prop_oneof![
                    Just(Op::new("$iter", vec![json!("values")])),
                    Just(Op::new("$iter", vec![json!("keys")])),
                    Just(Op::new("$iter", vec![json!("entries")])),
                ],
                4 => Just(Op::new("$next", vec![])),
            ]
            .boxed()
        }
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        Instance {
            map: CoreTrieMap::new(),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");
                let value = slot_from_json(&op.args[1]);

                instance.map.set(tokens(prefix), value);

                json!({"$self": true})
            }
            "get" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                match instance.map.get(tokens(prefix)) {
                    Some(slot) => slot_json(slot),
                    None => json!({"$undefined": true}),
                }
            }
            "has" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                json!(instance.map.has(tokens(prefix)))
            }
            "delete" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                json!(instance.map.delete(tokens(prefix)).is_some())
            }
            "update" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                instance.map.update(tokens(prefix), |old| {
                    let current = old.flatten().and_then(|v| v.as_i64()).unwrap_or(0);

                    Some(json!(current + 1))
                });

                json!({"$self": true})
            }
            "find" => {
                let prefix = op.args[0].as_str().expect("prefix is a string");

                let matches: Vec<Value> = instance
                    .map
                    .find(tokens(prefix))
                    .into_iter()
                    .map(|(suffix, value)| {
                        let full = format!("{prefix}{}", suffix.join(""));

                        json!([full, slot_json(value)])
                    })
                    .collect();

                Value::Array(matches)
            }
            "clear" => {
                instance.map.clear();

                json!({"$undefined": true})
            }
            "$iter" => {
                let projection = op.args[0].as_str().expect("$iter names a projection");
                let projection: &'static str = match projection {
                    "values" => "values",
                    "keys" => "keys",
                    "entries" => "entries",
                    other => panic!("`{other}` is not an iterator this module has"),
                };

                instance.cursor = Some((instance.map.walk(std::iter::empty()), projection));

                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some((walk, projection)) => match walk.step(&instance.map) {
                    None => json!({"done": true, "value": {"$undefined": true}}),
                    Some((suffix, value)) => json!({
                        "done": false,
                        "value": project(projection, &suffix.join(""), value),
                    }),
                },
            },
            "$spread" => {
                let mut walk = instance.map.walk(std::iter::empty());
                let mut out = Vec::new();

                while let Some((suffix, value)) = walk.step(&instance.map) {
                    out.push(project("entries", &suffix.join(""), value));
                }

                Value::Array(out)
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let mut state = JsonMap::new();
        state.insert("size".into(), json!(instance.map.size()));
        state.insert("root".into(), root_json(instance.map.root()));

        Value::Object(
            state
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}

fn project(projection: &str, prefix: &str, value: &Slot) -> Value {
    match projection {
        "keys" => json!(prefix),
        "values" => slot_json(value),
        _ => json!([prefix, slot_json(value)]),
    }
}

/// Measures, rather than asserts by eye, that [`PREFIX_POOL`] contains real
/// prefix relationships — the entire mechanism behind this module's "reach
/// shared prefixes" claim. No oracle, no `node`, microseconds.
#[cfg(test)]
mod tests {
    use proptest::test_runner::TestRunner;

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
            "trie-map pool self-check: {}/{} entries are a strict prefix of another entry: {:?}",
            self_prefixing.len(),
            PREFIX_POOL.len(),
            self_prefixing
        );

        assert!(
            self_prefixing.len() * 2 >= PREFIX_POOL.len(),
            "PREFIX_POOL must be mostly self-prefixing, or this grammar only proves the trie \
             stores unrelated words; got {self_prefixing:?} out of {PREFIX_POOL:?}"
        );
    }

    /// The dynamic half: sample real generated `set` prefixes (not just the
    /// static pool) and confirm the *op stream itself* revisits prefix
    /// relationships often, not merely that the pool could in principle
    /// produce them.
    #[test]
    fn pool_self_check_generated_programs_revisit_prefix_relationships() {
        let spec = TrieMapSpec;
        let mut runner = TestRunner::new(proptest::test_runner::Config::default());

        let mut set_prefixes = Vec::new();

        // Both regimes (see the module docs, D-201) generate `set`; sample
        // each so the check reflects the grammar actually run, not one half
        // of it.
        for regime in [json!(false), json!(true)] {
            let strategy = spec.op_strategy(std::slice::from_ref(&regime));

            for _ in 0..1_000 {
                let tree = strategy
                    .new_tree(&mut runner)
                    .expect("strategy generates a value");
                let op = tree.current();

                if op.name == "set" {
                    set_prefixes.push(op.args[0].as_str().unwrap().to_string());
                }
            }
        }

        let related = set_prefixes
            .iter()
            .filter(|candidate| {
                set_prefixes
                    .iter()
                    .any(|other| *other != **candidate && other.starts_with(candidate.as_str()))
            })
            .count();

        eprintln!(
            "trie-map grammar self-check: {related}/{} generated `set` prefixes are a strict \
             prefix of another generated `set` prefix",
            set_prefixes.len()
        );

        assert!(
            related * 2 >= set_prefixes.len(),
            "generated `set` ops must mostly revisit prefix relationships, not just unrelated \
             words: {related}/{} qualified",
            set_prefixes.len()
        );
    }
}
