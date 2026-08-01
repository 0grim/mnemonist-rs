//! [`ModuleSpec`] for `set` — the second free-function module.
//!
//! The mode is the one `crate::modules::sort` introduced: no instance, no
//! observable state, and the comparison rests on the return value **plus every
//! argument after the call**. Here that second half is not a nicety — `add`,
//! `subtract`, `intersect` and `disjunct` all return `undefined` and do their
//! whole job to their first argument, so without the argument echo four of the
//! fourteen functions would be compared against nothing at all.
//!
//! # What the grammar reaches that `test/set.js` does not
//!
//! * **Order that is not the obvious order.** `intersection` iterates its
//!   *smallest* argument, so the result's order depends on which one that was,
//!   and ties go to the first. Sets here vary in size freely, so both branches
//!   occur constantly. The original file's two-set case uses equal sizes and
//!   its variadic case intersects down to a single member, where order is not
//!   visible.
//! * **Empty sets, everywhere.** `intersection` bails out on the first empty
//!   argument, `difference` short-circuits on either, and `jaccard`/`overlap`
//!   answer `0` rather than dividing. The original file has exactly one empty
//!   set and uses it in three of fourteen blocks.
//! * **Both arity throws.** `intersection` and `union` refuse fewer than two
//!   arguments with a specific message; the original file never calls either
//!   with one.
//! * **More than four sets**, and sets whose members are strings *and* numbers
//!   at once.
//! * **Repeat application.** A program is 1..80 calls, so a set that has been
//!   through `disjunct` is fed to `intersect`, and so on.
//!
//! # Deliberately excluded
//!
//! * **Object members.** `Set` compares them by identity and the bridge
//!   refuses them (`docs/modules/set.md`); the fuzzer works against core, whose
//!   member type is generic, so an object member is not even expressible here.
//! * **`NaN` and `-0` members.** They are SameValueZero's two special cases and
//!   they belong to the *bridge's* key type, not to core — `JsKey` normalises
//!   both, and core is generic over any `Hash + Eq`. Fuzzing them here would
//!   test `serde_json`'s round-tripping rather than the port.
//!   `tests/boundary/set.js` covers them where they live.
//! * **The three `===` shortcuts.** Upstream skips work when two arguments are
//!   the same *object*. The oracle decodes every `{"$set": …}` into a fresh
//!   `Set`, so the identity never holds on either side and the shortcuts are
//!   equally unreachable — no false divergence, and no coverage either.
//!   `tests/boundary/set.js` passes one object twice; core's own tests pass one
//!   reference twice.

use mnemonist_core::structures::set::{self as core_set, OrderedSet};
use proptest::collection;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

pub struct SetSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proptest-regressions/set.txt");

/// The single upstream file this unit spans.
const FILES: &[&str] = &["set"];

/// The member pool, by index.
///
/// Small so that intersections are frequent rather than almost always empty,
/// and mixed so that `1` and `"1"` are two members — which they are to a `Set`,
/// and which a port that stringified its keys would get wrong.
const MEMBER_POOL: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Member {
    Number(i64),
    Text(&'static str),
}

impl Member {
    fn at(index: usize) -> Self {
        match index {
            0 => Self::Number(0),
            1 => Self::Number(1),
            2 => Self::Number(2),
            3 => Self::Number(3),
            4 => Self::Number(4),
            5 => Self::Number(5),
            6 => Self::Text("1"),
            7 => Self::Text("a"),
            8 => Self::Text("b"),
            _ => Self::Text(""),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Number(value) => json!(value),
            Self::Text(text) => json!(text),
        }
    }

    fn from_json(value: &Value) -> Self {
        match value {
            Value::Number(number) => {
                Self::Number(number.as_i64().expect("members are small integers"))
            }
            Value::String(text) => match text.as_str() {
                "1" => Self::Text("1"),
                "a" => Self::Text("a"),
                "b" => Self::Text("b"),
                "" => Self::Text(""),
                other => panic!("`{other}` is not a member this grammar generates"),
            },
            other => panic!("`{other}` is not a member this grammar generates"),
        }
    }
}

impl ModuleSpec for SetSpec {
    type Instance = ();

    fn module(&self) -> &'static str {
        "set"
    }

    fn functions(&self) -> &'static [&'static str] {
        FILES
    }

    /// None — see [`ModuleSpec::functions`] and the module docs.
    fn observations(&self) -> &'static [&'static str] {
        &[]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            // The two variadic ones, from ONE set (which throws) to five.
            3 => sets(1..6).prop_map(|args| Op::new("intersection", args)),
            3 => sets(1..6).prop_map(|args| Op::new("union", args)),
            2 => sets(2..3).prop_map(|args| Op::new("difference", args)),
            2 => sets(2..3).prop_map(|args| Op::new("symmetricDifference", args)),
            2 => sets(2..3).prop_map(|args| Op::new("isSubset", args)),
            2 => sets(2..3).prop_map(|args| Op::new("isSuperset", args)),
            3 => sets(2..3).prop_map(|args| Op::new("add", args)),
            3 => sets(2..3).prop_map(|args| Op::new("subtract", args)),
            3 => sets(2..3).prop_map(|args| Op::new("intersect", args)),
            3 => sets(2..3).prop_map(|args| Op::new("disjunct", args)),
            2 => sets(2..3).prop_map(|args| Op::new("intersectionSize", args)),
            2 => sets(2..3).prop_map(|args| Op::new("unionSize", args)),
            2 => sets(2..3).prop_map(|args| Op::new("jaccard", args)),
            2 => sets(2..3).prop_map(|args| Op::new("overlap", args)),
        ]
        .boxed()
    }

    /// Shorter than the default: each op carries its own subjects rather than
    /// accumulating state in one instance.
    fn program_len(&self) -> std::ops::Range<usize> {
        1..80
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {}

    fn apply(&self, _instance: &mut Self::Instance, op: &Op) -> Value {
        let mut sets: Vec<OrderedSet<Member>> = op.args.iter().map(read).collect();

        // The arguments are echoed back from the PARSED sets, never from
        // `op.args`. The oracle re-encodes the real `Set` objects it built, so
        // a generated `{"$set": [1, 1, 2]}` comes back as `{"$set": [1, 2]}`;
        // echoing the raw argument would report a divergence on every
        // generated duplicate and none of them would be about the port.
        let echoed =
            |sets: &[OrderedSet<Member>]| -> Vec<Value> { sets.iter().map(write).collect() };

        match op.name {
            // The two variadic queries. Both can throw, and the throw is
            // compared like any other result.
            "intersection" | "union" => {
                let borrowed: Vec<&OrderedSet<Member>> = sets.iter().collect();
                let outcome = if op.name == "intersection" {
                    core_set::intersection(&borrowed)
                } else {
                    core_set::union(&borrowed)
                };

                let result = match outcome {
                    Ok(set) => write(&set),
                    Err(message) => json!({"$throw": message}),
                };

                returned(result, echoed(&sets))
            }

            "difference" => {
                let result = write(&core_set::difference(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }
            "symmetricDifference" => {
                let result = write(&core_set::symmetric_difference(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }
            "isSubset" => {
                let result = json!(core_set::is_subset(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }
            "isSuperset" => {
                let result = json!(core_set::is_superset(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }

            // The four mutators. `$return` is `undefined`; everything they do
            // is in the echoed first argument.
            "add" | "subtract" | "intersect" | "disjunct" => {
                let right = sets.pop().expect("two set arguments");
                let mut left = sets.pop().expect("two set arguments");

                match op.name {
                    "add" => core_set::add(&mut left, &right),
                    "subtract" => core_set::subtract(&mut left, &right),
                    "intersect" => core_set::intersect(&mut left, &right),
                    _ => core_set::disjunct(&mut left, &right),
                };

                returned(
                    json!({"$undefined": true}),
                    vec![write(&left), write(&right)],
                )
            }

            "intersectionSize" => {
                let result = json!(core_set::intersection_size(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }
            "unionSize" => {
                let result = json!(core_set::union_size(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }
            "jaccard" => {
                let result = number_json(core_set::jaccard(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }
            "overlap" => {
                let result = number_json(core_set::overlap(&sets[0], &sets[1]));

                returned(result, echoed(&sets))
            }

            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    /// Nothing. See [`ModuleSpec::functions`].
    fn observe(&self, _instance: &mut Self::Instance) -> Value {
        json!({})
    }
}

/// `count` sets of 0..=8 members each, as `{"$set": […]}` arguments.
///
/// Sizes vary freely and independently, which is what makes `intersection`'s
/// smallest-argument rule -- and its tie-break -- reachable.
fn sets(count: std::ops::Range<usize>) -> BoxedStrategy<Vec<Value>> {
    collection::vec(collection::vec(0usize..MEMBER_POOL, 0..9), count)
        .prop_map(|sets| {
            sets.into_iter()
                .map(|members| {
                    let encoded: Vec<Value> = members
                        .into_iter()
                        .map(|index| Member::at(index).to_json())
                        .collect();

                    // Duplicates in the generated vector are legitimate: `new
                    // Set([1, 1])` is a one-member set, and dropping them here
                    // would stop the port's own de-duplication from being
                    // compared.
                    json!({"$set": encoded})
                })
                .collect()
        })
        .boxed()
}

/// Rebuild the [`OrderedSet`] a `{"$set": […]}` argument stands for.
fn read(argument: &Value) -> OrderedSet<Member> {
    let members = argument["$set"]
        .as_array()
        .expect("a set argument carries its members");

    OrderedSet::from_members(members.iter().map(Member::from_json))
}

/// A set, in the shape the oracle's `encode` renders one.
fn write(set: &OrderedSet<Member>) -> Value {
    let members: Vec<Value> = set.iter().map(Member::to_json).collect();

    json!({"$set": members})
}

/// The envelope `fuzz/oracle.js` wraps every free-function result in.
fn returned(result: Value, args: Vec<Value>) -> Value {
    json!({"$return": result, "$args": args})
}

/// A JavaScript number, encoded as `JSON.stringify` would encode it.
///
/// `jaccard` and `overlap` are the only functions here that can return a
/// non-integer, and both return the *integer* `0` on their zero shortcut.
/// serde_json compares `0` and `0.0` unequal, so emitting `json!(0.0)` there
/// would be a false divergence on every empty intersection. Duplicated from
/// `crate::modules::default_map`, which found the same trap on its first
/// campaign; shared code would be tidier and would put a merge conflict in a
/// file three worktrees edit.
fn number_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}
