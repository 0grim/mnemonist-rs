//! Port of upstream `multi-set.js` (mnemonist v0.40.4).
//!
//! A `MultiSet` is a `Map` from item to multiplicity (a count). Every write
//! upstream funnels through JavaScript's numeric coercion rules — `count ===
//! 0`, `count < 0`, `count = count || 1` — before touching `this.items`, so
//! this module's whole job is reproducing *those specific* rules over `f64`,
//! not over some cleaned-up integer count.
//!
//! # Counts are `f64`, on purpose
//!
//! `typeof count !== 'number'` is the only type guard upstream applies; it
//! never checks that `count` is an integer. So `set.add('a', 1.5)` is legal
//! upstream and leaves `size`/multiplicity fractional — an accepted, if
//! obscure, permissiveness rather than a bug (`docs/modules/multi-set.md`
//! covers it as "what we test in addition"). Modelling counts as anything
//! narrower than `f64` would need to either reject fractional input upstream
//! accepts, or round it to something upstream never produces; `f64`
//! reproduces it exactly, `NaN` included (see the next section).
//!
//! Argument-type validation itself (`typeof count !== 'number'`, which a
//! literal JS string like `'56'` fails) is a JavaScript concern the bridge
//! owns — same division as every other T2/T3 module's `typeof` guard.
//!
//! # `count = count || 1` folds a *falsy number* to `1`, not just `undefined`
//!
//! `||` tests truthiness, and among numbers only `0`, `-0` and `NaN` are
//! falsy. `count === 0` (which is `true` for `-0` too, per `===`) already
//! returns early in `add`/`remove`, and `count <= 0` already does in `set`,
//! so the only value the `||` line can actually still be looking at by the
//! time it runs is `NaN` (or `undefined`, when the caller omits the
//! argument) — both become `1`. `add('hello', NaN)` therefore behaves as
//! `add('hello', 1)`, confirmed by reading (this is unconditional numeric
//! coercion, not a runtime-dependent code path). [`fold_falsy`] is that one
//! rule, applied identically in [`MultiSet::add`], [`MultiSet::remove`] and
//! [`MultiSet::set`].
//!
//! # `set` on an existing key **adds**, it does not replace — likely upstream bug
//!
//! ```js
//! MultiSet.prototype.set = function(item, count) {
//!   ...
//!   currentCount = this.items.get(item);
//!   if (typeof currentCount === 'number') {
//!     this.items.set(item, currentCount + count);   // <- added, not replaced
//!   } else {
//!     this.dimension++;
//!     this.items.set(item, count);
//!   }
//!   this.size += count;
//!   return this;
//! };
//! ```
//!
//! A method named `set` reads as "make the multiplicity exactly `count`",
//! and `test/multi-set.js`'s own two `.set` cases never call it twice on the
//! same key with two *positive* counts (its double-call case follows a
//! positive `set` with a negative one, which takes the early
//! delete-on-non-positive branch instead) — so this is unexercised by gate 4
//! and worth flagging as a likely defect: NOTES.md B-160. [`MultiSet::set`]
//! reproduces the addition faithfully; a "corrected" replace-semantics
//! version would be *more correct than upstream* and therefore wrong per
//! CLAUDE.md's bug-for-bug mandate.
//!
//! # `dimension` is a **tracked counter**, not derived — unlike `multi-map`
//!
//! `crate::structures::multi_map::MultiMap::dimension` reads `items.len()`
//! directly, because every place that would touch its counter is already
//! guarded by checking whether the key is actually present. `MultiSet` does
//! not have that property, and reading `items.len()` here would silently
//! *fix* two real upstream defects rather than reproduce them:
//!
//! * **NOTES.md B-161 — `#.delete` on an absent item still decrements
//!   `dimension` and corrupts `size` to `NaN`, and reports `true`.**
//!   Upstream's guard is `if (count === 0) return false;`, but
//!   `this.items.get(item)` on a missing item is `undefined`, and
//!   `undefined === 0` is `false` — so the guard *never* actually fires (no
//!   live entry's multiplicity is ever exactly `0`; every method here
//!   deletes an item outright rather than leaving a zero behind). Confirmed
//!   by reading: the guard is dead code, and the fall-through does
//!   `this.size -= undefined` (`NaN`), `this.dimension--` unconditionally,
//!   and `this.items.delete(item)` (a harmless no-op on a missing key)
//!   before returning `true`.
//! * **NOTES.md B-162 — `#.edit` never touches `dimension` at all**, even
//!   when it removes a real key. If `b` already exists, `edit(a, b)` deletes
//!   `a` from `this.items` — the real distinct-key count drops by one — but
//!   `this.dimension` is left exactly where it was, so it overcounts by one
//!   from that point on.
//!
//! Neither is reachable through `test/multi-set.js`, which never calls
//! `#.delete` on a missing item and never asserts `.dimension` after an
//! `#.edit` that merges two already-existing keys. [`MultiSet::delete`] and
//! [`MultiSet::edit`] reproduce both bug-for-bug; see `docs/modules/
//! multi-set.md`'s "Bugs this found" for the write-up and the native tests
//! that pin them, since a derived `dimension` cannot express either.

use std::hash::Hash;

use crate::map::{MapCursor, OrderedMap};
use crate::structures::fixed_reverse_heap::FixedReverseHeap;
use crate::structures::heap::VecStore;
use crate::utils::comparators::{DefaultReverseComparator, Relational, Thrown};

/// Upstream's `count = count || 1`: a falsy *number* — `0`, `-0` or `NaN` —
/// becomes `1`. Callers of this function have already handled the
/// early-return branches that catch `0`/`-0` on their own, so in practice
/// only `NaN` (or a caller passing it directly) reaches here.
fn fold_falsy(count: f64) -> f64 {
    if count == 0.0 || count.is_nan() {
        1.0
    } else {
        count
    }
}

/// Upstream's `multi-set.top` argument guard message.
pub const TOP_ARITY: &str = "mnemonist/multi-set.top: n must be a number > 0.";

/// Upstream's `MultiSet`.
#[derive(Debug, Clone)]
pub struct MultiSet<K> {
    items: OrderedMap<K, f64>,
    /// Total multiplicity across every item. `f64` because a fractional
    /// count (see the module docs) makes this fractional too, faithfully.
    size: f64,
    /// Upstream's `dimension`, tracked rather than derived — see the module
    /// docs (B-161, B-162) for why the two can disagree. `i64` because
    /// B-161's bug can drive it negative.
    dimension: i64,
}

impl<K> Default for MultiSet<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> MultiSet<K> {
    pub fn new() -> Self {
        Self {
            items: OrderedMap::new(),
            size: 0.0,
            dimension: 0,
        }
    }

    pub fn size(&self) -> f64 {
        self.size
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.size = 0.0;
        self.dimension = 0;
    }

    /// The backing map, for the bridge's cursors and the differential fuzzer.
    pub fn items(&self) -> &OrderedMap<K, f64> {
        &self.items
    }
}

impl<K: Hash + Eq + Clone> MultiSet<K> {
    /// Upstream's `dimension`. See the module docs for why this is a
    /// tracked counter rather than `items.len()`.
    pub fn dimension(&self) -> i64 {
        self.dimension
    }

    pub fn has(&self, item: &K) -> bool {
        self.items.contains_key(item)
    }

    /// `#.multiplicity` / `#.get` / `#.count` — `0` for an absent item.
    pub fn multiplicity(&self, item: &K) -> f64 {
        self.items.get(item).copied().unwrap_or(0.0)
    }

    /// `#.frequency` — `0` on an empty set, never a division by zero `NaN`.
    pub fn frequency(&self, item: &K) -> f64 {
        if self.size == 0.0 {
            return 0.0;
        }

        self.multiplicity(item) / self.size
    }

    /// `#.add`. `count === 0` is a no-op; a negative count delegates to
    /// [`MultiSet::remove`] with the sign flipped, exactly as upstream's
    /// `return this.remove(item, -count)` does — including that this path
    /// does **not** return "this" upstream (`remove` has no return value),
    /// an inconsistency `test/multi-set.js` never observes and this port
    /// does not model (`docs/modules/multi-set.md`, deliberate divergences).
    pub fn add(&mut self, item: K, count: f64) {
        if count == 0.0 {
            return;
        }

        if count < 0.0 {
            self.remove(item, -count);
            return;
        }

        let count = fold_falsy(count);
        let current = self.items.get(&item).copied();

        if current.is_none() {
            self.dimension += 1;
        }

        self.size += count;
        self.items.set(item, current.unwrap_or(0.0) + count);
    }

    /// `#.remove`. Mirrors [`MultiSet::add`]'s early branches, symmetrically.
    pub fn remove(&mut self, item: K, count: f64) {
        if count == 0.0 {
            return;
        }

        if count < 0.0 {
            self.add(item, -count);
            return;
        }

        let count = fold_falsy(count);

        let Some(current) = self.items.get(&item).copied() else {
            return;
        };

        let updated = (current - count).max(0.0);

        if updated == 0.0 {
            self.items.delete(&item);
            self.size -= current;
            self.dimension -= 1;
        } else {
            self.items.set(item, updated);
            self.size -= count;
        }
    }

    /// `#.set` — sets the multiplicity to `count`, deleting the item for a
    /// non-positive one. See the module docs: for an *existing* item and a
    /// positive `count`, this **adds** rather than replaces, matching
    /// upstream's own (likely unintended) behaviour bug-for-bug.
    pub fn set(&mut self, item: K, count: f64) {
        if count <= 0.0 {
            if let Some(current) = self.items.delete(&item) {
                self.size -= current;
                self.dimension -= 1;
            }

            return;
        }

        let count = fold_falsy(count);
        let current = self.items.get(&item).copied();

        if current.is_none() {
            self.dimension += 1;
        }

        self.items.set(item, current.unwrap_or(0.0) + count);
        self.size += count;
    }

    /// `#.delete`. NOTES.md B-161: upstream's guard (`count === 0`) never
    /// actually fires, so deleting an item **not in the set** still
    /// decrements `dimension`, sets `size` to `NaN` (`- undefined` in
    /// JavaScript), and reports `true`. Reproduced exactly — see the module
    /// docs.
    pub fn delete(&mut self, item: &K) -> bool {
        let current = self.items.get(item).copied();

        // Upstream's `count === 0` guard: dead in practice (see the module
        // docs), kept for the same reason a dead branch elsewhere in this
        // codebase is kept -- it is what upstream actually wrote.
        if current == Some(0.0) {
            return false;
        }

        match current {
            Some(count) => {
                self.size -= count;
                self.items.delete(item);
            }
            None => {
                // `this.size -= undefined` -- JavaScript's `NaN`.
                self.size = f64::NAN;
            }
        }

        self.dimension -= 1;

        true
    }

    /// `#.edit` — moves `a`'s multiplicity onto `b`, combining with
    /// whatever `b` already had. A no-op if `a` is absent. Executed in
    /// upstream's own order (`set` on `b` before `delete` of `a`), which
    /// matters when `a === b`: the multiplicity is doubled and then the
    /// (now sole) entry is deleted outright. `dimension` is **never**
    /// touched here, matching upstream exactly (NOTES.md B-162) even though
    /// a real key can disappear (when `b` already existed).
    pub fn edit(&mut self, a: K, b: K) {
        let am = self.multiplicity(&a);

        if am == 0.0 {
            return;
        }

        let bm = self.multiplicity(&b);

        self.items.set(b, am + bm);
        self.items.delete(&a);
    }

    /// `#.top(n)` — the `n` most frequent items, ties broken by
    /// [`FixedReverseHeap`]'s own eviction order (insertion order among
    /// ties, since it evicts the *current* worst survivor).
    pub fn top(&self, n: usize) -> Result<Vec<(K, f64)>, &'static str>
    where
        K: Clone,
    {
        if n == 0 {
            return Err(TOP_ARITY);
        }

        let heap =
            FixedReverseHeap::new(VecStore::<CountKey<K>>::new(), DefaultReverseComparator, n);

        for (item, count) in self.items.iter() {
            heap.push(Some(CountKey(item.clone(), *count)))
                .expect("VecStore never fails");
        }

        let survivors = heap.consume().expect("VecStore never fails");

        Ok(survivors
            .to_vec()
            .into_iter()
            .flatten()
            .map(|CountKey(item, count)| (item, count))
            .collect())
    }

    /// A fresh cursor over `(item, count)` pairs in insertion order —
    /// upstream's `multiplicities()`/`keys()` (via `MapCursor` directly) and
    /// this module's flattened `values()`/`forEach` (via [`RepeatCursor`]).
    pub fn repeat_cursor(&self) -> RepeatCursor<K> {
        RepeatCursor::open()
    }
}

/// `MultiSet.isSubset(A, B)`: every item of `A` has at least `A`'s
/// multiplicity in `B`.
///
/// The `A === B` shortcut is upstream's object-identity check; `std::ptr::eq`
/// is its Rust equivalent and is exact at the bridge boundary, where the two
/// arguments are two `&Core` borrows extracted from (possibly) the same JS
/// object.
pub fn is_subset<K: Hash + Eq + Clone>(a: &MultiSet<K>, b: &MultiSet<K>) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }

    if a.dimension() > b.dimension() {
        return false;
    }

    for (item, count) in a.items.iter() {
        if b.multiplicity(item) < *count {
            return false;
        }
    }

    true
}

/// `MultiSet.isSuperset(A, B)` — upstream's own `isSubset(B, A)`.
pub fn is_superset<K: Hash + Eq + Clone>(a: &MultiSet<K>, b: &MultiSet<K>) -> bool {
    is_subset(b, a)
}

/// `#.top`'s sort key: an item paired with its count, ordered naturally by
/// count (`<`/`>` mean exactly what they say).
///
/// [`DefaultReverseComparator`] applied to this is
/// [`crate::utils::comparators::default_reverse_comparator`]: `if a < b
/// return 1; if a > b return -1; else 0` — the same function, branch order
/// swapped, as upstream's `MULTISET_ITEM_COMPARATOR` (`if (a[1] > b[1])
/// return -1; if (a[1] < b[1]) return 1; return 0`). `FixedReverseHeap`
/// keeps the `n` items that sort *first* under whatever comparator it is
/// given and hands `consume()` back in that same order — confirmed by
/// `fixed_reverse_heap`'s own `a_reverse_comparator_keeps_the_largest_items`
/// test, which applies this exact comparator shape to plain numbers and
/// gets the `n` largest, descending. That is `#.top`'s contract exactly.
#[derive(Debug, Clone)]
struct CountKey<K>(K, f64);

impl<K> Relational<Thrown> for CountKey<K> {
    fn js_lt(&self, other: &Self) -> Result<bool, Thrown> {
        Ok(self.1 < other.1)
    }

    fn js_gt(&self, other: &Self) -> Result<bool, Thrown> {
        Ok(self.1 > other.1)
    }
}

/// A live cursor that repeats each item `multiplicity` times — upstream's
/// `values()` and the walk `forEach` drives eagerly.
///
/// The repeat count is compared with `<` against the *raw* `f64`
/// multiplicity on every step, exactly as upstream's `for (i = 0; i <
/// multiplicity; i++)` does, so a fractional multiplicity yields
/// `ceil(multiplicity)` repeats rather than being rounded first —
/// `multiplicity = 2.5` yields `i = 0, 1, 2` (three), since `2 < 2.5` is
/// still true. Unreachable through `test/multi-set.js`, reachable through a
/// fractional `add`/`set` count.
#[derive(Debug, Clone)]
pub struct RepeatCursor<K> {
    outer: MapCursor,
    current: Option<K>,
    i: usize,
    limit: f64,
}

impl<K> Default for RepeatCursor<K> {
    fn default() -> Self {
        Self::open()
    }
}

impl<K> RepeatCursor<K> {
    pub fn open() -> Self {
        Self {
            outer: MapCursor::open(),
            current: None,
            i: 0,
            limit: 0.0,
        }
    }

    /// One step. `None` is permanent, exactly like [`MapCursor::step`].
    pub fn step(&mut self, items: &OrderedMap<K, f64>) -> Option<K>
    where
        K: Hash + Eq + Clone,
    {
        loop {
            if (self.i as f64) < self.limit {
                self.i += 1;
                return self.current.clone();
            }

            let (key, count) = self.outer.step(items)?;

            self.current = Some(key.clone());
            self.limit = *count;
            self.i = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multiplicities(set: &MultiSet<&'static str>) -> Vec<(&'static str, f64)> {
        set.items.iter().map(|(k, v)| (*k, *v)).collect()
    }

    fn values(set: &MultiSet<&'static str>) -> Vec<&'static str> {
        let mut cursor = set.repeat_cursor();
        let mut out = Vec::new();

        while let Some(item) = cursor.step(set.items()) {
            out.push(item);
        }

        out
    }

    #[test]
    fn reproduces_the_upstream_walkthrough() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("hello", 1.0);
        set.add("hello", 1.0);
        set.add("world", 1.0);

        assert_eq!(set.size(), 3.0);
        assert_eq!(set.dimension(), 2);
        assert_eq!(values(&set), vec!["hello", "hello", "world"]);
    }

    #[test]
    fn adding_zero_is_a_noop() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("hello", 0.0);

        assert_eq!(set.size(), 0.0);
        assert!(!set.has(&"hello"));
    }

    #[test]
    fn a_negative_add_removes() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("hello", 4.0);
        set.add("hello", -2.0);

        assert_eq!(set.size(), 2.0);
        assert_eq!(set.multiplicity(&"hello"), 2.0);
    }

    #[test]
    fn frequency_matches_the_upstream_example() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("apple", 5.0);
        set.add("pear", 2.0);
        set.add("melon", 3.0);

        assert_eq!(set.frequency(&"apple"), 0.5);
    }

    #[test]
    fn remove_more_than_present_floors_at_zero_and_deletes() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("hello", 5.0);
        set.remove("hello", 1.0);

        assert_eq!(set.size(), 4.0);
        assert_eq!(set.multiplicity(&"hello"), 4.0);

        set.remove("hello", 16.0);

        assert_eq!(set.size(), 0.0);
        assert_eq!(set.dimension(), 0);
        assert!(!set.has(&"hello"));
    }

    #[test]
    fn a_negative_remove_adds() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.remove("hello", -2.0);

        assert_eq!(set.size(), 2.0);
        assert_eq!(set.multiplicity(&"hello"), 2.0);
    }

    #[test]
    fn set_replaces_a_missing_item_but_adds_to_an_existing_one() {
        // B-160: upstream's `set` adds to an existing multiplicity instead
        // of replacing it. This test pins the port's faithful reproduction.
        let mut set: MultiSet<&str> = MultiSet::new();

        set.set("hello", 4.0);
        assert_eq!(set.multiplicity(&"hello"), 4.0);

        set.set("hello", 3.0);
        assert_eq!(
            set.multiplicity(&"hello"),
            7.0,
            "a second positive #.set adds rather than replacing -- B-160"
        );
    }

    #[test]
    fn set_to_a_non_positive_count_deletes() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.set("hello", 4.0);
        set.set("hello", -34.0);

        assert_eq!(set.size(), 0.0);
        assert_eq!(set.dimension(), 0);
        assert!(!set.has(&"hello"));
    }

    #[test]
    fn edit_moves_and_combines_multiplicity() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.edit("a", "b");
        assert_eq!(set.size(), 0.0);

        set.add("a", 1.0);
        set.edit("a", "b");
        assert_eq!(multiplicities(&set), vec![("b", 1.0)]);

        set.add("c", 1.0);
        set.edit("b", "c");
        assert_eq!(multiplicities(&set), vec![("c", 2.0)]);
    }

    #[test]
    fn deleting_an_existing_item_behaves_normally() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("hello", 1.0);
        set.add("hello", 1.0);
        set.add("world", 1.0);

        assert!(set.delete(&"hello"));

        assert_eq!(set.size(), 1.0);
        assert_eq!(set.dimension(), 1);
        assert_eq!(set.multiplicity(&"hello"), 0.0);
    }

    #[test]
    fn b_161_deleting_an_absent_item_corrupts_size_and_dimension_but_reports_true() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("hello", 1.0);

        // Deleting a key that was never in the set at all.
        let reported = set.delete(&"absent");

        assert!(
            reported,
            "B-161: upstream's dead `count === 0` guard means #.delete \
             always reports true, even for a key that was never present"
        );
        assert!(
            set.size().is_nan(),
            "B-161: `this.size -= undefined` is NaN in JavaScript"
        );
        assert_eq!(
            set.dimension(),
            0,
            "B-161: dimension decrements even though nothing was removed"
        );
        // The real entry is undisturbed.
        assert_eq!(set.multiplicity(&"hello"), 1.0);
    }

    #[test]
    fn b_162_edit_into_an_existing_key_does_not_adjust_dimension() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("a", 1.0);
        set.add("b", 1.0);
        assert_eq!(set.dimension(), 2);

        // `a` is merged into the already-existing `b`, so the set now truly
        // holds one distinct key -- but `dimension` was never touched.
        set.edit("a", "b");

        assert!(!set.has(&"a"));
        assert!(set.has(&"b"));
        assert_eq!(
            set.dimension(),
            2,
            "B-162: dimension still reports the pre-edit count of distinct keys"
        );
    }

    #[test]
    fn top_returns_the_n_most_frequent_descending() {
        let mut set: MultiSet<char> = MultiSet::new();

        for ch in "This is a very interesting albeit boring string.".chars() {
            set.add(ch, 1.0);
        }

        let top5 = set.top(5).unwrap();

        assert_eq!(
            top5,
            vec![('i', 7.0), (' ', 7.0), ('r', 4.0), ('e', 4.0), ('s', 4.0)]
        );

        assert_eq!(set.top(1).unwrap(), vec![('i', 7.0)]);
    }

    #[test]
    fn top_rejects_a_zero_n() {
        let set: MultiSet<&str> = MultiSet::new();

        assert_eq!(set.top(0), Err(TOP_ARITY));
    }

    #[test]
    fn size_stays_consistent_across_redundant_removes_issue_197() {
        let mut set: MultiSet<&str> = MultiSet::new();

        set.add("one", 1.0);
        set.add("one", 1.0);
        set.remove("one", 1.0);
        set.remove("one", 1.0);

        assert_eq!(set.size(), 0.0);
        assert_eq!(set.dimension(), 0);

        set.remove("one", 1.0);

        assert_eq!(set.size(), 0.0);
        assert_eq!(set.dimension(), 0);
    }

    #[test]
    fn subset_and_superset_match_the_upstream_examples() {
        let letters: MultiSet<char> = "aaabcdd".chars().collect::<CountingHelper>().into();
        let less_letters: MultiSet<char> = "aabc".chars().collect::<CountingHelper>().into();
        let other_letters: MultiSet<char> = "zk".chars().collect::<CountingHelper>().into();
        let overlapping: MultiSet<char> = "aaaac".chars().collect::<CountingHelper>().into();

        assert!(is_superset(&letters, &less_letters));
        assert!(!is_superset(&less_letters, &letters));
        assert!(!is_superset(&letters, &other_letters));
        assert!(!is_superset(&other_letters, &letters));
        assert!(!is_superset(&overlapping, &letters));
        assert!(!is_superset(&letters, &overlapping));

        assert!(!is_subset(&letters, &less_letters));
        assert!(is_subset(&less_letters, &letters));
        assert!(!is_subset(&letters, &other_letters));
        assert!(!is_subset(&other_letters, &letters));
        assert!(!is_subset(&overlapping, &letters));
        assert!(!is_subset(&letters, &overlapping));
    }

    #[test]
    fn a_set_is_its_own_subset_and_superset_by_identity() {
        let mut set: MultiSet<&str> = MultiSet::new();
        set.add("a", 1.0);

        assert!(is_subset(&set, &set));
        assert!(is_superset(&set, &set));
    }

    /// A tiny local helper turning a `char` iterator into a `MultiSet`,
    /// since this module deliberately has no `FromIterator`/`.from()` of its
    /// own -- that plumbing is `MultiSet.from`'s job, one layer up at the
    /// bridge.
    struct CountingHelper(Vec<char>);

    impl FromIterator<char> for CountingHelper {
        fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
            Self(iter.into_iter().collect())
        }
    }

    impl From<CountingHelper> for MultiSet<char> {
        fn from(helper: CountingHelper) -> Self {
            let mut set = MultiSet::new();

            for ch in helper.0 {
                set.add(ch, 1.0);
            }

            set
        }
    }
}
