//! Port of upstream `critbit-tree-map.js` (mnemonist v0.40.4).
//!
//! A crit-bit tree (a.k.a. PATRICIA trie): a binary tree that branches on the
//! position of the first bit at which two keys differ, rather than on a
//! shared token alphabet the way `trie_map` does. Every stored key lives in
//! an **external node** (a leaf); every **internal node** holds only a
//! branching decision — which byte, and which bit within that byte.
//!
//! # Arena, not `Box`
//!
//! Upstream links `InternalNode`/`ExternalNode` objects directly
//! (`node.left`/`node.right`). Porting that as `Box<Node<V>>` would need
//! either unsafe pointer juggling or a recursive splice-and-return shape to
//! satisfy the borrow checker for the "bubble up to the right ancestor"
//! step below — and upstream's own algorithm is already exactly an
//! index-based, iterative bubble-up over an explicit `ancestors`/`path`
//! stack. So this port keeps upstream's own shape: two parallel arenas,
//! `keys`/`values` for external nodes and `internals` for internal nodes,
//! addressed by [`Ptr`] — upstream's own encoding (`0` empty, positive an
//! internal node, negative an external one) kept verbatim because
//! `fixed_critbit_tree_map` (which genuinely cannot use `Box` — its arrays
//! are pre-allocated and bounds-checked) needs the identical encoding, and
//! keeping both variants shaped the same way is what makes the family
//! resemblance — and the divergence, at capacity — easy to see.
//!
//! # The critical-bit computation, ported byte-for-byte from `utils/bitwise.js`
//!
//! Keys are `Vec<u8>` here, not `String`: upstream's `charCodeAt` returns a
//! UTF-16 code unit (0..=65535), and every bitwise helper it feeds — `msb8`,
//! `criticalBit8Mask` — masks with `0xff`, silently discarding anything above
//! bit 7. That is only ever a no-op for Latin-1/ASCII code points (< 256);
//! for anything wider it means upstream's own critical-bit arithmetic does
//! not compute what "first differing bit" would suggest at all (see
//! `find_critical_bit`'s doc comment for the mechanism). No test in either
//! original suite ever supplies a key outside that range, so this port
//! narrows to bytes and does not attempt to reproduce the wide-character
//! case — see D-245 in DECISIONS-CANDIDATES.md.
//!
//! [`msb8`] and [`mask_for`] are the literal `bitwise.js` functions,
//! specialised to `u8` (Rust's `!x` on a `u8` already truncates to 8 bits, so
//! no `& 0xff` is needed the way JavaScript's 32-bit `~` requires one).
//! [`get_direction`] is upstream's own `(1 + (byte | mask)) >> 8` trick,
//! kept exactly rather than replaced with a clearer bit-extraction, because
//! the trick's degenerate case — a mask of `0xff`, which happens exactly
//! when the two keys' diverging byte pair XORs to `0` (the tail-extension
//! comparison against an implicit `0`, see below) — makes *every* present
//! byte route right regardless of its value, and a hand-rolled
//! "extract bit N" reimplementation does not reproduce that for free.
//!
//! # Keys that differ only in length
//!
//! `find_critical_bit`'s tail branch — one key is a strict prefix of the
//! other — compares the longer key's next byte against an *implicit* `0`,
//! never against a real byte from the shorter key (there isn't one). This is
//! `mask = bitwise.criticalBit8Mask(b.charCodeAt(i))` upstream: a one-argument
//! call into a two-argument function, so the missing `b` parameter is
//! `undefined`, and `a ^ undefined` is `a ^ NaN`, which XOR coerces to `a ^ 0`
//! — upstream's own comment says as much ("NOTE: x ^ 0 is the same as x").
//! [`diverging_byte`] returns `0` for the absent side to match, and every
//! caller feeds that pair through the same `mask_for` used everywhere else —
//! no special case needed, which is exactly why porting the real bitwise
//! trick (rather than a cleaner reinterpretation) pays for itself here.
//!
//! # `set`'s bubble-up, ported variable-for-variable
//!
//! `ancestors`/`path`/`best` below are upstream's own three variables,
//! unchanged in name and shape: `ancestors` is every internal node walked
//! through before reaching the conflicting external leaf, root-first;
//! `path[i]` is whether that step went left; `best` is the *deepest* ancestor
//! whose own critical bit is not "more specific" than the new one — found by
//! walking `ancestors` **backwards** (leaf-to-root) and stopping at the first
//! one that is not skipped. Comparing `(byte_index, mask)` as a Rust tuple
//! reproduces upstream's packed `(byteIndex << 8) | mask` integer ordering
//! exactly, since `mask` never exceeds `0xff`.

/// One arena slot reference: upstream's own encoding, verbatim.
///
/// `0` is upstream's `null` (an empty tree, only ever seen as `root` before
/// the first key is set). A positive value `p` is an internal node at
/// `internals[p - 1]`; a negative value `p` is an external node (a stored
/// key/value pair) at `keys[-p - 1]`/`values[-p - 1]`.
type Ptr = i64;

const EMPTY: Ptr = 0;

fn internal_ptr(index: usize) -> Ptr {
    index as Ptr + 1
}

fn external_ptr(index: usize) -> Ptr {
    -(index as Ptr) - 1
}

/// Upstream's `utils/bitwise.js#msb8`: the value's highest set bit, alone.
fn msb8(x: u8) -> u8 {
    let mut x = x;
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x & !(x >> 1)
}

/// Upstream's `criticalBit8Mask`: every bit set **except** the critical one.
///
/// `!msb8(...)` on a `u8` already truncates to 8 bits, unlike JavaScript's
/// `~`, which operates on a 32-bit integer and needs the explicit `& 0xff`
/// upstream writes.
fn mask_for(a: u8, b: u8) -> u8 {
    !msb8(a ^ b)
}

/// Upstream's `getDirection`: `0`/left or `1`/right, packed as
/// `(1 + (byte | mask)) >> 8` so that a mask of `0xff` (see the module docs)
/// makes every present byte route right regardless of its value.
///
/// A key shorter than `byte_index` always routes left — upstream's
/// `if (byteIndex > key.length - 1) return 0;`.
fn get_direction(key: &[u8], byte_index: usize, mask: u8) -> bool {
    match key.get(byte_index) {
        None => false,
        Some(&byte) => (1u16 + (byte as u16 | mask as u16)) >> 8 == 1,
    }
}

/// The byte index at which `a` and `b` first differ, and the two real byte
/// values there — upstream's own "swap so `a` is the shortest, scan, then
/// compare the tail against an implicit `0`" — or `None` if the keys are
/// identical.
fn diverging_byte(a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
    let (shorter, longer) = if a.len() <= b.len() { (a, b) } else { (b, a) };

    for i in 0..shorter.len() {
        if shorter[i] != longer[i] {
            return Some((i, shorter[i], longer[i]));
        }
    }

    if shorter.len() == longer.len() {
        return None;
    }

    Some((shorter.len(), 0, longer[shorter.len()]))
}

/// One internal (branching) node: upstream's `InternalNode`.
#[derive(Debug, Clone)]
struct Internal {
    byte_index: usize,
    mask: u8,
    left: Ptr,
    right: Ptr,
}

/// A read-only view of one node, matching upstream's own raw shape exactly:
/// `{critbit, left, right}` for an `InternalNode`, `{key, value}` for an
/// `ExternalNode`, or [`RootNode::Empty`] for `null` (an empty tree or an
/// absent child — upstream never distinguishes the two, and neither does
/// this).
///
/// Exists for the bridge's `root` getter and the differential fuzz spec's
/// `root` observation — upstream's `root` is a real, argument-free property,
/// so it is the one place a *structural* comparison is possible through the
/// oracle's generic property-read protocol (see `fuzz/oracle.js`'s own
/// docs); every other public method needs at least one argument. `critbit`
/// is reassembled into upstream's own packed `(byteIndex << 8) | mask`
/// integer — not this port's internal `(byte_index, mask)` tuple — because
/// that packed value is exactly what upstream's real `InternalNode.critbit`
/// holds, and reproducing it here is what makes the comparison catch a
/// critical-bit computation bug rather than merely a `root`-rendering one.
pub enum RootNode<'a, V> {
    Empty,
    Internal {
        critbit: u32,
        left: Box<RootNode<'a, V>>,
        right: Box<RootNode<'a, V>>,
    },
    External {
        key: &'a [u8],
        value: &'a V,
    },
}

/// A crit-bit tree map from byte-string keys to arbitrary values.
///
/// See the module docs for the arena representation and the reasoning behind
/// every non-obvious step of `set`.
#[derive(Debug, Clone)]
pub struct CritBitTreeMap<V> {
    keys: Vec<Vec<u8>>,
    // `Option<V>`, not `V`: a deleted external node becomes unreachable from
    // `root` (every pointer that used to lead to it has been rewired, in
    // `delete`), but its slot cannot be removed from this `Vec` outright --
    // every other arena reference is a bare index, stable only as long as
    // nothing before it is removed and shifts everything after it down. So
    // `delete` takes the value out in place with `Option::take` and leaves a
    // `None` hole rather than reclaiming the slot, matching upstream's own
    // strategy of leaving a deleted object for the garbage collector rather
    // than compacting anything.
    values: Vec<Option<V>>,
    internals: Vec<Internal>,
    root: Ptr,
    size: usize,
}

impl<V> Default for CritBitTreeMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> CritBitTreeMap<V> {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            values: Vec::new(),
            internals: Vec::new(),
            root: EMPTY,
            size: 0,
        }
    }

    /// Upstream's `size`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Upstream's `clear`.
    pub fn clear(&mut self) {
        self.keys.clear();
        self.values.clear();
        self.internals.clear();
        self.root = EMPTY;
        self.size = 0;
    }

    /// Every stored value, mutably. Exists for the bridge: a value that
    /// holds a JS reference must have it released — on `clear` and at
    /// finalization — before it is dropped, matching
    /// `trie_map::TrieMap::values_mut`'s own reasoning. A `delete`d slot is
    /// already `None` (see `delete`'s doc comment), so filtering to
    /// `Some` here is exactly "every value still reachable from `root`",
    /// with no separate reachability walk needed.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.values.iter_mut().filter_map(Option::as_mut)
    }

    /// Upstream's `root` property. See [`RootNode`]'s docs.
    pub fn root(&self) -> RootNode<'_, V> {
        self.node_at(self.root)
    }

    fn node_at(&self, pointer: Ptr) -> RootNode<'_, V> {
        if pointer == EMPTY {
            return RootNode::Empty;
        }

        if pointer > 0 {
            let node = &self.internals[(pointer - 1) as usize];
            let critbit = ((node.byte_index as u32) << 8) | node.mask as u32;

            RootNode::Internal {
                critbit,
                left: Box::new(self.node_at(node.left)),
                right: Box::new(self.node_at(node.right)),
            }
        } else {
            let index = (-pointer - 1) as usize;

            RootNode::External {
                key: &self.keys[index],
                value: self.values[index]
                    .as_ref()
                    .expect("a reachable external node always holds a value"),
            }
        }
    }

    /// Upstream's `set`. Returns the value displaced, if the key already
    /// existed — upstream returns `this` for chaining and gives no other way
    /// to observe the old value; the bridge and the fuzz spec both want it,
    /// matching `trie_map::TrieMap::set`'s own convention.
    pub fn set(&mut self, key: impl Into<Vec<u8>>, value: V) -> Option<V> {
        let key = key.into();

        if self.size == 0 {
            // The freshly pushed index, NOT a hardcoded `0`: after a
            // `delete` has emptied the tree, `keys`/`values` already hold
            // stale, orphaned entries (see `delete`'s doc comment; this
            // port's arena is append-only, unlike upstream's own
            // garbage-collected object references), so the next insert
            // lands at `keys.len()`, wherever that is. A hardcoded `0`
            // here -- an earlier draft of this port had exactly that --
            // pointed `root` at a stale, already-taken slot the moment a
            // second insert followed a delete-to-empty, and
            // `CritBitTreeMap::root`'s own "always holds a value" panic
            // caught it within the first few generated operations of this
            // module's differential-fuzz campaign.
            let index = self.keys.len();

            self.keys.push(key);
            self.values.push(Some(value));
            self.size = 1;
            self.root = external_ptr(index);

            return None;
        }

        let mut pointer = self.root;
        let mut ancestors: Vec<usize> = Vec::new();
        let mut path: Vec<bool> = Vec::new();

        loop {
            if pointer > 0 {
                let internal_index = (pointer - 1) as usize;
                let node = &self.internals[internal_index];
                let go_right = get_direction(&key, node.byte_index, node.mask);
                let child = if go_right { node.right } else { node.left };

                if child == EMPTY {
                    let new_external = self.keys.len();

                    self.keys.push(key);
                    self.values.push(Some(value));
                    self.size += 1;

                    let node = &mut self.internals[internal_index];
                    let slot = if go_right {
                        &mut node.right
                    } else {
                        &mut node.left
                    };
                    *slot = external_ptr(new_external);

                    return None;
                }

                ancestors.push(internal_index);
                path.push(!go_right);
                pointer = child;
            } else {
                let external_index = (-pointer - 1) as usize;

                let Some((byte_index, a_byte, b_byte)) =
                    diverging_byte(&key, &self.keys[external_index])
                else {
                    // Identical key: replace the value in place.
                    return self.values[external_index].replace(value);
                };

                let mask = mask_for(a_byte, b_byte);
                let new_goes_left = !get_direction(&key, byte_index, mask);

                self.size += 1;

                let new_external = self.keys.len();
                self.keys.push(key);
                self.values.push(Some(value));

                let internal_index = self.internals.len();
                let (left, right) = if new_goes_left {
                    (external_ptr(new_external), pointer)
                } else {
                    (pointer, external_ptr(new_external))
                };

                self.internals.push(Internal {
                    byte_index,
                    mask,
                    left,
                    right,
                });

                // Bubbling up: the deepest ancestor whose own critical bit is
                // not "more specific" (a larger `(byte_index, mask)` pair)
                // than the new one. See the module docs.
                let mut best: Option<usize> = None;

                for i in (0..ancestors.len()).rev() {
                    let ancestor = &self.internals[ancestors[i]];

                    if (ancestor.byte_index, ancestor.mask) > (byte_index, mask) {
                        continue;
                    }

                    best = Some(i);
                    break;
                }

                match best {
                    None => {
                        self.root = internal_ptr(internal_index);

                        if let Some(&parent) = ancestors.first() {
                            let slot = if new_goes_left {
                                &mut self.internals[internal_index].right
                            } else {
                                &mut self.internals[internal_index].left
                            };
                            *slot = internal_ptr(parent);
                        }
                    }
                    Some(best) if best == ancestors.len() - 1 => {
                        let parent = ancestors[best];
                        let went_left = path[best];
                        let slot = if went_left {
                            &mut self.internals[parent].left
                        } else {
                            &mut self.internals[parent].right
                        };
                        *slot = internal_ptr(internal_index);
                    }
                    Some(best) => {
                        let parent = ancestors[best];
                        let went_left = path[best];
                        let child = ancestors[best + 1];

                        let parent_slot = if went_left {
                            &mut self.internals[parent].left
                        } else {
                            &mut self.internals[parent].right
                        };
                        *parent_slot = internal_ptr(internal_index);

                        let new_slot = if new_goes_left {
                            &mut self.internals[internal_index].right
                        } else {
                            &mut self.internals[internal_index].left
                        };
                        *new_slot = internal_ptr(child);
                    }
                }

                return None;
            }
        }
    }

    /// Upstream's `get`.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        let mut pointer = self.root;

        loop {
            if pointer == EMPTY {
                return None;
            }

            if pointer > 0 {
                let node = &self.internals[(pointer - 1) as usize];
                let go_right = get_direction(key, node.byte_index, node.mask);

                pointer = if go_right { node.right } else { node.left };
            } else {
                let index = (-pointer - 1) as usize;

                if self.keys[index] != key {
                    return None;
                }

                return self.values[index].as_ref();
            }
        }
    }

    /// Upstream's `has`.
    pub fn has(&self, key: &[u8]) -> bool {
        let mut pointer = self.root;

        loop {
            if pointer == EMPTY {
                return false;
            }

            if pointer > 0 {
                let node = &self.internals[(pointer - 1) as usize];
                let go_right = get_direction(key, node.byte_index, node.mask);

                pointer = if go_right { node.right } else { node.left };
            } else {
                let index = (-pointer - 1) as usize;

                return self.keys[index] == key;
            }
        }
    }

    /// Upstream's `delete`. Returns the removed value, unlike upstream's
    /// plain boolean — a caller that only wants the boolean uses
    /// `.is_some()`; the bridge needs the value to release a held JS
    /// reference, matching `trie_map::TrieMap::delete`'s convention.
    ///
    /// Deleted internal/external arena slots are never reclaimed — upstream
    /// leaves them to the garbage collector, unreachable from `root` but not
    /// otherwise gone; this port leaves them in `keys`/`values`/`internals`
    /// similarly unreachable. Neither is observable through any public
    /// method.
    pub fn delete(&mut self, key: &[u8]) -> Option<V> {
        let mut pointer = self.root;

        let mut parent: Option<usize> = None;
        let mut grandparent: Option<usize> = None;
        let mut went_left_for_parent = false;
        let mut went_left_for_grandparent = false;

        loop {
            if pointer == EMPTY {
                return None;
            }

            if pointer > 0 {
                let internal_index = (pointer - 1) as usize;
                let node = &self.internals[internal_index];
                let go_right = get_direction(key, node.byte_index, node.mask);

                grandparent = parent;
                went_left_for_grandparent = went_left_for_parent;
                parent = Some(internal_index);
                went_left_for_parent = !go_right;

                pointer = if go_right { node.right } else { node.left };
            } else {
                let index = (-pointer - 1) as usize;

                if self.keys[index] != key {
                    return None;
                }

                self.size -= 1;

                match (parent, grandparent) {
                    (None, _) => {
                        self.root = EMPTY;
                    }
                    (Some(parent_index), None) => {
                        let node = &self.internals[parent_index];

                        self.root = if went_left_for_parent {
                            node.right
                        } else {
                            node.left
                        };
                    }
                    (Some(parent_index), Some(grandparent_index)) => {
                        let surviving = {
                            let node = &self.internals[parent_index];

                            if went_left_for_parent {
                                node.right
                            } else {
                                node.left
                            }
                        };

                        let slot = if went_left_for_grandparent {
                            &mut self.internals[grandparent_index].left
                        } else {
                            &mut self.internals[grandparent_index].right
                        };
                        *slot = surviving;
                    }
                }

                // The slot is now unreachable from `root` -- every pointer
                // that used to lead here was just rewired above -- so taking
                // the value and leaving `None` behind costs nothing further
                // and needs no `V: Default` bound.
                return self.values[index].take();
            }
        }
    }

    /// Upstream's `forEach`: an inorder walk over an explicit stack, kept in
    /// upstream's own iterative shape (see the module docs) so that the
    /// visiting order matches exactly.
    pub fn for_each<F: FnMut(&V, &[u8])>(&self, mut f: F) {
        let mut stack: Vec<Ptr> = Vec::new();
        let mut current = self.root;

        loop {
            if current != EMPTY {
                stack.push(current);

                current = if current > 0 {
                    self.internals[(current - 1) as usize].left
                } else {
                    EMPTY
                };
            } else if let Some(popped) = stack.pop() {
                current = popped;

                if current < 0 {
                    let index = (-current - 1) as usize;
                    let value = self.values[index]
                        .as_ref()
                        .expect("a reachable external node always holds a value");

                    f(value, &self.keys[index]);
                }

                current = if current > 0 {
                    self.internals[(current - 1) as usize].right
                } else {
                    EMPTY
                };
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    /// 1:1 port of `test/critbit-tree-map.js`'s "should be possible to set
    /// values." block.
    #[test]
    fn reproduces_the_upstream_set_suite() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        tree.set(key("abc"), 1);
        assert_eq!(tree.size(), 1);
        assert_eq!(tree.get(b"abc"), Some(&1));
        assert_eq!(tree.get(b"whatever"), None);

        tree.set(key("abc"), 2);
        assert_eq!(tree.size(), 1);
        assert_eq!(tree.get(b"abc"), Some(&2));

        tree.set(key("azb"), 2);
        tree.set(key("zzzzzzz"), 3);

        assert_eq!(tree.size(), 3);
        assert_eq!(tree.get(b"azb"), Some(&2));
        assert_eq!(tree.get(b"zzzzzzz"), Some(&3));
        assert_eq!(tree.get(b"zzzzzzzaaaa"), None);

        assert!(tree.has(b"abc"));
        assert!(!tree.has(b"whatever"));
    }

    /// 1:1 port of the "should be possible to delete elements." block.
    #[test]
    fn reproduces_the_upstream_delete_suite() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        tree.set(key("abc"), 1);
        assert_eq!(tree.delete(b"abc"), Some(1));
        assert_eq!(tree.size(), 0);
        assert_eq!(tree.delete(b"abc"), None);

        let data = ["abc", "def", "abgd", "zza", "idzzzudzzduuzduz"];

        for (i, k) in data.iter().enumerate() {
            tree.set(key(k), i as i32);
        }

        assert_eq!(tree.size(), data.len());

        for k in data.iter().rev() {
            tree.delete(k.as_bytes());
        }

        assert_eq!(tree.size(), 0);

        for k in data.iter() {
            assert_eq!(tree.delete(k.as_bytes()), None);
            assert!(!tree.has(k.as_bytes()));
        }
    }

    /// 1:1 port of "differences in string's lengths should not cause
    /// issues." — the prefix-relationship block.
    #[test]
    fn keys_that_differ_only_in_length_do_not_break() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        tree.set(key("abc"), 0);
        tree.set(key("zzz"), 0);
        tree.set(key("metastasis"), 1);
        tree.set(key("metastases"), 2);
        tree.set(key("meta"), 4);

        assert_eq!(tree.size(), 5);
        assert!(tree.has(b"metastases"));
        assert_eq!(tree.get(b"abc"), Some(&0));
    }

    /// 1:1 port of "should be possible to iterate over the tree." —
    /// `forEach` must visit in sorted key order.
    #[test]
    fn for_each_visits_in_sorted_key_order() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        let data = [("abc", 1), ("xyz", 2), ("Abc", 3), ("abcde", 4), ("bd", 5)];

        for &(k, v) in &data {
            tree.set(key(k), v);
        }

        let mut result: Vec<(String, i32)> = Vec::new();
        tree.for_each(|value, k| {
            result.push((String::from_utf8(k.to_vec()).unwrap(), *value));
        });

        let mut expected: Vec<(String, i32)> =
            data.iter().map(|&(k, v)| (k.to_string(), v)).collect();
        expected.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(result, expected);
    }

    /// Deep critical-bit positions: keys differing only in their last byte,
    /// sharing every byte up to that point — the gate 6 falsification
    /// target lives in exactly this path (`mask_for`/`get_direction`).
    #[test]
    fn keys_differing_only_in_the_last_byte_route_correctly() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        tree.set(key("abcda"), 1);
        tree.set(key("abcdb"), 2);
        tree.set(key("abcdc"), 3);

        assert_eq!(tree.get(b"abcda"), Some(&1));
        assert_eq!(tree.get(b"abcdb"), Some(&2));
        assert_eq!(tree.get(b"abcdc"), Some(&3));
        assert!(!tree.has(b"abcd"));
    }

    /// The tail-extension edge case: the byte immediately after a shared
    /// prefix is a literal `0x00` on one side, which drives `mask_for`
    /// through its `0xff` degenerate case (see the module docs).
    #[test]
    fn a_shared_prefix_followed_by_a_nul_byte_still_routes_correctly() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        tree.set(b"ab".to_vec(), 1);
        tree.set(vec![b'a', b'b', 0u8], 2);
        tree.set(vec![b'a', b'b', 1u8], 3);

        assert_eq!(tree.get(b"ab"), Some(&1));
        assert_eq!(tree.get(&[b'a', b'b', 0u8]), Some(&2));
        assert_eq!(tree.get(&[b'a', b'b', 1u8]), Some(&3));
    }

    #[test]
    fn clear_resets_size_and_removes_everything() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();
        tree.set(key("a"), 1);
        tree.set(key("b"), 2);

        tree.clear();

        assert_eq!(tree.size(), 0);
        assert!(!tree.has(b"a"));

        let mut seen = Vec::new();
        tree.for_each(|_, _| seen.push(()));
        assert!(seen.is_empty());
    }

    /// A port bug (not upstream's), caught by this module's own
    /// differential fuzzer within its first few generated operations,
    /// minimised to exactly this sequence: `set("a"); delete("a");
    /// set("a")`. `set`'s "tree is empty" fast path used to hardcode
    /// `root = external_ptr(0)`, which is only correct the very first time
    /// it runs -- upstream's own equivalent branch builds a brand new
    /// object and has no index at all to get wrong, but this port's
    /// append-only arena (see the module docs) had already pushed the
    /// first key at index 0 and left it there, orphaned, after the
    /// `delete`. The second `set` pushed its key at index 1 while `root`
    /// kept pointing at the stale, already-`take`n index 0, and
    /// `CritBitTreeMap::root`'s own "always holds a value" panic caught
    /// the mismatch immediately.
    #[test]
    fn setting_again_after_deleting_back_to_empty_does_not_point_root_at_a_stale_slot() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();

        tree.set(key("a"), 1);
        assert_eq!(tree.delete(b"a"), Some(1));
        assert_eq!(tree.size(), 0);

        assert_eq!(tree.set(key("a"), 2), None);
        assert_eq!(tree.size(), 1);
        assert_eq!(tree.get(b"a"), Some(&2));

        let mut seen = Vec::new();
        tree.for_each(|value, key| seen.push((key.to_vec(), *value)));
        assert_eq!(seen, vec![(b"a".to_vec(), 2)]);
    }

    #[test]
    fn a_deep_prefix_chain_is_fully_reachable() {
        let mut tree: CritBitTreeMap<i32> = CritBitTreeMap::new();
        let words = ["a", "ab", "abc", "abcd", "abcde", "abd"];

        for (i, w) in words.iter().enumerate() {
            tree.set(key(w), i as i32);
        }

        assert_eq!(tree.size(), words.len());

        for (i, w) in words.iter().enumerate() {
            assert_eq!(tree.get(w.as_bytes()), Some(&(i as i32)));
        }
    }
}
