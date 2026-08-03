//! [`ModuleSpec`] for `fibonacci-heap`.
//!
//! # The grammar's whole reason for existing: forcing `consolidate` to merge
//! trees repeatedly, not once
//!
//! `push` alone never links anything — every node starts as its own tree in
//! the root list, and nothing merges until `pop` runs `consolidate`. A
//! grammar with many pushes and one pop reaches `consolidate` exactly once,
//! against a root list that is all singleton (degree-0) trees, and can prove
//! only that the heap can store numbers. So this grammar keeps `push` and
//! `pop` both heavily weighted and runs long programs (`program_len`
//! widened, matching `heap`'s own reasoning): a heap has to be built up and
//! torn down *repeatedly*, at sizes well past a handful of elements, before
//! `consolidate`'s degree-bucket linking has anything to do more than once.
//! [`grammar_self_check`] measures this directly, in operations, rather than
//! inferring it from the op weights — see that module.
//!
//! # What this heap genuinely cannot do: `decreaseKey`, cascading cuts
//!
//! Read `mnemonist_core::structures::fibonacci_heap`'s own module docs, and
//! before that, `~/upstream-mnemonist/fibonacci-heap.js` itself: there is no
//! `decreaseKey`, no `delete`, no node `mark`, no cut, no cascading cut
//! anywhere in the source or its `.d.ts`. **No op alphabet, however wide, can
//! reach a code path that does not exist.** This is not a gap in this
//! grammar; it is upstream's own limitation, stated here rather than
//! silently worked around by pretending the alphabet is merely incomplete.
//!
//! # Mutating comparators: `push`/`pop`/`clear`, not `items.push`/`items.pop`
//!
//! `heap`'s grammar mutates through `instance.items`, a real JS array. This
//! structure has no public backing array at all — `push`, `peek`, `pop` and
//! `clear` are the entire surface — so the re-entrant factories reach
//! through those instead: `fibPushy` calls `instance.push(99)`, `fibPopper`
//! calls `instance.pop()` (a **nested** `pop` from inside another `pop`'s
//! `consolidate` — legitimate re-entrancy this port has to survive without
//! panicking or deadlocking), and `fibClearer` calls `instance.clear()`.
//! `ascending`/`descending`/`boom` are reused verbatim from `heap`'s own
//! table in `fuzz/oracle.js` — all three are already generic over any
//! instance with a `.items`-free surface, since none of them touch `.items`
//! at all.
//!
//! # Observable state
//!
//! `size` and `peek`. There is no `.items` to compare, so these two are the
//! whole of what upstream exposes about a heap's contents; `peek` is a
//! nullary method rather than a property, and the oracle's generic
//! `observe()` already calls whichever kind a name resolves to.
//!
//! `size` is a signed `i64` on both sides of this comparison — see
//! `mnemonist_core::structures::fibonacci_heap`'s own docs and NOTES.md
//! BUG-FIBONACCI-HEAP-1: a `fibClearer` comparator firing from inside a `pop`'s
//! `consolidate` drives `this.size` to `-1` upstream, and the port
//! reproduces that value exactly rather than clamping it to `0`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use mnemonist_core::structures::fibonacci_heap::FibonacciHeap;
use mnemonist_core::utils::comparators::{
    default_comparator, default_reverse_comparator, Comparator, Thrown,
};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::modules::heap::{factory, factory_name, number, slot, thrown};
use crate::spec::{ModuleSpec, Op};

/// Range the generator draws pushed values from. Small, matching `heap`'s
/// own reasoning: frequent duplicates make tie-breaking (DIV-UTILS-2's whole
/// subject) actually happen, not just theoretically possible.
const VALUES: std::ops::Range<i64> = 0..24;

/// The value `fibPushy` pushes, matching `fuzz/oracle.js`.
const PUSHY_VALUE: f64 = 99.0;

/// The heap under test, at the one instantiation this fuzzer uses.
pub type HeapUnderTest = FibonacciHeap<f64, FuzzComparator, Thrown>;

pub struct FibonacciHeapSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fibonacci-heap.txt"
);

/// What a generated comparator does. `Ascending`/`Descending`/`Boom` mirror
/// `heap`'s own `Kind` and its `$factory` names in `fuzz/oracle.js`
/// (`ascending`, `descending`, `boom`) exactly, since all three are already
/// generic; `Pushy`/`Popper`/`Clearer` are this module's own, because they
/// reach through `push`/`pop`/`clear` rather than a backing array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ascending,
    Descending,
    Pushy,
    Popper,
    Clearer,
    Boom,
}

impl Kind {
    /// The `$factory` name the oracle knows this by.
    pub fn factory(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
            Self::Pushy => "fibPushy",
            Self::Popper => "fibPopper",
            Self::Clearer => "fibClearer",
            Self::Boom => "boom",
        }
    }

    pub fn from_factory(name: &str) -> Self {
        match name {
            "ascending" => Self::Ascending,
            "descending" => Self::Descending,
            "fibPushy" => Self::Pushy,
            "fibPopper" => Self::Popper,
            "fibClearer" => Self::Clearer,
            "boom" => Self::Boom,
            other => panic!("unknown comparator factory `{other}`"),
        }
    }

    /// The comparison budget, matching the oracle's closures exactly.
    fn budget(self) -> i64 {
        match self {
            Self::Pushy => 3,
            Self::Popper => 2,
            Self::Clearer => 1,
            Self::Boom => 5,
            _ => 0,
        }
    }
}

/// The Rust half of one of `fuzz/oracle.js`'s comparator factories.
///
/// Stateful, because the JavaScript ones are: a budget counted down per
/// comparison. Interior mutability throughout, because a comparator is
/// called through `&self` from inside `consolidate` and JavaScript's closure
/// is under no more restriction than that — see the module docs.
pub struct FuzzComparator {
    kind: Kind,
    budget: Cell<i64>,
    /// The heap, once it exists. `Weak` because the comparator lives
    /// *inside* it, exactly as the JavaScript closure captures the variable
    /// that holds it (mirroring `crate::modules::heap::FuzzComparator`).
    heap: RefCell<Weak<HeapUnderTest>>,
}

impl FuzzComparator {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            budget: Cell::new(kind.budget()),
            heap: RefCell::new(Weak::new()),
        }
    }

    /// Close the loop: the comparator can now reach the heap that holds it.
    pub fn attach(&self, heap: &Rc<HeapUnderTest>) {
        *self.heap.borrow_mut() = Rc::downgrade(heap);
    }

    /// `budget-- > 0`, post-decrement, exactly as the oracle writes it.
    fn spend(&self) -> bool {
        let remaining = self.budget.get();

        self.budget.set(remaining - 1);

        remaining > 0
    }
}

impl Comparator<f64, Thrown> for FuzzComparator {
    fn compare(&self, a: &f64, b: &f64) -> Result<f64, Thrown> {
        match self.kind {
            Kind::Ascending => return default_comparator(a, b),
            Kind::Descending => return default_reverse_comparator(a, b),
            Kind::Pushy => {
                if self.spend() {
                    if let Some(heap) = self.heap.borrow().upgrade() {
                        heap.push(PUSHY_VALUE)?;
                    }
                }
            }
            Kind::Popper => {
                if self.spend() {
                    if let Some(heap) = self.heap.borrow().upgrade() {
                        // A nested `pop` from inside another `pop`'s
                        // `consolidate` -- legitimate re-entrancy, and the
                        // one this port's arena (not `Rc<RefCell<Node>>`)
                        // exists to survive without a borrow panic. See
                        // `mnemonist_core::structures::fibonacci_heap`'s
                        // module docs.
                        heap.pop()?;
                    }
                }
            }
            Kind::Clearer => {
                if self.spend() {
                    if let Some(heap) = self.heap.borrow().upgrade() {
                        heap.clear();
                    }
                }
            }
            // `if (budget-- <= 0) throw` -- five comparisons, then every one.
            Kind::Boom => {
                if !self.spend() {
                    return Err(Thrown("boom"));
                }
            }
        }

        default_comparator(a, b)
    }
}

/// The heap, held in an [`Rc`] so its own comparator can reach it.
pub struct Instance {
    heap: Rc<HeapUnderTest>,
}

impl ModuleSpec for FibonacciHeapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "fibonacci-heap"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "peek"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            // Weighted towards the pure comparators, same reasoning as
            // `heap`'s own grammar: the mutating ones are the point, but a
            // program whose every comparison has a side effect spends its
            // length on one sift rather than on interactions.
            4 => Just(vec![factory("ascending")]),
            2 => Just(vec![factory("descending")]),
            2 => Just(vec![factory("fibPushy")]),
            2 => Just(vec![factory("fibPopper")]),
            2 => Just(vec![factory("fibClearer")]),
            1 => Just(vec![factory("boom")]),
        ]
        .boxed()
    }

    /// Widened well past `heap`'s own `1..200`: `consolidate` only does
    /// anything once a `pop` runs against a root list with more than one
    /// tree, and this heap needs the population built up across many pushes
    /// before a pop's degree-merging has real work — see the module docs
    /// and [`grammar_self_check`]'s measured counts.
    fn program_len(&self) -> std::ops::Range<usize> {
        1..400
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            7 => VALUES.prop_map(|value| Op::new("push", vec![json!(value)])),
            5 => Just(Op::new("pop", vec![])),
            1 => Just(Op::new("clear", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        install_panic_capture();

        let kind = Kind::from_factory(factory_name(&args[0]));
        let heap = Rc::new(FibonacciHeap::new(FuzzComparator::new(kind)));

        heap.comparator().attach(&heap);

        Instance { heap }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        let heap = &instance.heap;

        match op.name {
            "push" => thrown(heap.push(number(&op.args[0])).map(|size| json!(size))),
            "pop" => pop(heap),
            "clear" => {
                heap.clear();
                json!({"$undefined": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.heap.size(),
            "peek": slot(instance.heap.peek()),
        })
    }
}

/// `#.pop`, with NOTES.md BUG-FIBONACCI-HEAP-1/BUG-FIBONACCI-HEAP-3's follow-on crashes caught and
/// re-encoded.
///
/// Once a `fibClearer` comparator has driven the heap into one of two
/// inconsistent states (BUG-FIBONACCI-HEAP-1: `size` negative and `min` null, from inside a
/// `pop`'s own `consolidate`; BUG-FIBONACCI-HEAP-3: `root` null while `min` is a real node,
/// from inside a `push`'s tie-break instead), upstream's *next* `pop` throws
/// a real `TypeError` -- one of two, depending on which state it is. Both
/// are reproduced in `FibonacciHeap::pop`/`consolidate` as Rust panics whose
/// message text IS the exact upstream wording (see those methods' own doc
/// comments), specifically so this wrapper needs no hand-maintained
/// translation table that could drift from what Node actually says: it reads
/// the panic's own message text and uses it directly as the `$throw` value.
///
/// # Why this reads the message via a panic hook, not `downcast_ref`
///
/// The obvious approach -- `catch_unwind`'s `Err` payload,
/// `downcast_ref::<&'static str>()` / `::<String>()` -- is what `.expect`'s
/// *documented* payload shape would suggest, and it is what a small isolated
/// repro of `Option::expect` under `catch_unwind` actually produces. It is
/// NOT what this release-profile binary's real panics produce: measured by
/// instrumenting both downcasts and logging the failure, this call site's
/// actual payload matches neither type, in a way an isolated repro did not
/// reproduce -- almost certainly this optimised build's std reshaping the
/// payload construction and not something this module should depend on
/// being stable. [`std::panic::PanicHookInfo`] is unaffected by that: it
/// exposes `Display`, which formats to the same text the default hook prints
/// to stderr regardless of the payload's concrete type, because it is what
/// the payload's OWN formatting produces rather than a guess at its layout.
fn pop(heap: &Rc<HeapUnderTest>) -> Value {
    PANIC_MESSAGE.with(|cell| *cell.borrow_mut() = None);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| heap.pop()));

    match result {
        Ok(value) => thrown(value.map(slot)),
        Err(_) => {
            let message = PANIC_MESSAGE
                .with(|cell| cell.borrow_mut().take())
                .unwrap_or_else(|| {
                    String::from("fibonacci-heap: pop panicked but the hook captured no message")
                });

            json!({"$throw": message})
        }
    }
}

thread_local! {
    /// Set by [`install_panic_capture`]'s hook, read (and cleared) by
    /// [`pop`] immediately after a `catch_unwind` that returned `Err`.
    /// Thread-local because proptest's own `TestRunner` can run cases on a
    /// forked worker thread; each needs its own slot.
    static PANIC_MESSAGE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Install a panic hook, once per process, that captures the formatted
/// panic message into [`PANIC_MESSAGE`] in addition to whatever the previous
/// hook did (the default hook's stderr banner is left in place deliberately
/// -- a campaign's raw output staying inspectable is worth more here than a
/// quieter log for panics this module already expects and handles).
fn install_panic_capture() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();

    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();

        std::panic::set_hook(Box::new(move |info| {
            PANIC_MESSAGE.with(|cell| *cell.borrow_mut() = Some(bare_message(info)));
            previous(info);
        }));
    });
}

/// The panic message alone, with the standard hook's
/// `"thread '<name>' panicked at <location>:\n"` banner stripped off.
///
/// `PanicHookInfo::payload()` is `&(dyn Any + Send)`, and downcasting it to
/// `&'static str` / `String` is the textbook way to recover a `.expect(...)`
/// message -- and, measured directly (a debug-logged downcast failure this
/// call site hit under this crate's release profile, which an isolated
/// `rustc -O` repro of the identical `.expect` call did NOT reproduce), not
/// reliable enough to depend on here. `Display`'s output is: this hook's
/// `previous(info)` call proves the default hook can always render *some*
/// text for any payload, so parsing that rendering is the stable interface,
/// not the payload's internal representation. The banner is exactly one
/// line (the location can never itself contain a newline), so splitting on
/// the first `'\n'` and falling back to the whole string when there is none
/// is exact, not a heuristic.
fn bare_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let rendered = info.to_string();

    match rendered.split_once('\n') {
        Some((_banner, message)) => message.to_owned(),
        None => rendered,
    }
}

#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    /// Runs `samples` generated `(ctor, ops)` programs directly against the
    /// core structure -- no oracle, no Node -- and counts, directly rather
    /// than by inferring it from the op weights:
    ///
    /// * how many times `consolidate` actually [`link`](FibonacciHeap)ed two
    ///   trees together (`merges()`), and
    /// * how many of those programs saw at least one link at all.
    ///
    /// This is the measurement CLAUDE.md's brief for this unit asked for
    /// directly: "a Fibonacci heap fuzzed with many pushes and one pop never
    /// consolidates, and a green campaign then proves only that it can store
    /// numbers." A campaign report with zero divergences says nothing about
    /// which branches actually ran; this does.
    fn sample(samples: usize) -> (u64, u64, u64) {
        let spec = FibonacciHeapSpec;
        let mut runner = TestRunner::new(Config::default());
        let mut total_ops = 0u64;
        let mut total_merges = 0u64;
        let mut programs_with_a_merge = 0u64;

        for _ in 0..samples {
            let ctor = spec
                .ctor_strategy()
                .new_tree(&mut runner)
                .expect("ctor_strategy never rejects")
                .current();
            let ops = proptest::collection::vec(spec.op_strategy(&ctor), spec.program_len())
                .new_tree(&mut runner)
                .expect("op_strategy never rejects")
                .current();
            let mut instance = spec.construct(&ctor);
            let before = instance.heap.merges();

            for op in &ops {
                total_ops += 1;
                spec.apply(&mut instance, op);
            }

            let merges_this_program = instance.heap.merges() - before;

            total_merges += merges_this_program;

            if merges_this_program > 0 {
                programs_with_a_merge += 1;
            }
        }

        (total_ops, total_merges, programs_with_a_merge)
    }

    #[test]
    fn consolidation_actually_fires_across_generated_programs() {
        let (ops, merges, programs_with_a_merge) = sample(400);

        eprintln!(
            "fibonacci-heap grammar: {ops} ops, {merges} tree merges across 400 programs \
             ({programs_with_a_merge} of them saw at least one)"
        );

        // Both floors are deliberately blunt: the point is not a precise
        // rate but ruling out "the campaign never actually reached
        // `consolidate`'s degree-merge path", which is exactly the failure
        // mode CLAUDE.md's brief warned this unit could fall into silently.
        assert!(
            merges > 1_000,
            "expected consolidate to link trees thousands of times over 400 programs \
             of up to {} ops each, got {merges}",
            FibonacciHeapSpec.program_len().end
        );
        assert!(
            programs_with_a_merge > 200,
            "expected a majority of 400 generated programs to trigger at least one \
             consolidation merge, got {programs_with_a_merge}"
        );
    }

    // The other half of the brief is deliberately NOT a test here: there is
    // no `decreaseKey` anywhere in upstream's `fibonacci-heap.js` (confirmed
    // by reading the vendored source and its `.d.ts` -- see
    // `mnemonist_core::structures::fibonacci_heap`'s module docs), so a
    // cascading cut is not merely unfuzzed by this grammar, it is
    // unreachable through ANY op alphabet whatsoever -- there is no method
    // to call. A test that asserts the absence of a method would only ever
    // pass by construction and would rot silently; the module docs and
    // `docs/modules/fibonacci-heap.md` are where this is recorded instead.
}
