//! JS bridge for [`mnemonist_core::structures::queue`].
//!
//! Structurally identical to [`crate::stack`] — same element type, same
//! dispatch behind `from`, same JS-installed `of`, same `scope` divergence in
//! `forEach`, same absent `inspect`. Two differences are worth naming.
//!
//! * **`offset` is exposed.** Upstream keeps it as a public property, and
//!   unlike `SparseSet`'s typed arrays it is a plain number, so handing it over
//!   loses nothing. It is also what makes the compaction schedule observable
//!   from JS at all.
//! * **The cursor's end is live.** `Queue.prototype.values` re-reads
//!   `items.length` on every step, so an enqueue during iteration is visible
//!   and a finished walk resumes. That is entirely a core behaviour
//!   ([`Sequence::limit`](mnemonist_core::cursor::Sequence::limit)); the bridge
//!   only has to not get in its way.
//!
//! Like [`crate::stack`], the core structure is held in a [`RefCell`] so that
//! `&self` is not `noalias readonly` and a JS callback's mutation is actually
//! seen. `crate::cursor::CellCursor` records the measured failure — and it was
//! measured on *this* module's `forEach`.

use std::cell::RefCell;

use mnemonist_core::cursor::Step;
use mnemonist_core::structures::queue::Queue as CoreQueue;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::{yielded, CellCursor};
use crate::foreach;
use crate::js_slot::JsSlot;

/// A FIFO queue of arbitrary JavaScript values.
#[napi(js_name = "Queue")]
pub struct JsQueue {
    inner: RefCell<CoreQueue<JsSlot>>,
}

#[napi]
impl JsQueue {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(CoreQueue::new()),
        }
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// The index of the front element inside the backing array. Public
    /// upstream, and the only way the compaction is visible from JS.
    #[napi(getter)]
    pub fn offset(&self) -> u32 {
        self.inner.borrow().offset() as u32
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Upstream returns the new size, not the instance.
    #[napi]
    pub fn enqueue(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = JsSlot::new(&env, &item)?;

        Ok(self.inner.borrow_mut().enqueue(slot) as u32)
    }

    /// `undefined` on an empty queue, which is upstream's bare `return;`.
    #[napi]
    pub fn dequeue(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow_mut().dequeue().into()
    }

    #[napi]
    pub fn peek(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek().into()
    }

    /// `#.toArray` — `items.slice(offset)`, oldest first.
    #[napi(js_name = "toArray")]
    pub fn to_array(&self) -> Vec<JsSlot> {
        self.inner.borrow().to_vec()
    }

    /// `#.toJSON`, which upstream defines as `toArray`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<JsSlot> {
        self.inner.borrow().to_vec()
    }

    /// `#.toString`, which upstream defines as `toArray().join(',')`.
    #[napi(js_name = "toString")]
    pub fn to_js_string(&self, env: Env) -> Result<String> {
        let slots = self.inner.borrow().to_vec();

        foreach::join(&env, &slots)
    }

    /// `#.forEach` — oldest first, with `(value, j, queue)` where `j` counts
    /// from zero rather than from the offset.
    ///
    /// Upstream freezes the starting index and the bound but re-reads
    /// `this.items` every iteration, so a callback that dequeues far enough to
    /// compact sends the remaining reads into the *new* array under the old
    /// absolute index. [`CoreQueue::slot`] is that live read.
    ///
    /// `scope` carries the same `arguments.length` divergence as everywhere
    /// else in the bridge.
    // The callback type is spelled out rather than aliased because napi's
    // macro reads the parameter type syntactically to generate the binding;
    // behind a `type` it stops recognising the `Function` shape.
    #[allow(clippy::type_complexity)]
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Either<JsSlot, Undefined>, u32, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let queue = this.object;
        let (start, length) = {
            let inner = self.inner.borrow();

            (inner.offset(), inner.items_len())
        };

        for (position, index) in (start..length).enumerate() {
            // The borrow ends with this statement, on purpose: a callback that
            // dequeues far enough to compact rebinds the backing array, and
            // upstream's re-read of `this.items` is only reproduced if this
            // read is genuinely a read.
            let value: Either<JsSlot, Undefined> = self.inner.borrow().slot(index).into();
            let arguments = (value, position as u32, queue).into();

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(queue, arguments)?,
            };
        }

        Ok(())
    }

    /// A fresh cursor over the values, oldest first.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsQueue>) -> Result<JsQueueValues> {
        let source = this.share_with(env, |queue| Ok(&queue.inner))?;

        Ok(JsQueueValues {
            cursor: CellCursor::open(source),
        })
    }

    /// A fresh cursor over `[index, value]` pairs, oldest first.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsQueue>) -> Result<JsQueueEntries> {
        let source = this.share_with(env, |queue| Ok(&queue.inner))?;

        Ok(JsQueueEntries {
            cursor: CellCursor::open(source),
            index: 0,
        })
    }

    /// `Queue.from(iterable)` — the five-branch coercion, then enqueue in order.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(foreach::collect(&env, iterable)?.into_iter().collect()),
        })
    }
}

impl Default for JsQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// The cursor `Queue.prototype.values()` hands out.
#[napi(iterator, js_name = "QueueValues")]
pub struct JsQueueValues {
    cursor: CellCursor<JsQueue, CoreQueue<JsSlot>>,
}

impl Generator for JsQueueValues {
    type Yield = Either<JsSlot, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// The cursor `Queue.prototype.entries()` hands out.
#[napi(iterator, js_name = "QueueEntries")]
pub struct JsQueueEntries {
    cursor: CellCursor<JsQueue, CoreQueue<JsSlot>>,
    /// `j` in upstream's `[j++, value]`, advanced only on a yielded step.
    index: u32,
}

impl Generator for JsQueueEntries {
    type Yield = (u32, Either<JsSlot, Undefined>);
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        let value: Either<JsSlot, Undefined> = match self.cursor.step() {
            Step::Item(slot) => Either::A(slot),
            Step::Gap => Either::B(()),
            Step::Done => return None,
        };
        let index = self.index;

        self.index += 1;

        Some((index, value))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}
