//! [`ModuleSpec`] for `passjoin-index`.
//!
//! # What this grammar is for
//!
//! Same controlled-edit-distance philosophy as `symspell`'s spec: a fixed
//! pool of words at known small edit distances from one another (not random
//! strings, which would all be far apart and make every `search` empty), and
//! `k` varied across `1..=3` -- the exact range `test/passjoin-index.js`
//! itself uses (`k1`/`k2`/`k3`) -- since `k` decides the partition arithmetic
//! and the candidate-generation branches CLAUDE.md calls out as the sharpest
//! place an off-by-one would hide.
//!
//! `levenshtein` is `fuzz/oracle.js`'s `pjLeven` factory, the real `leven`
//! npm package -- the exact function `test/passjoin-index.js` itself uses,
//! not a simplified stand-in the way `bk-tree`'s spec uses `bkAbsDiff` for an
//! arbitrary caller-supplied metric. [`leven`] below is this crate's own
//! plain Levenshtein implementation (no transposition), computing the
//! identical distance value `leven` does for any pair of strings -- the two
//! differ only in algorithmic optimisation, never in the number they return.

use mnemonist_core::structures::passjoin_index::PassjoinIndex as CoreIndex;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/passjoin-index.txt"
);

/// Every pair here is within edit distance 1-3 of at least one other entry.
/// Chosen, not randomly generated -- see the module docs.
const WORD_POOL: &[&str] = &[
    "benjamin", "benjomon", "benja", "benjo", "paule", "paul", "pa", "pat", "ab", "a", "b", "",
    "failed", "flailed", "railed",
];

fn word_at(index: usize) -> Value {
    json!(WORD_POOL[index % WORD_POOL.len()])
}

/// Plain textbook Levenshtein distance (insert/delete/substitute, no
/// transposition) -- computes the identical value the real `leven` npm
/// package does; see the module docs.
pub fn leven(a: &str, b: &str) -> i64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut row: Vec<i64> = (0..=n as i64).collect();

    for i in 1..=m {
        let mut prev = row[0];
        row[0] = i as i64;

        for j in 1..=n {
            let temp = row[j];
            row[j] = if a[i - 1] == b[j - 1] {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j - 1])
            };
            prev = temp;
        }
    }

    row[n]
}

pub struct PassjoinIndexSpec;

impl ModuleSpec for PassjoinIndexSpec {
    type Instance = CoreIndex;

    fn module(&self) -> &'static str {
        "passjoin-index"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "k"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (1u32..=3)
            .prop_map(|k| vec![json!({ "$factory": "pjLeven" }), json!(k)])
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        let word = (0..WORD_POOL.len()).prop_map(word_at);

        prop_oneof![
            5 => word.clone().prop_map(|w| Op::new("add", vec![w])),
            5 => word.prop_map(|w| Op::new("search", vec![w])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let k = args[1].as_u64().expect("ctor arg 1 is a JSON number") as i64;

        CoreIndex::new(k).expect("ctor_strategy only generates valid k")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "add" => {
                let value = op.args[0].as_str().expect("add's argument is a string");

                instance.add(value);

                json!({"$self": true})
            }
            "search" => {
                let query = op.args[0].as_str().expect("search's argument is a string");

                let matches: Result<Vec<String>, std::convert::Infallible> =
                    instance.try_search(query, |a, b| Ok(leven(a, b)));

                json!({ "$set": matches.expect("leven cannot fail") })
            }
            "clear" => {
                instance.clear();

                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.size(),
            "k": instance.k(),
        })
    }
}

/// Direct evidence that this grammar reaches the states CLAUDE.md's
/// fuzz-campaign guidance asks for: how many `search` calls come back with
/// at least one candidate, and how many pull in a candidate whose real
/// distance is exactly `k` (a threshold-boundary hit). Runs the strategies
/// directly, no oracle, no `node`.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    #[test]
    fn the_grammar_produces_non_empty_searches_and_threshold_boundary_hits() {
        let spec = PassjoinIndexSpec;
        let mut runner = TestRunner::new(Config::default());
        let mut non_empty_searches = 0u64;
        let mut boundary_hits = 0u64;
        let mut total_searches = 0u64;

        for _ in 0..400 {
            let ctor = spec
                .ctor_strategy()
                .new_tree(&mut runner)
                .expect("ctor_strategy never rejects")
                .current();
            let ops = proptest::collection::vec(spec.op_strategy(&ctor), 1..200)
                .new_tree(&mut runner)
                .expect("op_strategy never rejects")
                .current();
            let mut instance = spec.construct(&ctor);
            let k = instance.k();

            for op in &ops {
                if op.name == "search" {
                    let query = op.args[0].as_str().unwrap();
                    let before = instance.size();
                    let _ = before; // silence unused in case of future refactor

                    let matches = instance.search(query, leven);
                    total_searches += 1;

                    if !matches.is_empty() {
                        non_empty_searches += 1;
                    }

                    if matches.iter().any(|candidate| leven(query, candidate) == k) {
                        boundary_hits += 1;
                    }
                } else {
                    spec.apply(&mut instance, op);
                }
            }
        }

        eprintln!(
            "passjoin-index grammar: {non_empty_searches}/{total_searches} \
             searches non-empty, {boundary_hits} pulled in a candidate at \
             exactly distance k"
        );

        assert!(
            non_empty_searches > 100,
            "the grammar should routinely return non-empty candidate sets, \
             not rarely: only {non_empty_searches}/{total_searches}"
        );
        assert!(
            boundary_hits > 30,
            "the grammar should routinely exercise the k threshold boundary, \
             not rarely: only {boundary_hits}/{total_searches}"
        );
    }
}

/// This module's other job: the static helpers get their own campaign-free
/// but oracle-agnostic pin, since the differential campaign above only
/// exercises `add`/`search`/`clear` and never calls
/// `comparator`/`segments`/`segmentPos`/`partition`/`countKeys`/
/// `multiMatchAware*` at all (`test/passjoin-index.js` checks those against
/// fixed literal examples, already reproduced as native tests in
/// `mnemonist_core::structures::passjoin_index`). Re-imported here only so
/// `cargo clippy` catches an accidental unused-import regression on this
/// module's `use` list if a future edit removes a call site elsewhere.
#[cfg(test)]
mod statics_are_reachable {
    use std::cmp::Ordering;

    use mnemonist_core::structures::passjoin_index::{
        comparator, count_keys, multi_match_aware_interval, multi_match_aware_substrings,
        partition, segment_pos, segments,
    };

    #[test]
    fn every_static_helper_is_callable() {
        let _ = comparator("a", "b");
        let _ = count_keys(2, 5);
        let _ = partition(2, 5);
        let _ = segments(2, "hello");
        let _ = segment_pos(2, 1, "hello");
        let _ = multi_match_aware_interval(2, 0, 0, 5, 0, 2);
        let _ = multi_match_aware_substrings(2, "hello", 5, 0, 0, 2);
        assert_eq!(comparator("a", "a"), Ordering::Equal);
    }
}
