//! [`ModuleSpec`] for `inverted-index`.
//!
//! # Grammar: identity tokenizer, so documents ARE token arrays
//!
//! Documents must share tokens, or every posting list has length one and
//! the campaign proves only that the index can store words. Reaching real overlap with a genuine natural-
//! language tokenizer (stemming, stopwords, `lodash/words`) would mean
//! porting or mirroring one just for the fuzz harness — a second copy of
//! machinery the module itself does not need. Instead this grammar
//! constructs every `InvertedIndex` with `descriptor` omitted, so both sides
//! use upstream's own `identity` tokenizer, and generates documents as
//! **arrays of tokens drawn from a five-word pool** directly — `identity(doc)
//! === doc`, and `Array.isArray(doc)` is true by construction, so the
//! constructor's and `add`'s own guards are satisfied for free. A 1..4-token
//! document over a five-word pool collides with an earlier document's tokens
//! constantly; [`crate::modules::inverted_index`]'s own `grammar_self_check`
//! (below) measures how often, rather than asserting it from the op weights.
//!
//! # Two cursor shapes, because this module has two
//!
//! `documents()` is [`DocumentsCursor`] (a frozen length over `items`);
//! `tokens()` is a real `Map` cursor over `mapping` ([`MapCursor`]). Both are
//! fuzzed, tagged by [`FuzzCursor`], because they are genuinely different
//! walks — see `mnemonist_core::structures::inverted_index`'s own docs for
//! why one is not built on the other.
//!
//! # BUG-INVERTED-INDEX-1 lives here as an invariant, not a mutation table
//!
//! `$forEach` always drives `InvertedIndex::for_each`'s zero-length cursor,
//! so `seen` is `[]` on every single generated case, regardless of `size`.
//! There is nothing to interleave a mutation with — the callback never runs
//! to interleave anything from — so the mutation table
//! [`crate::spec::for_each_strategy`] takes is empty, and every generated
//! `$forEach` op is upstream's own "plain walk" shape. The op is included
//! anyway, specifically so the differential campaign is *positive* evidence
//! that the port's `forEach` matches upstream's brokenness across thousands
//! of index states, not merely the original suite's one hand-picked call.
//!
//! # Observable state
//!
//! `size`, `dimension`, `items` (the full stored document list, in order)
//! and `mapping` (the full token → posting-list index, as an
//! order-sensitive `$map` — entry order is part of what `tokens()` promises,
//! same reasoning as `default-map`'s `items`).

use mnemonist_core::map::MapCursor;
use mnemonist_core::structures::inverted_index::{DocumentsCursor, InvertedIndex, TokensCursor};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{for_each_strategy, ModuleSpec, Op};

/// Walk the index's current `mapping` in first-seen order, applying `visit`
/// to each `(token, postings)` pair — used by [`InvertedIndexSpec::observe`]
/// and the grammar self-check, neither of which needs a persistent cursor
/// (both always want the WHOLE map, right now).
fn for_each_mapping_entry(
    index: &InvertedIndex<Vec<String>, String>,
    mut visit: impl FnMut(&str, &[usize]),
) {
    let mapping = index.mapping();
    let mut cursor = MapCursor::open();

    while let Some((token, postings)) = cursor.step(&mapping) {
        visit(token, postings);
    }
}

pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/inverted-index.txt"
);

/// Small and deliberately narrow -- see the module docs on why documents
/// need to collide on tokens constantly rather than each minting its own.
const TOKEN_POOL: [&str; 5] = ["a", "b", "c", "d", "e"];

fn token_at(index: usize) -> String {
    TOKEN_POOL[index % TOKEN_POOL.len()].to_owned()
}

fn tokens_strategy(len: std::ops::RangeInclusive<usize>) -> BoxedStrategy<Vec<String>> {
    proptest::collection::vec(0..TOKEN_POOL.len(), len)
        .prop_map(|indices| indices.into_iter().map(token_at).collect())
        .boxed()
}

fn decode_tokens(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("a document/query argument is a JSON array of tokens")
        .iter()
        .map(|token| {
            token
                .as_str()
                .expect("every token in this grammar is a plain string")
                .to_owned()
        })
        .collect()
}

/// One of the two cursor shapes this module has -- see the module docs.
/// Both are self-contained (each captures its own `Rc` clone of the array/map
/// object it walks), so neither needs the `Instance`'s own `index` to step.
enum FuzzCursor {
    Documents(DocumentsCursor<Vec<String>>),
    Tokens(TokensCursor<String>),
}

pub struct Instance {
    index: InvertedIndex<Vec<String>, String>,
    cursor: Option<FuzzCursor>,
}

pub struct InvertedIndexSpec;

impl ModuleSpec for InvertedIndexSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "inverted-index"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "dimension", "items", "mapping"]
    }

    /// `new InvertedIndex()` — descriptor omitted, so both sides fall back to
    /// upstream's own `identity`. See the module docs.
    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        // At least one token, so `add` always indexes something; a query MAY
        // be empty (upstream's own `if (!tokens.length) return [];` branch).
        let doc = tokens_strategy(1..=4).prop_map(|tokens| Op::new("add", vec![json!(tokens)]));
        let query = tokens_strategy(0..=3).prop_map(|tokens| Op::new("get", vec![json!(tokens)]));

        prop_oneof![
            5 => doc,
            4 => query,
            1 => Just(Op::new("clear", vec![])),
            2 => prop_oneof![
                Just(Op::new("$iter", vec![json!("documents")])),
                Just(Op::new("$iter", vec![json!("tokens")])),
            ],
            4 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            // Always the plain-walk shape -- see the module docs on BUG-INVERTED-INDEX-1.
            1 => for_each_strategy(&[]),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        Instance {
            index: InvertedIndex::new(),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "add" => {
                let tokens = decode_tokens(&op.args[0]);

                // Identity tokenizer: the tokens ARE the document. See the
                // module docs.
                instance.index.add(tokens.clone(), tokens);

                json!({"$self": true})
            }
            "get" => {
                let tokens = decode_tokens(&op.args[0]);
                let docs = instance.index.get(&tokens);

                Value::Array(docs.into_iter().map(|doc| json!(doc)).collect())
            }
            "clear" => {
                instance.index.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                let which = op.args[0].as_str().expect("$iter names a projection");

                instance.cursor = Some(match which {
                    "documents" => FuzzCursor::Documents(instance.index.documents()),
                    "tokens" => FuzzCursor::Tokens(instance.index.tokens()),
                    other => panic!("`{other}` is not an iterator this module has"),
                });

                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(FuzzCursor::Documents(cursor)) => match cursor.step() {
                    None => json!({"done": true, "value": {"$undefined": true}}),
                    Some(doc) => json!({"done": false, "value": doc}),
                },
                Some(FuzzCursor::Tokens(cursor)) => match cursor.step() {
                    None => json!({"done": true, "value": {"$undefined": true}}),
                    Some(token) => json!({"done": false, "value": token}),
                },
            },
            // `Array.from(instance)` — the collection's `Symbol.iterator`,
            // aliased to `documents` upstream (`ITERATOR_FACTORIES`), so a
            // fresh cursor every call, unlike `$next`.
            "$spread" => {
                let mut cursor = instance.index.documents();
                let mut docs = Vec::new();

                while let Some(doc) = cursor.step() {
                    docs.push(json!(doc));
                }

                Value::Array(docs)
            }
            // BUG-INVERTED-INDEX-1: `InvertedIndex::for_each` hands back a cursor frozen at
            // length zero, so this is always `[]` -- see the module docs.
            "$forEach" => {
                let mut cursor = instance.index.for_each();
                let mut seen = Vec::new();

                while let Some(doc) = cursor.step() {
                    seen.push(json!(doc));
                }

                json!({"seen": seen})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        let items: Vec<Value> = instance
            .index
            .items()
            .iter()
            .map(|doc| json!(doc))
            .collect();

        let mut mapping_entries = Vec::new();
        for_each_mapping_entry(&instance.index, |token, postings| {
            mapping_entries.push(json!([token, postings]));
        });

        json!({
            "size": instance.index.size(),
            "dimension": instance.index.dimension(),
            "items": items,
            "mapping": {"$map": mapping_entries},
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    /// Measures, rather than infers from the op weights, how often generated
    /// documents actually share a token with an earlier document — the
    /// one property this whole grammar exists to produce.
    ///
    /// Generates `(ctor, ops)` pairs by hand via `ValueTree::new_tree` and
    /// runs them in an ordinary `for` loop, the same shape
    /// `crate::modules::lru_cache`'s own `grammar_self_check` uses — it
    /// needs mutable accumulators across many generated programs, and
    /// `TestRunner::run` requires its closure to be `Fn`, not `FnMut`.
    #[test]
    fn grammar_self_check_documents_collide_on_tokens_constantly() {
        let spec = InvertedIndexSpec;
        let mut runner = TestRunner::new(Config::default());

        let mut total_docs = 0u64;
        let mut multi_doc_postings = 0u64;
        let mut total_postings = 0u64;

        // Only `add`, generated directly rather than by filtering the full
        // op alphabet down to it -- filtering a `prop_oneof!` this unbalanced
        // rejects too often for `proptest`'s local-reject budget over 300
        // draws per case.
        let add_op = tokens_strategy(1..=4).prop_map(|tokens| Op::new("add", vec![json!(tokens)]));
        let strategy = proptest::collection::vec(add_op, 1..300);

        for _ in 0..400 {
            let ops = strategy
                .new_tree(&mut runner)
                .expect("op_strategy never rejects")
                .current();
            let mut instance = spec.construct(&[]);

            for op in &ops {
                spec.apply(&mut instance, op);
                total_docs += 1;
            }

            for_each_mapping_entry(&instance.index, |_token, postings| {
                total_postings += 1;
                if postings.len() > 1 {
                    multi_doc_postings += 1;
                }
            });
        }

        assert!(
            total_docs > 1000,
            "the self-check ran too little to mean anything"
        );
        assert!(
            total_postings > 0,
            "no tokens were ever recorded -- the grammar is broken"
        );

        let collision_rate = multi_doc_postings as f64 / total_postings as f64;

        eprintln!(
            "inverted-index grammar: {total_docs} documents added, {total_postings} \
             posting lists, {multi_doc_postings} spanning more than one document \
             ({:.1}%)",
            100.0 * collision_rate
        );

        assert!(
            collision_rate > 0.3,
            "only {collision_rate:.2} of posting lists span more than one document \
             ({multi_doc_postings}/{total_postings}) -- the grammar proves only that \
             the index can store words, which is the failure this grammar exists \
             to avoid"
        );
    }
}
