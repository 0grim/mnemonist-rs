//! [`ModuleSpec`] for `linked-list`.
//!
//! # Grammar, and what it is built to reach
//!
//! CLAUDE.md's brief for this unit named the interesting territory directly:
//! "`unshift`/`shift`/`push` against a live cursor." So this grammar keeps
//! at most a handful of live nodes (`VALUES` is a small pool, `program_len`
//! wide) and interleaves the three mutating ops with cursor lifecycle ops
//! constantly, rather than draining a cursor immediately after opening it —
//! see `mnemonist_core::structures::linked_list`'s module docs for the three
//! liveness rules this is built to exercise:
//!
//! * a `push` while a cursor has not yet passed the (old) tail IS visible;
//! * a `shift`/`unshift` is NEVER visible to an already-open cursor;
//! * a cursor that has reported `{done: true}` stays done even if the list
//!   grows afterwards.
//!
//! `$forEach` is the sharpest test of the first rule: the mutation table
//! below weights `push` heavily specifically so that a generated program
//! commonly pushes while `forEach`'s own walk is mid-flight, sitting on what
//! was the tail a moment before.
//!
//! # Observable state
//!
//! `size`, `first()`, `last()` (the pair B-241 depends on — see the module
//! docs), and `toArray()`.
//!
//! # Deliberately excluded
//!
//! Nothing about the grammar itself; `JsSlot`/`WeakKey`-shaped identity
//! questions do not apply here (`Value` is compared by content, matching
//! upstream's own primitive-only test file), and `mnemonist-core` is generic
//! enough that the port under test here is `LinkedList<Value>` directly —
//! no bridge-specific mirror type is needed, unlike `default-map`'s
//! `FuzzKey`.

use mnemonist_core::structures::linked_list::LinkedList;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{for_each, for_each_args, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY};

/// Range the generator draws pushed/unshifted values from. Small and
/// disjoint from anything computed, so a program's own trace is
/// unambiguous when read back from a shrunk repro.
const VALUES: std::ops::Range<i64> = 0..24;

pub struct LinkedListSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/linked-list.txt"
);

/// The list plus the one cursor a program can have open, and which
/// projection it was opened as.
pub struct Instance {
    list: LinkedList<Value>,
    /// The cursor, which projection it was opened as, and `entries`' own
    /// running index -- upstream's `i` in `[i++, value]`, advanced only on a
    /// yielded step, exactly like `JsLinkedListEntries` at the bridge.
    cursor: Option<(
        mnemonist_core::structures::linked_list::ListCursor,
        &'static str,
        u64,
    )>,
}

impl ModuleSpec for LinkedListSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "linked-list"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "first", "last", "toArray"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let value = VALUES.prop_map(|value| json!(value));

        prop_oneof![
            // push/unshift outweigh shift, so a program keeps enough live
            // nodes to make the liveness rules reachable rather than
            // emptying the list every few ops.
            5 => value.clone().prop_map(|v| Op::new("push", vec![v])),
            4 => value.prop_map(|v| Op::new("unshift", vec![v])),
            3 => Just(Op::new("shift", vec![])),
            2 => Just(Op::new("first", vec![])),
            2 => Just(Op::new("last", vec![])),
            1 => Just(Op::new("peek", vec![])),
            1 => Just(Op::new("clear", vec![])),
            2 => prop_oneof![
                Just(Op::new("$iter", vec![json!("values")])),
                Just(Op::new("$iter", vec![json!("entries")])),
            ],
            4 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            // Heaviest of the cursor-lifecycle ops: this is the one that
            // reaches "push while forEach's own walk is mid-flight."
            3 => for_each_strategy(MUTATIONS),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        Instance {
            list: LinkedList::new(),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "push" => json!(instance.list.push(op.args[0].clone())),
            "unshift" => json!(instance.list.unshift(op.args[0].clone())),
            "shift" => optional(instance.list.shift()),
            "first" => optional(instance.list.first().cloned()),
            "last" => optional(instance.list.last().cloned()),
            "peek" => optional(instance.list.peek().cloned()),
            "clear" => {
                instance.list.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                let projection = op.args[0].as_str().expect("$iter names a projection");
                let projection: &'static str = match projection {
                    "values" => "values",
                    "entries" => "entries",
                    other => panic!("`{other}` is not an iterator this module has"),
                };

                instance.cursor = Some((instance.list.values(), projection, 0));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some((cursor, projection, index)) => {
                    let projection = *projection;

                    match cursor.step(&instance.list) {
                        None => json!({"done": true, "value": {"$undefined": true}}),
                        Some(item) => {
                            let value = project(projection, item, *index);
                            *index += 1;

                            json!({"done": false, "value": value})
                        }
                    }
                }
            },
            // `Array.from(list)` -- the COLLECTION's Symbol.iterator, aliased
            // to `values` upstream, so a fresh cursor every call (the factory
            // half of D-07), unlike `$next`, which must never restart.
            "$spread" => {
                let mut cursor = instance.list.values();
                let mut items = Vec::new();

                while let Some(item) = cursor.step(&instance.list) {
                    items.push(item.clone());
                }

                Value::Array(items)
            }
            // Upstream's own loop:
            //
            // ```js
            // var n = this.head, i = 0;
            // while (n) {
            //   callback.call(scope, n.item, i, this);
            //   n = n.next;   // AFTER the callback -- see the core module's docs
            //   i++;
            // }
            // ```
            //
            // Driven by the SAME cursor `$next` uses -- see
            // `mnemonist_core::structures::linked_list`'s module docs for why
            // `forEach` and the lazy iterators are one walk primitive here,
            // not two.
            "$forEach" => {
                let spec = for_each(op);
                let mut cursor = instance.list.values();
                let mut seen = Vec::new();
                let mut fired = 0usize;
                let mut index = 0u64;

                // `current` then `advance`, NOT `step`: upstream's own
                // `callback.call(...); n = n.next;` runs the callback BEFORE
                // advancing, which is exactly what let this op catch the
                // port defect documented in
                // `mnemonist_core::structures::linked_list`'s module docs.
                // `step`'s eager advance is right for `values`/`entries` and
                // wrong here.
                while let Some(item) = cursor.current(&instance.list).cloned() {
                    let received = vec![item, json!(index)];

                    seen.push(Value::Array(received.clone()));
                    index += 1;

                    if fired < spec.limit {
                        if let Some(args) = for_each_args(&spec, &received) {
                            fired += 1;

                            match spec.method.expect("for_each_args returned Some") {
                                "push" => {
                                    instance.list.push(args[0].clone());
                                }
                                "unshift" => {
                                    instance.list.unshift(args[0].clone());
                                }
                                "shift" => {
                                    instance.list.shift();
                                }
                                "clear" => instance.list.clear(),
                                other => {
                                    panic!("`{other}` is not a $forEach mutation for this module")
                                }
                            }
                        }
                    }

                    cursor.advance(&instance.list);
                }

                json!({ "seen": seen })
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.list.size(),
            "first": optional(instance.list.first().cloned()),
            "last": optional(instance.list.last().cloned()),
            "toArray": instance.list.to_array(),
        })
    }
}

/// What the callback may do to the list, and how often.
///
/// `push` is uncapped and the heaviest: it is the one mutation the module
/// docs' liveness rules say IS visible to an in-flight walk, and only when
/// it fires while the walk is sitting on the (old) tail. `shift`/`unshift`
/// are included specifically to prove the NEGATIVE half of those rules under
/// the differential fuzzer, not just by hand-written Rust tests.
// `push`'s limit is a small FIXED cap, not `FOR_EACH_MANY` -- unlike every
// other mutation table in this port. A `push` while the walk sits on the
// (old) tail relinks that exact tail's `.next` to the freshly pushed node,
// and this walk's own `advance()` then moves onto THAT node next -- which is
// now itself the tail. An uncapped `push` therefore chases its own tail
// forever: the walk visits a node, pushes, advances onto the node it just
// pushed, which is the new tail, and repeats without end. This is not a
// divergence from upstream (a real `forEach` with an uncapped push-at-the-
// tail callback would loop identically, and does when tried against Node) --
// it is a genuine unbounded program this grammar must not generate, because a
// campaign is supposed to run thousands of finite cases, not hang on one.
// Found the hard way: an early run of this module's campaign with
// `FOR_EACH_MANY` here simply never returned. `unshift`/`shift`/`clear` have
// no equivalent hazard -- see the module docs' liveness rules -- so they keep
// the uncapped limit every other table in this port uses.
const PUSH_LIMIT: u64 = 8;

const MUTATIONS: &[(&str, &str, u64)] = &[
    ("push", "arg0", PUSH_LIMIT),
    ("unshift", "arg0", FOR_EACH_MANY),
    ("shift", "none", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

fn optional(value: Option<Value>) -> Value {
    value.unwrap_or_else(|| json!({"$undefined": true}))
}

fn project(projection: &str, item: &Value, index: u64) -> Value {
    match projection {
        "values" => item.clone(),
        _ => json!([index, item]),
    }
}
