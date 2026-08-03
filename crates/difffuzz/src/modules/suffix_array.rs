//! [`ModuleSpec`]s for `suffix-array` and `generalized-suffix-array`.
//!
//! # Where the entropy is, and why the op alphabet is small
//!
//! Every other module in this directory is a mutable container: the
//! constructor is trivial and the interesting behaviour is the op sequence.
//! Suffix arrays are the opposite. `SuffixArray` has **no mutating method at
//! all** — the whole computation happens in the constructor, and the only
//! things left to call are `toString` and `toJSON`. So this spec inverts the
//! usual balance: the constructor strategy carries the entropy and the programs
//! are short.
//!
//! That is a grammar decision worth stating rather than leaving to be inferred,
//! because "a grammar that omits a method omits every bug reachable only
//! through it" cuts the other way here: the alphabet below is **complete** for
//! `SuffixArray` (`toString`, `toJSON`) and for `GeneralizedSuffixArray`
//! (`toString`, `toJSON`, `longestCommonSubsequence`). The only upstream method
//! not fuzzed is `inspect`, which the port deliberately does not have — a Node
//! display convenience with no upstream assertion.
//!
//! # The input alphabet is chosen to hit both known defects
//!
//! Generated sequences draw from nine symbols picked for what they collide
//! with, not for realism:
//!
//! | symbol | why |
//! |---|---|
//! | `a` `b` `c` `A` | ordinary ASCII, and the repeated triples that make the recursion fire |
//! | `U+0000` | equal to the value `convert` pads with, so padding and content are indistinguishable |
//! | `U+0001` | equal to `GeneralizedSuffixArray`'s separator, so a member can forge one |
//! | `U+0100` | low byte `0x00` — collides with the padding under BUG-SUFFIX-ARRAY-1's 8-bit radix |
//! | `U+0141` | low byte `0x41` — collides with `A` under the same |
//! | `U+0201` | low byte `0x01` — collides with the separator under the same |
//!
//! Lengths run to 45, which covers all three residues of `l % 3` (BUG-SUFFIX-ARRAY-2 needs
//! `1`), several recursion depths, and the point where a reduced string's ranks
//! start to exceed 255.
//!
//! # Two module keys, one file
//!
//! `GeneralizedSuffixArray` is a second export of the same upstream file, and
//! the oracle addresses a module by `require`-ing `bench/upstream/<key>.js` and
//! calling `new` on whatever it exports. So it needs its own key and its own
//! file to require; `bench/upstream/generalized-suffix-array.js` is a two-line
//! re-export, labelled there as harness scaffolding rather than vendored
//! source. Gate 9 for the `suffix-array` unit is satisfied by the
//! `module=suffix-array` campaigns; the generalized ones are additional
//! evidence for the same unit.
//!
//! # What the grammar deliberately excludes
//!
//! * **An empty member list for `GeneralizedSuffixArray`.** Upstream reads
//!   `strings[0].length` unguarded and throws a `TypeError` from the
//!   *constructor*, which reaches the oracle's `init` rather than an op, and an
//!   `init` failure is apparatus failure by protocol — it would abort the
//!   campaign instead of reporting a divergence. The port refuses the same
//!   input with an error; the disagreement is documented in
//!   `docs/modules/suffix-array.md` rather than fuzzed.
//! * **Mixed member kinds** (a string next to a token array). Upstream decides
//!   from `strings[0]` and then spreads a string into its characters; the port
//!   rejects it. Another documented divergence, so fuzzing it would only
//!   re-report a known decision (DESIGN.md §3.7).

use mnemonist_core::structures::suffix_array::{GeneralizedSuffixArray, Sequence, SuffixArray};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Symbols the generator draws from. See the module docs for why each is here.
const ALPHABET: &[char] = &[
    'a', 'b', 'c', 'A', '\u{0}', '\u{1}', '\u{100}', '\u{141}', '\u{201}',
];

/// Tokens the arbitrary-sequence generator draws from.
///
/// Deliberately includes multi-character tokens, the empty token, and
/// integer-like tokens: upstream's alphabet is an object keyed by each token's
/// string form, and `Object.keys` orders integer-like keys numerically before
/// everything else — a difference the following `.sort()` is supposed to erase.
const TOKENS: &[&str] = &["a", "b", "ab", "", "0", "1", "10", "\u{1}", "\u{100}"];

/// Longest sequence the generator builds. See the module docs for why 45.
const MAX_LENGTH: usize = 45;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/suffix-array.txt"
);

/// As above, for the generalized variant.
pub const GENERALIZED_REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/generalized-suffix-array.txt"
);

/// One sequence argument: a JS string, or an array of token strings.
fn sequence_strategy() -> BoxedStrategy<Value> {
    prop_oneof![
        // Weighted towards strings: that is the shape both known defects were
        // found in, and the shape every upstream caller uses.
        3 => proptest::collection::vec(proptest::sample::select(ALPHABET), 0..=MAX_LENGTH)
            .prop_map(|chars| Value::String(chars.into_iter().collect())),
        1 => proptest::collection::vec(proptest::sample::select(TOKENS), 0..=MAX_LENGTH)
            .prop_map(|tokens| json!(tokens)),
    ]
    .boxed()
}

/// Rebuild the core's [`Sequence`] from what the oracle was sent.
fn sequence_from(value: &Value) -> Sequence {
    match value {
        Value::String(text) => Sequence::Text(text.encode_utf16().collect()),
        Value::Array(tokens) => Sequence::Tokens(
            tokens
                .iter()
                .map(|token| {
                    token
                        .as_str()
                        .expect("the generator only produces string tokens")
                        .to_owned()
                })
                .collect(),
        ),
        other => panic!("ctor argument `{other}` is not a sequence"),
    }
}

/// A [`Sequence`] encoded the way `fuzz/oracle.js` encodes the JS value it
/// corresponds to: a string stays a string, a token array becomes an array of
/// strings.
fn sequence_to_json(sequence: &Sequence) -> Value {
    match sequence {
        Sequence::Text(units) => Value::String(String::from_utf16_lossy(units)),
        Sequence::Tokens(tokens) => json!(tokens),
    }
}

pub struct SuffixArraySpec;

impl ModuleSpec for SuffixArraySpec {
    type Instance = SuffixArray;

    fn module(&self) -> &'static str {
        "suffix-array"
    }

    fn observations(&self) -> &'static [&'static str] {
        // `length`, `hasArbitrarySequence`, `string` and `array` are
        // properties; `toJSON` and `toString` are nullary methods. The oracle
        // tells them apart by `typeof`.
        &[
            "length",
            "hasArbitrarySequence",
            "string",
            "array",
            "toJSON",
            "toString",
        ]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        sequence_strategy()
            .prop_map(|sequence| vec![sequence])
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            Just(Op::new("toString", Vec::new())),
            Just(Op::new("toJSON", Vec::new())),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        // Short on purpose: the structure is immutable, so a long program
        // re-reads the same answer. Spending the budget on constructions
        // instead is what exercises the algorithm.
        1..4
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        SuffixArray::new(sequence_from(&args[0]))
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "toString" => json!(instance.to_joined_string()),
            "toJSON" => json!(instance.array()),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "length": instance.len(),
            "hasArbitrarySequence": instance.has_arbitrary_sequence(),
            "string": sequence_to_json(instance.sequence()),
            "array": instance.array(),
            "toJSON": instance.array(),
            "toString": instance.to_joined_string(),
        })
    }
}

pub struct GeneralizedSuffixArraySpec;

impl ModuleSpec for GeneralizedSuffixArraySpec {
    type Instance = GeneralizedSuffixArray;

    fn module(&self) -> &'static str {
        "generalized-suffix-array"
    }

    fn observations(&self) -> &'static [&'static str] {
        &[
            "length",
            "size",
            "firstLength",
            "hasArbitrarySequence",
            "text",
            "array",
            "toJSON",
            "toString",
        ]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // One argument, an array of 1..=4 members, all of the same kind. Empty
        // and mixed lists are excluded; see the module docs.
        prop_oneof![
            proptest::collection::vec(
                proptest::collection::vec(proptest::sample::select(ALPHABET), 0..=MAX_LENGTH)
                    .prop_map(|chars| Value::String(chars.into_iter().collect())),
                1..=4,
            ),
            proptest::collection::vec(
                proptest::collection::vec(proptest::sample::select(TOKENS), 0..=MAX_LENGTH)
                    .prop_map(|tokens| json!(tokens)),
                1..=4,
            ),
        ]
        .prop_map(|members| vec![Value::Array(members)])
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            // Weighted towards the one op with real logic in it.
            3 => Just(Op::new("longestCommonSubsequence", Vec::new())),
            1 => Just(Op::new("toString", Vec::new())),
            1 => Just(Op::new("toJSON", Vec::new())),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..4
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let members: Vec<Sequence> = args[0]
            .as_array()
            .expect("ctor arg 0 is the member list")
            .iter()
            .map(sequence_from)
            .collect();

        GeneralizedSuffixArray::new(&members)
            .expect("the generator produces non-empty, single-kind member lists")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "longestCommonSubsequence" => sequence_to_json(&instance.longest_common_subsequence()),
            "toString" => json!(instance.to_joined_string()),
            "toJSON" => json!(instance.array()),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "length": instance.len(),
            "size": instance.size(),
            "firstLength": instance.first_length(),
            "hasArbitrarySequence": instance.has_arbitrary_sequence(),
            "text": sequence_to_json(instance.text()),
            "array": instance.array(),
            "toJSON": instance.array(),
            "toString": instance.to_joined_string(),
        })
    }
}
