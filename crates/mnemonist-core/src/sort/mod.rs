//! Ports of the `sort/` helpers.
//!
//! These are the first ported *functions* rather than a structure: nothing
//! here owns state, and there is no instance for a bridge class to hold. Both
//! files sort a caller-supplied slice in place, and both come in two flavours
//! — one that permutes the values, one that permutes a separate index array
//! and leaves the values alone.
//!
//! # The `Option` comparisons, and why the indices variants need them
//!
//! The value flavours only ever touch `array[lo..hi]`, which the caller has
//! already promised is in range. The *indices* flavours dereference twice —
//! `array[indices[j]]` — and the inner value is an arbitrary number the caller
//! supplied. Upstream reads past the end of `array` there and gets
//! `undefined`, and every relational comparison against `undefined` is `false`
//! because `ToNumber(undefined)` is `NaN`.
//!
//! So the indices variants compare `Option<&T>` through `gt`, `ge` and
//! `le` below, each of which is false whenever either side is absent. That
//! is not defensive programming; it is the comparison upstream performs, and a
//! port that panicked on the out-of-range read instead would refuse inputs
//! upstream accepts.

pub mod insertion;
pub mod quick;

/// JavaScript `a > b`, with an absent operand standing for `undefined`.
///
/// `undefined` relationally compares as `NaN`, so every comparison involving
/// one is false — including `undefined > undefined`.
pub(crate) fn gt<T: PartialOrd>(left: Option<&T>, right: Option<&T>) -> bool {
    matches!((left, right), (Some(a), Some(b)) if a > b)
}

/// JavaScript `a >= b`, with an absent operand standing for `undefined`.
pub(crate) fn ge<T: PartialOrd>(left: Option<&T>, right: Option<&T>) -> bool {
    matches!((left, right), (Some(a), Some(b)) if a >= b)
}

/// JavaScript `a <= b`, with an absent operand standing for `undefined`.
pub(crate) fn le<T: PartialOrd>(left: Option<&T>, right: Option<&T>) -> bool {
    matches!((left, right), (Some(a), Some(b)) if a <= b)
}

/// What every sort here asserts about its window before it starts.
///
/// Upstream asserts nothing and walks off the end, reading `undefined` and
/// writing into holes. There is no honest Rust reproduction of a JS array hole
/// — see [`crate::utils::typed_arrays::PointerVec::get`], which takes the same
/// position — so the window is checked and the bridge rejects an out-of-range
/// one with a message that names the limit.
pub(crate) fn check_window(lo: usize, hi: usize, len: usize, what: &str) {
    assert!(
        lo <= hi,
        "sort window lo={lo} is past hi={hi}, which upstream would silently \
         treat as empty and this port refuses"
    );
    assert!(
        hi <= len,
        "sort window hi={hi} is past the end of {what} (len={len}); upstream \
         reads `undefined` there, which has no Rust representation"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every comparison with an absent operand is false, including the one
    /// where *both* are absent — `undefined > undefined` is `false`, not an
    /// equality.
    #[test]
    fn absent_operands_make_every_comparison_false() {
        let one = 1i32;
        let two = 2i32;

        assert!(gt(Some(&two), Some(&one)));
        assert!(!gt(Some(&one), Some(&two)));
        assert!(!gt(None::<&i32>, Some(&one)));
        assert!(!gt(Some(&one), None));
        assert!(!gt(None::<&i32>, None));

        assert!(ge(Some(&one), Some(&one)));
        assert!(!ge(None::<&i32>, None));

        assert!(le(Some(&one), Some(&one)));
        assert!(!le(None::<&i32>, None));
    }

    /// `NaN` is incomparable in both languages, so the `PartialOrd` operators
    /// already have JavaScript's answer and need no special-casing.
    #[test]
    fn nan_compares_false_in_every_direction() {
        let nan = f64::NAN;
        let one = 1.0f64;

        assert!(!gt(Some(&nan), Some(&one)));
        assert!(!gt(Some(&one), Some(&nan)));
        assert!(!ge(Some(&nan), Some(&nan)));
        assert!(!le(Some(&nan), Some(&nan)));
    }

    #[test]
    #[should_panic(expected = "is past the end")]
    fn a_window_past_the_end_is_refused() {
        check_window(0, 4, 3, "the array");
    }

    #[test]
    #[should_panic(expected = "is past hi")]
    fn an_inverted_window_is_refused() {
        check_window(3, 1, 8, "the array");
    }
}
