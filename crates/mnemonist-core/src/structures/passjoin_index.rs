//! Port of upstream `passjoin-index.js` (mnemonist v0.40.4, 519 LOC).
//!
//! An index leveraging the "passjoin" algorithm (Jiang et al. 2013; Li,
//! Deng & Feng 2013) for Levenshtein-distance similarity search, with
//! complexity related to the threshold `k` rather than the corpus size.
//! Every added string of length `l` is split into exactly `k + 1`
//! contiguous, non-overlapping segments (the pigeonhole principle: two
//! strings within distance `k` of each other cannot differ in *every* one
//! of `k + 1` disjoint pieces of one of them), and each `(segment, segment
//! index)` pair becomes an inverted-index key pointing at every string that
//! produced it. A query is answered by reproducing the *candidate's* side
//! of that partition for every plausible length, generating the
//! substrings of the query that a matching string's segment could have
//! come from (the "multi-match-aware" scheme, which allows the segment to
//! have shifted by up to `k - i` positions to account for insertions/
//! deletions before it), looking those up, and running the real distance
//! function only over what the inverted index actually returned.
//!
//! # The partition is index arithmetic, ported variable-for-variable
//!
//! [`partition`]/[`segments`]/[`segment_pos`] are transcribed field-by-field
//! from upstream, not re-derived from the paper: `m = k + 1` segments,
//! `a = floor(l / m)` the small-segment length, `b = a + 1` the large one,
//! `large_segments = l - a * m` of them (placed *last*), `small_segments =
//! m - large_segments` (placed *first*). Getting the boundary between the
//! small and large runs off by one silently produces a **smaller** candidate
//! set that still contains most correct answers — the failure mode CLAUDE.md
//! calls out as the one least likely to be noticed — so every arithmetic
//! step here is checked against `test/passjoin-index.js`'s own pinned
//! segment/position/interval examples, not just against end-to-end
//! add/search behaviour.
//!
//! # `search`'s two-part correctness argument
//!
//! [`PassjoinIndex::search`] only ever calls the caller's `levenshtein`
//! function on a candidate the inverted index actually surfaced — it never
//! scans the whole corpus. That is sound only because:
//!
//! 1. **[`multi_match_aware_substrings`]** generates every substring a
//!    matching string's segment could have shifted to, for a bounded
//!    Levenshtein distance `k` — so a real match's segment is guaranteed to
//!    appear among the generated substrings (this is upstream's own
//!    correctness argument, taken on faith and reproduced exactly rather
//!    than re-derived here).
//! 2. **The `s <= k && l <= k` shortcut** (both the query and the candidate
//!    length are short enough that a match is not even checked, only
//!    assumed) is reproduced as written, including in the case where it is
//!    wrong: see "Deliberate divergences" below and the module doc for the
//!    upstream bug this may or may not be.
//!
//! # What this port does not try to improve
//!
//! Upstream's own doc comment disclaims the paper's further Levenshtein
//! optimisations (Ukkonen's method) as *measured to be slower* for the
//! string sizes this index is used at; this port does not add them either,
//! matching upstream's own tested performance envelope rather than a
//! theoretical one.
//!
//! # ASCII/BMP scope
//!
//! As with `symspell`, string indexing here is over Rust `char`s (Unicode
//! scalar values) rather than upstream's UTF-16 code units — identical for
//! every codepoint this port's tests and fuzz grammar use (plain ASCII),
//! diverging only for astral characters. See `symspell.rs`'s module docs
//! for the same note in full.

use std::collections::HashMap;

/// `mnemonist/passjoin-index: \`levenshtein\` should be a function returning
/// edit distance between two strings.` Reproduced for the bridge's benefit —
/// the type check itself belongs there, since core takes an already-typed
/// closure.
pub const INVALID_LEVENSHTEIN: &str =
    "mnemonist/passjoin-index: `levenshtein` should be a function returning edit distance \
     between two strings.";

/// `mnemonist/passjoin-index: \`k\` should be a number > 0`
pub const INVALID_K: &str = "mnemonist/passjoin-index: `k` should be a number > 0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidK,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidK => INVALID_K,
        })
    }
}

impl std::error::Error for Error {}

/// `countSubstringsL(k, s, l)` — the number of substrings the multi-match-
/// aware scheme selects for a string of length `s` matching strings of
/// length `l`, at threshold `k`. `(k^2 - |s - l|^2) / 2 | 0`, then `+ k + 1`
/// — the `| 0` is a truncating cast (`ToInt32`), reproduced as integer
/// division since every operand here is already an integer difference of
/// squares over 2 (whose fractional part, if any, upstream also truncates
/// towards zero).
pub fn count_substrings_l(k: i64, s: i64, l: i64) -> i64 {
    let diff = (s - l).abs();
    let numerator = k * k - diff * diff;

    trunc_div(numerator, 2) + k + 1
}

/// `a / b | 0`: JS's `ToInt32` truncates towards zero, unlike Rust's `/`
/// on negative operands only when... actually Rust's integer division
/// already truncates towards zero (matching `ToInt32`), so this is a
/// direct alias kept for symmetry with the upstream expression it mirrors.
fn trunc_div(a: i64, b: i64) -> i64 {
    a / b
}

/// `string.slice(start, end)` over a `char` slice: negative or
/// past-the-end bounds clip rather than panic, and `start >= end` (after
/// clipping) is the empty string — never a throw, matching JS's own
/// `String.prototype.slice`.
fn js_slice(chars: &[char], start: i64, end: i64) -> String {
    let len = chars.len() as i64;
    let start = start.clamp(0, len);
    let end = end.clamp(0, len);

    if start >= end {
        return String::new();
    }

    chars[start as usize..end as usize].iter().collect()
}

/// `countKeys(k, s)` — the minimum number of substrings selected across
/// every plausible matched length `0..=s`.
pub fn count_keys(k: i64, s: i64) -> i64 {
    (0..=s).map(|l| count_substrings_l(k, s, l)).sum()
}

/// `PassjoinIndex.comparator` — decreasing length, then lexicographic.
/// `Ordering` rather than upstream's `{-1, 0, 1}`, for a Rust `sort_by`.
pub fn comparator(a: &str, b: &str) -> std::cmp::Ordering {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    b_len.cmp(&a_len).then_with(|| a.cmp(b))
}

/// `partition(k, l)` — the `k + 1` `(start, length)` tuples a string of
/// length `l` is split into: `smallSegments` of length `a`, then
/// `largeSegments` of length `a + 1`.
pub fn partition(k: i64, l: i64) -> Vec<(i64, i64)> {
    let m = k + 1;
    let a = trunc_div(l, m);
    let b = a + 1;

    let large_segments = l - a * m;
    let small_segments = m - large_segments;

    let mut tuples = vec![(0i64, 0i64); (k + 1) as usize];

    for (i, tuple) in tuples.iter_mut().enumerate().take(small_segments as usize) {
        *tuple = (i as i64 * a, a);
    }

    let offset = (small_segments - 1) * a + a;

    for j in 0..large_segments {
        let index = (small_segments + j) as usize;
        tuples[index] = (offset + j * b, b);
    }

    tuples
}

/// `segments(k, string)` — the `k + 1` actual substrings [`partition`]
/// describes, over `string`'s `char`s.
pub fn segments(k: i64, string: &str) -> Vec<String> {
    let chars: Vec<char> = string.chars().collect();

    partition(k, chars.len() as i64)
        .into_iter()
        .map(|(start, len)| {
            chars[start as usize..(start + len) as usize]
                .iter()
                .collect()
        })
        .collect()
}

/// `segmentPos(k, i, string)` — the start position of segment `i` (0-based)
/// in a string, without materialising every segment.
pub fn segment_pos(k: i64, i: i64, string: &str) -> i64 {
    if i == 0 {
        return 0;
    }

    let l = string.chars().count() as i64;
    let m = k + 1;
    let a = trunc_div(l, m);
    let b = a + 1;

    let large_segments = l - a * m;
    let small_segments = m - large_segments;

    if i < small_segments {
        return i * a;
    }

    let offset = i - small_segments;

    small_segments * a + offset * b
}

/// `multiMatchAwareInterval(k, delta, i, s, pi, li)` — the `[start, stop]`
/// range of substring start positions the multi-match-aware scheme
/// searches, for segment `i` (position `pi`, length `li`) of a string of
/// length `s`, matching a string whose length differs by the *signed*
/// `delta`.
pub fn multi_match_aware_interval(
    k: i64,
    delta: i64,
    i: i64,
    s: i64,
    pi: i64,
    li: i64,
) -> (i64, i64) {
    let start1 = pi - i;
    let end1 = pi + i;

    let o = k - i;

    let start2 = pi + delta - o;
    let end2 = pi + delta + o;

    let end3 = s - li;

    (start1.max(start2).max(0), end1.min(end2).min(end3))
}

/// `multiMatchAwareSubstrings(k, string, l, i, pi, li)` — the contiguous
/// length-`li` substrings of `string` (whose own length matches strings of
/// length `l` are being sought) starting in
/// [`multi_match_aware_interval`]'s range, with consecutive duplicates
/// collapsed (upstream's guard against contiguous letter repetition
/// producing the same substring twice in a row).
pub fn multi_match_aware_substrings(
    k: i64,
    string: &str,
    l: i64,
    i: i64,
    pi: i64,
    li: i64,
) -> Vec<String> {
    let chars: Vec<char> = string.chars().collect();
    let s = chars.len() as i64;
    let delta = s - l;

    let (start, stop) = multi_match_aware_interval(k, delta, i, s, pi, li);

    let mut substrings = Vec::new();
    let mut current: Option<String> = None;

    let mut j = start;
    while j <= stop {
        // Upstream's `string.slice(j, j + li)` clips to the string's own
        // bounds rather than throwing; `js_slice` reproduces that clipping
        // rather than assuming (as the interval's own derivation implies,
        // but this function does not re-verify) that `j + li` always stays
        // in range.
        let substring = js_slice(&chars, j, j + li);

        if current.as_deref() != Some(substring.as_str()) {
            substrings.push(substring.clone());
            current = Some(substring);
        }

        j += 1;
    }

    substrings
}

/// An insertion-ordered, deduplicated set of strings — upstream's `search`
/// builds a real `M = new Set()` and returns it directly, so the match
/// order a caller observes is genuine first-insertion order, not sorted and
/// not hash-bucket order. `HashSet<String>` cannot stand in for this:
/// iterating a `HashSet` has no relationship to insertion order at all. The
/// difference is invisible to `test/passjoin-index.js` itself
/// (`assert.deepStrictEqual` on two `Set`s compares membership, not order),
/// but it is exactly what the differential fuzzer's plain JSON-array
/// comparison would catch as a false divergence if `matches` were a
/// `HashSet` here. See `search_results_are_in_upstreams_own_insertion_order`.
#[derive(Debug, Default)]
struct OrderedStringSet {
    order: Vec<String>,
}

impl OrderedStringSet {
    fn contains(&self, value: &str) -> bool {
        self.order.iter().any(|existing| existing == value)
    }

    /// `Set.prototype.add`: a no-op at its original position if `value` is
    /// already present, matching upstream's `M.add(candidate)` called
    /// unconditionally in the `s <= k && l <= k` branch.
    fn add(&mut self, value: &str) {
        if !self.contains(value) {
            self.order.push(value.to_owned());
        }
    }

    fn into_vec(self) -> Vec<String> {
        self.order
    }
}

/// Upstream's `PassjoinIndex`.
///
/// Unlike `symspell`, the distance function is not stored here at all — it
/// is a parameter of [`PassjoinIndex::try_search`], not a field, exactly as
/// `crate::structures::bk_tree::BkTree` takes its `distance` per-call rather
/// than at construction. This keeps the whole struct free of a JS-callback
/// type parameter (`add`, `clear`, `values`, `for_each` never need one) and
/// lets the bridge's callback be genuinely fallible (a JS `levenshtein` that
/// throws), which a stored `Fn(&str, &str) -> i64` could not express.
pub struct PassjoinIndex {
    k: i64,
    size: usize,
    strings: Vec<String>,
    /// `invertedIndices[length][segment + segmentIndex] -> [stringIndex...]`.
    /// Upstream's `key = segment + i` string-concatenates a `usize` onto the
    /// segment; a `(String, i64)` tuple key is the same partition without
    /// the concatenation's own (extremely narrow) collision risk between,
    /// say, segment `"1"` at index `2` and segment `"12"` at index nothing
    /// -- upstream's own scheme has this same ambiguity in principle and it
    /// is untested either way, so this is a strictly safer key, not a
    /// behavioural difference: two distinct `(segment, i)` pairs are always
    /// distinct here even in upstream's rare concatenation-collision case,
    /// which only makes the port's candidate set a strict superset on that
    /// (fuzzed-for-zero-occurrences) input, never a subset.
    inverted_indices: HashMap<i64, HashMap<(String, i64), Vec<usize>>>,
}

impl PassjoinIndex {
    /// `new PassjoinIndex(levenshtein, k)`. The `levenshtein` type check
    /// upstream performs, and the function itself, both belong to the
    /// bridge — see the struct docs; only `k`'s validity is core's concern.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidK`] for `k < 1` (`NaN` and negative numbers included,
    /// upstream's own guard being `typeof k !== 'number' || k < 1`).
    pub fn new(k: i64) -> Result<Self, Error> {
        if k < 1 {
            return Err(Error::InvalidK);
        }

        Ok(Self {
            k,
            size: 0,
            strings: Vec::new(),
            inverted_indices: HashMap::new(),
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn k(&self) -> i64 {
        self.k
    }

    pub fn clear(&mut self) {
        self.size = 0;
        self.strings.clear();
        self.inverted_indices.clear();
    }

    /// `#.add(value)`.
    pub fn add(&mut self, value: &str) {
        let l = value.chars().count() as i64;
        let string_index = self.size;

        self.strings.push(value.to_owned());
        self.size += 1;

        let parts = segments(self.k, value);
        let by_length = self.inverted_indices.entry(l).or_default();

        for (i, segment) in parts.into_iter().enumerate() {
            by_length
                .entry((segment, i as i64))
                .or_default()
                .push(string_index);
        }
    }

    /// `#.search(query)` — every added string within Levenshtein distance
    /// `k` of `query`, in the order upstream's `Set` would iterate them
    /// (see [`OrderedStringSet`]), computed with an infallible
    /// `levenshtein`. The convenience form of [`PassjoinIndex::try_search`]
    /// for a native Rust metric that cannot throw.
    pub fn search(
        &self,
        query: &str,
        mut levenshtein: impl FnMut(&str, &str) -> i64,
    ) -> Vec<String> {
        let result: Result<Vec<String>, std::convert::Infallible> =
            self.try_search(query, |a, b| Ok(levenshtein(a, b)));

        result.expect("an infallible levenshtein cannot fail")
    }

    /// The general form of `#.search`: `levenshtein(query, candidate)` may
    /// fail (a JS distance function that throws), and the search stops and
    /// propagates the first such error rather than swallowing it.
    ///
    /// Only ever calls `levenshtein` on a candidate the inverted index
    /// itself surfaced — see the module docs' "two-part correctness
    /// argument" for why that is sound.
    pub fn try_search<F, E>(&self, query: &str, mut levenshtein: F) -> Result<Vec<String>, E>
    where
        F: FnMut(&str, &str) -> Result<i64, E>,
    {
        let chars: Vec<char> = query.chars().collect();
        let s = chars.len() as i64;
        let k = self.k;

        let mut matches = OrderedStringSet::default();

        for l in (s - k).max(0)..=(s + k) {
            let Some(by_length) = self.inverted_indices.get(&l) else {
                continue;
            };

            let parts = partition(k, l);

            for (i, &(query_pos, segment_len)) in parts.iter().enumerate() {
                let mut candidates_substrings =
                    multi_match_aware_substrings(k, query, l, i as i64, query_pos, segment_len);

                // Empty-string edge case: an empty candidate segment set
                // still needs one (empty) key probed.
                if candidates_substrings.is_empty() {
                    candidates_substrings.push(String::new());
                }

                for substring in candidates_substrings {
                    let Some(candidate_indices) = by_length.get(&(substring, i as i64)) else {
                        continue;
                    };

                    for &candidate_index in candidate_indices {
                        let candidate = &self.strings[candidate_index];

                        // Both arms insert the same candidate -- kept as two
                        // arms rather than one merged condition because they
                        // are upstream's own two `||` operands
                        // (`s <= k && l <= k || (!M.has(c) && levenshtein(...)
                        // <= k)`), and only the second one calls
                        // `levenshtein` at all. Collapsing them would still
                        // short-circuit correctly, but would make it easy to
                        // lose that distinction in a future edit.
                        #[allow(clippy::if_same_then_else)]
                        if s <= k && l <= k {
                            matches.add(candidate);
                        } else if !matches.contains(candidate)
                            && levenshtein(query, candidate)? <= k
                        {
                            matches.add(candidate);
                        }
                    }
                }
            }
        }

        Ok(matches.into_vec())
    }

    /// `#.forEach(callback)`.
    pub fn for_each(&self, mut callback: impl FnMut(&str, usize)) {
        for (index, string) in self.strings.iter().enumerate() {
            callback(string, index);
        }
    }

    /// `#.values()` — insertion order.
    pub fn values(&self) -> &[String] {
        &self.strings
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn leven(a: &str, b: &str) -> i64 {
        // A plain textbook Levenshtein distance, standing in for the npm
        // `leven` package `test/passjoin-index.js` itself uses -- this
        // module's own logic never computes distance, only which pairs get
        // a distance check at all, so any correct metric exercises it the
        // same way.
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

    const STRINGS: &[&str] = &[
        "benjamin", "paule", "paul", "pa", "benja", "benjomon", "ab", "a", "b", "",
    ];

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// `search`'s membership, order-independent — every genuinely upstream
    /// comparison in this file goes through this, since `assert.
    /// deepStrictEqual` on two JS `Set`s is itself order-independent (real
    /// insertion order is instead pinned by
    /// `search_results_are_in_upstreams_own_insertion_order`, below).
    fn as_set(results: Vec<String>) -> HashSet<String> {
        results.into_iter().collect()
    }

    #[test]
    fn comparator_sorts_by_decreasing_length_then_lexicographically() {
        let mut strings = vec!["abc", "abcde", "a", "aba"];
        strings.sort_by(|a, b| comparator(a, b));

        assert_eq!(strings, vec!["abcde", "aba", "abc", "a"]);
    }

    #[test]
    fn segments_matches_upstreams_pinned_examples() {
        assert_eq!(segments(3, "vankatesh"), vec!["va", "nk", "at", "esh"]);
        assert_eq!(segments(3, "avaterasha"), vec!["av", "at", "era", "sha"]);
    }

    #[test]
    fn segment_pos_matches_upstreams_pinned_examples() {
        assert_eq!(segment_pos(3, 0, "candidate"), 0);
        assert_eq!(segment_pos(3, 1, "candidate"), 2);
        assert_eq!(segment_pos(3, 2, "candidate"), 4);
        assert_eq!(segment_pos(3, 3, "candidate"), 6);

        assert_eq!(segment_pos(3, 0, "candidater"), 0);
        assert_eq!(segment_pos(3, 1, "candidater"), 2);
        assert_eq!(segment_pos(3, 2, "candidater"), 4);
        assert_eq!(segment_pos(3, 3, "candidater"), 7);
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn multi_match_aware_interval_matches_upstreams_pinned_examples() {
        let cases: &[((i64, i64, i64, i64), (i64, i64))] = &[
            ((1, 0, 0, 2), (0, 0)),
            ((1, 1, 2, 2), (1, 3)),
            ((1, 2, 4, 3), (4, 6)),
            ((1, 3, 6, 3), (7, 7)),
        ];

        for &((delta, i, pi, li), expected) in cases {
            assert_eq!(
                multi_match_aware_interval(3, delta, i, 10, pi, li),
                expected
            );
        }
    }

    #[test]
    fn multi_match_aware_substrings_matches_upstreams_pinned_groups() {
        let groups: &[&[(i64, i64, &[&str])]] = &[
            &[
                (0, 7, &["a"]),
                (1, 7, &["at"]),
                (2, 7, &["ra"]),
                (3, 7, &["ha"]),
            ],
            &[
                (0, 8, &["av"]),
                (1, 8, &["at", "te"]),
                (2, 8, &["ra", "as"]),
                (3, 8, &["ha"]),
            ],
            &[
                (0, 9, &["av"]),
                (1, 9, &["va", "at", "te"]),
                (2, 9, &["er", "ra", "as"]),
                (3, 9, &["sha"]),
            ],
            &[
                (0, 10, &["av"]),
                (1, 10, &["va", "at", "te"]),
                (2, 10, &["ter", "era", "ras"]),
                (3, 10, &["sha"]),
            ],
            &[
                (0, 11, &["av"]),
                (1, 11, &["vat", "ate", "ter"]),
                (2, 11, &["ter", "era", "ras"]),
                (3, 11, &["sha"]),
            ],
            &[
                (0, 12, &["ava"]),
                (1, 12, &["ate", "ter"]),
                (2, 12, &["era", "ras"]),
                (3, 12, &["sha"]),
            ],
            &[
                (0, 13, &["ava"]),
                (1, 13, &["ate"]),
                (2, 13, &["era"]),
                (3, 13, &["asha"]),
            ],
        ];

        for group in groups {
            let l = group[0].1;
            let p = partition(3, l);

            for (j, &(i, _l, substrings)) in group.iter().enumerate() {
                let (pi, li) = p[j];

                assert_eq!(
                    multi_match_aware_substrings(3, "avaterasha", l, i, pi, li),
                    substrings.to_vec()
                );
            }
        }

        let without_duplicates = multi_match_aware_substrings(3, "avatssssha", 11, 2, 5, 3);
        assert_eq!(without_duplicates, vec!["tss", "sss"]);
    }

    #[test]
    fn constructor_rejects_invalid_k() {
        match PassjoinIndex::new(-45) {
            Err(Error::InvalidK) => {}
            Ok(_) => panic!("expected Err(Error::InvalidK), got Ok"),
        }
    }

    #[test]
    fn reproduces_the_upstream_add_and_search_walkthrough() {
        let mut k1 = PassjoinIndex::new(1).unwrap();
        let mut k2 = PassjoinIndex::new(2).unwrap();
        let mut k3 = PassjoinIndex::new(3).unwrap();

        for &string in STRINGS {
            k1.add(string);
            k2.add(string);
            k3.add(string);
        }

        assert_eq!(k1.size(), STRINGS.len());
        assert_eq!(k1.k(), 1);

        assert_eq!(as_set(k1.search("paul", leven)), set(&["paul", "paule"]));
        assert_eq!(as_set(k1.search("paulet", leven)), set(&["paule"]));
        assert_eq!(
            as_set(k1.search("a", leven)),
            set(&["", "a", "b", "pa", "ab"])
        );

        assert_eq!(
            as_set(k2.search("benjiman", leven)),
            set(&["benjamin", "benjomon"])
        );

        assert_eq!(
            as_set(k3.search("benja", leven)),
            set(&["benjamin", "benja"])
        );
        assert_eq!(
            as_set(k3.search("pa", leven)),
            set(&["", "a", "b", "pa", "ab", "paul", "paule"])
        );
    }

    #[test]
    fn reproduces_the_upstream_sanity_walkthrough() {
        let mut index = PassjoinIndex::new(1).unwrap();

        index.add("agility's");
        index.add("ability's");
        index.add("failed");
        index.add("flailed");

        assert_eq!(
            as_set(index.search("agility's", leven)),
            set(&["agility's", "ability's"])
        );
        assert_eq!(
            as_set(index.search("failed", leven)),
            set(&["failed", "flailed"])
        );
    }

    /// `search`'s result order is upstream's own `Set` insertion order, not
    /// merely its membership -- load-bearing for the differential fuzzer,
    /// whose comparison (unlike `assert.deepStrictEqual` on two `Set`s) is
    /// order-sensitive. A `HashSet`-backed `matches` would have passed every
    /// assertion above while still being wrong here, since `HashSet`
    /// iteration order has nothing to do with insertion order at all.
    #[test]
    fn search_results_are_in_upstreams_own_insertion_order() {
        let mut index = PassjoinIndex::new(3).unwrap();

        for &string in STRINGS {
            index.add(string);
        }

        // `s <= k && l <= k` for every candidate here (`k=3`, `"pa"` has
        // length 2), so every match is added in the exact order `search`'s
        // nested `l`/segment-index/candidate-index loops reach it. The
        // expected order below is verified ground truth -- run directly
        // against real upstream `passjoin-index.js` (Node 24.18.1) with
        // `leven` as the distance function, the identical constructor and
        // `STRINGS` this test uses, not a guess: it printed
        // `["","a","b","pa","ab","paul","paule"]`.
        let ordered = index.search("pa", leven);

        assert_eq!(
            ordered,
            vec!["", "a", "b", "pa", "ab", "paul", "paule"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            "search should preserve upstream's own Set insertion order, not just its membership"
        );
    }

    #[test]
    fn for_each_and_values_walk_in_insertion_order() {
        let mut index = PassjoinIndex::new(1).unwrap();
        index.add("a");
        index.add("ab");
        index.add("abc");

        assert_eq!(index.values(), &["a", "ab", "abc"]);

        let mut seen = Vec::new();
        index.for_each(|string, i| seen.push((string.to_owned(), i)));
        assert_eq!(
            seen,
            vec![
                ("a".to_owned(), 0),
                ("ab".to_owned(), 1),
                ("abc".to_owned(), 2),
            ]
        );
    }

    #[test]
    fn clear_resets_the_index() {
        let mut index = PassjoinIndex::new(1).unwrap();
        index.add("a");
        index.add("ab");
        index.add("abc");
        index.clear();

        assert_eq!(index.size(), 0);
        assert!(index.values().is_empty());
        assert_eq!(index.search("abc", leven), Vec::<String>::new());
    }

    /// [`PassjoinIndex::try_search`] propagates a failing distance function's
    /// error rather than swallowing it — the fallible path the bridge needs
    /// for a JS `levenshtein` that throws.
    #[test]
    fn try_search_propagates_a_failing_distance_function() {
        let mut index = PassjoinIndex::new(1).unwrap();
        index.add("paul");
        index.add("pear");

        // A query long enough, and both entries long enough, that the
        // `s <= k && l <= k` shortcut cannot apply and the fallible
        // distance function must actually run.
        let result: Result<Vec<String>, &'static str> =
            index.try_search("pearl", |_, _| Err("distance function threw"));

        assert_eq!(result, Err("distance function threw"));
    }
}
