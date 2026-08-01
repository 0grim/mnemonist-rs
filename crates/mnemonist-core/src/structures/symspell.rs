//! Port of upstream `symspell.js` (mnemonist v0.40.4, 548 LOC).
//!
//! A [SymSpell](https://github.com/wolfgarbe/symspell) index: instead of
//! computing edit distance between a query and every dictionary word, it
//! precomputes — for every added word — the set of strings reachable by
//! deleting up to `maxDistance` characters, and indexes *those* as keys
//! pointing back at the word(s) they came from. A query then only has to
//! generate its own deletes (symmetric with the index side, hence
//! "symmetric delete") and look them up directly, at the cost of a larger
//! index rather than a slower query.
//!
//! # The dictionary's two-shaped entries are load-bearing, not cosmetic
//!
//! Upstream's `this.dictionary` (a `Object.create(null)` map from string to
//! entry) stores an entry two different ways: a bare `number` (the *first*
//! word index a given delete-form was reached from, when nothing has
//! reached it a second time yet) or a full `{suggestions: Set, count}`
//! object (once a second delete reaches the same form, or once the form is
//! itself an added word). This is not merely a memory optimisation to skip
//! allocating an object for a delete-form nobody has revisited — the
//! promotion point (`typeof item === 'number'`) is a real branch upstream's
//! own `add`/lookup logic takes, and [`Entry`] reproduces the same two
//! shapes and the same promotion rather than always allocating
//! [`DictItem`], so that `first` (`suggestions.values().next().value`, used
//! by [`add_lowest_distance`]'s verbosity pruning) is always genuinely "the
//! first index this form was ever reached from", matching a JS `Set`'s
//! insertion-order iteration.
//!
//! One word's dictionary key can serve **both** roles at once: a word that
//! is itself a real, added dictionary entry (`count > 0`) can simultaneously
//! be a delete-form some *other*, longer word reaches (`suggestions`
//! non-empty) — `test/symspell.js`'s own data exercises this (`'Hell'` is
//! both an added word and `'Hello'`'s length-1 delete).
//!
//! # `maxDistance` is a threshold, not an integer count
//!
//! Upstream's constructor message says "should be an integer greater than
//! 0", but the actual guard is `typeof maxDistance !== 'number' ||
//! maxDistance <= 0` — no integrality check at all. A fractional
//! `maxDistance` (e.g. `2.5`) is accepted and used exactly as given in every
//! comparison (`distance <= maxDistance`, `distance < max`). This port keeps
//! `max_distance` as `f64` for that reason, matching runtime behaviour over
//! the doc comment's (inaccurate) description of it.
//!
//! # Distance is upstream's own Damerau-Levenshtein, not a general one
//!
//! [`damerau_levenshtein`] is transcribed from upstream's private helper
//! (the Lowrance-Wagner algorithm with an infinite-distance sentinel row/
//! column and a per-character "last seen row" map), not from a textbook
//! definition or the `damerau-levenshtein` npm package `test/symspell.js`
//! itself uses only to *validate* results, never to compute them. Bug-for-bug
//! fidelity means matching upstream's own function, which is what the
//! differential fuzzer replays against.
//!
//! # ASCII/BMP scope
//!
//! String lengths and substrings are computed over Rust `char`s (Unicode
//! scalar values), not upstream's UTF-16 code units. The two agree for every
//! codepoint in the Basic Multilingual Plane (which includes plain ASCII,
//! the only alphabet `test/symspell.js` and this port's fuzz grammar use)
//! and diverge only for astral characters (surrogate pairs in UTF-16, one
//! scalar value in Rust) — not exercised by any test or campaign, and
//! recorded as a stated scope limit rather than silently assumed away.

use std::collections::HashMap;

/// `mnemonist/SymSpell.constructor: invalid \`maxDistance\` option. Should be
/// a integer greater than 0.` (verbatim, including upstream's grammar).
pub const INVALID_MAX_DISTANCE: &str =
    "mnemonist/SymSpell.constructor: invalid `maxDistance` option. Should be a integer greater \
     than 0.";

/// `mnemonist/SymSpell.constructor: invalid \`verbosity\` option. Should be
/// either 0, 1 or 2.`
pub const INVALID_VERBOSITY: &str =
    "mnemonist/SymSpell.constructor: invalid `verbosity` option. Should be either 0, 1 or 2.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidMaxDistance,
    InvalidVerbosity,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidMaxDistance => INVALID_MAX_DISTANCE,
            Self::InvalidVerbosity => INVALID_VERBOSITY,
        })
    }
}

impl std::error::Error for Error {}

/// `createDictionaryItem`'s full shape: a set of word indices reachable from
/// this dictionary key by one delete-chain, plus how many times the key
/// itself was added as a real word (`0` if it never was).
#[derive(Debug, Clone, Default)]
pub struct DictItem {
    /// Insertion-ordered, de-duplicated — a JS `Set`'s iteration order.
    suggestions: Vec<usize>,
    count: usize,
}

impl DictItem {
    fn with_seed(index: usize) -> Self {
        Self {
            suggestions: vec![index],
            count: 0,
        }
    }

    fn has(&self, index: usize) -> bool {
        self.suggestions.contains(&index)
    }

    /// The first-ever suggestion added — upstream's
    /// `suggestions.values().next().value`.
    fn first(&self) -> Option<usize> {
        self.suggestions.first().copied()
    }
}

/// One dictionary slot: the compact form (a bare word index, upstream's
/// `typeof item === 'number'`) or the promoted, full form. See the module
/// docs for why both are reproduced rather than always allocating
/// [`DictItem`].
#[derive(Debug, Clone)]
enum Entry {
    Compact(usize),
    Full(DictItem),
}

impl Entry {
    fn count(&self) -> usize {
        match self {
            Self::Compact(_) => 0,
            Self::Full(item) => item.count,
        }
    }

    /// Upstream's local `item = createDictionaryItem(item)` promotion, as
    /// read during `lookup` — never persisted back to the dictionary (that
    /// only happens in `add`), so a `Compact` entry is read here exactly as
    /// if it had been promoted: one suggestion (the encoded index) and a
    /// `count` of `0`. Skipping this for `Compact` entries (an earlier draft
    /// of this port did, via a `None`-returning `as_full`) silently dropped
    /// every suggestion reachable only through a delete-form nothing else
    /// had reached yet — caught by the very first differential-fuzz
    /// campaign run for this module: `add("jello")` then `search("hello")`
    /// at `maxDistance: 1` found nothing, where upstream finds `jello`.
    fn suggestions(&self) -> std::borrow::Cow<'_, [usize]> {
        match self {
            Self::Compact(seed) => std::borrow::Cow::Owned(vec![*seed]),
            Self::Full(item) => std::borrow::Cow::Borrowed(&item.suggestions),
        }
    }
}

/// One search hit — upstream's `{term, distance, count}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub term: String,
    pub distance: i64,
    pub count: usize,
}

/// Upstream's `SymSpell`.
#[derive(Debug, Clone)]
pub struct SymSpell {
    max_distance: f64,
    verbosity: u8,
    size: usize,
    dictionary: HashMap<String, Entry>,
    max_length: usize,
    words: Vec<String>,
}

impl SymSpell {
    /// `new SymSpell({maxDistance, verbosity})`, both already defaulted by
    /// the caller (upstream's `DEFAULT_MAX_DISTANCE = 2`, `DEFAULT_VERBOSITY
    /// = 2`) — resolving JS's "was this option even passed" belongs to the
    /// bridge, same split `crate::structures::vector`'s constructors make.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidMaxDistance`] for `max_distance <= 0`. `NaN` is
    /// **not** included: `NaN <= 0.0` is `false` in both languages (every
    /// `NaN` comparison is), so upstream's own guard lets a `NaN`
    /// `maxDistance` through uncaught and it propagates into every later
    /// comparison as `false` — reproduced here rather than "fixed", since
    /// silently rejecting it would be *more* correct than upstream, which
    /// this port does not do. See `docs/modules/symspell.md`.
    /// [`Error::InvalidVerbosity`] unless `verbosity` is `0`, `1` or `2`.
    pub fn new(max_distance: f64, verbosity: u8) -> Result<Self, Error> {
        if max_distance <= 0.0 {
            return Err(Error::InvalidMaxDistance);
        }

        if verbosity > 2 {
            return Err(Error::InvalidVerbosity);
        }

        Ok(Self {
            max_distance,
            verbosity,
            size: 0,
            dictionary: HashMap::new(),
            max_length: 0,
            words: Vec::new(),
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn max_distance(&self) -> f64 {
        self.max_distance
    }

    pub fn verbosity(&self) -> u8 {
        self.verbosity
    }

    pub fn clear(&mut self) {
        self.size = 0;
        self.dictionary.clear();
        self.max_length = 0;
        self.words.clear();
    }

    /// `#.add(word)`.
    pub fn add(&mut self, word: &str) {
        let is_new_word = {
            let entry = self.dictionary.get(word).cloned();

            match entry {
                Some(Entry::Compact(seed)) => {
                    let mut item = DictItem::with_seed(seed);
                    item.count += 1;
                    let is_first = item.count == 1;
                    self.dictionary.insert(word.to_owned(), Entry::Full(item));
                    is_first
                }
                Some(Entry::Full(mut item)) => {
                    item.count += 1;
                    let is_first = item.count == 1;
                    self.dictionary.insert(word.to_owned(), Entry::Full(item));
                    is_first
                }
                None => {
                    let mut item = DictItem::default();
                    item.count += 1;
                    self.dictionary.insert(word.to_owned(), Entry::Full(item));

                    if word.chars().count() > self.max_length {
                        self.max_length = word.chars().count();
                    }

                    true
                }
            }
        };

        if is_new_word {
            let number = self.words.len();
            self.words.push(word.to_owned());

            let mut deletes = std::collections::HashSet::new();
            edits(word, 0, self.max_distance, &mut deletes);

            // Iteration order over a `HashSet` is not upstream's insertion
            // order, but `add`'s own effect on `deletes.forEach`'s bodies is
            // order-independent: each iteration only ever touches the
            // dictionary entry keyed by that single `deletedItem`, so which
            // order distinct keys are visited in cannot change the final
            // dictionary contents. See the module docs' fuzz section for
            // the campaign that checks this holds for `search`'s own output
            // ordering too (which does depend on `words`' insertion order,
            // untouched here).
            for deleted_item in deletes {
                match self.dictionary.get(&deleted_item).cloned() {
                    Some(Entry::Compact(seed)) => {
                        let mut item = DictItem::with_seed(seed);

                        if !item.has(number) {
                            add_lowest_distance(
                                &self.words,
                                self.verbosity,
                                &mut item,
                                word,
                                number,
                                &deleted_item,
                            );
                        }

                        self.dictionary.insert(deleted_item, Entry::Full(item));
                    }
                    Some(Entry::Full(mut item)) => {
                        if !item.has(number) {
                            add_lowest_distance(
                                &self.words,
                                self.verbosity,
                                &mut item,
                                word,
                                number,
                                &deleted_item,
                            );
                        }

                        self.dictionary.insert(deleted_item, Entry::Full(item));
                    }
                    None => {
                        self.dictionary.insert(deleted_item, Entry::Compact(number));
                    }
                }
            }
        }

        self.size += 1;
    }

    /// `#.search(input)`.
    pub fn search(&self, input: &str) -> Vec<Suggestion> {
        lookup(
            &self.dictionary,
            &self.words,
            self.verbosity,
            self.max_distance,
            self.max_length,
            input,
        )
    }
}

/// `edits(word, distance, max, deletes)` — every string reachable from
/// `word` by 1..=`max` character deletions, deduplicated as it goes so
/// repeated characters do not cause redundant recursion.
fn edits(word: &str, distance: usize, max: f64, deletes: &mut std::collections::HashSet<String>) {
    let distance = distance + 1;
    let chars: Vec<char> = word.chars().collect();
    let length = chars.len();

    if length <= 1 {
        return;
    }

    for i in 0..length {
        let mut deleted: String = chars[..i].iter().collect();
        deleted.extend(chars[i + 1..].iter());

        if !deletes.contains(&deleted) {
            let inserted = deletes.insert(deleted.clone());
            debug_assert!(inserted);

            if (distance as f64) < max {
                edits(&deleted, distance, max, deletes);
            }
        }
    }
}

/// `addLowestDistance(words, verbosity, item, suggestion, int, deletedItem)`.
fn add_lowest_distance(
    words: &[String],
    verbosity: u8,
    item: &mut DictItem,
    suggestion: &str,
    int: usize,
    deleted_item: &str,
) {
    let deleted_len = deleted_item.chars().count() as i64;
    let suggestion_len = suggestion.chars().count() as i64;

    if let Some(first) = item.first() {
        let first_len = words[first].chars().count() as i64;

        if verbosity < 2
            && !item.suggestions.is_empty()
            && (first_len - deleted_len) > (suggestion_len - deleted_len)
        {
            item.suggestions.clear();
            item.count = 0;
        }
    }

    let first_len = item
        .first()
        .map(|first| words[first].chars().count() as i64);

    let should_add = verbosity == 2
        || item.suggestions.is_empty()
        || first_len
            .is_none_or(|first_len| (first_len - deleted_len) >= (suggestion_len - deleted_len));

    if should_add && !item.has(int) {
        item.suggestions.push(int);
    }
}

/// Upstream's private Damerau-Levenshtein (Lowrance-Wagner with an infinite
/// sentinel), transcribed 1:1 — see the module docs for why this is not a
/// textbook implementation.
pub fn damerau_levenshtein(source: &str, target: &str) -> i64 {
    let source: Vec<char> = source.chars().collect();
    let target: Vec<char> = target.chars().collect();
    let m = source.len();
    let n = target.len();
    let inf = (m + n) as i64;

    // `H[i][j]`, 0-indexed exactly as upstream's sparse `H[][]` (built via
    // `H[i+1] = []` on demand) — here a dense `(m + 2) x (n + 2)` grid.
    let mut h = vec![vec![0i64; n + 2]; m + 2];

    h[0][0] = inf;

    for i in 0..=m {
        h[i + 1][1] = i as i64;
        h[i + 1][0] = inf;
    }

    for j in 0..=n {
        h[1][j + 1] = j as i64;
        h[0][j + 1] = inf;
    }

    // `sd`: last row a character was seen in, defaulting to 0 for every
    // character appearing in `source + target`.
    let mut sd: HashMap<char, i64> = HashMap::new();
    for &letter in source.iter().chain(target.iter()) {
        sd.entry(letter).or_insert(0);
    }

    for i in 1..=m {
        let mut db: i64 = 0;

        for j in 1..=n {
            let i1 = *sd.get(&target[j - 1]).unwrap_or(&0);
            let j1 = db;

            if source[i - 1] == target[j - 1] {
                h[i + 1][j + 1] = h[i][j];
                db = j as i64;
            } else {
                h[i + 1][j + 1] = h[i][j].min(h[i + 1][j]).min(h[i][j + 1]) + 1;
            }

            h[i + 1][j + 1] = h[i + 1][j + 1]
                .min(h[i1 as usize][j1 as usize] + (i as i64 - i1 - 1) + 1 + (j as i64 - j1 - 1));
        }

        sd.insert(source[i - 1], i as i64);
    }

    h[m + 1][n + 1]
}

/// `lookup(dictionary, words, verbosity, maxDistance, maxLength, input)`.
#[allow(clippy::too_many_arguments)]
fn lookup(
    dictionary: &HashMap<String, Entry>,
    words: &[String],
    verbosity: u8,
    max_distance: f64,
    max_length: usize,
    input: &str,
) -> Vec<Suggestion> {
    let input_chars: Vec<char> = input.chars().collect();
    let length = input_chars.len();

    if (length as f64) - max_distance > max_length as f64 {
        return Vec::new();
    }

    let mut candidates: std::collections::VecDeque<String> =
        std::collections::VecDeque::from([input.to_owned()]);
    let mut candidate_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut suggestion_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut suggestions: Vec<Suggestion> = Vec::new();

    while let Some(candidate) = candidates.pop_front() {
        let candidate_len = candidate.chars().count() as i64;
        let len_diff = length as i64 - candidate_len;

        if verbosity < 2 {
            if let Some(first) = suggestions.first() {
                if len_diff > first.distance {
                    break;
                }
            }
        }

        if let Some(entry) = dictionary.get(&candidate) {
            let item_count = entry.count();

            if item_count > 0 && !suggestion_set.contains(&candidate) {
                suggestion_set.insert(candidate.clone());
                suggestions.push(Suggestion {
                    term: candidate.clone(),
                    distance: len_diff,
                    count: item_count,
                });

                if verbosity < 2 && len_diff == 0 {
                    break;
                }
            }

            {
                // `entry.suggestions()` reproduces upstream's local
                // `item = createDictionaryItem(item)` promotion for a
                // `Compact` entry (never persisted) -- see the method docs.
                let entry_suggestions = entry.suggestions();

                for &index in entry_suggestions.as_ref() {
                    let suggestion = &words[index];

                    if suggestion_set.contains(suggestion) {
                        continue;
                    }
                    suggestion_set.insert(suggestion.clone());

                    let suggestion_chars: Vec<char> = suggestion.chars().collect();
                    let suggestion_len = suggestion_chars.len() as i64;

                    let mut distance: i64 = 0;

                    if input != suggestion {
                        if suggestion_len == candidate_len {
                            distance = len_diff;
                        } else if length as i64 == candidate_len {
                            distance = suggestion_len - candidate_len;
                        } else {
                            let l = suggestion_chars.len();
                            let mut ii = 0usize;
                            let mut jj = 0usize;

                            while ii < l && ii < length && suggestion_chars[ii] == input_chars[ii] {
                                ii += 1;
                            }

                            while jj < l.saturating_sub(ii)
                                && jj < length
                                && suggestion_chars[l - jj - 1] == input_chars[length - jj - 1]
                            {
                                jj += 1;
                            }

                            if ii > 0 || jj > 0 {
                                let suggestion_slice: String =
                                    suggestion_chars[ii..l - jj].iter().collect();
                                let input_slice: String =
                                    input_chars[ii..length - jj].iter().collect();

                                distance = damerau_levenshtein(&suggestion_slice, &input_slice);
                            } else {
                                distance = damerau_levenshtein(suggestion, input);
                            }
                        }
                    }

                    if verbosity < 2 {
                        if let Some(first) = suggestions.first() {
                            if first.distance > distance {
                                suggestions.clear();
                            }
                        }
                    }

                    if verbosity < 2 {
                        if let Some(first) = suggestions.first() {
                            if distance > first.distance {
                                continue;
                            }
                        }
                    }

                    // Compared as `f64`, not cast first: a `NaN`
                    // `max_distance` (accepted by the constructor -- see its
                    // docs) must make this comparison `false` for every
                    // `distance`, matching JS's `distance <= NaN`. Casting
                    // `max_distance` to `i64` first would instead saturate
                    // `NaN` to `0` and wrongly admit `distance == 0`.
                    if (distance as f64) <= max_distance {
                        if let Some(target) = dictionary.get(suggestion) {
                            suggestions.push(Suggestion {
                                term: suggestion.clone(),
                                distance,
                                count: target.count(),
                            });
                        }
                    }
                }
            }
        }

        if (len_diff as f64) < max_distance {
            if verbosity < 2 {
                if let Some(first) = suggestions.first() {
                    if len_diff >= first.distance {
                        continue;
                    }
                }
            }

            let candidate_chars: Vec<char> = candidate.chars().collect();

            for i in 0..candidate_chars.len() {
                let mut deleted: String = candidate_chars[..i].iter().collect();
                deleted.extend(candidate_chars[i + 1..].iter());

                if !candidate_set.contains(&deleted) {
                    candidate_set.insert(deleted.clone());
                    candidates.push_back(deleted);
                }
            }
        }
    }

    if verbosity == 0 {
        suggestions.truncate(1);
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &[&str] = &[
        "Hello", "Mello", "John", "Book", "Back", "World", "Hello", "Jello", "Hell", "Trello",
    ];

    fn indexed(index: &mut SymSpell, words: &[&str]) {
        for word in words {
            index.add(word);
        }
    }

    #[test]
    fn constructor_rejects_invalid_options() {
        assert_eq!(
            SymSpell::new(-23.0, 2).unwrap_err(),
            Error::InvalidMaxDistance
        );
        assert_eq!(SymSpell::new(2.0, 45).unwrap_err(), Error::InvalidVerbosity);
    }

    /// 1:1 transcription of "should correctly index & perform basic search
    /// queries."
    #[test]
    fn reproduces_the_upstream_basic_search() {
        let mut index = SymSpell::new(2.0, 2).unwrap();
        indexed(&mut index, DATA);

        assert_eq!(index.size(), 10);
        assert_eq!(index.search("shawarma"), Vec::new());

        assert_eq!(
            index.search("ello"),
            vec![
                Suggestion {
                    term: "Hello".into(),
                    distance: 1,
                    count: 2
                },
                Suggestion {
                    term: "Mello".into(),
                    distance: 1,
                    count: 1
                },
                Suggestion {
                    term: "Jello".into(),
                    distance: 1,
                    count: 1
                },
                Suggestion {
                    term: "Trello".into(),
                    distance: 2,
                    count: 1
                },
                Suggestion {
                    term: "Hell".into(),
                    distance: 2,
                    count: 1
                },
            ]
        );
    }

    /// "should be possible to increase the maximum edit distance."
    #[test]
    fn a_wider_max_distance_finds_more_suggestions() {
        let mut index = SymSpell::new(4.0, 2).unwrap();
        indexed(&mut index, DATA);

        let terms: Vec<String> = index.search("ello").into_iter().map(|s| s.term).collect();

        assert_eq!(
            terms,
            vec!["Hello", "Mello", "Jello", "Trello", "Hell", "John", "Book", "World"]
        );
    }

    /// "should possible to use different verbosity settings."
    #[test]
    fn verbosity_changes_how_many_suggestions_come_back() {
        let mut lazy = SymSpell::new(2.0, 0).unwrap();
        let mut less_lazy = SymSpell::new(2.0, 1).unwrap();

        indexed(&mut lazy, DATA);
        indexed(&mut less_lazy, DATA);

        assert_eq!(
            lazy.search("ello"),
            vec![Suggestion {
                term: "Hello".into(),
                distance: 1,
                count: 2
            }]
        );

        let terms: Vec<String> = less_lazy
            .search("ello")
            .into_iter()
            .map(|s| s.term)
            .collect();
        assert_eq!(terms, vec!["Hello", "Mello", "Jello"]);
    }

    #[test]
    fn clear_resets_the_index() {
        let mut index = SymSpell::new(2.0, 2).unwrap();
        indexed(&mut index, DATA);
        index.clear();

        assert_eq!(index.size(), 0);
        assert_eq!(index.search("ello"), Vec::new());
    }

    /// A word that is both a real entry and another word's delete-form at
    /// once — `'Hell'` is both added directly and reached as `'Hello'`'s
    /// length-1 delete. Exercises the `Entry::Compact` -> `Entry::Full`
    /// promotion from both directions.
    #[test]
    fn a_word_can_be_both_a_real_entry_and_another_words_delete_form() {
        let mut index = SymSpell::new(2.0, 2).unwrap();
        index.add("Hello");
        index.add("Hell");

        let terms: Vec<String> = index.search("Hell").into_iter().map(|s| s.term).collect();

        assert!(terms.iter().any(|t| t == "Hell"));
        assert!(terms.iter().any(|t| t == "Hello"));
    }

    /// The differential fuzzer's own first divergence, minimised: a
    /// `Compact` dictionary entry (`"jello"`'s length-1 delete `"ello"` was
    /// never reached by a second word) must still contribute its one
    /// suggestion during `lookup`, exactly as upstream's local
    /// `createDictionaryItem(item)` promotion does. Verified against real
    /// upstream `symspell.js` on Node 24.18.1: `search("hello")` after
    /// `add("jello")` at `{maxDistance: 1, verbosity: 0}` returns
    /// `[{"term":"jello","distance":1,"count":1}]`.
    #[test]
    fn a_compact_dictionary_entry_still_contributes_its_suggestion() {
        let mut index = SymSpell::new(1.0, 0).unwrap();
        index.add("jello");

        assert_eq!(
            index.search("hello"),
            vec![Suggestion {
                term: "jello".to_owned(),
                distance: 1,
                count: 1,
            }]
        );
    }

    #[test]
    fn damerau_levenshtein_matches_known_distances() {
        assert_eq!(damerau_levenshtein("hello", "hello"), 0);
        assert_eq!(damerau_levenshtein("hello", "hallo"), 1);
        assert_eq!(damerau_levenshtein("", "abc"), 3);
        assert_eq!(damerau_levenshtein("ab", "ba"), 1);
    }
}
