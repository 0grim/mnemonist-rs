//! `X.of(...)` — the one upstream static with no Rust representation.
//!
//! Every structure that has a `from` also has
//!
//! ```js
//! Stack.of = function () {
//!   return Stack.from(arguments);
//! };
//! ```
//!
//! and that is *all* it is: arity glue with no logic. It cannot be written as
//! a `#[napi]` method, because napi-rs has no variadic parameter and `arguments`
//! is not a value Rust can construct.
//!
//! # Why this is evaluated JavaScript rather than a native closure
//!
//! napi does offer a way out: `create_function_from_closure` hands the callback
//! every raw argument, so `of` could be written natively — collect the
//! arguments, build the structure, return it. It would pass every test.
//!
//! What it would cost is that `Stack.of(1, 2, 3)` would stop putting a real
//! `arguments` object through the real dispatch. The installer below is
//! upstream's line, verbatim, evaluated once at module load, so it does. The
//! script is a fixed literal — no interpolation, nothing caller-supplied — and
//! it makes the addon self-contained: a shim that added `of` would mean
//! `require('@port/addon').Stack` was incomplete without the test harness,
//! which is exactly backwards (D-07's reasoning, applied to a static instead of
//! to `Symbol.iterator`).
//!
//! # What this does NOT buy, corrected after measuring
//!
//! An earlier draft of this comment claimed that routing `of` through `from`
//! makes the original suite exercise the `toString() === '[object Arguments]'`
//! clause of `crate::foreach`'s branch 1. **That is false, and deleting the
//! clause proves it:** with the clause removed, all 22 assertions in
//! `test/stack.js` and `test/queue.js` still pass, `of` included. A modern
//! `arguments` object carries `Symbol.iterator`, so it simply falls through to
//! branches 3 and 4, which drain it in the same order with the same numeric
//! second argument. The clause is observable only for something that claims the
//! tag without being iterable — a hijacked `toString`, which is what
//! `tests/boundary/foreach.js` uses and the only coverage it has.

use napi::bindgen_prelude::*;

/// Classes whose `of` static is `X.from(arguments)`.
///
/// Data rather than code per class, for the same reason as
/// `crate::cursor::ITERATOR_FACTORIES`: adding a module should not add a place
/// to get this wrong.
const VARIADIC_FACTORIES: &[&str] = &["Stack", "Queue"];

/// Upstream's definition, with the constructor passed in so the closure over
/// it is the same one upstream closes over.
const INSTALLER: &str =
    "(function (Ctor) { Ctor.of = function () { return Ctor.from(arguments); }; })";

/// Cursor factories whose product must not carry a `#.return` method.
///
/// `(class, method)`, matching `crate::cursor::ITERATOR_FACTORIES`' shape.
const CURSOR_FACTORIES: &[(&str, &str)] = &[
    ("Stack", "values"),
    ("Stack", "entries"),
    ("Queue", "values"),
    ("Queue", "entries"),
    // Appended at the end, never inserted -- see the note on
    // `crate::cursor::ITERATOR_FACTORIES`.
    ("FixedStack", "values"),
    ("FixedStack", "entries"),
    ("FixedDeque", "values"),
    ("FixedDeque", "entries"),
    ("CircularBuffer", "values"),
    ("CircularBuffer", "entries"),
];

/// Take `#.return` off every cursor, because upstream's cursors do not have one.
///
/// # The behaviour this restores, measured
///
/// `obliterator/iterator` is four lines: a constructor that stores a `next`
/// closure, and an identity `Symbol.iterator`. **No `return`, no `done` flag.**
/// So breaking out of a `for…of` leaves the cursor exactly where it stopped and
/// a later `next()` resumes:
///
/// ```js
/// var it = Stack.from([1, 2, 3]).values();
/// for (var v of it) break;      // yields 3
/// it.next();                    // {value: 2, done: false}
/// ```
///
/// napi's `#[napi(iterator)]` sets `next`, `return` and `throw` as **own
/// properties on every instance**, and its `return` writes a
/// `[[GeneratorState]]` flag that makes every later `next()` answer
/// `{done: true}`. So a `break` silently kills the cursor, and the port
/// answered `{done: true}` where upstream answered `{value: 2}`.
///
/// The flag is per-instance and set before the Rust `complete` runs, so
/// `Generator::complete` cannot prevent it; the only thing that can is `return`
/// not being found at all, which is upstream's situation. `IteratorClose` does
/// `GetMethod(iterator, "return")` and skips a `undefined` one, so deleting the
/// property *is* the fix, and it is what upstream has.
///
/// This corrects a claim in `crate::sparse_set`'s docs — that napi's default
/// `complete` is observably the same as having no `return` — which was reasoned
/// about rather than measured, and is wrong. `SparseSet` is left alone here
/// because it is already in `tests/scope.txt`; the same two rows would fix it.
const CURSOR_PATCH: &str = "(function (Ctor, method) { \
     var original = Ctor.prototype[method]; \
     Ctor.prototype[method] = function () { \
       var cursor = original.call(this); \
       delete cursor['return']; \
       return cursor; \
     }; \
   })";

/// Give every class in the table its `of` static, and strip `#.return` from
/// every cursor.
///
/// Called from the module-export hook in [`crate::cursor`], which is the one
/// `#[napi(module_exports)]` the addon has.
pub fn install_variadic_factories(exports: &mut Object, env: &Env) -> Result<()> {
    let installer: Function<'_, Unknown, Unknown> = env.run_script(INSTALLER)?;

    for class in VARIADIC_FACTORIES {
        installer.call(class_of(exports, class, "of")?)?;
    }

    let patch: Function<'_, FnArgs<(Unknown, String)>, Unknown> = env.run_script(CURSOR_PATCH)?;

    for (class, method) in CURSOR_FACTORIES {
        let constructor = class_of(exports, class, method)?;

        patch.call((constructor, (*method).to_owned()).into())?;
    }

    // Appended at the very end of this function, deliberately: it is the last
    // statement before the `Ok`, so a merge conflict cannot land inside an
    // existing loop or match arm. `heap.js` ends with its own load-time
    // assignments -- `MaxHeap.prototype = Heap.prototype`, `Heap.MinHeap`,
    // `Heap.MaxHeap` -- and they belong in the addon for the same reason `of`
    // does: a shim that added them would mean `require('@port/addon').Heap` was
    // incomplete without the test harness.
    crate::comparators::install_comparator_factories(exports, env)?;
    crate::heap::install_heap_statics(exports, env)?;

    Ok(())
}

fn class_of<'env>(exports: &Object<'env>, class: &str, what: &str) -> Result<Unknown<'env>> {
    exports.get(class)?.ok_or_else(|| {
        Error::new(
            Status::GenericFailure,
            format!(
                "cannot install `{class}.{what}`: exports.{class} does not exist. The \
                 tables in this module and the addon's exports have drifted apart."
            ),
        )
    })
}
