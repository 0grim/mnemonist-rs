//! Port of upstream `bi-map.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A `BiMap` keeps two `Map`s in lockstep — `items` (key → value) and
//! `inverse.items` (value → key) — so a lookup works in either direction and
//! `set`/`delete` keep both sides a true bijection. Inherits the T3 split
//! (`docs/DECISIONS.md`'s iteration section) directly: [`crate::map::OrderedMap`] owns order and
//! liveness, and the bridge's `JsKey` supplies SameValueZero.
//!
//! # One core type parameter, not two
//!
//! Both `items` and `inverse.items` are real `Map`s, so both a `BiMap`'s keys
//! *and* its values become `Map` keys somewhere — the value in the forward
//! map is the key in the inverse one. Audited across the T3 family
//! (`docs/modules/default-map.md`), every key or value that reaches a `Map`
//! in the family's tests is a string or a number, so the bridge instantiates
//! this at a single `JsKey` for both `K` and `V`. Core stays generic over one
//! type parameter for the same reason `OrderedMap` is generic at all: nothing
//! here depends on that coincidence.
//!
//! # `set` reproduces upstream's three-way constraint resolution verbatim
//!
//! ```js
//! function set(key, value) {
//!   if (this.items.has(key)) {
//!     var currentValue = this.items.get(key);
//!     if (currentValue === value) return this;      // (a) already exactly this relation
//!     else this.inverse.items.delete(currentValue);  // (b) key was pointing elsewhere
//!   }
//!   if (this.inverse.items.has(value)) {
//!     var currentKey = this.inverse.items.get(value);
//!     if (currentKey === key) return this;           // (c) unreachable after (b) rebinds nothing
//!     else this.items.delete(currentKey);             // (d) value was claimed by another key
//!   }
//!   this.items.set(key, value);
//!   this.inverse.items.set(value, key);
//!   ...
//! }
//! ```
//!
//! [`BiMap::set`] is exactly that, expressed as [`OrderedMap`] calls, and
//! [`BiMap::set_reverse`] is the *same* function with `items`/`inverse`
//! swapped — which is what upstream's `InverseMap.prototype.set = set` is:
//! one function bound to two receivers. Sharing the core of both through
//! `link` keeps that fact visible rather than duplicating the four branches.
//!
//! # `InverseMap` is a *view*, not a second structure
//!
//! Upstream constructs a real second object (`this.inverse = new
//! InverseMap(this)`) whose six generic methods (`has`, `get`, `forEach`,
//! `keys`, `values`, `entries`) are `Map.prototype[name].apply(this.items,
//! ...)` — i.e. delegate straight to the *other* map. Nothing here needs a
//! second Rust value for that: [`BiMap::items`] and [`BiMap::inverse`] are the
//! two `OrderedMap`s, and the bridge builds a second JS-visible wrapper that
//! reads/writes through the *same* `BiMap`, exactly mirroring how upstream's
//! `InverseMap` reads/writes through the same two `Map`s its `BiMap` does.
//!
//! # `size` is a real counter, and `clear` desyncs it from `inverse.size`
//!
//! `set` and `delete` both refresh **both** counters on every call, from
//! either direction — `this.size = this.items.size; this.inverse.size =
//! this.inverse.items.size;` touches the same two properties whether `this`
//! is the `BiMap` or its `InverseMap`, because `this.items`/`this.inverse`
//! mean the complementary thing from either side. So neither of those two
//! methods can ever desynchronise the pair — `clear` is the only method that
//! can, and the next section is why.
//!
//! `clear` cannot make the same claim:
//!
//! ```js
//! function clear() {
//!   this.size = 0;
//!   this.items.clear();
//!   this.inverse.items.clear();
//! }
//! ```
//!
//! This empties **both** underlying maps regardless of which side calls it,
//! but resets only **one** counter — `this.size`, whichever `this` is. Calling
//! `bimap.clear()` zeroes `bimap.size` and leaves `bimap.inverse.size` at
//! whatever it was; calling `bimap.inverse.clear()` zeroes `bimap.inverse.size`
//! and leaves `bimap.size` stale. Verified against Node 24.18.1:
//!
//! ```text
//! var m = new BiMap(); m.set('a', 'a');
//! m.clear();
//! m.size            // -> 0
//! m.inverse.size    // -> 1, STALE — items.size and inverse.items.size are both 0
//! ```
//!
//! Recorded as **BUG-BI-MAP-1**. [`BiMap::size`]/[`BiMap::inverse_size`] are
//! therefore real stored counters, not derived from [`OrderedMap::len`], and
//! [`BiMap::clear`]/[`BiMap::clear_reverse`] are the two directions of the one
//! upstream function, reproducing exactly which single counter each resets.

use std::hash::Hash;

use crate::map::OrderedMap;

/// Upstream's `BiMap`.
///
/// One core value backs both `BiMap` and its `.inverse`: `items` is the
/// forward map (key → value), `inverse` is the reverse one (value → key).
#[derive(Debug, Clone)]
pub struct BiMap<K> {
    items: OrderedMap<K, K>,
    inverse: OrderedMap<K, K>,
    /// Upstream's `this.size`. A real stored counter, not `items.len()` — see
    /// "`clear` desyncs it from `inverse.size`" above. `set`/`delete` refresh
    /// it from `items.len()` on every call (from either direction), but
    /// `clear`/`clear_reverse` touch only one of the two counters, exactly as
    /// upstream's shared `clear` function touches only `this.size`.
    size: usize,
    /// Upstream's `this.inverse.size`. See [`BiMap::size`].
    inverse_size: usize,
}

impl<K> Default for BiMap<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> BiMap<K> {
    /// An empty bi-map — `new BiMap()`. Both directions and both size
    /// counters start at zero.
    pub fn new() -> Self {
        Self {
            items: OrderedMap::new(),
            inverse: OrderedMap::new(),
            size: 0,
            inverse_size: 0,
        }
    }

    /// The forward map — upstream's `this.items`, and what `.keys()`,
    /// `.values()`, `.entries()` and `.forEach()` walk.
    pub fn items(&self) -> &OrderedMap<K, K> {
        &self.items
    }

    /// The reverse map — upstream's `this.inverse.items`, and what
    /// `.inverse.keys()` etc. walk.
    pub fn inverse(&self) -> &OrderedMap<K, K> {
        &self.inverse
    }

    /// Upstream's `size`. A stored counter — see the module docs (BUG-BI-MAP-1) for
    /// why this must not be `items().len()`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Upstream's `inverse.size`. See [`BiMap::size`].
    pub fn inverse_size(&self) -> usize {
        self.inverse_size
    }

    /// Upstream's `clear`, called through the forward view: empties **both**
    /// underlying maps but resets only `this.size` — BUG-BI-MAP-1. `inverse_size` is
    /// left exactly as it was; it is not recomputed from the now-empty
    /// `inverse` map, because upstream's `this.inverse.size` is not touched
    /// either.
    pub fn clear(&mut self) {
        self.items.clear();
        self.inverse.clear();
        self.size = 0;
    }

    /// `InverseMap.prototype.clear` — the *same* upstream function as
    /// [`BiMap::clear`], called with `this` being the inverse view: empties
    /// both maps but resets only `this.size`, which from the inverse side is
    /// `inverse_size`. `size` is left stale — BUG-BI-MAP-1.
    pub fn clear_reverse(&mut self) {
        self.items.clear();
        self.inverse.clear();
        self.inverse_size = 0;
    }
}

impl<K: Hash + Eq + Clone> BiMap<K> {
    /// Upstream's `get`, delegated to `this.items.get(key)`.
    pub fn get(&self, key: &K) -> Option<&K> {
        self.items.get(key)
    }

    /// `.inverse.get(value)`, i.e. `this.items.get(value)` with `this` being
    /// the inverse map — `inverse.items.get(value)` in forward terms.
    pub fn get_reverse(&self, value: &K) -> Option<&K> {
        self.inverse.get(value)
    }

    /// Upstream's `has`, delegated to `this.items.has(key)`.
    pub fn has(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    /// `.inverse.has(value)` — membership in the reverse direction.
    pub fn has_reverse(&self, value: &K) -> bool {
        self.inverse.contains_key(value)
    }

    /// Upstream's `set`. See the module docs for the four-branch constraint
    /// resolution this reproduces. Unconditionally resyncs both counters from
    /// the live maps afterwards: upstream's `set` only touches `this.size` /
    /// `this.inverse.size` on the falling-through insert path, never on an
    /// early-return no-op, but a no-op is only reachable when `items`/
    /// `inverse` already hold the colliding entry — which cannot be true right
    /// after a `clear()` left a stale counter, since `clear` genuinely empties
    /// both maps. So an unconditional resync here can only ever recompute the
    /// value the maps already have, and stays exactly in step with upstream.
    pub fn set(&mut self, key: K, value: K) {
        link(&mut self.items, &mut self.inverse, key, value);
        self.resync_counters();
    }

    /// `InverseMap.prototype.set`, called on the inverse view: the same
    /// function, with `items`/`inverse` swapped and the arguments in the
    /// order the inverse map's callers give them (`value` first).
    pub fn set_reverse(&mut self, value: K, key: K) {
        link(&mut self.inverse, &mut self.items, value, key);
        self.resync_counters();
    }

    /// Upstream's `delete`. `None` is upstream's `false`; `Some(value)` is
    /// `true`, carrying the value that was released from both sides.
    ///
    /// Resyncs the counters ONLY when something was actually released.
    /// Unlike `set`, a no-op `delete` (key absent) is very much reachable with
    /// a stale counter still sitting there — right after `clear()` the maps
    /// are genuinely empty, so `delete` on any key is a no-op — and upstream's
    /// `del` does not touch either counter on that path. Resyncing
    /// unconditionally here would "heal" the stale counter early and hide
    /// BUG-BI-MAP-1 on exactly the case fuzzing found first.
    pub fn delete(&mut self, key: &K) -> Option<K> {
        let released = unlink(&mut self.items, &mut self.inverse, key);

        if released.is_some() {
            self.resync_counters();
        }

        released
    }

    /// `InverseMap.prototype.delete`, called on the inverse view. See
    /// [`BiMap::delete`] for why the resync is conditional.
    pub fn delete_reverse(&mut self, value: &K) -> Option<K> {
        let released = unlink(&mut self.inverse, &mut self.items, value);

        if released.is_some() {
            self.resync_counters();
        }

        released
    }

    /// Both counters from the live maps. Called after every `set`/`delete` —
    /// never after `clear`/`clear_reverse`, which is exactly what makes the
    /// two counters able to desync in the first place (BUG-BI-MAP-1).
    fn resync_counters(&mut self) {
        self.size = self.items.len();
        self.inverse_size = self.inverse.len();
    }
}

/// The shared body of `BiMap.prototype.set` / `InverseMap.prototype.set`.
///
/// `primary` is `this.items`, `secondary` is `this.inverse.items` — so calling
/// this with `(items, inverse, key, value)` is `BiMap::set`, and calling it
/// with `(inverse, items, value, key)` is the inverse view's `set`, exactly as
/// upstream binds one function to two receivers.
///
/// # One hash lookup per map, not two
///
/// A direct transcription of upstream's four branches reads `primary.get(&key)`
/// and then, at the very end, unconditionally writes `primary.set(key, value)`
/// — two separate hash lookups for the same key on every call (and the same
/// shape for `secondary`/`value`). Upstream cannot avoid that: a JS `Map` has
/// no "look up and get a handle to update in place" operation. `OrderedMap`
/// does ([`OrderedMap::get_mut`]), so when `key` (respectively `value`) is
/// already present, this updates the existing slot in place via
/// [`std::mem::replace`] and skips the closing `set` for that side entirely —
/// one lookup instead of two. When a side is genuinely new, there is nothing
/// to merge (the first lookup is a miss) and this costs exactly what the
/// direct transcription did.
///
/// This changes nothing observable: overwriting via `get_mut` keeps the
/// existing key's slot position, exactly like [`OrderedMap::set`]'s own
/// overwrite path, and produces the identical final key/value in both maps
/// that the original four-branch reading did. The two "already unreachable"
/// early returns (`(a)`/`(c)` in the module docs) are reached exactly as
/// often either way, since neither branch's *condition* changed — only which
/// call reads and writes the slot.
fn link<K: Hash + Eq + Clone>(
    primary: &mut OrderedMap<K, K>,
    secondary: &mut OrderedMap<K, K>,
    key: K,
    value: K,
) {
    let mut primary_already_set = false;

    if let Some(slot) = primary.get_mut(&key) {
        if *slot == value {
            return;
        }

        let current_value = std::mem::replace(slot, value.clone());
        secondary.delete(&current_value);
        primary_already_set = true;
    }

    let mut secondary_already_set = false;

    if let Some(slot) = secondary.get_mut(&value) {
        if *slot == key {
            return;
        }

        let current_key = std::mem::replace(slot, key.clone());
        primary.delete(&current_key);
        secondary_already_set = true;
    }

    if !primary_already_set {
        primary.set(key.clone(), value.clone());
    }

    if !secondary_already_set {
        secondary.set(value, key);
    }
}

/// The shared body of `BiMap.prototype.delete` / `InverseMap.prototype.delete`.
fn unlink<K: Hash + Eq + Clone>(
    primary: &mut OrderedMap<K, K>,
    secondary: &mut OrderedMap<K, K>,
    key: &K,
) -> Option<K> {
    let value = primary.delete(key)?;
    secondary.delete(&value);

    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(map: &OrderedMap<&'static str, &'static str>) -> Vec<(&'static str, &'static str)> {
        map.iter().map(|(k, v)| (*k, *v)).collect()
    }

    /// 1:1 port of the upstream suite's eleven `it` blocks, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut map: BiMap<&str> = BiMap::new();

        map.set("one", "hello");
        map.set("two", "world");
        assert_eq!(map.size(), 2);
        assert_eq!(map.inverse_size(), 2);

        // …constraints: key already set…
        map.set("two", "monde");
        assert_eq!(map.size(), 2);
        assert_eq!(map.inverse_size(), 2);

        // …value already set…
        map.set("three", "monde");
        assert_eq!(map.size(), 2);
        assert_eq!(map.inverse_size(), 2);

        // …key & value already set, cross-linked…
        let mut cross: BiMap<&str> = BiMap::new();
        cross.set("A", "B");
        cross.set("C", "D");
        cross.set("A", "D");
        assert_eq!(cross.size(), 1);
        assert_eq!(cross.inverse_size(), 1);

        // …has (a fresh map, as upstream's `it` block uses)…
        let mut existence: BiMap<&str> = BiMap::new();
        existence.set("one", "hello");
        existence.set("two", "world");
        assert!(existence.has(&"one"));
        assert!(!existence.has(&"three"));

        // …delete…
        let mut deletes: BiMap<&str> = BiMap::new();
        deletes.set("one", "hello");
        deletes.delete(&"one");
        assert_eq!(deletes.size(), 0);
        assert!(!deletes.has(&"one"));
        assert!(!deletes.has_reverse(&"hello"));

        // …clear…
        let mut cleared: BiMap<&str> = BiMap::new();
        cleared.set("one", "hello");
        cleared.clear();
        assert_eq!(cleared.size(), 0);
        assert!(!cleared.has(&"one"));

        // …get…
        let mut got: BiMap<&str> = BiMap::new();
        got.set("one", "hello");
        assert_eq!(got.get(&"one"), Some(&"hello"));
        assert_eq!(got.get_reverse(&"hello"), Some(&"one"));

        // …forEach / iteration, mirrored below via `walk`…
        let mut iterated: BiMap<&str> = BiMap::new();
        iterated.set("one", "hello");
        iterated.set("two", "world");
        assert_eq!(
            walk(iterated.items()),
            vec![("one", "hello"), ("two", "world")]
        );
        assert_eq!(
            walk(iterated.inverse()),
            vec![("hello", "one"), ("world", "two")]
        );

        // …from…
        let mut from_pairs: BiMap<&str> = BiMap::new();
        for (key, value) in [("one", "hello"), ("two", "world")] {
            from_pairs.set(key, value);
        }
        assert_eq!(from_pairs.size(), 2);
        assert_eq!(from_pairs.get(&"one"), Some(&"hello"));
    }

    #[test]
    fn set_is_a_no_op_when_the_exact_relation_already_exists() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("one", "hello");

        // Re-setting the identical pair must not touch insertion order.
        map.set("two", "world");
        map.set("one", "hello");

        assert_eq!(
            walk(map.items()),
            vec![("one", "hello"), ("two", "world")],
            "re-asserting an existing relation must not move it"
        );
    }

    #[test]
    fn rebinding_a_key_releases_its_old_value_from_the_inverse() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("one", "hello");
        map.set("one", "bonjour");

        assert_eq!(map.get(&"one"), Some(&"bonjour"));
        assert!(!map.has_reverse(&"hello"), "the old value must be released");
        assert!(map.has_reverse(&"bonjour"));
        assert_eq!(map.size(), 1);
        assert_eq!(map.inverse_size(), 1);
    }

    #[test]
    fn rebinding_a_value_releases_its_old_key_from_the_forward_map() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("one", "hello");
        map.set("two", "hello");

        assert!(!map.has(&"one"), "the old key must be released");
        assert!(map.has(&"two"));
        assert_eq!(map.get(&"two"), Some(&"hello"));
        assert_eq!(map.size(), 1);
    }

    /// The two-sided constraint in one call: `key` was pointing elsewhere AND
    /// `value` was claimed by another key.
    #[test]
    fn set_can_rebind_both_sides_of_the_bijection_in_one_call() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("a", "x");
        map.set("b", "y");

        map.set("a", "y");

        assert_eq!(map.get(&"a"), Some(&"y"));
        assert!(!map.has(&"b"), "b's old value was reassigned to a");
        assert!(!map.has_reverse(&"x"), "a's old value is released");
        assert_eq!(map.size(), 1);
        assert_eq!(map.inverse_size(), 1);
    }

    #[test]
    fn the_inverse_view_supports_the_full_method_set() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set_reverse("hello", "one");

        assert_eq!(map.get(&"one"), Some(&"hello"));
        assert_eq!(map.get_reverse(&"hello"), Some(&"one"));
        assert!(map.has_reverse(&"hello"));

        assert_eq!(map.delete_reverse(&"hello"), Some("one"));
        assert!(!map.has(&"one"));
        assert_eq!(map.size(), 0);
    }

    #[test]
    fn delete_on_a_missing_key_reports_it_and_changes_nothing() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("one", "hello");

        assert_eq!(map.delete(&"missing"), None);
        assert_eq!(map.size(), 1);
    }

    #[test]
    fn a_deleted_key_reinserted_moves_to_the_end() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("a", "1");
        map.set("b", "2");
        map.set("c", "3");
        map.delete(&"a");
        map.set("a", "9");

        assert_eq!(walk(map.items()), vec![("b", "2"), ("c", "3"), ("a", "9")]);
    }

    #[test]
    fn clear_called_on_the_inverse_view_also_empties_the_forward_map() {
        let mut map: BiMap<&str> = BiMap::new();
        map.set("one", "hello");
        map.set("two", "world");

        // `InverseMap.prototype.clear` is the SAME function as `BiMap`'s; the
        // bridge calls it through the inverse view, but the effect on the
        // shared core is identical either way: both underlying maps empty.
        map.clear_reverse();

        assert!(!map.has(&"one"));
        assert!(!map.has_reverse(&"hello"));
    }

    /// BUG-BI-MAP-1: `clear` empties both maps but resets only the counter on the
    /// side it was called from — verified against Node 24.18.1 (see the
    /// module docs). A port that also zeroes `inverse_size` here is *more
    /// correct* than upstream, which this project treats as a defect, not an
    /// improvement.
    #[test]
    fn clear_desyncs_size_from_inverse_size_bug_bi_map_1() {
        let mut forward: BiMap<&str> = BiMap::new();
        forward.set("a", "a");
        forward.clear();

        assert_eq!(forward.size(), 0, "clear() always resets its own side");
        assert_eq!(
            forward.inverse_size(),
            1,
            "clear() must NOT resync inverse_size — BUG-BI-MAP-1"
        );
        assert!(
            !forward.has(&"a"),
            "the underlying maps are empty either way"
        );
        assert!(!forward.has_reverse(&"a"));

        let mut reverse: BiMap<&str> = BiMap::new();
        reverse.set("a", "a");
        reverse.clear_reverse();

        assert_eq!(
            reverse.inverse_size(),
            0,
            "clear_reverse() resets its own side"
        );
        assert_eq!(
            reverse.size(),
            1,
            "clear_reverse() must NOT resync size — BUG-BI-MAP-1"
        );

        // The stale counter heals on the very next set/delete, exactly as it
        // does upstream, because that call recomputes both from the live maps.
        forward.set("b", "c");
        assert_eq!(forward.size(), 1);
        assert_eq!(forward.inverse_size(), 1);
    }

    #[test]
    fn an_empty_map_reports_nothing() {
        let map: BiMap<&str> = BiMap::new();

        assert_eq!(map.size(), 0);
        assert_eq!(map.get(&"x"), None);
        assert!(!map.has(&"x"));
        assert_eq!(walk(map.items()), vec![]);
    }
}
