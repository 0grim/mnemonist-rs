//! [`ModuleSpec`] for `heap`.
//!
//! # The grammar's whole reason for existing: a comparator that mutates
//!
//! B-31 survived 2.94M operations because the `queue` alphabet had no
//! `forEach`, and a grammar that omits a method omits every bug reachable only
//! through it. The equivalent omission here would be a grammar whose
//! comparators are all pure, which is what every one of `test/heap.js`'s
//! fourteen cases uses — so the alphabet supplies a **comparator that mutates
//! the heap mid-operation**, in all three shapes that differ:
//!
//! | comparator | what it does to the array being sifted | why it is distinct |
//! |---|---|---|
//! | [`Kind::Pushy`] | `items.push(99)` | grows it under an index the sift already chose |
//! | [`Kind::Popper`] | `items.pop()` | shrinks it, so the walk reads past its frozen `endIndex` and gets `undefined` |
//! | [`Kind::Clearer`] | `heap.clear()` | **rebinds** it, so the sift finishes into a detached array (D-41) |
//! | [`Kind::Boom`] | throws | leaves `items.length` one ahead of `size`, permanently (B-70) |
//!
//! A port that modelled `items` as a `Vec<f64>` would answer identically for
//! the first two and be silently wrong for the third. A port whose algorithms
//! held `&mut Vec` could not have run any of them.
//!
//! # The budget is part of what is compared
//!
//! Each mutating comparator fires for its first *k* comparisons and then stops.
//! That makes the result depend on the **number and order of comparisons**, not
//! merely on the final ordering — so a sift that is correct but performs a
//! different number of comparisons diverges here, where a black-box
//! push/pop-only grammar would never notice.
//!
//! # Observable state
//!
//! `size` and `items`. They are separate quantities upstream and can genuinely
//! disagree (B-70), so comparing both is the point; `toArray` is an *op* rather
//! than an observation because it runs the comparator, and an observation with
//! side effects would fire after every op and make every divergence report
//! ambiguous.
//!
//! # Deliberately excluded
//!
//! `Heap.nsmallest` / `nlargest` and the raw-array statics, because they are
//! functions rather than methods on the instance and the oracle's protocol
//! addresses one instance. They are covered by `test/heap.js` (four cases) and
//! by `tests/boundary/heap.js` (six). `inspect` is not ported.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use mnemonist_core::structures::heap::{Heap, Store, VecStore};
use mnemonist_core::utils::comparators::{
    default_comparator, default_reverse_comparator, Comparator, Thrown,
};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Range the generator draws pushed values from.
///
/// Small, so duplicates are frequent: a heap's tie-breaking is observable
/// through `toArray`, and `sift_up`'s `>= 0` test is the only thing that
/// decides it.
const VALUES: std::ops::Range<i64> = 0..24;

/// The value [`Kind::Pushy`] appends, matching `fuzz/oracle.js`.
const PUSHY_VALUE: f64 = 99.0;

/// The heap under test, at the one instantiation the fuzzer uses.
pub type HeapUnderTest = Heap<VecStore<f64>, FuzzComparator>;

pub struct HeapSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proptest-regressions/heap.txt");

/// What a generated comparator does, mirroring `fuzz/oracle.js`'s factories
/// name for name.
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
            Self::Pushy => "pushy",
            Self::Popper => "popper",
            Self::Clearer => "clearer",
            Self::Boom => "boom",
        }
    }

    pub fn from_factory(name: &str) -> Self {
        match name {
            "ascending" => Self::Ascending,
            "descending" => Self::Descending,
            "pushy" => Self::Pushy,
            "popper" => Self::Popper,
            "clearer" => Self::Clearer,
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
/// comparison. Interior mutability throughout, because a comparator is called
/// through `&self` from inside a sift and JavaScript's closure is under no more
/// restriction than that.
pub struct FuzzComparator {
    kind: Kind,
    budget: Cell<i64>,
    /// The heap, once it exists. `Weak` because the comparator lives *inside*
    /// it, exactly as the JavaScript closure captures the variable that holds
    /// it.
    heap: RefCell<Weak<HeapUnderTest>>,
    /// A fixed array to mutate when there is no heap to ask — which is the
    /// `fixed-reverse-heap` case, whose `items` is never rebound.
    items: RefCell<Option<VecStore<f64>>>,
}

impl FuzzComparator {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            budget: Cell::new(kind.budget()),
            heap: RefCell::new(Weak::new()),
            items: RefCell::new(None),
        }
    }

    /// Close the loop: the comparator can now reach the heap that holds it.
    pub fn attach(&self, heap: &Rc<HeapUnderTest>) {
        *self.heap.borrow_mut() = Rc::downgrade(heap);
    }

    /// As [`attach`](Self::attach), for a structure whose array never moves.
    pub fn attach_items(&self, items: VecStore<f64>) {
        *self.items.borrow_mut() = Some(items);
    }

    /// `instance.items` — read live, because a `clear()` rebinds it and the
    /// oracle's closure would see the new one.
    fn current_items(&self) -> Option<VecStore<f64>> {
        if let Some(heap) = self.heap.borrow().upgrade() {
            return Some(heap.items());
        }

        self.items.borrow().clone()
    }

    /// `budget-- > 0`, post-decrement, exactly as the oracle writes it.
    fn spend(&self) -> bool {
        let remaining = self.budget.get();

        self.budget.set(remaining - 1);

        remaining > 0
    }
}

impl Comparator<Option<f64>, Thrown> for FuzzComparator {
    fn compare(&self, a: &Option<f64>, b: &Option<f64>) -> Result<f64, Thrown> {
        match self.kind {
            Kind::Ascending => return default_comparator(a, b),
            Kind::Descending => return default_reverse_comparator(a, b),
            Kind::Pushy => {
                if self.spend() {
                    if let Some(items) = self.current_items() {
                        items.push(Some(PUSHY_VALUE))?;
                    }
                }
            }
            Kind::Popper => {
                if self.spend() {
                    if let Some(items) = self.current_items() {
                        items.pop()?;
                    }
                }
            }
            Kind::Clearer => {
                if self.spend() {
                    if let Some(heap) = self.heap.borrow().upgrade() {
                        heap.clear()?;
                    }
                }
            }
            // `if (budget-- <= 0) throw` — five comparisons, then every one.
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

impl ModuleSpec for HeapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "heap"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            // Weighted towards the pure comparators: the mutating ones are the
            // point, but a program whose every comparison has a side effect
            // spends its length on one sift rather than on interactions.
            4 => Just(vec![factory("ascending")]),
            2 => Just(vec![factory("descending")]),
            2 => Just(vec![factory("pushy")]),
            2 => Just(vec![factory("popper")]),
            2 => Just(vec![factory("clearer")]),
            1 => Just(vec![factory("boom")]),
        ]
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            6 => VALUES.prop_map(|value| Op::new("push", vec![json!(value)])),
            3 => Just(Op::new("pop", vec![])),
            2 => Just(Op::new("peek", vec![])),
            2 => VALUES.prop_map(|value| Op::new("replace", vec![json!(value)])),
            2 => VALUES.prop_map(|value| Op::new("pushpop", vec![json!(value)])),
            1 => Just(Op::new("clear", vec![])),
            1 => Just(Op::new("consume", vec![])),
            2 => Just(Op::new("toArray", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let kind = Kind::from_factory(factory_name(&args[0]));
        let heap = Rc::new(Heap::new(VecStore::new(), FuzzComparator::new(kind)));

        heap.comparator().attach(&heap);

        Instance { heap }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        let heap = &instance.heap;

        match op.name {
            "push" => thrown(heap.push(Some(number(&op.args[0]))).map(|size| json!(size))),
            "pop" => thrown(heap.pop().map(slot)),
            "peek" => thrown(heap.peek().map(slot)),
            "replace" => thrown(heap.replace(Some(number(&op.args[0]))).map(slot)),
            "pushpop" => thrown(heap.pushpop(Some(number(&op.args[0]))).map(slot)),
            "clear" => thrown(heap.clear().map(|()| json!({"$undefined": true}))),
            "consume" => thrown(heap.consume().map(|items| slots(&items))),
            "toArray" => thrown(heap.to_array().map(|items| slots(&items))),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.heap.size(),
            "items": slots(&instance.heap.items()),
        })
    }
}

/// `{"$factory": "<name>"}` — how a function travels over JSON.
pub(crate) fn factory(name: &str) -> Value {
    json!({ "$factory": name })
}

pub(crate) fn factory_name(value: &Value) -> &str {
    value
        .get("$factory")
        .and_then(Value::as_str)
        .expect("a comparator argument is a $factory envelope")
}

/// A generated argument, as the heap stores it.
pub(crate) fn number(value: &Value) -> f64 {
    value.as_f64().expect("generated arguments are numbers")
}

/// An exception, in the shape `fuzz/oracle.js` reports one.
///
/// The oracle catches an exception thrown *by an operation* and encodes it as a
/// comparable result rather than as apparatus failure. A comparator that throws
/// is the first module in this repo where that path carries the finding rather
/// than an edge case.
pub(crate) fn thrown(result: Result<Value, Thrown>) -> Value {
    match result {
        Ok(value) => value,
        Err(Thrown(message)) => json!({ "$throw": message }),
    }
}

/// One slot: a number, or the array's `undefined`.
pub(crate) fn slot(value: Option<f64>) -> Value {
    match value {
        None => json!({"$undefined": true}),
        Some(number) => encode_number(number),
    }
}

/// Every slot of an array, holes included.
pub(crate) fn slots(store: &VecStore<f64>) -> Value {
    Value::Array(store.to_vec().into_iter().map(slot).collect())
}

/// A number in the shape `fuzz/oracle.js`'s `encode` produces.
///
/// Integral values go out as JSON integers, because that is what a JS number
/// round-trips to and `serde_json` does not consider `5.0` equal to `5`.
pub(crate) fn encode_number(value: f64) -> Value {
    if value.is_nan() {
        return json!({"$nan": true});
    }

    if value.is_infinite() {
        return json!({"$infinity": if value > 0.0 { 1 } else { -1 }});
    }

    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        return json!(value as i64);
    }

    json!(value)
}
