//! Port of upstream `utils/comparators.js` (79 LOC).
//!
//! Four exports, and they are the whole of capability tier T2: every heap in
//! the library is parameterised by one of these, and `heap`, `fixed-reverse-heap`,
//! `fibonacci-heap`, `kd-tree` and `vp-tree` all reach for them.
//!
//! # A comparator is a *callback*, and that is the whole difficulty
//!
//! Upstream's comparator is an ordinary JavaScript function invoked from inside
//! a sift loop, once per comparison. Across the FFI boundary that makes it
//! re-entrant JavaScript running in the middle of a Rust operation, so three
//! properties have to be designed for rather than discovered:
//!
//! * **it can fail.** `compare` therefore returns `Result`, with the error type
//!   supplied by the caller rather than fixed here, so `mnemonist-core` never
//!   mentions `napi::Error` and a native caller never meets a `Result` it
//!   cannot construct;
//! * **it can mutate the very heap it is comparing.** Nothing in this file
//!   arranges that — [`crate::structures::heap`]'s [`Store`](crate::structures::heap::Store)
//!   does — but every signature here takes `&self` so that it *can*;
//! * **its answer is a JavaScript number, not an [`Ordering`](std::cmp::Ordering).**
//!   Upstream tests `< 0`, `> 0` and `>= 0` on whatever came back, so a
//!   comparator returning `NaN` makes all three false and a comparator
//!   returning `0.5` is "greater". Collapsing that to a three-valued `Ordering`
//!   would quietly repair inconsistent comparators, so the return type is `f64`.
//!
//! # `<` and `>` are language operators, not library logic
//!
//! `DEFAULT_COMPARATOR` is four lines of `if` around two relational operators.
//! The `if`s are ported here; the operators are [`Relational`], because JS `<`
//! on two arbitrary values runs `ToPrimitive`, which can call user code and can
//! throw. The bridge implements [`Relational`] for a JavaScript value; core
//! implements it for the Rust types it actually stores.

/// What a Rust caller gets when a ported algorithm throws.
///
/// `mnemonist-core` has no exceptions and no `napi::Error`, but upstream does
/// `throw new Error('mnemonist/heap.replace: …')` from the middle of an
/// algorithm. Core raises by *message*, through
/// [`Store::raise`](crate::structures::heap::Store::raise), and the caller
/// decides what a raised message is: the bridge makes it a JS exception, a Rust
/// caller gets this.
///
/// It is a method on the store rather than a trait on the error type because
/// the error type belongs to the bridge — `napi::Error` — and neither the trait
/// nor the type would be local to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thrown(pub &'static str);

impl std::fmt::Display for Thrown {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for Thrown {}

/// JavaScript's `<` and `>` on two values of the same Rust type.
///
/// Fallible because the JavaScript operators are: `a < b` runs `ToPrimitive` on
/// both operands, which invokes `valueOf`/`toString` and can throw, and a
/// `Symbol` operand throws outright. Core's own implementations never fail.
///
/// Deliberately **not** a blanket impl over [`PartialOrd`]: `Option<T>` is
/// `PartialOrd` when `T` is, and its ordering says `None < Some(_)`, whereas
/// JavaScript's says `undefined` compares false against everything. A blanket
/// impl would silently pick the wrong one for exactly the type the shrink
/// window produces.
pub trait Relational<E> {
    /// JavaScript's `self < other`.
    ///
    /// # Errors
    ///
    /// `E` whenever the JS operator would throw — a `Symbol` operand, or a
    /// `valueOf`/`toString` that throws. Core's own impls never return one.
    fn js_lt(&self, other: &Self) -> Result<bool, E>;

    /// JavaScript's `self > other`. Errors on the same conditions as
    /// [`js_lt`](Relational::js_lt); note that neither is the negation of the
    /// other, because `undefined` compares `false` both ways.
    fn js_gt(&self, other: &Self) -> Result<bool, E>;
}

macro_rules! relational_via_partial_ord {
    ($($type:ty),* $(,)?) => {
        $(
            impl<E> Relational<E> for $type {
                fn js_lt(&self, other: &Self) -> Result<bool, E> {
                    Ok(self < other)
                }

                fn js_gt(&self, other: &Self) -> Result<bool, E> {
                    Ok(self > other)
                }
            }
        )*
    };
}

// `f64` is included for the same reason the bridge needs it: every JS number is
// one, and `NaN` makes both operators false in Rust exactly as in JavaScript.
relational_via_partial_ord!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f64, String, char);

/// `undefined` compares false against everything, itself included.
///
/// This is the slot type of every [`Store`](crate::structures::heap::Store) core
/// supplies: `None` is a hole or an assigned `undefined`, and `ToNumber` of
/// either is `NaN`, so `undefined < x`, `undefined > x`, `x < undefined` and
/// `x > undefined` are all `false`. It is reachable through the public API
/// only when a comparator mutates the heap mid-sift, which is precisely the
/// case this tier exists to get right.
impl<E, T: Relational<E>> Relational<E> for Option<T> {
    fn js_lt(&self, other: &Self) -> Result<bool, E> {
        match (self, other) {
            (Some(left), Some(right)) => left.js_lt(right),
            _ => Ok(false),
        }
    }

    fn js_gt(&self, other: &Self) -> Result<bool, E> {
        match (self, other) {
            (Some(left), Some(right)) => left.js_gt(right),
            _ => Ok(false),
        }
    }
}

/// A value whose JavaScript reading is `undefined`.
///
/// Only [`sort_with`](crate::structures::heap::sort_with) needs it, because
/// `Array.prototype.sort` moves `undefined` to the end *without* consulting the
/// comparator — the one place in this unit where undefined-ness is not just
/// another value.
pub trait MaybeUndefined {
    /// Whether this value reads as `undefined` in JavaScript.
    fn is_undefined(&self) -> bool;
}

impl<T> MaybeUndefined for Option<T> {
    fn is_undefined(&self) -> bool {
        self.is_none()
    }
}

/// A slot type that may be able to hold JavaScript's `Infinity`.
///
/// `Heap.nsmallest`/`nlargest` open their `n === 1` fast paths with
/// `var min = Infinity` and then test `min === Infinity` to mean "nothing seen
/// yet". The sentinel is therefore a **real value in the domain**, and an
/// element that happens to be `Infinity` resets it — NOTES BUG-HEAP-3. Reproducing
/// that needs the slot type's own answer to "are you `Infinity`", which is what
/// this trait asks.
///
/// `infinity` returns [`None`] for a slot type that cannot represent it (an
/// integer store). Such a store cannot exhibit the bug either, so answering
/// "no" is not a papered-over divergence — it is the same statement one level
/// up.
pub trait Sentinel: Sized {
    /// `Infinity`, or `-Infinity` when `negative`, as a value of this slot
    /// type — or [`None`] if the type cannot represent it, which is also a
    /// type that cannot exhibit BUG-HEAP-3.
    fn infinity(negative: bool) -> Option<Self>;

    /// `value === Infinity`, or `value === -Infinity` when `negative`.
    fn is_infinity(&self, negative: bool) -> bool;
}

impl Sentinel for Option<f64> {
    fn infinity(negative: bool) -> Option<Self> {
        Some(Some(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }))
    }

    fn is_infinity(&self, negative: bool) -> bool {
        match self {
            Some(value) => {
                *value
                    == if negative {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }
            }
            None => false,
        }
    }
}

macro_rules! sentinel_unrepresentable {
    ($($type:ty),* $(,)?) => {
        $(
            impl Sentinel for Option<$type> {
                fn infinity(_negative: bool) -> Option<Self> {
                    None
                }

                fn is_infinity(&self, _negative: bool) -> bool {
                    false
                }
            }
        )*
    };
}

sentinel_unrepresentable!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, String, char);

/// A comparison callback: upstream's `function(a, b) { … }`, as a trait.
///
/// `E` is a type parameter rather than an associated type so that one
/// comparator can be used at whatever error type the store raises —
/// [`DefaultComparator`] is written once and serves both a native
/// [`Thrown`]-flavoured heap and the bridge's `napi::Error`-flavoured one.
pub trait Comparator<T: ?Sized, E> {
    /// Upstream returns *a number*, and the algorithms test its sign three
    /// different ways. See the module docs for why this is not an `Ordering`.
    fn compare(&self, a: &T, b: &T) -> Result<f64, E>;
}

/// `DEFAULT_COMPARATOR`, verbatim.
///
/// ```js
/// var DEFAULT_COMPARATOR = function(a, b) {
///   if (a < b) return -1;
///   if (a > b) return 1;
///   return 0;
/// };
/// ```
pub fn default_comparator<E, T: Relational<E> + ?Sized>(a: &T, b: &T) -> Result<f64, E> {
    if a.js_lt(b)? {
        return Ok(-1.0);
    }

    if a.js_gt(b)? {
        return Ok(1.0);
    }

    Ok(0.0)
}

/// `DEFAULT_REVERSE_COMPARATOR`, verbatim.
///
/// Note that upstream ships this as its own function rather than as
/// `reverseComparator(DEFAULT_COMPARATOR)`, and the two are **not** the same
/// function object even though they agree on every input. Nothing in this unit
/// uses it; it is ported because it is one of the file's four exports.
pub fn default_reverse_comparator<E, T: Relational<E> + ?Sized>(a: &T, b: &T) -> Result<f64, E> {
    if a.js_lt(b)? {
        return Ok(1.0);
    }

    if a.js_gt(b)? {
        return Ok(-1.0);
    }

    Ok(0.0)
}

/// The zero-sized comparator [`default_comparator`] belongs to.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultComparator;

impl<E, T: Relational<E> + ?Sized> Comparator<T, E> for DefaultComparator {
    fn compare(&self, a: &T, b: &T) -> Result<f64, E> {
        default_comparator(a, b)
    }
}

/// The zero-sized comparator [`default_reverse_comparator`] belongs to.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultReverseComparator;

impl<E, T: Relational<E> + ?Sized> Comparator<T, E> for DefaultReverseComparator {
    fn compare(&self, a: &T, b: &T) -> Result<f64, E> {
        default_reverse_comparator(a, b)
    }
}

/// `reverseComparator(comparator)` — the argument swap, as a wrapper type.
///
/// ```js
/// function reverseComparator(comparator) {
///   return function(a, b) { return comparator(b, a); };
/// }
/// ```
///
/// Note what it does **not** do: negate. `comparator(b, a)` and
/// `-comparator(a, b)` differ whenever the comparator is not antisymmetric —
/// one returning a constant `1`, say — and `MaxHeap` is built on this one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Reversed<C>(pub C);

impl<E, T: ?Sized, C: Comparator<T, E>> Comparator<T, E> for Reversed<C> {
    fn compare(&self, a: &T, b: &T) -> Result<f64, E> {
        self.0.compare(b, a)
    }
}

/// `reverseComparator`, spelled as a function for symmetry with upstream.
pub fn reverse_comparator<C>(comparator: C) -> Reversed<C> {
    Reversed(comparator)
}

/// A borrowed comparator is a comparator; the algorithms take `&C` throughout
/// and [`Reversed`] wraps whatever it is handed.
impl<E, T: ?Sized, C: Comparator<T, E> + ?Sized> Comparator<T, E> for &C {
    fn compare(&self, a: &T, b: &T) -> Result<f64, E> {
        (*self).compare(a, b)
    }
}

/// `createTupleComparator(size)` — lexicographic over the first `size` members.
///
/// Upstream unrolls the `size === 2` case into a separate closure. The unrolled
/// body is the loop body twice, with no behavioural difference, so this is one
/// implementation; the unrolling is a speed decision about a JIT that has no
/// counterpart here.
///
/// Reading past the end of a tuple yields `undefined` upstream, and
/// `undefined < undefined` is false both ways, so a short tuple simply
/// contributes no ordering. [`Vec::get`] reproduces that by returning `None`.
#[derive(Debug, Clone, Copy)]
pub struct TupleComparator {
    /// How many leading members participate in the comparison. Members past
    /// this index are ignored, and a tuple shorter than this contributes no
    /// ordering for its missing members.
    pub size: usize,
}

/// `createTupleComparator(size)`.
pub fn create_tuple_comparator(size: usize) -> TupleComparator {
    TupleComparator { size }
}

impl<E, T: Relational<E>> Comparator<Vec<T>, E> for TupleComparator {
    fn compare(&self, a: &Vec<T>, b: &Vec<T>) -> Result<f64, E> {
        let mut i = 0;

        while i < self.size {
            // `a[i]` past the end is `undefined`; `Option<T>`'s impl above is
            // the same "compares false against everything" rule.
            let left = a.get(i);
            let right = b.get(i);

            if left.js_lt(&right)? {
                return Ok(-1.0);
            }

            if left.js_gt(&right)? {
                return Ok(1.0);
            }

            i += 1;
        }

        Ok(0.0)
    }
}

/// The same lexicographic rule as the `Vec<T>` impl above, over a fixed-size
/// array instead of a heap-allocated one.
///
/// `kd-tree.rs`'s `k_nearest_neighbors`/`linear_k_nearest_neighbors` build a
/// fresh tuple for every node visited during a query — `[dist, visited,
/// pivot]` or `[dist, i]`, always exactly `N` long. Boxing each one as a
/// `Vec<f64>` would make the `Store::get`/`set` clone on every sift step a
/// fresh heap allocation; `[T; N]` is `Copy` for `T: Copy`, so the same clone
/// is a stack copy. Behaviourally this impl is
/// identical to the `Vec<T>` one: both tuples here are always exactly `N`
/// elements, matching the comparator's own `size`, so the "shorter than `N`"
/// case the `Vec` impl's doc comment calls out is never reached by either.
/// `[T; N]::get` auto-derefs to the slice method and answers `None` past the
/// end the same way `Vec::get` does, so nothing about the "past the end is
/// `undefined`" behaviour changes either.
impl<E, T: Relational<E>, const N: usize> Comparator<[T; N], E> for TupleComparator {
    fn compare(&self, a: &[T; N], b: &[T; N]) -> Result<f64, E> {
        let mut i = 0;

        while i < self.size {
            let left = a.get(i);
            let right = b.get(i);

            if left.js_lt(&right)? {
                return Ok(-1.0);
            }

            if left.js_gt(&right)? {
                return Ok(1.0);
            }

            i += 1;
        }

        Ok(0.0)
    }
}

/// `Option<&T>` needs the same rule as `Option<T>`; a blanket impl over
/// references would collide with the macro above.
impl<E, T: Relational<E>> Relational<E> for &T {
    fn js_lt(&self, other: &Self) -> Result<bool, E> {
        (*self).js_lt(other)
    }

    fn js_gt(&self, other: &Self) -> Result<bool, E> {
        (*self).js_gt(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The error type never produced by core's own comparators.
    type E = Thrown;

    #[test]
    fn default_comparator_is_minus_one_zero_one() {
        assert_eq!(default_comparator::<E, i64>(&1, &2), Ok(-1.0));
        assert_eq!(default_comparator::<E, i64>(&2, &1), Ok(1.0));
        assert_eq!(default_comparator::<E, i64>(&2, &2), Ok(0.0));
    }

    #[test]
    fn default_reverse_comparator_is_the_mirror() {
        assert_eq!(default_reverse_comparator::<E, i64>(&1, &2), Ok(1.0));
        assert_eq!(default_reverse_comparator::<E, i64>(&2, &1), Ok(-1.0));
        assert_eq!(default_reverse_comparator::<E, i64>(&2, &2), Ok(0.0));
    }

    /// Upstream ships `DEFAULT_REVERSE_COMPARATOR` *and*
    /// `reverseComparator(DEFAULT_COMPARATOR)`; they agree pointwise.
    #[test]
    fn the_two_reverses_agree_pointwise() {
        let reversed = Reversed(DefaultComparator);

        for a in -3i64..3 {
            for b in -3i64..3 {
                assert_eq!(
                    Comparator::<i64, E>::compare(&reversed, &a, &b),
                    default_reverse_comparator::<E, i64>(&a, &b)
                );
            }
        }
    }

    /// `reverseComparator` swaps arguments, it does not negate — and for a
    /// comparator that is not antisymmetric those differ.
    #[test]
    fn reverse_swaps_arguments_rather_than_negating() {
        struct AlwaysOne;

        impl<E> Comparator<i64, E> for AlwaysOne {
            fn compare(&self, _a: &i64, _b: &i64) -> Result<f64, E> {
                Ok(1.0)
            }
        }

        let reversed = Reversed(AlwaysOne);

        assert_eq!(Comparator::<i64, E>::compare(&reversed, &1, &2), Ok(1.0));
        assert_eq!(Comparator::<i64, E>::compare(&AlwaysOne, &1, &2), Ok(1.0));
    }

    /// `NaN` makes every relational operator false in both languages, so the
    /// default comparator reports "equal" for values that are not.
    #[test]
    fn nan_compares_equal_to_everything() {
        assert_eq!(default_comparator::<E, f64>(&f64::NAN, &1.0), Ok(0.0));
        assert_eq!(default_comparator::<E, f64>(&1.0, &f64::NAN), Ok(0.0));
        assert_eq!(default_comparator::<E, f64>(&f64::NAN, &f64::NAN), Ok(0.0));
    }

    /// `undefined` is not "less than everything" the way `Option`'s derived
    /// ordering would have it.
    #[test]
    fn undefined_compares_equal_to_everything() {
        let undefined: Option<i64> = None;

        assert_eq!(default_comparator::<E, _>(&undefined, &Some(1)), Ok(0.0));
        assert_eq!(default_comparator::<E, _>(&Some(1), &undefined), Ok(0.0));
        assert_eq!(default_comparator::<E, _>(&undefined, &undefined), Ok(0.0));
        // …whereas Rust's own ordering says None < Some, which is the trap.
        assert!(undefined < Some(1));
    }

    #[test]
    fn tuple_comparator_is_lexicographic() {
        let comparator = create_tuple_comparator(2);

        assert_eq!(
            Comparator::<Vec<i64>, E>::compare(&comparator, &vec![1, 9], &vec![2, 0]),
            Ok(-1.0)
        );
        assert_eq!(
            Comparator::<Vec<i64>, E>::compare(&comparator, &vec![2, 0], &vec![2, 1]),
            Ok(-1.0)
        );
        assert_eq!(
            Comparator::<Vec<i64>, E>::compare(&comparator, &vec![2, 1], &vec![2, 1]),
            Ok(0.0)
        );
    }

    /// A tuple shorter than `size` contributes no ordering past its end,
    /// because `a[i]` is `undefined` there.
    #[test]
    fn tuple_comparator_reads_past_a_short_tuple_as_undefined() {
        let comparator = create_tuple_comparator(3);

        assert_eq!(
            Comparator::<Vec<i64>, E>::compare(&comparator, &vec![1], &vec![1, 2]),
            Ok(0.0)
        );
    }

    #[test]
    fn a_thrown_message_survives_display() {
        assert_eq!(Thrown("boom").to_string(), "boom");
    }
}
