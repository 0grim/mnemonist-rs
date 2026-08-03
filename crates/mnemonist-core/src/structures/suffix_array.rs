//! Port of upstream `suffix-array.js` (mnemonist v0.40.4).
//!
//! `SuffixArray` and `GeneralizedSuffixArray`, both built by the same
//! Kärkkäinen–Sanders (DC3) recursion. Zero dependencies upstream, zero here.
//!
//! # This port reproduces two upstream defects on purpose
//!
//! Both were found by comparing upstream against a naive `O(n² log n)` suffix
//! sort on Node 24.18.1, and both are reachable from the documented API with
//! ordinary input. `docs/modules/suffix-array.md` has the full write-up; the
//! short version is what a reader of this file needs to know before touching
//! anything:
//!
//! **BUG-SUFFIX-ARRAY-1 — the radix sort silently narrows to 8 bits.** `sort` scans for the
//! largest symbol with `Math.max` in order to pick a radix width. Its scan
//! reads `string[array[i] + offset]`, and for `offset` of 1 and 2 that index
//! routinely runs past the padded sequence — the padding is `length % 3`
//! elements, which is not enough. In JavaScript the read is `undefined`,
//! `Math.max(undefined, j)` is `NaN`, and `NaN >> 24 && 32 || …` falls all the
//! way through to `8`. So the sort compares only the **low byte** of each
//! symbol. Any alphabet in which two symbols share a low byte — including any
//! character at or above U+0100, whose low byte can collide with the `0`
//! padding — is then mis-sorted.
//!
//! **BUG-SUFFIX-ARRAY-2 — the reduced string has no separator when `l % 3 == 1`.** DC3
//! concatenates the ranks of the ≡1 positions with the ranks of the ≡2
//! positions and recurses on the result. That is only sound if the first group
//! ends in a rank nothing else can equal. Upstream sizes the groups from
//! `al = (2 * l / 3) | 0`, which for `l % 3 == 1` omits the position that would
//! have carried the sentinel, so the two halves run into each other. Whenever
//! the recursion actually fires — that is, whenever some triple repeats — the
//! answer can be wrong. `new SuffixArray('aaaaaaa').array` is
//! `[6, 5, 3, 0, 2, 4, 1]`; the correct answer is `[6, 5, 4, 3, 2, 1, 0]`.
//!
//! Neither is this port's bug to fix. Its porting rule is explicit that a
//! divergence in which the port is *more* correct is a defect in the port.
//!
//! # Everything is read through `Sparse`, and that is load-bearing
//!
//! The two defects above are both *consequences of reading past the end of an
//! array*, and one of them (BUG-SUFFIX-ARRAY-1) depends on the difference between "read a
//! zero" and "read `undefined`" — the first would still sort correctly, the
//! second poisons `Math.max`. A port that indexed with `[]` would panic where
//! upstream computes an answer, and a port that clamped to `0` would compute a
//! *different, more correct* answer. So every sequence in this module is a
//! `Sparse`, whose `get` returns `None` for both a hole and an out-of-range
//! index, exactly as JavaScript's `undefined` does.
//!
//! `compare` returns `f64` for the same reason: upstream's `||` chain treats
//! `NaN` as falsy and its caller tests `< 0`, which `NaN` fails. Modelling that
//! with an `Ordering` would quietly delete a branch.

use std::collections::BTreeMap;

/// `''`, the token upstream splices between the members of a
/// generalized suffix array.
pub const SEPARATOR: u16 = 1;

/// What a suffix array was built over.
///
/// Upstream keeps a single `string` property and a `hasArbitrarySequence`
/// boolean derived from `typeof string !== 'string'`. The two cases behave
/// differently — different alphabets, different `slice`, different `!==` — so
/// they are one enum here rather than a boolean and a `dyn Any`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sequence {
    /// A JavaScript string, as UTF-16 code units. The alphabet is the code
    /// unit itself, via `charCodeAt`.
    Text(Vec<u16>),
    /// An arbitrary sequence. Upstream keys its alphabet by each token's
    /// *string* form (they become property names) and orders it by sorting
    /// those strings, so tokens are held as `String` here.
    Tokens(Vec<String>),
}

impl Sequence {
    /// `sequence.length`.
    pub fn len(&self) -> usize {
        match self {
            Self::Text(units) => units.len(),
            Self::Tokens(tokens) => tokens.len(),
        }
    }

    /// `sequence.length === 0`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `typeof sequence !== 'string'`.
    pub fn is_arbitrary(&self) -> bool {
        matches!(self, Self::Tokens(_))
    }

    /// `sequence.slice(start, end)`, clamped the way JavaScript's is.
    pub fn slice(&self, start: usize, end: usize) -> Self {
        let end = end.min(self.len());
        let start = start.min(end);

        match self {
            Self::Text(units) => Self::Text(units[start..end].to_vec()),
            Self::Tokens(tokens) => Self::Tokens(tokens[start..end].to_vec()),
        }
    }

    /// Whether positions `left` and `right` hold the same element, using
    /// upstream's `!==`.
    ///
    /// For [`Self::Text`] that is code-unit equality, which is what comparing
    /// two one-character strings does. For [`Self::Tokens`] upstream compares
    /// the *values*, so two distinct objects with the same `toString` would be
    /// unequal here while sharing an alphabet symbol; representing tokens as
    /// `String` collapses that case. See the module doc's divergence list.
    fn same(&self, left: usize, right: usize) -> bool {
        match self {
            Self::Text(units) => units.get(left) == units.get(right),
            Self::Tokens(tokens) => tokens.get(left) == tokens.get(right),
        }
    }
}

/// A JavaScript array of numbers: indices may be missing, and a missing index
/// reads as `undefined` rather than panicking.
///
/// Holes are real here, not defensive: `lookup` is populated only at positions
/// ≢ 0 (mod 3), and the reads that would hit a hole are the ones upstream's
/// `||` chain relies on turning into `NaN`.
#[derive(Debug, Clone, Default)]
struct Sparse {
    slots: Vec<Option<i64>>,
}

impl Sparse {
    fn from_dense(values: Vec<i64>) -> Self {
        Self {
            slots: values.into_iter().map(Some).collect(),
        }
    }

    /// `array[index]`, where an absent or out-of-range index is `undefined`.
    fn get(&self, index: i64) -> Option<i64> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.slots.get(index).copied())
            .flatten()
    }

    /// `array[index] = value`, growing the array with holes as JavaScript does.
    fn set(&mut self, index: usize, value: i64) {
        if index >= self.slots.len() {
            self.slots.resize(index + 1, None);
        }

        self.slots[index] = Some(value);
    }
}

/// `array[index]` as a JavaScript *number*, so an absent index is `NaN`.
fn number(array: &Sparse, index: i64) -> f64 {
    array.get(index).map_or(f64::NAN, |value| value as f64)
}

/// JavaScript truthiness for a number: everything but `0`, `-0` and `NaN`.
fn truthy(value: f64) -> bool {
    value != 0.0 && !value.is_nan()
}

/// Upstream's `sort`: a 4-bit LSD radix sort of `array` by `string[a + offset]`.
///
/// Mutates `array` in place, and is stable — the scatter pass walks `array`
/// backwards and the gather pass drains each bucket backwards, so the two
/// reversals cancel.
///
/// **This is where BUG-SUFFIX-ARRAY-1 lives.** The width scan uses `Math.max`, which returns
/// `NaN` the moment one read is out of range, and the `&&`/`||` ladder that
/// turns the maximum into a bit count treats `NaN` as "no high bits" and yields
/// `8`. See the module docs.
fn sort(string: &Sparse, array: &mut [i64], offset: i64) {
    let l = array.len();
    let mut max: i64 = -1;
    // `Math.max(undefined, j)` is NaN, and NaN is sticky.
    let mut saw_undefined = false;

    for i in (0..l).rev() {
        match string.get(array[i] + offset) {
            None => saw_undefined = true,
            Some(value) => {
                if value > max {
                    max = value;
                }
            }
        }
    }

    // `j >> 24 && 32 || j >> 16 && 24 || j >> 8 && 16 || 8`, with `j` possibly
    // NaN (every shift is then 0, so every clause is falsy) and possibly -1
    // (an empty array, where `-1 >> 24` is -1 and the answer is 32).
    let bits: u32 = if saw_undefined {
        8
    } else {
        let max = max as i32;

        if max >> 24 != 0 {
            32
        } else if max >> 16 != 0 {
            24
        } else if max >> 8 != 0 {
            16
        } else {
            8
        }
    };

    let mut d: u32 = 0;

    while d < bits {
        let mut buckets: Vec<Vec<i64>> = vec![Vec::new(); 16];

        for i in (0..l).rev() {
            // `undefined >> d & 15` is 0, so a missing symbol sorts as if it
            // were zero -- which is precisely why BUG-SUFFIX-ARRAY-1 collides U+0100 with the
            // padding.
            let symbol = string.get(array[i] + offset).unwrap_or(0) as i32;
            let bucket = ((symbol >> d) & 15) as usize;

            buckets[bucket].push(array[i]);
        }

        let mut write = 0usize;

        for bucket in &buckets {
            for value in bucket.iter().rev() {
                array[write] = *value;
                write += 1;
            }
        }

        d += 4;
    }
}

/// Upstream's `compare`: order two suffixes given the rank `lookup`.
///
/// Returns a JavaScript number, `NaN` included. The caller tests `< 0`, which
/// `NaN` fails, so a `NaN` here means "do not take the left one".
fn compare(string: &Sparse, lookup: &Sparse, m: i64, n: i64) -> f64 {
    let head = number(string, m) - number(string, n);

    if truthy(head) {
        return head;
    }

    if m % 3 == 2 {
        let second = number(string, m + 1) - number(string, n + 1);

        if truthy(second) {
            return second;
        }

        number(lookup, m + 2) - number(lookup, n + 2)
    } else {
        number(lookup, m + 1) - number(lookup, n + 1)
    }
}

/// Upstream's `build`: the DC3 recursion.
///
/// `l` is the *unpadded* length; `string` may be longer (the top-level call
/// pads it) or exactly `l` (every recursive call does not pad at all, which is
/// where the out-of-range reads that trigger BUG-SUFFIX-ARRAY-1 come from).
///
/// **This is where BUG-SUFFIX-ARRAY-2 lives**, in `al`: `(2 * l / 3) | 0` omits the extra
/// ≡1 (mod 3) position that would separate the two halves of the reduced
/// string when `l % 3 == 1`.
fn build(string: &Sparse, l: usize) -> Vec<i64> {
    if l == 1 {
        return vec![0];
    }

    let al = 2 * l / 3;
    let bl = l - al;
    let r = (al + 1) >> 1;

    // `a[i] = ((i * 3) >> 1) + 1` -- the ≡1 and ≡2 (mod 3) positions,
    // interleaved.
    let mut a: Vec<i64> = (0..al).map(|i| ((i as i64 * 3) >> 1) + 1).collect();

    for offset in (0..3i64).rev() {
        sort(string, &mut a, offset);
    }

    // Index of position `value` inside the reduced string: the ≡1 group
    // occupies `0..r` and the ≡2 group `r..al`. An empty `a` makes `a[0]`
    // `undefined`, and upstream's `(undefined / 3) | 0` is 0 while
    // `undefined % 3 === 1` is false, so the index is `r`.
    let reduced_index = |value: Option<i64>| -> usize {
        match value {
            None => r,
            Some(value) => (value / 3) as usize + if value % 3 == 1 { 0 } else { r },
        }
    };

    let mut ranks = Sparse::default();
    let mut rank: i64 = 1;

    ranks.set(reduced_index(a.first().copied()), rank);

    for i in 1..al {
        let (this, previous) = (a[i], a[i - 1]);
        let differs = string.get(this) != string.get(previous)
            || string.get(this + 1) != string.get(previous + 1)
            || string.get(this + 2) != string.get(previous + 2);

        if differs {
            rank += 1;
        }

        ranks.set(reduced_index(Some(this)), rank);
    }

    // Ties remain, so the ranks are not yet a permutation: recurse on them.
    if rank < al as i64 {
        let order = build(&ranks, al);

        for i in (0..al).rev() {
            a[i] = if order[i] < r as i64 {
                order[i] * 3 + 1
            } else {
                (order[i] - r as i64) * 3 + 2
            };
        }
    }

    let mut lookup = Sparse::default();

    for i in (0..al).rev() {
        lookup.set(a[i] as usize, i as i64);
    }

    // The two sentinels that make `compare` terminate at the end of the
    // sequence: a suffix that has run out sorts before one that has not.
    lookup.set(l, -1);
    lookup.set(l + 1, -2);

    // The ≡0 (mod 3) positions, in the order their successors already have,
    // then stably re-sorted by their own first symbol.
    let mut b: Vec<i64> = if l % 3 == 1 {
        vec![l as i64 - 1]
    } else {
        Vec::new()
    };

    for &value in a.iter().take(al) {
        if value % 3 == 1 {
            b.push(value - 1);
        }
    }

    sort(string, &mut b, 0);

    // Merge the two sorted halves.
    let mut result: Vec<i64> = Vec::with_capacity(l);
    let (mut i, mut j) = (0usize, 0usize);

    while i < al && j < bl {
        if compare(string, &lookup, a[i], b[j]) < 0.0 {
            result.push(a[i]);
            i += 1;
        } else {
            result.push(b[j]);
            j += 1;
        }
    }

    while i < al {
        result.push(a[i]);
        i += 1;
    }

    while j < bl {
        result.push(b[j]);
        j += 1;
    }

    result
}

/// Upstream's `convert`: the sequence as an alphabet-indexed, padded array.
///
/// The padding is `length % 3` zeros — upstream's choice, and not enough to
/// keep `build`'s reads in range. See BUG-SUFFIX-ARRAY-1.
fn convert(sequence: &Sequence) -> Sparse {
    let length = sequence.len();
    let padding = length % 3;
    let mut values: Vec<i64> = Vec::with_capacity(length + padding);

    match sequence {
        Sequence::Text(units) => values.extend(units.iter().map(|&unit| unit as i64)),
        Sequence::Tokens(tokens) => {
            // `Object.keys(uniqueTokens).sort()` -- distinct tokens by their
            // string form, in JavaScript's default (UTF-16 code unit) string
            // order, numbered from 1.
            let mut alphabet: BTreeMap<Vec<u16>, i64> = BTreeMap::new();

            for token in tokens {
                alphabet.insert(token.encode_utf16().collect(), 0);
            }

            for (symbol, (_, value)) in alphabet.iter_mut().enumerate() {
                *value = symbol as i64 + 1;
            }

            for token in tokens {
                let key: Vec<u16> = token.encode_utf16().collect();

                values.push(alphabet[&key]);
            }
        }
    }

    values.resize(length + padding, 0);

    Sparse::from_dense(values)
}

/// Suffix array over one sequence.
#[derive(Debug, Clone)]
pub struct SuffixArray {
    sequence: Sequence,
    array: Vec<usize>,
}

impl SuffixArray {
    /// Build the suffix array of `sequence`.
    pub fn new(sequence: Sequence) -> Self {
        let length = sequence.len();
        let array = build(&convert(&sequence), length)
            .into_iter()
            // Every value written by `build` is a position in `0..l`; the
            // arithmetic that produces them cannot go negative.
            .map(|position| position as usize)
            .collect();

        Self { sequence, array }
    }

    /// `#.string` — the sequence the array was built over.
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// `#.length`, which is the *sequence's* length, not the array's.
    /// They are equal, but upstream reads the former.
    pub fn len(&self) -> usize {
        self.sequence.len()
    }

    /// Whether the underlying sequence is empty, in which case the suffix
    /// array is empty too.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `#.hasArbitrarySequence`.
    pub fn has_arbitrary_sequence(&self) -> bool {
        self.sequence.is_arbitrary()
    }

    /// `#.array` / `#.toJSON` — suffix start positions, lexicographically
    /// ordered (modulo BUG-SUFFIX-ARRAY-1 and BUG-SUFFIX-ARRAY-2).
    pub fn array(&self) -> &[usize] {
        &self.array
    }

    /// `#.toString`, which upstream defines as `this.array.join(',')`.
    pub fn to_joined_string(&self) -> String {
        join(&self.array)
    }
}

/// Suffix array over several sequences spliced together with [`SEPARATOR`].
#[derive(Debug, Clone)]
pub struct GeneralizedSuffixArray {
    text: Sequence,
    size: usize,
    first_length: usize,
    array: Vec<usize>,
}

impl GeneralizedSuffixArray {
    /// Build the generalized suffix array of `sequences`.
    ///
    /// # Errors
    ///
    /// An empty list. Upstream reads `strings[0].length` unguarded and throws a
    /// `TypeError`; there is no meaningful array to build, so this is an error
    /// rather than a panic — a panic would cross the FFI boundary and take the
    /// host process with it.
    ///
    /// Upstream decides text-vs-tokens from `strings[0]` alone and then treats
    /// every member the same way, so a mixed list is `strings[0]`'s kind
    /// applied to all of them. Here a mixed list is rejected instead; see the
    /// divergence list in `docs/modules/suffix-array.md`.
    pub fn new(sequences: &[Sequence]) -> Result<Self, &'static str> {
        let first = sequences.first().ok_or(
            "mnemonist/GeneralizedSuffixArray.constructor: cannot build one from no sequences.",
        )?;
        let arbitrary = first.is_arbitrary();

        if sequences.iter().any(|s| s.is_arbitrary() != arbitrary) {
            return Err(
                "mnemonist/GeneralizedSuffixArray.constructor: sequences must all be of one kind.",
            );
        }

        let text = if arbitrary {
            let mut tokens: Vec<String> = Vec::new();

            for (index, sequence) in sequences.iter().enumerate() {
                match sequence {
                    Sequence::Tokens(members) => tokens.extend(members.iter().cloned()),
                    Sequence::Text(_) => unreachable!("kinds were checked above"),
                }

                if index + 1 < sequences.len() {
                    tokens.push(String::from_utf16_lossy(&[SEPARATOR]));
                }
            }

            Sequence::Tokens(tokens)
        } else {
            // `strings.join('')`.
            let mut units: Vec<u16> = Vec::new();

            for (index, sequence) in sequences.iter().enumerate() {
                match sequence {
                    Sequence::Text(members) => units.extend_from_slice(members),
                    Sequence::Tokens(_) => unreachable!("kinds were checked above"),
                }

                if index + 1 < sequences.len() {
                    units.push(SEPARATOR);
                }
            }

            Sequence::Text(units)
        };

        let length = text.len();
        let array = build(&convert(&text), length)
            .into_iter()
            .map(|position| position as usize)
            .collect();

        Ok(Self {
            text,
            size: sequences.len(),
            first_length: first.len(),
            array,
        })
    }

    /// `#.text`.
    pub fn text(&self) -> &Sequence {
        &self.text
    }

    /// `#.size` — how many sequences were spliced together.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `#.firstLength`.
    pub fn first_length(&self) -> usize {
        self.first_length
    }

    /// `#.length` — the length of the spliced text, separators included.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Whether the underlying sequence is empty, in which case the suffix
    /// array is empty too.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `#.hasArbitrarySequence`.
    pub fn has_arbitrary_sequence(&self) -> bool {
        self.text.is_arbitrary()
    }

    /// `#.array` / `#.toJSON`.
    pub fn array(&self) -> &[usize] {
        &self.array
    }

    /// `#.toString`.
    pub fn to_joined_string(&self) -> String {
        join(&self.array)
    }

    /// `#.longestCommonSubsequence` — despite the name, the longest common
    /// **substring** of the first sequence and any other.
    ///
    /// Upstream's guards are asymmetric and are reproduced as written: a pair
    /// is skipped when both positions are `< firstLength` or both are
    /// `> firstLength`. A position *equal* to `firstLength` — the separator
    /// itself — satisfies neither, so pairs involving it are always considered.
    pub fn longest_common_subsequence(&self) -> Sequence {
        let mut lcs = match &self.text {
            Sequence::Text(_) => Sequence::Text(Vec::new()),
            Sequence::Tokens(_) => Sequence::Tokens(Vec::new()),
        };
        let length = self.len();

        for i in 1..length {
            let s = self.array[i];
            let t = self.array[i - 1];

            if s < self.first_length && t < self.first_length {
                continue;
            }

            if s > self.first_length && t > self.first_length {
                continue;
            }

            let mut lcp = (length - s).min(length - t);

            for j in 0..lcp {
                if !self.text.same(s + j, t + j) {
                    lcp = j;
                    break;
                }
            }

            if lcp > lcs.len() {
                lcs = self.text.slice(s, s + lcp);
            }
        }

        lcs
    }
}

/// `array.join(',')`.
fn join(array: &[usize]) -> String {
    array
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Sequence {
        Sequence::Text(value.encode_utf16().collect())
    }

    fn chars(value: &str) -> Sequence {
        Sequence::Tokens(value.chars().map(|c| c.to_string()).collect())
    }

    fn words(values: &[&str]) -> Sequence {
        Sequence::Tokens(values.iter().map(|&s| s.to_owned()).collect())
    }

    fn as_text(sequence: &Sequence) -> String {
        match sequence {
            Sequence::Text(units) => String::from_utf16_lossy(units),
            Sequence::Tokens(tokens) => tokens.join(""),
        }
    }

    /// A reference suffix array: sort every start position by the suffix it
    /// begins, comparing element by element. `O(n² log n)` and obviously
    /// correct, which is the point — it is the oracle BUG-SUFFIX-ARRAY-1 and BUG-SUFFIX-ARRAY-2 were found
    /// against.
    fn naive(sequence: &Sequence) -> Vec<usize> {
        let mut positions: Vec<usize> = (0..sequence.len()).collect();

        positions.sort_by(|&a, &b| match sequence {
            Sequence::Text(units) => units[a..].cmp(&units[b..]),
            Sequence::Tokens(tokens) => tokens[a..]
                .iter()
                .map(|t| t.encode_utf16().collect::<Vec<u16>>())
                .cmp(tokens[b..].iter().map(|t| t.encode_utf16().collect())),
        });

        positions
    }

    // ------------------------------------------------- upstream's own suite

    /// `test/suffix-array.js`, `'should produce the correct array.'`
    #[test]
    fn matches_the_upstream_suites_own_arrays() {
        let sa = SuffixArray::new(text("banana"));

        assert_eq!(sa.len(), 6);
        assert_eq!(sa.sequence(), &text("banana"));
        assert_eq!(sa.array(), [5, 3, 1, 0, 4, 2]);

        let sa = SuffixArray::new(text("This is a long string."));

        assert_eq!(
            sa.array(),
            [7, 4, 9, 14, 21, 0, 8, 13, 20, 1, 18, 5, 2, 10, 12, 19, 11, 17, 6, 3, 15, 16]
        );
    }

    /// `'should also work with arbitrary sequences.'`
    #[test]
    fn matches_the_upstream_suites_own_arbitrary_sequence() {
        let sa = SuffixArray::new(chars("banana"));

        assert_eq!(sa.len(), 6);
        assert_eq!(sa.sequence(), &chars("banana"));
        assert_eq!(sa.array(), [5, 3, 1, 0, 4, 2]);
    }

    /// `GeneralizedSuffixArray`, `'should produce the correct array.'`, both
    /// the string and the token form.
    #[test]
    fn matches_the_upstream_suites_own_generalized_arrays() {
        let sa = GeneralizedSuffixArray::new(&[text("banana"), text("ananas")]).unwrap();

        assert_eq!(sa.len(), 13);
        assert_eq!(sa.size(), 2);
        assert_eq!(sa.array(), [6, 5, 3, 1, 7, 9, 11, 0, 4, 2, 8, 10, 12]);

        let sa = GeneralizedSuffixArray::new(&[chars("banana"), chars("ananas")]).unwrap();

        assert_eq!(sa.len(), 13);
        assert_eq!(sa.size(), 2);
        assert_eq!(sa.array(), [6, 5, 3, 1, 7, 9, 11, 0, 4, 2, 8, 10, 12]);
    }

    /// `'should be possible to extract the longest common subsequence.'`
    #[test]
    fn matches_the_upstream_suites_own_lcs() {
        let sa = GeneralizedSuffixArray::new(&[text("banana"), text("ananas")]).unwrap();

        assert_eq!(sa.longest_common_subsequence(), text("anana"));

        let sa = GeneralizedSuffixArray::new(&[text("abcd"), text("cdef")]).unwrap();

        assert_eq!(sa.longest_common_subsequence(), text("cd"));

        let sa = GeneralizedSuffixArray::new(&[
            words(&["the", "cat", "eats", "the", "mouse"]),
            words(&["the", "mouse", "eats", "cheese"]),
        ])
        .unwrap();

        assert_eq!(sa.longest_common_subsequence(), words(&["the", "mouse"]));
    }

    // ------------------------------------------------------------ the bugs

    /// **BUG-SUFFIX-ARRAY-2**, pinned. `l % 3 == 1` with a repeated triple loses the
    /// separator between the two halves of the reduced string, and the answer
    /// is wrong. Every value here is upstream's, from Node 24.18.1.
    ///
    /// If this test ever "passes with the correct answer", the port has been
    /// silently fixed and no longer reproduces the library it claims to port.
    #[test]
    fn b91_lengths_congruent_to_one_mod_three_are_mis_sorted() {
        for (input, upstream) in [
            ("aaaaaaa", vec![6, 5, 3, 0, 2, 4, 1]),
            ("aaaaaaaaaa", vec![9, 8, 6, 3, 0, 5, 2, 7, 4, 1]),
            ("abcabcbbcb", vec![0, 3, 9, 6, 1, 4, 7, 2, 8, 5]),
        ] {
            let sa = SuffixArray::new(text(input));

            assert_eq!(sa.array(), upstream, "upstream's answer for {input:?}");
            assert_ne!(
                sa.array().to_vec(),
                naive(&text(input)),
                "{input:?} is supposed to be WRONG -- see BUG-SUFFIX-ARRAY-2"
            );
        }

        // The correct answers, for contrast.
        assert_eq!(naive(&text("aaaaaaa")), [6, 5, 4, 3, 2, 1, 0]);
    }

    /// **BUG-SUFFIX-ARRAY-1**, pinned. Two symbols that share a low byte are not separated,
    /// because an out-of-range read during the width scan collapses the radix
    /// to 8 bits. `U+0100` collides with the `0` padding; `U+0141` collides
    /// with `U+0041`. Both inputs have `length % 3 == 0`, so BUG-SUFFIX-ARRAY-2 is not
    /// involved. Values from Node 24.18.1.
    #[test]
    fn b90_symbols_sharing_a_low_byte_are_mis_sorted() {
        for (input, upstream) in [
            ("\u{100}\u{100}\u{100}\u{100}\u{201}\u{100}\u{100}\u{201}\u{201}\u{201}\u{201}\u{201}\u{100}\u{201}\u{201}",
             vec![0, 1, 2, 5, 3, 12, 6, 4, 14, 11, 10, 13, 9, 8, 7]),
            ("\u{141}\u{141}AAAAA\u{141}AAA\u{141}",
             vec![9, 3, 10, 2, 4, 5, 8, 6, 11, 7, 1, 0]),
        ] {
            let sequence = text(input);
            let sa = SuffixArray::new(sequence.clone());

            assert_eq!(sa.array(), upstream, "upstream's answer for {input:?}");
            assert_ne!(
                sa.array().to_vec(),
                naive(&sequence),
                "{input:?} is supposed to be WRONG -- see BUG-SUFFIX-ARRAY-1"
            );
        }
    }

    /// The complement of the two bugs: pure-ASCII input whose length is not
    /// `1 (mod 3)` **is** correct, exhaustively over every binary string of
    /// length 1..=14. That is what makes BUG-SUFFIX-ARRAY-1 and BUG-SUFFIX-ARRAY-2 precise claims rather
    /// than "it is sometimes wrong".
    #[test]
    fn ascii_inputs_off_the_bad_residue_are_exactly_right() {
        for length in 1..=14usize {
            if length % 3 == 1 {
                continue;
            }

            for encoding in 0..(1u32 << length) {
                let input: String = (0..length)
                    .map(|i| if encoding >> i & 1 == 1 { 'b' } else { 'a' })
                    .collect();
                let sequence = text(&input);

                assert_eq!(
                    SuffixArray::new(sequence.clone()).array(),
                    naive(&sequence),
                    "{input:?}"
                );
            }
        }
    }

    /// ...and the residue that is wrong is wrong for a *reason*, not at random:
    /// `l % 3 == 1` is fine until the recursion fires, which needs a repeated
    /// triple. Short inputs at the bad residue are still correct.
    #[test]
    fn the_bad_residue_is_correct_until_the_recursion_fires() {
        for input in ["a", "aaaa", "abcd", "abcdefg"] {
            let sequence = text(input);

            assert_eq!(
                SuffixArray::new(sequence.clone()).array(),
                naive(&sequence),
                "{input:?}"
            );
        }
    }

    // --------------------------------------------------------------- gaps

    /// The empty sequence, in both flavours. Upstream's suite never builds one,
    /// and it is the input that drives `build` through `al == 0`, where `a[0]`
    /// is `undefined` and the reduced index falls back to `r`.
    #[test]
    fn empty_sequences() {
        let sa = SuffixArray::new(text(""));

        assert_eq!(sa.len(), 0);
        assert!(sa.array().is_empty());
        assert_eq!(sa.to_joined_string(), "");

        let sa = SuffixArray::new(Sequence::Tokens(Vec::new()));

        assert_eq!(sa.len(), 0);
        assert!(sa.array().is_empty());
    }

    /// Length one and two, the base case and the smallest recursion.
    #[test]
    fn the_shortest_sequences() {
        assert_eq!(SuffixArray::new(text("a")).array(), [0]);
        assert_eq!(SuffixArray::new(text("ab")).array(), [0, 1]);
        assert_eq!(SuffixArray::new(text("ba")).array(), [1, 0]);
        assert_eq!(SuffixArray::new(text("aa")).array(), [1, 0]);
    }

    /// `#.toString` and `#.toJSON`. Upstream defines both and its suite calls
    /// neither.
    #[test]
    fn to_string_joins_the_array_with_commas() {
        let sa = SuffixArray::new(text("banana"));

        assert_eq!(sa.to_joined_string(), "5,3,1,0,4,2");

        let sa = GeneralizedSuffixArray::new(&[text("banana"), text("ananas")]).unwrap();

        assert_eq!(sa.to_joined_string(), "6,5,3,1,7,9,11,0,4,2,8,10,12");
    }

    /// The token alphabet is ordered by the tokens' *string* forms, so numeric
    /// tokens sort as `"1" < "10" < "2"`. Upstream's suite only ever uses
    /// tokens whose string order matches their intended order.
    #[test]
    fn the_token_alphabet_is_ordered_as_strings() {
        // Built over tokens whose lexicographic order differs from any numeric
        // reading; the suffix array must follow the string order.
        let sequence = words(&["10", "2", "1"]);
        let sa = SuffixArray::new(sequence.clone());

        assert_eq!(sa.array(), naive(&sequence));
        // "1" < "10" < "2", so the suffix starting at "1" comes first.
        assert_eq!(sa.array()[0], 2);
    }

    /// `GeneralizedSuffixArray` with one member: no separator is spliced, and
    /// the LCS is empty because there is nothing to be common *with*.
    #[test]
    fn a_generalized_array_of_one() {
        let sa = GeneralizedSuffixArray::new(&[text("banana")]).unwrap();

        assert_eq!(sa.size(), 1);
        assert_eq!(sa.len(), 6);
        assert_eq!(sa.first_length(), 6);
        assert_eq!(sa.array(), [5, 3, 1, 0, 4, 2]);
        assert_eq!(sa.longest_common_subsequence(), text(""));
    }

    /// Three members, which upstream's suite never builds (it uses two
    /// everywhere except the `it.skip`).
    #[test]
    fn a_generalized_array_of_three() {
        let sa = GeneralizedSuffixArray::new(&[text("abcd"), text("bcde"), text("cdef")]).unwrap();

        assert_eq!(sa.size(), 3);
        assert_eq!(sa.len(), 14);
        assert_eq!(sa.first_length(), 4);
        assert_eq!(as_text(&sa.longest_common_subsequence()), "bcd");
    }

    /// Disjoint members: the longest common substring is empty.
    #[test]
    fn disjoint_members_have_no_common_substring() {
        let sa = GeneralizedSuffixArray::new(&[text("abc"), text("xyz")]).unwrap();

        assert_eq!(sa.longest_common_subsequence(), text(""));
    }

    /// An empty list is refused rather than panicking; upstream throws a
    /// `TypeError` from `strings[0].length`.
    #[test]
    fn a_generalized_array_of_none_is_refused() {
        assert!(GeneralizedSuffixArray::new(&[]).is_err());
    }

    /// A mixed list is refused. Upstream would take `strings[0]`'s kind and
    /// apply it to everything, spreading a string into its characters. See the
    /// divergence list.
    #[test]
    fn a_mixed_generalized_array_is_refused() {
        assert!(GeneralizedSuffixArray::new(&[text("ab"), chars("cd")]).is_err());
    }

    /// The separator is a real element of the text, so it occupies a position
    /// and `length` counts it.
    #[test]
    fn the_separator_occupies_a_position() {
        let sa = GeneralizedSuffixArray::new(&[text("ab"), text("cd")]).unwrap();

        assert_eq!(sa.len(), 5);
        assert_eq!(sa.first_length(), 2);
        assert_eq!(as_text(sa.text()), "ab\u{1}cd");
    }
}
