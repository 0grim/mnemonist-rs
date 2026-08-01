//! Port of upstream `bi-map.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A `BiMap` keeps two `Map`s in lockstep — `items` (key → value) and
//! `inverse.items` (value → key) — so a lookup works in either direction and
//! `set`/`delete` keep both sides a true bijection. Inherits the T3 split
//! (DESIGN.md §3.8) directly: [`crate::map::OrderedMap`] owns order and
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
//! [`link`] keeps that fact visible rather than duplicating the four branches.
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
//! # `size` needs no drift-prone counter
//!
//! Unlike `default-map`'s `size` (B-40), nothing here can desynchronise `size`
//! from the entry count: every successful `set`/`delete` touches both maps in
//! the same call, and the early-return branches ((a) and (c) above) change
//! neither. So [`BiMap::size`] and [`BiMap::inverse_size`] simply read
//! [`OrderedMap::len`] — always equal to each other, because the structure is
//! a bijection by construction.

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
}

impl<K> Default for BiMap<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> BiMap<K> {
    pub fn new() -> Self {
        Self {
            items: OrderedMap::new(),
            inverse: OrderedMap::new(),
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

    /// Upstream's `size`: always `items().len()`. See the module docs for why
    /// no stored counter is needed.
    pub fn size(&self) -> usize {
        self.items.len()
    }

    /// Upstream's `inverse.size`: always equal to [`BiMap::size`], because the
    /// structure is a bijection.
    pub fn inverse_size(&self) -> usize {
        self.inverse.len()
    }

    /// Upstream's `clear`, shared verbatim by `BiMap` and `InverseMap`:
    /// `this.items.clear(); this.inverse.items.clear();` empties both sides
    /// regardless of which one `this` was.
    pub fn clear(&mut self) {
        self.items.clear();
        self.inverse.clear();
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

    pub fn has(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    pub fn has_reverse(&self, value: &K) -> bool {
        self.inverse.contains_key(value)
    }

    /// Upstream's `set`. See the module docs for the four-branch constraint
    /// resolution this reproduces.
    pub fn set(&mut self, key: K, value: K) {
        link(&mut self.items, &mut self.inverse, key, value);
    }

    /// `InverseMap.prototype.set`, called on the inverse view: the same
    /// function, with `items`/`inverse` swapped and the arguments in the
    /// order the inverse map's callers give them (`value` first).
    pub fn set_reverse(&mut self, value: K, key: K) {
        link(&mut self.inverse, &mut self.items, value, key);
    }

    /// Upstream's `delete`. `None` is upstream's `false`; `Some(value)` is
    /// `true`, carrying the value that was released from both sides.
    pub fn delete(&mut self, key: &K) -> Option<K> {
        unlink(&mut self.items, &mut self.inverse, key)
    }

    /// `InverseMap.prototype.delete`, called on the inverse view.
    pub fn delete_reverse(&mut self, value: &K) -> Option<K> {
        unlink(&mut self.inverse, &mut self.items, value)
    }
}

/// The shared body of `BiMap.prototype.set` / `InverseMap.prototype.set`.
///
/// `primary` is `this.items`, `secondary` is `this.inverse.items` — so calling
/// this with `(items, inverse, key, value)` is `BiMap::set`, and calling it
/// with `(inverse, items, value, key)` is the inverse view's `set`, exactly as
/// upstream binds one function to two receivers.
fn link<K: Hash + Eq + Clone>(
    primary: &mut OrderedMap<K, K>,
    secondary: &mut OrderedMap<K, K>,
    key: K,
    value: K,
) {
    if let Some(current_value) = primary.get(&key) {
        if *current_value == value {
            return;
        }

        let current_value = current_value.clone();
        secondary.delete(&current_value);
    }

    if let Some(current_key) = secondary.get(&value) {
        if *current_key == key {
            return;
        }

        let current_key = current_key.clone();
        primary.delete(&current_key);
    }

    primary.set(key.clone(), value.clone());
    secondary.set(value, key);
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
        // shared core is identical either way.
        map.clear();

        assert_eq!(map.size(), 0);
        assert_eq!(map.inverse_size(), 0);
        assert!(!map.has(&"one"));
        assert!(!map.has_reverse(&"hello"));
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
