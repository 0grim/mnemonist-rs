//! Port of upstream `fixed-critbit-tree-map.js` (mnemonist v0.40.4).
//!
//! The same crit-bit tree as [`crate::structures::critbit_tree_map`], over
//! pre-allocated, fixed-size storage instead of growable arenas — mirroring
//! upstream's own independent (not code-shared) file, which re-derives the
//! whole algorithm against typed arrays rather than importing anything from
//! `critbit-tree-map.js`. Two upstream properties make the fixed variant
//! genuinely different from "the same tree with a capacity check bolted on",
//! and both are measured, not assumed — see NOTES.md B-260/B-261.
//!
//! # 1. There is no capacity guard at all
//!
//! `this.capacity`/`this.size` exist, but nothing in `set` ever compares
//! them. The constructor's own comment says so: `// TODO: yell if capacity
//! is already full!`. Two of the five backing arrays are genuinely
//! fixed-size JavaScript typed arrays (`this.lefts`/`this.rights`, each
//! `capacity - 1` slots — the maximum number of internal nodes a tree of
//! `capacity` leaves can ever need) and a third almost is
//! (`this.critbits`, `capacity` slots — one more than `lefts`/`rights`, an
//! upstream over-allocation that turns out to matter, below); `this.keys`/
//! `this.values` are plain `Array`s, which grow without bound. So inserting
//! past `capacity` distinct keys does not fail cleanly: the two counts
//! drift apart, in a specific and reproducible way.
//!
//! Verified against real Node 24.18.1: a capacity-4 tree accepts a 5th
//! distinct key (`size` becomes 5) with **no error at all** — the new
//! internal node's own `this.lefts`/`this.rights` write lands at index 3,
//! past the 3-slot typed array, which JavaScript silently drops (an
//! out-of-range typed-array write is a no-op, not a growth, and not a
//! throw). That corrupts exactly that node: `get`/`has` for the two keys
//! reachable only through it silently return `undefined`/`false` — a
//! wrong-but-quiet answer, indistinguishable from "not found" — while every
//! other key remains perfectly readable. The **6th** distinct key is what
//! actually throws: its insertion walks *through* the corrupted node,
//! reads `this.lefts`/`this.rights` at an out-of-range index (JavaScript
//! typed-array reads past the end are `undefined`, not the class zero),
//! and upstream's own walk has no defence against a non-numeric pointer:
//!
//! ```text
//! TypeError: Cannot read properties of undefined (reading 'length')
//!     at findCriticalBit (fixed-critbit-tree-map.js:55:20)
//!     at FixedCritBitTreeMap.set (fixed-critbit-tree-map.js:199:17)
//! ```
//!
//! [`BoundedSlots`] models exactly this: a fixed-length slot store whose
//! writes past the end are dropped and whose reads past the end come back
//! [`None`] — JavaScript's `undefined` — rather than the in-bounds zero,
//! [`Some(0)`]. The distinction is load-bearing: an in-bounds `0` is a
//! genuinely empty child slot (attach a new leaf here); an out-of-bounds
//! read is the corruption above. [`Error::Corrupted`] surfaces the crash
//! with upstream's own message text, so a caller two layers up (the napi
//! bridge, or the differential fuzzer) sees exactly upstream's `TypeError`
//! rather than a divergent one — a `panic!` would take the whole process
//! down at the FFI boundary, which upstream's own crash does not do.
//!
//! `this.critbits`'s extra slot (`capacity`, not `capacity - 1`) means an
//! internal node's *own* critical bit can still be read correctly one
//! generation after `this.lefts`/`this.rights` have already started
//! silently dropping that same node's children — the direction computed at
//! a corrupted node is real, only its children are gone. This port keeps
//! that exact asymmetry (`critbits: BoundedSlots` sized `capacity`,
//! `lefts`/`rights` sized `capacity - 1`) rather than rounding it off to one
//! shared bound, because [`Error::Corrupted`] is only reachable in exactly
//! the order upstream reaches it if the two stay different sizes.
//!
//! # 2. `set`'s "attach to an existing internal node" branch writes to the
//! wrong slot — and is unreachable anyway
//!
//! ```js
//! if (newPointer === 0) {
//!   pointer = this.size++;
//!   leftOrRight[newPointer] = -(pointer + 1);   // BUG: should be leftOrRight[<the
//!   this.keys[pointer] = key;                   //      internal node index that
//!   this.values[pointer] = value;               //      was in `pointer` before
//!   return this;                                //      this line reused it>
//! }
//! ```
//! `newPointer` is the value the branch's own condition just tested — always
//! `0` — so this always writes to `lefts[0]`/`rights[0]` rather than to the
//! internal node actually being visited, unless that node's own index
//! happens to be `0`. Measured rather than assumed: every internal node's
//! *own* creation (the "reaching an external node" branch) writes both of
//! its children unconditionally, so no internal node's child slot is ever
//! genuinely `0` — a bare zero-read here always means "out of bounds"
//! ([`Error::Corrupted`], above), never "empty and legitimately attachable".
//! 20,000 fuzzed operations over a shared-prefix pool (see
//! `crates/difffuzz/src/modules/fixed_critbit_tree_map.rs`) hit this branch
//! zero times, matching the identical measurement already made for
//! `critbit_tree_map`'s own dead `!node.left`/`!node.right` checks. This
//! port still contains the branch — deleting unreachable upstream code
//! would be a structural divergence with no upside — and reproduces the
//! exact wrong write target (`self.lefts.set(0, ...)`, not
//! `self.lefts.set(internal_index, ...)`) in case a future, wider campaign
//! ever does reach it. See NOTES.md B-262.
//!
//! # No `delete`
//!
//! `fixed-critbit-tree-map.js` has no `delete` method at all — not
//! unimplemented, simply absent, and `test/fixed-critbit-tree-map.js` even
//! comments its own "should be possible to delete elements." block out
//! rather than asserting anything. This port has no `delete` either.

use std::fmt;

/// `typeof capacity !== 'number' || capacity <= 0`, verbatim.
pub const BAD_CAPACITY: &str =
    "mnemonist/fixed-critbit-tree-map: `capacity` should be a positive number.";

/// What V8 says reading `.length` off `undefined` — verbatim, so a caller
/// two layers up sees the same text upstream's own crash produces. See the
/// module docs, part 1.
pub const CORRUPTED_MESSAGE: &str = "Cannot read properties of undefined (reading 'length')";

/// What upstream throws, or — for `Corrupted` — the point at which upstream
/// crashes instead of throwing cleanly. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `capacity <= 0`. The non-numeric half of upstream's check is a
    /// JavaScript-only notion, checked at the bridge.
    Capacity,
    /// More than `capacity` distinct keys were inserted, and this
    /// operation's walk passed through the resulting corrupted node. See
    /// the module docs, part 1.
    Corrupted,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity => formatter.write_str(BAD_CAPACITY),
            Self::Corrupted => formatter.write_str(CORRUPTED_MESSAGE),
        }
    }
}

impl std::error::Error for Error {}

/// A pointer/slot reference, encoded exactly as
/// [`crate::structures::critbit_tree_map`]'s `Ptr`: `0` empty, positive an
/// internal node, negative an external one.
type Ptr = i64;

const EMPTY: Ptr = 0;

fn internal_ptr(index: usize) -> Ptr {
    index as Ptr + 1
}

fn external_ptr(index: usize) -> Ptr {
    -(index as Ptr) - 1
}

/// Upstream's `utils/bitwise.js#msb8` — see `critbit_tree_map`'s copy for
/// the reasoning; kept duplicated rather than shared because the two files
/// are independent upstream too.
fn msb8(x: u8) -> u8 {
    let mut x = x;
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x & !(x >> 1)
}

/// Upstream's `fixed-critbit-tree-map.js#findCriticalBit`'s mask: the
/// **direct** critical bit, not inverted — the one place this file's
/// bitwise convention differs from `critbit-tree-map.js`'s
/// `criticalBit8Mask`. `getDirection` here is `byte & mask`, so this is
/// deliberately not the same helper as the unbounded variant's `mask_for`.
fn mask_for(a: u8, b: u8) -> u8 {
    msb8(a ^ b)
}

/// Upstream's `getDirection`: `byte & mask`, compared only ever to zero
/// (`dir === 0 ? left : right`) — so this returns the "went right" boolean
/// directly rather than the raw nonzero value.
///
/// A key shorter than `byte_index` always routes left, matching
/// `critbit_tree_map::get_direction` and upstream's identical
/// `if (byteIndex > key.length - 1) return 0;` guard in both files.
fn get_direction(key: &[u8], byte_index: usize, mask: u8) -> bool {
    match key.get(byte_index) {
        None => false,
        Some(&byte) => (byte & mask) != 0,
    }
}

/// The byte index at which `a` and `b` first differ (or the tail-extension
/// position, comparing against an implicit `0`), and the two real byte
/// values there. Identical in shape to `critbit_tree_map::diverging_byte`;
/// duplicated for the same reason `mask_for` is not shared.
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

/// A fixed-length slot store mirroring one of upstream's typed arrays:
/// [`set`](BoundedSlots::set) past the end is a silent no-op (never grows,
/// never panics) and [`get`](BoundedSlots::get) past the end is [`None`] —
/// JavaScript's `undefined` — kept distinct from [`Some`] of the type's own
/// zero, which is what an in-bounds, never-written slot reads as. See the
/// module docs, part 1, for why that distinction is the entire mechanism
/// behind [`Error::Corrupted`].
#[derive(Debug, Clone)]
struct BoundedSlots<T> {
    slots: Vec<T>,
}

impl<T: Copy + Default> BoundedSlots<T> {
    fn new(len: usize) -> Self {
        Self {
            slots: vec![T::default(); len],
        }
    }

    fn get(&self, index: usize) -> Option<T> {
        self.slots.get(index).copied()
    }

    fn set(&mut self, index: usize, value: T) {
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = value;
        }
        // Out of range: silently dropped, matching a write past the end of
        // a real typed array.
    }
}

/// A crit-bit tree map of fixed capacity, over byte-string keys.
///
/// See the module docs for the two upstream properties that make this
/// genuinely different from `critbit_tree_map`, not just a bounded version
/// of it.
#[derive(Debug, Clone)]
pub struct FixedCritBitTreeMap<V> {
    capacity: usize,
    keys: Vec<Vec<u8>>,
    values: Vec<V>,
    /// `this.critbits`: `capacity` slots — one more than `lefts`/`rights`.
    /// See the module docs, part 1.
    critbits: BoundedSlots<(usize, u8)>,
    /// `this.lefts`: `capacity - 1` slots.
    lefts: BoundedSlots<Ptr>,
    /// `this.rights`: `capacity - 1` slots.
    rights: BoundedSlots<Ptr>,
    /// `this.offset`: the next internal node index, an unbounded counter —
    /// upstream's own `this.offset++`, never compared against anything.
    next_internal: usize,
    root: Ptr,
}

impl<V> FixedCritBitTreeMap<V> {
    /// `new FixedCritBitTreeMap(capacity)`.
    pub fn new(capacity: usize) -> Result<Self, Error> {
        if capacity == 0 {
            return Err(Error::Capacity);
        }

        Ok(Self {
            capacity,
            keys: Vec::new(),
            values: Vec::new(),
            critbits: BoundedSlots::new(capacity),
            lefts: BoundedSlots::new(capacity - 1),
            rights: BoundedSlots::new(capacity - 1),
            next_internal: 0,
            root: EMPTY,
        })
    }

    /// `this.capacity`.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// `this.size` — upstream's own `this.size++` is literally
    /// `this.keys`'s next free index, so this is that count directly.
    pub fn size(&self) -> usize {
        self.keys.len()
    }

    /// Upstream's `clear`. The TODO in the real source
    /// (`// TODO...` — the constructor's arrays are never reallocated) means
    /// this, like `FixedStack::clear`, resets the bookkeeping fields only;
    /// `keys`/`values`/`lefts`/`rights`/`critbits` are left exactly as they
    /// were, unreachable from the fresh empty `root` but not wiped.
    pub fn clear(&mut self) {
        self.root = EMPTY;
        // Upstream's own `clear` does not reset `size` via a separate
        // counter (there is none — see `size`'s doc comment) — it is
        // `this.keys.length`, but upstream's `clear` never truncates
        // `this.keys` either. Reproduced here by simply not touching
        // `self.keys`/`self.values`: a `clear`-then-`set` would keep
        // growing them exactly as upstream's own arrays do, not restart
        // from index 0. This is the one place `size()` and "number of
        // *live* entries" observably part ways, matching upstream.
    }

    /// Every stored value, mutably. Exists for the bridge, matching
    /// `critbit_tree_map::CritBitTreeMap::values_mut`'s reasoning — a value
    /// that holds a JS reference must have it released before it is
    /// dropped. There is no `delete` on this structure (see the module
    /// docs), so every stored value is always live: no filtering is needed
    /// the way the unbounded variant's `Option<V>` slots need.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.values.iter_mut()
    }

    /// Upstream's `set`. `Ok(Some(old))` replaces an existing key's value;
    /// `Ok(None)` is a fresh insertion; `Err(Error::Corrupted)` is upstream's
    /// own crash once more than `capacity` distinct keys have been inserted
    /// and this call's walk passes through the resulting corrupted node —
    /// see the module docs, part 1.
    pub fn set(&mut self, key: impl Into<Vec<u8>>, value: V) -> Result<Option<V>, Error> {
        let key = key.into();

        if self.keys.is_empty() {
            self.keys.push(key);
            self.values.push(value);
            self.root = external_ptr(0);

            return Ok(None);
        }

        let mut pointer = self.root;
        let mut ancestors: Vec<usize> = Vec::new();
        let mut path: Vec<bool> = Vec::new();

        loop {
            if pointer > 0 {
                let internal_index = (pointer - 1) as usize;

                let Some((byte_index, mask)) = self.critbits.get(internal_index) else {
                    // Reading past `critbits` requires an index that has
                    // already read past the (smaller) `lefts`/`rights`
                    // bound too -- see the module docs -- so this is
                    // reached only via the identical crash below in
                    // practice, kept as its own branch for a store whose
                    // bound genuinely differs from `lefts`/`rights`.
                    return Err(Error::Corrupted);
                };

                let go_right = get_direction(&key, byte_index, mask);
                let slots = if go_right { &self.rights } else { &self.lefts };

                let child = match slots.get(internal_index) {
                    Some(child) => child,
                    None => return Err(Error::Corrupted),
                };

                if child == EMPTY {
                    // Upstream's own bugged branch: writes to slot `0`, not
                    // to `internal_index` -- see the module docs, part 2.
                    // Measured unreachable, kept for fidelity regardless.
                    let new_external = self.keys.len();

                    self.keys.push(key);
                    self.values.push(value);

                    let slots = if go_right {
                        &mut self.rights
                    } else {
                        &mut self.lefts
                    };
                    slots.set(0, external_ptr(new_external));

                    return Ok(None);
                }

                ancestors.push(internal_index);
                path.push(!go_right);
                pointer = child;
            } else {
                let external_index = (-pointer - 1) as usize;

                let Some((byte_index, a_byte, b_byte)) =
                    diverging_byte(&key, &self.keys[external_index])
                else {
                    return Ok(Some(std::mem::replace(
                        &mut self.values[external_index],
                        value,
                    )));
                };

                let mask = mask_for(a_byte, b_byte);
                let new_goes_left = !get_direction(&key, byte_index, mask);

                let new_external = self.keys.len();
                self.keys.push(key);
                self.values.push(value);

                let internal_index = self.next_internal;
                self.next_internal += 1;

                self.critbits.set(internal_index, (byte_index, mask));

                let (left, right) = if new_goes_left {
                    (external_ptr(new_external), pointer)
                } else {
                    (pointer, external_ptr(new_external))
                };
                self.lefts.set(internal_index, left);
                self.rights.set(internal_index, right);

                let mut best: Option<usize> = None;

                for i in (0..ancestors.len()).rev() {
                    let Some((ancestor_byte, ancestor_mask)) = self.critbits.get(ancestors[i])
                    else {
                        // An ancestor's own critbit is unreadable: cannot
                        // happen in practice (see the module docs, part 1 --
                        // every ancestor in this list already had a
                        // successful child read to get here), kept as a
                        // conservative stop rather than an unreachable!().
                        break;
                    };

                    // NOT a plain tuple comparison: upstream's own tie-break
                    // (`this.critbits[ancestor] & 0xff) < (critbit & 0xff)`)
                    // skips when the ANCESTOR's mask is SMALLER at equal
                    // byte_index -- the opposite direction from
                    // `critbit_tree_map`'s inverted-mask convention, because
                    // this file's `mask_for` is the direct bit value (larger
                    // mask = more significant bit), not its complement. A
                    // `>` tuple compare here would silently swap this
                    // tie-break and corrupt the tree exactly at capacity,
                    // with no error to notice it by -- ask how this was
                    // found before trusting it again.
                    let skip = match ancestor_byte.cmp(&byte_index) {
                        std::cmp::Ordering::Greater => true,
                        std::cmp::Ordering::Equal => ancestor_mask < mask,
                        std::cmp::Ordering::Less => false,
                    };

                    if skip {
                        continue;
                    }

                    best = Some(i);
                    break;
                }

                match best {
                    None => {
                        self.root = internal_ptr(internal_index);

                        if let Some(&parent) = ancestors.first() {
                            let slots = if new_goes_left {
                                &mut self.rights
                            } else {
                                &mut self.lefts
                            };
                            slots.set(internal_index, internal_ptr(parent));
                        }
                    }
                    Some(best) if best == ancestors.len() - 1 => {
                        let parent = ancestors[best];
                        let went_left = path[best];
                        let slots = if went_left {
                            &mut self.lefts
                        } else {
                            &mut self.rights
                        };
                        slots.set(parent, internal_ptr(internal_index));
                    }
                    Some(best) => {
                        let parent = ancestors[best];
                        let went_left = path[best];
                        let child = ancestors[best + 1];

                        let parent_slots = if went_left {
                            &mut self.lefts
                        } else {
                            &mut self.rights
                        };
                        parent_slots.set(parent, internal_ptr(internal_index));

                        let new_slots = if new_goes_left {
                            &mut self.rights
                        } else {
                            &mut self.lefts
                        };
                        new_slots.set(internal_index, internal_ptr(child));
                    }
                }

                return Ok(None);
            }
        }
    }

    /// Upstream's `get`. Silently returns `None` — never
    /// [`Error::Corrupted`] — for a key whose path runs through a corrupted
    /// node: upstream's own `get` never calls `findCriticalBit`, so it never
    /// throws there either. See the module docs, part 1.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        let mut pointer = self.root;

        loop {
            if pointer == EMPTY {
                return None;
            }

            if pointer > 0 {
                let internal_index = (pointer - 1) as usize;
                let (byte_index, mask) = self.critbits.get(internal_index)?;
                let go_right = get_direction(key, byte_index, mask);
                let slots = if go_right { &self.rights } else { &self.lefts };

                pointer = slots.get(internal_index)?;
            } else {
                let index = (-pointer - 1) as usize;

                if self.keys[index] != key {
                    return None;
                }

                return Some(&self.values[index]);
            }
        }
    }

    /// Upstream's `has`.
    pub fn has(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    /// Upstream's `forEach`, over the arena in `root`-driven inorder —
    /// identical shape to `critbit_tree_map::for_each`.
    ///
    /// A `lefts`/`rights` read past the fixed bound reads back as
    /// upstream's own `undefined`, which this walk's `current !== 0` /
    /// `current < 0` / `current > 0` checks all treat as false — pushed
    /// once, then popped with no callback and no further descent. That is
    /// observationally identical to never pushing it at all, so reads here
    /// fold `None` straight to [`EMPTY`] rather than threading a separate
    /// corrupted state through — unlike `set`, which must not, because
    /// `set`'s crash is observable (`Error::Corrupted`) and `forEach`'s
    /// silent skip is not.
    pub fn for_each<F: FnMut(&V, &[u8])>(&self, mut f: F) {
        let mut stack: Vec<Ptr> = Vec::new();
        let mut current = self.root;

        loop {
            if current != EMPTY {
                stack.push(current);

                current = if current > 0 {
                    self.lefts.get((current - 1) as usize).unwrap_or(EMPTY)
                } else {
                    EMPTY
                };
            } else if let Some(popped) = stack.pop() {
                current = popped;

                if current < 0 {
                    let index = (-current - 1) as usize;

                    f(&self.values[index], &self.keys[index]);
                }

                current = if current > 0 {
                    self.rights.get((current - 1) as usize).unwrap_or(EMPTY)
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

    /// `test/fixed-critbit-tree-map.js`'s "should throw if given bad
    /// arguments." block (the numeric half; the non-numeric half is a
    /// JavaScript-only notion, checked at the bridge).
    #[test]
    fn rejects_a_zero_capacity() {
        assert_eq!(
            FixedCritBitTreeMap::<i32>::new(0).unwrap_err(),
            Error::Capacity
        );
        assert_eq!(
            FixedCritBitTreeMap::<i32>::new(0).unwrap_err().to_string(),
            BAD_CAPACITY
        );
    }

    /// 1:1 port of "should be possible to set values." (capacity 3).
    #[test]
    fn reproduces_the_upstream_set_suite() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(3).unwrap();

        tree.set(key("abc"), 1).unwrap();
        assert_eq!(tree.size(), 1);
        assert_eq!(tree.get(b"abc"), Some(&1));
        assert_eq!(tree.get(b"whatever"), None);

        tree.set(key("abc"), 2).unwrap();
        assert_eq!(tree.size(), 1);
        assert_eq!(tree.get(b"abc"), Some(&2));

        tree.set(key("azb"), 2).unwrap();
        tree.set(key("zzzzzzz"), 3).unwrap();

        assert_eq!(tree.size(), 3);
        assert_eq!(tree.get(b"azb"), Some(&2));
        assert_eq!(tree.get(b"zzzzzzz"), Some(&3));
        assert_eq!(tree.get(b"zzzzzzzaaaa"), None);

        assert!(tree.has(b"abc"));
        assert!(!tree.has(b"whatever"));
    }

    /// 1:1 port of "differences in string's lengths should not cause
    /// issues." (capacity 5).
    #[test]
    fn keys_that_differ_only_in_length_do_not_break() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(5).unwrap();

        tree.set(key("abc"), 0).unwrap();
        tree.set(key("zzz"), 0).unwrap();
        tree.set(key("metastasis"), 1).unwrap();
        tree.set(key("metastases"), 2).unwrap();
        tree.set(key("meta"), 4).unwrap();

        assert_eq!(tree.size(), 5);
        assert!(tree.has(b"metastases"));
        assert_eq!(tree.get(b"abc"), Some(&0));
    }

    /// 1:1 port of "should be possible to iterate over the tree." (capacity
    /// 5, exactly full — the shape the original suite's `forEach` block
    /// checks, and the one shape where nothing here is corrupted yet).
    #[test]
    fn for_each_visits_in_sorted_key_order_when_exactly_at_capacity() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(5).unwrap();

        let data = [("abc", 1), ("xyz", 2), ("Abc", 3), ("abcde", 4), ("bd", 5)];

        for &(k, v) in &data {
            tree.set(key(k), v).unwrap();
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

    /// The capacity-exceeding sequence measured against real Node 24.18.1
    /// (module docs, part 1): a capacity-4 tree accepts a 5th distinct key
    /// silently (with two of its keys becoming permanently unreachable),
    /// then crashes on the 6th.
    #[test]
    fn exceeding_capacity_silently_corrupts_then_crashes_exactly_as_upstream_does() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(4).unwrap();

        for (i, k) in ["a", "ab", "abc", "abcd"].iter().enumerate() {
            assert!(tree.set(key(k), i as i32).is_ok());
        }

        // The 5th distinct key: still `Ok`, capacity has no guard.
        assert_eq!(tree.set(key("abcde"), 4), Ok(None));
        assert_eq!(tree.size(), 5);

        // Corrupted, but silently: `get` on the two affected keys returns
        // `None`, indistinguishable from "never inserted" -- not an error.
        assert_eq!(tree.get(b"abcd"), None);
        assert_eq!(tree.get(b"abcde"), None);
        // ...while every key inserted before the overflow is untouched.
        assert_eq!(tree.get(b"a"), Some(&0));
        assert_eq!(tree.get(b"ab"), Some(&1));
        assert_eq!(tree.get(b"abc"), Some(&2));

        // The 6th distinct key walks through the corrupted node and hits
        // upstream's own crash -- surfaced as `Err`, not a Rust panic.
        assert_eq!(tree.set(key("abcdef"), 5), Err(Error::Corrupted));
        assert_eq!(
            tree.set(key("abcdef"), 5).unwrap_err().to_string(),
            CORRUPTED_MESSAGE
        );
    }

    /// The degenerate case: capacity 1 has zero `lefts`/`rights` slots at
    /// all, so the second distinct key corrupts the tree's only internal
    /// node before anything is ever read back.
    #[test]
    fn a_capacity_of_one_corrupts_on_the_second_key() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(1).unwrap();

        assert_eq!(tree.set(key("a"), 1), Ok(None));
        assert_eq!(tree.set(key("b"), 2), Ok(None));

        assert_eq!(tree.size(), 2);
        assert_eq!(tree.get(b"a"), None);
        assert_eq!(tree.get(b"b"), None);
    }

    #[test]
    fn clear_empties_the_tree_but_does_not_shrink_the_backing_arrays() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(4).unwrap();
        tree.set(key("a"), 1).unwrap();
        tree.set(key("b"), 2).unwrap();

        tree.clear();

        assert!(!tree.has(b"a"));

        let mut seen = Vec::new();
        tree.for_each(|_, _| seen.push(()));
        assert!(seen.is_empty());
    }

    /// Deep critical-bit positions within capacity: the same shape as
    /// `critbit_tree_map`'s equivalent test, confirming the direct-mask
    /// convention (`byte & mask`) routes identically to the inverted one
    /// when nothing has overflowed.
    #[test]
    fn keys_differing_only_in_the_last_byte_route_correctly_within_capacity() {
        let mut tree: FixedCritBitTreeMap<i32> = FixedCritBitTreeMap::new(3).unwrap();

        tree.set(key("abcda"), 1).unwrap();
        tree.set(key("abcdb"), 2).unwrap();
        tree.set(key("abcdc"), 3).unwrap();

        assert_eq!(tree.get(b"abcda"), Some(&1));
        assert_eq!(tree.get(b"abcdb"), Some(&2));
        assert_eq!(tree.get(b"abcdc"), Some(&3));
    }
}
