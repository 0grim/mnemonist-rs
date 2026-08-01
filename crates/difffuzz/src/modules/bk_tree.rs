//! [`ModuleSpec`] for `bk-tree`.
//!
//! # What this grammar is for
//!
//! Not a `Map`-backed module at all — see
//! `mnemonist_core::structures::bk_tree`'s docs — so this campaign is the
//! first to fuzz a genuine tree shape rather than `OrderedMap`. The
//! interesting algorithm is entirely in *how children are keyed by distance
//! and how `search` walks the range around a query's own distance*, so:
//!
//! * **Items are small integers in a narrow range** ([`RANGE`]) with
//!   **`distance(a, b) = |a - b|`** (`bkAbsDiff` in `fuzz/oracle.js`) — a real
//!   metric, cheap to compute identically on both sides, and dense enough
//!   over the range that repeated `add`s collide on distance constantly,
//!   which is the only way a node ever grows more than one child.
//! * **`search` is weighted almost as heavily as `add`.** There is no
//!   observation of the tree's shape (see below), so `search`'s return
//!   value — which reflects both *membership* (`d <= n`) and *traversal
//!   order* (children pushed ascending, popped descending — see the core
//!   module's docs) — is what the comparison actually rests on.
//!
//! # Observable state: `size` only
//!
//! Deliberately thin. Upstream's `root` is a plain, JSON-shaped object
//! (`{item, children: {...}}`), reachable and comparable in principle, but
//! nothing here exposes an equivalent from `mnemonist_core::structures::bk_tree`
//! — unlike `sparse-set`'s `dense`/`sparse`, which are `pub` specifically for
//! this reason. Relying on `search`'s result instead is deliberate: a
//! `search` with a radius wide enough to visit the whole tree, run after every
//! mutating op, reveals precisely the same information `root` would (which
//! items exist, at what distance from every point queried, and in what
//! order), and this campaign's `search` weight is chosen so that no more than
//! a couple of `add`s happen between one search and the next.
//!
//! # What this grammar deliberately does not cover
//!
//! **Object items** (`{value: 'hello'}`), and **string items** with a real
//! edit-distance metric — both in the original suite. Integers keep the
//! distance function a one-line, unmistakably-correct mirror on both sides;
//! `mnemonist_napi::bk_tree`'s bridge is exercised against strings and
//! `levenshtein` by the original mocha suite instead, and against `Item`
//! objects by `mnemonist_core::structures::bk_tree`'s own native tests.
//!
//! **A throwing distance function.** Upstream's `distance` is never given a
//! reason to throw in this grammar (`Math.abs` cannot), so the `try_add`/
//! `try_search` fallible path — the one this module was built to prove out —
//! is covered by `mnemonist_core::structures::bk_tree`'s native tests instead,
//! which control the failure directly rather than hoping a generated program
//! provokes it.

use mnemonist_core::structures::bk_tree::BkTree;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/bk-tree.txt"
);

/// Items and queries are drawn from `0..RANGE`. Narrow on purpose: with
/// `distance = |a - b|`, a range this small guarantees repeated collisions on
/// the same distance from any given node, which is the only way a node grows
/// more than one child.
const RANGE: i64 = 12;

fn item_at(index: usize) -> i64 {
    index as i64 % RANGE
}

fn dist(a: &i64, b: &i64) -> i64 {
    (a - b).abs()
}

pub struct BkTreeSpec;

impl ModuleSpec for BkTreeSpec {
    type Instance = BkTree<i64>;

    fn module(&self) -> &'static str {
        "bk-tree"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(vec![json!({"$factory": "bkAbsDiff"})]).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let item = (0..RANGE as usize).prop_map(|i| json!(item_at(i)));
        // Wide enough relative to `RANGE` that most searches visit the whole
        // tree, which is what makes `search` a stand-in for the "root"
        // observation this module does not have.
        let radius = 0..=(RANGE * 2);
        let query = (0..RANGE as usize).prop_map(|i| json!(item_at(i)));

        prop_oneof![
            5 => item.prop_map(|v| Op::new("add", vec![v])),
            4 => (radius, query).prop_map(|(n, q)| Op::new("search", vec![json!(n), q])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        BkTree::new()
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "add" => {
                let item = op.args[0].as_i64().expect("items are integers");

                instance.add(item, dist);

                json!({"$self": true})
            }
            "search" => {
                let n = op.args[0].as_i64().expect("n is an integer");
                let query = op.args[1].as_i64().expect("queries are integers");

                let found = instance.search(n, &query, dist);

                Value::Array(
                    found
                        .into_iter()
                        .map(|hit| json!({"item": hit.item, "distance": hit.distance}))
                        .collect(),
                )
            }
            "clear" => {
                instance.clear();
                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({ "size": instance.size() })
    }
}
