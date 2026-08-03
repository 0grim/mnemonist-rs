//! Port of upstream `set.js`.
//!
//! Fourteen free functions over JavaScript `Set`s — the second unit in this
//! port with no instance, and the first whose *values* are sets rather than
//! numbers.
//!
//! # `set.js` is NOT a `Map`-backed module
//!
//! Worth stating because the filename suggests otherwise and the T3 audit in
//! [`crate::map`] covers every module that is. `set.js` contains zero
//! `new Map(` and six `new Set(`, holds no state of its own, and exports
//! nothing constructible. What it needs from the bridge is native JS `Set`
//! coercion, not storage.
//!
//! # Insertion order is the contract, not an implementation detail
//!
//! Eight of the fourteen `describe` blocks in `test/set.js` assert
//! `Array.from(result)` against an **ordered** array, so every function that
//! returns or mutates a set is pinned to a specific order. Three places where
//! that is load-bearing and easy to get wrong:
//!
//! * `intersection` iterates the **smallest** input, so the result's order
//!   follows whichever argument happened to be smallest — and ties go to the
//!   first. `intersection(new Set([3,2,1]), new Set([1,2]))` is `[1, 2]` while
//!   `intersection(new Set([3,2,1]), new Set([1,2,3]))` is `[3, 2, 1]`.
//!   Confirmed against Node 24.18.1.
//! * `disjunct` decides what to add **while `A` still holds the intersection**,
//!   and deletes afterwards. See [`disjunct`] for what that does and does not
//!   buy: the *decision* order is observable, the *write* order is not.
//! * `Set.add` on a member that is already present does **not** move it, which
//!   is [`crate::map::OrderedMap::set`]'s behaviour and the reason this is
//!   built on it rather than on a fresh structure.
//!
//! # The four mutating functions return a *trace*
//!
//! `add`, `subtract`, `intersect` and `disjunct` return `undefined` upstream
//! and do all their work on their first argument. They mutate the
//! [`OrderedSet`] here too, but they *also* return the [`SetOp`] sequence they
//! applied, in order.
//!
//! That is for the bridge, and it is not decoration. The bridge holds a real
//! JavaScript `Set` that the caller still owns; replaying `add`/`delete` calls
//! onto it in upstream's own order reproduces upstream exactly, where
//! rebuilding it from a final member list would silently change what an
//! iterator already open over that set sees. Nothing in `test/set.js` can tell
//! the difference — which is precisely why it is worth not guessing.

use std::hash::Hash;

use crate::map::OrderedMap;

/// Upstream's throw when `intersection` is called with fewer than two sets.
pub const INTERSECTION_ARITY: &str = "mnemonist/Set.intersection: needs at least two arguments.";

/// Upstream's throw when `union` is called with fewer than two sets.
pub const UNION_ARITY: &str = "mnemonist/Set.union: needs at least two arguments.";

/// One mutation applied to the first argument of a mutating set function.
///
/// The order of a [`Vec<SetOp>`] is upstream's call order, exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOp<T> {
    /// `A.add(member)` — a no-op upstream if it is already present, and this
    /// variant is still emitted for it, because upstream still makes the call.
    Add(T),
    /// `A.delete(member)`.
    Delete(T),
}

/// An insertion-ordered set with JavaScript `Set` semantics.
///
/// A newtype over [`OrderedMap<T, ()>`], because a JS `Set` *is* a `Map` whose
/// values are ignored — same insertion order, same
/// delete-then-reinsert-moves-to-the-end rule, same SameValueZero keying (which
/// is a property of `T`, supplied by the bridge). Reimplementing it would mean
/// two places to get those three rules wrong.
#[derive(Debug, Clone, Default)]
pub struct OrderedSet<T> {
    members: OrderedMap<T, ()>,
}

impl<T: Hash + Eq + Clone> OrderedSet<T> {
    /// An empty set — `new Set()`.
    pub fn new() -> Self {
        Self {
            members: OrderedMap::new(),
        }
    }

    /// Build from members in insertion order, as `new Set(iterable)` does.
    /// Duplicates keep their **first** position.
    pub fn from_members(members: impl IntoIterator<Item = T>) -> Self {
        let mut set = Self::new();

        for member in members {
            set.add(member);
        }

        set
    }

    /// `Set.prototype.add`. Returns whether the member was new — upstream
    /// returns the set itself, which is not a value this port needs.
    pub fn add(&mut self, member: T) -> bool {
        self.members.set(member, ()).is_none()
    }

    /// `Set.prototype.delete`, returning whether anything was removed.
    pub fn delete(&mut self, member: &T) -> bool {
        self.members.delete(member).is_some()
    }

    /// `Set.prototype.has`.
    pub fn has(&self, member: &T) -> bool {
        self.members.contains_key(member)
    }

    /// `Set.prototype.size`.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the set has no members — `size === 0`.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Members in insertion order — `Set.prototype.values()`.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.members.keys()
    }

    /// Members in insertion order, owned. A convenience for callers that need
    /// a snapshot; unlike a JS iterator it cannot observe later mutations.
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }

    /// Replay a trace produced by one of the mutating functions.
    ///
    /// Exists so a Rust caller can apply a trace it was handed, and so the
    /// bridge's replay onto a real JS `Set` has an in-crate counterpart to be
    /// tested against.
    pub fn apply(&mut self, ops: &[SetOp<T>]) {
        for op in ops {
            match op {
                SetOp::Add(member) => {
                    self.add(member.clone());
                }
                SetOp::Delete(member) => {
                    self.delete(member);
                }
            }
        }
    }
}

impl<T: Hash + Eq + Clone> PartialEq for OrderedSet<T> {
    /// Order-sensitive, deliberately. `Array.from(a)` vs `Array.from(b)` is
    /// what `test/set.js` compares, so two sets with the same members in
    /// different orders are two different answers here.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T: Hash + Eq + Clone> Eq for OrderedSet<T> {}

/// `exports.intersection` — variadic, iterating the smallest input.
///
/// # Errors
///
/// [`INTERSECTION_ARITY`] for fewer than two sets, as upstream throws.
pub fn intersection<T: Hash + Eq + Clone>(
    sets: &[&OrderedSet<T>],
) -> Result<OrderedSet<T>, &'static str> {
    if sets.len() < 2 {
        return Err(INTERSECTION_ARITY);
    }

    let mut result = OrderedSet::new();

    // Find the smallest, bailing out entirely on the first empty one. Ties go
    // to the first, because the comparison is strict — and that decides the
    // result's ORDER, which the original suite asserts.
    let mut smallest: Option<&OrderedSet<T>> = None;

    for set in sets {
        if set.is_empty() {
            return Ok(result);
        }

        if smallest.is_none_or(|current| set.len() < current.len()) {
            smallest = Some(set);
        }
    }

    let smallest = smallest.expect("at least two sets, none of them empty");

    for member in smallest.iter() {
        let keep = sets.iter().all(|set| {
            // Upstream's `if (set === smallestSet) continue;` — identity, not
            // equality. Reproduced with pointer equality; see the module docs
            // on why the bridge cannot supply it and why that is unobservable.
            std::ptr::eq(*set, smallest) || set.has(member)
        });

        if keep {
            result.add(member.clone());
        }
    }

    Ok(result)
}

/// `exports.union` — variadic, in argument then insertion order.
///
/// # Errors
///
/// [`UNION_ARITY`] for fewer than two sets, as upstream throws.
pub fn union<T: Hash + Eq + Clone>(sets: &[&OrderedSet<T>]) -> Result<OrderedSet<T>, &'static str> {
    if sets.len() < 2 {
        return Err(UNION_ARITY);
    }

    let mut result = OrderedSet::new();

    for set in sets {
        for member in set.iter() {
            result.add(member.clone());
        }
    }

    Ok(result)
}

/// `exports.difference` — `A \ B`.
///
/// Upstream short-circuits twice, and the second one is observable as an
/// *identity*: an empty `B` returns `new Set(A)`, a copy, not `A` itself.
pub fn difference<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> OrderedSet<T> {
    if a.is_empty() {
        return OrderedSet::new();
    }

    if b.is_empty() {
        return a.clone();
    }

    let mut result = OrderedSet::new();

    for member in a.iter() {
        if !b.has(member) {
            result.add(member.clone());
        }
    }

    result
}

/// `exports.symmetricDifference` — `A \ B` first, then `B \ A`.
pub fn symmetric_difference<T: Hash + Eq + Clone>(
    a: &OrderedSet<T>,
    b: &OrderedSet<T>,
) -> OrderedSet<T> {
    let mut result = OrderedSet::new();

    for member in a.iter() {
        if !b.has(member) {
            result.add(member.clone());
        }
    }

    for member in b.iter() {
        if !a.has(member) {
            result.add(member.clone());
        }
    }

    result
}

/// `exports.isSubset` — every member of `A` is in `B`.
pub fn is_subset<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }

    if a.len() > b.len() {
        return false;
    }

    a.iter().all(|member| b.has(member))
}

/// `exports.isSuperset`, which upstream defines as `isSubset(B, A)`.
pub fn is_superset<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> bool {
    is_subset(b, a)
}

/// `exports.add` — every member of `B` into `A`.
///
/// Returns the trace, which is one `Add` per member of `B` including the ones
/// already in `A`: upstream makes the call regardless, and a bridge replaying
/// the trace onto a live `Set` should too.
pub fn add<T: Hash + Eq + Clone>(a: &mut OrderedSet<T>, b: &OrderedSet<T>) -> Vec<SetOp<T>> {
    let ops: Vec<SetOp<T>> = b.iter().cloned().map(SetOp::Add).collect();

    a.apply(&ops);

    ops
}

/// `exports.subtract` — every member of `B` out of `A`.
pub fn subtract<T: Hash + Eq + Clone>(a: &mut OrderedSet<T>, b: &OrderedSet<T>) -> Vec<SetOp<T>> {
    let ops: Vec<SetOp<T>> = b.iter().cloned().map(SetOp::Delete).collect();

    a.apply(&ops);

    ops
}

/// `exports.intersect` — drop from `A` everything not in `B`.
///
/// Upstream deletes *while iterating* `A`, which a JS `Set` iterator tolerates
/// (the current entry is simply gone). The trace is collected first here, which
/// is the same sequence of calls in the same order.
pub fn intersect<T: Hash + Eq + Clone>(a: &mut OrderedSet<T>, b: &OrderedSet<T>) -> Vec<SetOp<T>> {
    let ops: Vec<SetOp<T>> = a
        .iter()
        .filter(|member| !b.has(member))
        .cloned()
        .map(SetOp::Delete)
        .collect();

    a.apply(&ops);

    ops
}

/// `exports.disjunct` — turn `A` into the symmetric difference of `A` and `B`.
///
/// Upstream's three phases, in order:
///
/// 1. collect `A ∩ B` into `toRemove`;
/// 2. add every member of `B` not in `A`;
/// 3. delete `toRemove`.
///
/// # What phase 2 preceding phase 3 does and does not buy — measured
///
/// The load-bearing part is that the `!A.has(member)` test in phase 2 sees an
/// `A` that **still holds the intersection**. Delete first and every member of
/// `A ∩ B` passes that test, gets re-added, and the answer becomes `A ∪ B`.
/// That sabotage turns `test/set.js`'s `#.disjunct` block red.
///
/// The *write* order, by contrast, buys nothing observable. Reordering only
/// the writes — deleting first while still testing against the original `A` —
/// leaves both the result and its order unchanged, because a member of
/// `B \ A` is appended at the end either way and a member of `A ∩ B` is gone
/// either way. Sabotaged and confirmed: `test/set.js` stayed at 16 passing,
/// and so did `tests/boundary/set.js`.
///
/// The trace is still emitted add-then-delete, because that is the sequence of
/// calls upstream makes and the bridge replays them onto a live JavaScript
/// `Set`. It is faithfulness with no test able to see it, and it is labelled as
/// such rather than justified with a benefit it does not have.
pub fn disjunct<T: Hash + Eq + Clone>(a: &mut OrderedSet<T>, b: &OrderedSet<T>) -> Vec<SetOp<T>> {
    let to_remove: Vec<T> = a.iter().filter(|member| b.has(member)).cloned().collect();

    let mut ops: Vec<SetOp<T>> = b
        .iter()
        .filter(|member| !a.has(member))
        .cloned()
        .map(SetOp::Add)
        .collect();

    ops.extend(to_remove.into_iter().map(SetOp::Delete));

    a.apply(&ops);

    ops
}

/// `exports.intersectionSize` — `|A ∩ B|`, walking the smaller set.
pub fn intersection_size<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> usize {
    let (small, large) = if a.len() > b.len() { (b, a) } else { (a, b) };

    if small.is_empty() {
        return 0;
    }

    if std::ptr::eq(small, large) {
        return small.len();
    }

    small.iter().filter(|member| large.has(member)).count()
}

/// `exports.unionSize` — `|A| + |B| - |A ∩ B|`.
pub fn union_size<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> usize {
    a.len() + b.len() - intersection_size(a, b)
}

/// `exports.jaccard` — `|A ∩ B| / |A ∪ B|`.
///
/// Upstream returns `0` for an empty intersection *without dividing*, so
/// `jaccard(∅, ∅)` is `0` rather than the `NaN` the formula would give. That is
/// a convention, not a bug, and it is reproduced.
pub fn jaccard<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> f64 {
    let shared = intersection_size(a, b);

    if shared == 0 {
        return 0.0;
    }

    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// `exports.overlap` — `|A ∩ B| / min(|A|, |B|)`.
///
/// Same zero convention as [`jaccard`], and for the same reason: the guard is
/// what keeps the division away from a zero denominator.
pub fn overlap<T: Hash + Eq + Clone>(a: &OrderedSet<T>, b: &OrderedSet<T>) -> f64 {
    let shared = intersection_size(a, b);

    if shared == 0 {
        return 0.0;
    }

    shared as f64 / a.len().min(b.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(members: &[i32]) -> OrderedSet<i32> {
        OrderedSet::from_members(members.iter().copied())
    }

    fn members(set: &OrderedSet<i32>) -> Vec<i32> {
        set.to_vec()
    }

    /// `new Set('contact')` — the one place `test/set.js` uses a non-numeric
    /// member, and the only reason `jaccard` and `overlap` have a case where
    /// the two sets differ in size.
    fn chars(text: &str) -> OrderedSet<char> {
        OrderedSet::from_members(text.chars())
    }

    // ---------------------------------------------------------- OrderedSet

    #[test]
    fn construction_keeps_first_position_for_duplicates() {
        assert_eq!(members(&set(&[3, 1, 3, 2, 1])), vec![3, 1, 2]);
    }

    /// A JS `Set` moves a member to the end only after a `delete`; re-adding a
    /// present member leaves it where it is. Both halves, because getting one
    /// right and the other wrong is the likely mistake.
    #[test]
    fn re_adding_does_not_move_but_delete_then_add_does() {
        let mut s = set(&[1, 2, 3]);

        s.add(1);
        assert_eq!(members(&s), vec![1, 2, 3]);

        assert!(s.delete(&1));
        s.add(1);
        assert_eq!(members(&s), vec![2, 3, 1]);

        assert!(!s.delete(&99));
    }

    #[test]
    fn reports_membership_and_size() {
        let s = set(&[4, 5]);

        assert!(s.has(&4));
        assert!(!s.has(&6));
        assert_eq!(s.len(), 2);
        assert!(!s.is_empty());
        assert!(OrderedSet::<i32>::new().is_empty());
    }

    /// Equality is order-sensitive, because the original suite compares
    /// `Array.from` output.
    #[test]
    fn equality_is_order_sensitive() {
        assert_eq!(set(&[1, 2]), set(&[1, 2]));
        assert_ne!(set(&[1, 2]), set(&[2, 1]));
        assert_ne!(set(&[1, 2]), set(&[1, 2, 3]));
    }

    // -------------------------------------------------- the twelve queries

    #[test]
    fn intersection_matches_the_original_suite() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);
        assert_eq!(members(&intersection(&[&a, &b]).unwrap()), vec![2, 3]);

        let a = set(&[1, 2, 3, 4]);
        let b = set(&[2, 3, 4]);
        let c = set(&[1, 4]);
        let d = set(&[4, 5, 6]);
        assert_eq!(members(&intersection(&[&a, &b, &c, &d]).unwrap()), vec![4]);
    }

    /// The result's ORDER follows the smallest input, and ties go to the first.
    /// Confirmed against Node 24.18.1; the original suite cannot see it,
    /// because its variadic case intersects down to a single member.
    #[test]
    fn intersection_order_follows_the_smallest_set() {
        let big = set(&[3, 2, 1]);
        let small = set(&[1, 2]);

        assert_eq!(members(&intersection(&[&big, &small]).unwrap()), vec![1, 2]);

        let same_size = set(&[1, 2, 3]);
        assert_eq!(
            members(&intersection(&[&big, &same_size]).unwrap()),
            vec![3, 2, 1],
            "a tie goes to the first argument"
        );
    }

    #[test]
    fn intersection_bails_out_on_the_first_empty_set() {
        let a = set(&[1, 2]);
        let empty = OrderedSet::new();

        assert!(intersection(&[&empty, &a]).unwrap().is_empty());
        assert!(intersection(&[&a, &empty]).unwrap().is_empty());
    }

    #[test]
    fn the_two_variadic_functions_need_two_arguments() {
        let a = set(&[1]);

        assert_eq!(intersection(&[&a]).unwrap_err(), INTERSECTION_ARITY);
        assert_eq!(intersection::<i32>(&[]).unwrap_err(), INTERSECTION_ARITY);
        assert_eq!(union(&[&a]).unwrap_err(), UNION_ARITY);
        assert_eq!(union::<i32>(&[]).unwrap_err(), UNION_ARITY);
    }

    /// The identity shortcut, exercised by passing one reference twice.
    #[test]
    fn passing_the_same_set_twice_is_the_set_itself() {
        let a = set(&[1, 2]);

        assert_eq!(members(&intersection(&[&a, &a]).unwrap()), vec![1, 2]);
        assert!(is_subset(&a, &a));
        assert_eq!(intersection_size(&a, &a), 2);
        assert_eq!(union_size(&a, &a), 2);
        assert_eq!(jaccard(&a, &a), 1.0);
        assert_eq!(overlap(&a, &a), 1.0);
    }

    #[test]
    fn union_matches_the_original_suite() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);
        assert_eq!(members(&union(&[&a, &b]).unwrap()), vec![1, 2, 3, 4]);

        let a = set(&[1, 2, 3, 4]);
        let b = set(&[2, 3, 4]);
        let c = set(&[1, 4]);
        let d = set(&[4, 5, 6]);
        assert_eq!(
            members(&union(&[&a, &b, &c, &d]).unwrap()),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn difference_matches_the_original_suite_and_its_two_shortcuts() {
        let a = set(&[1, 2, 3, 4, 5]);
        let b = set(&[2, 3]);
        assert_eq!(members(&difference(&a, &b)), vec![1, 4, 5]);

        let empty = OrderedSet::new();
        assert!(difference(&empty, &a).is_empty());
        // An empty B returns a COPY of A, order intact.
        assert_eq!(members(&difference(&a, &empty)), members(&a));
    }

    #[test]
    fn symmetric_difference_is_a_then_b() {
        let a = set(&[1, 2, 3]);
        let b = set(&[3, 4, 5]);

        assert_eq!(members(&symmetric_difference(&a, &b)), vec![1, 2, 4, 5]);
        // Reversing the arguments reverses the halves, which the suite never
        // checks.
        assert_eq!(members(&symmetric_difference(&b, &a)), vec![4, 5, 1, 2]);
        assert!(symmetric_difference(&a, &a).is_empty());
    }

    #[test]
    fn subset_and_superset_match_the_original_suite() {
        let a = set(&[1, 2]);
        let b = set(&[1, 2, 3]);
        let c = set(&[2, 4]);

        assert!(is_subset(&a, &b));
        assert!(!is_subset(&c, &b));
        assert!(is_superset(&b, &a));
        assert!(!is_superset(&b, &c));

        // The empty set is a subset of everything, including itself.
        let empty = OrderedSet::new();
        assert!(is_subset(&empty, &b));
        assert!(is_subset(&empty, &empty));
        assert!(!is_subset(&b, &empty));
    }

    // ------------------------------------------- the four mutating functions

    #[test]
    fn add_matches_the_original_suite_and_traces_every_call() {
        let mut a = set(&[1, 2]);
        let ops = add(&mut a, &set(&[2, 3]));

        assert_eq!(members(&a), vec![1, 2, 3]);
        // `2` was already present; upstream still calls `add`, so the trace
        // still records it.
        assert_eq!(ops, vec![SetOp::Add(2), SetOp::Add(3)]);
    }

    #[test]
    fn subtract_matches_the_original_suite() {
        let mut a = set(&[1, 2]);
        let ops = subtract(&mut a, &set(&[2, 3]));

        assert_eq!(members(&a), vec![1]);
        assert_eq!(ops, vec![SetOp::Delete(2), SetOp::Delete(3)]);
    }

    #[test]
    fn intersect_matches_the_original_suite() {
        let mut a = set(&[1, 2]);
        let ops = intersect(&mut a, &set(&[2, 3]));

        assert_eq!(members(&a), vec![2]);
        assert_eq!(ops, vec![SetOp::Delete(1)]);
    }

    #[test]
    fn disjunct_matches_the_original_suite_in_upstreams_phase_order() {
        let mut a = set(&[1, 2]);
        let ops = disjunct(&mut a, &set(&[2, 3]));

        assert_eq!(members(&a), vec![1, 3]);
        assert_eq!(ops, vec![SetOp::Add(3), SetOp::Delete(2)]);
    }

    /// The part of `disjunct`'s phase order that is actually load-bearing:
    /// phase 2 tests against an `A` that still holds the intersection. Delete
    /// first and every shared member passes `!A.has`, is re-added, and the
    /// answer becomes the union instead of the symmetric difference.
    ///
    /// Pinned separately from the block above because the block above passes
    /// under that sabotage *and* under the harmless one — reordering only the
    /// writes — and the two need telling apart. See [`super::disjunct`].
    #[test]
    fn disjunct_decides_what_to_add_before_it_deletes_anything() {
        let mut a = set(&[1, 2, 3]);
        let ops = disjunct(&mut a, &set(&[2, 3, 4]));

        // 2 and 3 are shared: removed, and never re-added.
        assert_eq!(members(&a), vec![1, 4]);
        assert_eq!(
            ops,
            vec![SetOp::Add(4), SetOp::Delete(2), SetOp::Delete(3)],
            "no shared member may appear as an Add"
        );
    }

    /// Self-application, which the original suite never does. All four have a
    /// defined answer and none of them loops.
    #[test]
    fn the_mutating_functions_applied_to_their_own_argument() {
        // Rust cannot alias `&mut a` and `&a`, so the second argument is a
        // clone — which is what upstream's second argument *is*, for every
        // purpose except `intersection`'s identity shortcut.
        let mut a = set(&[1, 2, 3]);
        let same = a.clone();
        add(&mut a, &same);
        assert_eq!(members(&a), vec![1, 2, 3]);

        let mut a = set(&[1, 2, 3]);
        let same = a.clone();
        subtract(&mut a, &same);
        assert!(a.is_empty());

        let mut a = set(&[1, 2, 3]);
        let same = a.clone();
        intersect(&mut a, &same);
        assert_eq!(members(&a), vec![1, 2, 3]);

        let mut a = set(&[1, 2, 3]);
        let same = a.clone();
        disjunct(&mut a, &same);
        assert!(a.is_empty());
    }

    // ------------------------------------------------------ the four metrics

    #[test]
    fn intersection_size_matches_the_original_suite() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);
        let empty = OrderedSet::new();

        assert_eq!(intersection_size(&a, &b), 2);
        assert_eq!(intersection_size(&a, &empty), 0);
        // …and the swap, which the suite only reaches in one direction.
        assert_eq!(intersection_size(&empty, &a), 0);
        assert_eq!(intersection_size(&b, &a), 2);
    }

    #[test]
    fn union_size_matches_the_original_suite() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);
        let empty = OrderedSet::new();

        assert_eq!(union_size(&a, &b), 4);
        assert_eq!(union_size(&a, &empty), 3);
        assert_eq!(union_size(&empty, &empty), 0);
    }

    #[test]
    fn jaccard_matches_the_original_suite() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);
        let empty = OrderedSet::new();

        assert_eq!(jaccard(&a, &b), 2.0 / 4.0);
        assert_eq!(jaccard(&a, &empty), 0.0);
        assert_eq!(jaccard(&chars("contact"), &chars("context")), 4.0 / 7.0);
    }

    #[test]
    fn overlap_matches_the_original_suite() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);
        let empty = OrderedSet::new();

        assert_eq!(overlap(&a, &b), 2.0 / 3.0);
        assert_eq!(overlap(&a, &empty), 0.0);
        assert_eq!(overlap(&chars("contact"), &chars("context")), 4.0 / 5.0);
    }

    /// Both metrics answer `0` for two empty sets rather than dividing by
    /// zero. Upstream's convention, and `test/set.js` never asks.
    #[test]
    fn the_ratios_answer_zero_rather_than_nan_when_nothing_is_shared() {
        let empty: OrderedSet<i32> = OrderedSet::new();

        assert_eq!(jaccard(&empty, &empty), 0.0);
        assert_eq!(overlap(&empty, &empty), 0.0);
        assert_eq!(union_size(&empty, &empty), 0);
    }
}
