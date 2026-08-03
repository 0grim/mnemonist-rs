//! [`ModuleSpec`] for `symspell`.
//!
//! # What this grammar is for
//!
//! A **controlled-edit-distance vocabulary**, not random strings: every
//! word below sits at a known, small Damerau-Levenshtein distance from at
//! least one other word in the pool (substitutions, one deletion, one
//! insertion). This is not decoration: random strings would all be far
//! apart, every query would return an empty candidate set, and the campaign
//! would be green while proving nothing. `grammar_self_check` below measures that this pool actually
//! produces non-empty searches and threshold-boundary hits rather than
//! assuming it.
//!
//! `maxDistance` is varied across `1..=4` — the same range
//! `test/symspell.js` itself exercises (`2` default, `4` explicitly) — and
//! `verbosity` across all three of its valid values, since both gate which
//! branches `lookup` even reaches -- a distance of 1 exercises almost
//! nothing.

use mnemonist_core::structures::symspell::SymSpell as CoreSymSpell;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/symspell.txt"
);

/// Every pair here is within Damerau-Levenshtein distance 1-2 of at least
/// one other entry (substitution, single insertion or single deletion),
/// plus one deliberately distant word (`"zzz"`) so an empty-result search is
/// reachable too. Chosen, not randomly generated -- see the module docs.
const WORD_POOL: &[&str] = &[
    "hello", "mello", "jello", "jell", "hell", "hallo", "help", "world", "word", "ward", "john",
    "joan", "trello", "zzz",
];

fn word_at(index: usize) -> Value {
    json!(WORD_POOL[index % WORD_POOL.len()])
}

pub struct SymSpellSpec;

impl ModuleSpec for SymSpellSpec {
    type Instance = CoreSymSpell;

    fn module(&self) -> &'static str {
        "symspell"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "maxDistance", "verbosity"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (1u32..=4, 0u32..=2)
            .prop_map(|(max_distance, verbosity)| {
                vec![json!({
                    "maxDistance": max_distance,
                    "verbosity": verbosity,
                })]
            })
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
        let max_distance = args[0]["maxDistance"]
            .as_f64()
            .expect("ctor arg 0.maxDistance is a JSON number");
        let verbosity = args[0]["verbosity"]
            .as_u64()
            .expect("ctor arg 0.verbosity is a JSON number") as u8;

        CoreSymSpell::new(max_distance, verbosity)
            .expect("ctor_strategy only generates valid options")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "add" => {
                let word = op.args[0].as_str().expect("add's argument is a string");

                instance.add(word);

                json!({"$self": true})
            }
            "search" => {
                let query = op.args[0].as_str().expect("search's argument is a string");

                let suggestions: Vec<Value> = instance
                    .search(query)
                    .into_iter()
                    .map(|suggestion| {
                        json!({
                            "term": suggestion.term,
                            "distance": suggestion.distance,
                            "count": suggestion.count,
                        })
                    })
                    .collect();

                json!(suggestions)
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
            // `number_json`, not a bare `json!(f64)`: this constructor only
            // ever generates whole numbers, but `json!(1.0_f64)` renders as
            // `1.0`, a distinct `serde_json::Value` from the `1` the oracle
            // sends for the same JS number -- caught by this campaign's own
            // very first run, at construction, before a single op executed.
            "maxDistance": number_json(instance.max_distance()),
            "verbosity": instance.verbosity(),
        })
    }
}

/// Same rendering `sort`/`default_map`/`bloom_filter`/`set`/`vector`/
/// `multi_array` all use, duplicated per module rather than factored out to
/// match the existing pattern in this crate: a whole number in the JS
/// safe-integer range
/// prints without a decimal point, matching `JSON.stringify`.
fn number_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        return json!(value as i64);
    }

    json!(value)
}

/// Direct evidence that this grammar reaches the states a campaign for this
/// module has to reach: how many `search` calls come back with
/// at least one suggestion, and how many suggestions land at exactly the
/// configured `maxDistance` (a threshold-boundary hit, not merely "close").
/// Runs the strategies directly, no oracle, no `node`.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    #[test]
    fn the_grammar_produces_non_empty_searches_and_threshold_boundary_hits() {
        let spec = SymSpellSpec;
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
            let max_distance = instance.max_distance();

            for op in &ops {
                let result = spec.apply(&mut instance, op);

                if op.name == "search" {
                    total_searches += 1;

                    let suggestions = result.as_array().expect("search returns an array");

                    if !suggestions.is_empty() {
                        non_empty_searches += 1;
                    }

                    if suggestions
                        .iter()
                        .any(|s| s["distance"].as_f64() == Some(max_distance))
                    {
                        boundary_hits += 1;
                    }
                }
            }
        }

        eprintln!(
            "symspell grammar: {non_empty_searches}/{total_searches} searches \
             non-empty, {boundary_hits} at exactly the configured maxDistance"
        );

        assert!(
            non_empty_searches > 100,
            "the grammar should routinely return non-empty candidate sets, \
             not rarely: only {non_empty_searches}/{total_searches}"
        );
        assert!(
            boundary_hits > 30,
            "the grammar should routinely hit the maxDistance boundary \
             exactly, not rarely: only {boundary_hits}/{total_searches}"
        );
    }
}
