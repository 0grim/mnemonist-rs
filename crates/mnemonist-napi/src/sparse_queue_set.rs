//! JS bridge for [`mnemonist_core::structures::sparse_queue_set`].
//!
//! Thin translation only; every behavioural decision lives in the core crate.
//! Four adaptations.
//!
//! 1. **`enqueue` returns `this`.** Upstream returns the instance for chaining;
//!    the core returns whether the member was newly enqueued, which upstream
//!    exposes only through `size`. The bool is dropped here.
//! 2. **`dequeue` returns `Either<u32, Undefined>`.** Upstream's empty-queue
//!    return is a bare `return;`, i.e. `undefined`, and napi renders
//!    `Option::None` as `null` — a different value to `assert.strictEqual`.
//! 3. **`start` is a read-only getter** where upstream's is a writable data
//!    property, as are `size` and `capacity`. Reproducing the writability would
//!    mean accepting arbitrary values into a field every method's arithmetic
//!    trusts; the original suite never writes any of the three.
//! 4. **`dense` and `sparse` are not exposed**, for the same reason as in
//!    `sparse-set`: they are public typed arrays upstream and a JS caller can
//!    write *through* them, but napi can only hand out a copy. They are exposed
//!    in Rust, and the differential fuzzer compares both slot for slot after
//!    every op.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::sparse_queue_set::SparseQueueSet as CoreQueue;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::{yielded, BridgeCursor};

/// What upstream's `forEach` callback is invoked with: the member (possibly
/// `undefined`), the ordinal, and the queue itself.
type ForEachArgs<'a> = FnArgs<(Either<u32, Undefined>, u32, Object<'a>)>;

/// A FIFO queue over the members `0..capacity`, with O(1) membership.
#[napi(js_name = "SparseQueueSet")]
pub struct JsSparseQueueSet {
    inner: CoreQueue,
}

#[napi]
impl JsSparseQueueSet {
    #[napi(constructor)]
    pub fn new(capacity: u32) -> Result<Self> {
        CoreQueue::new(capacity as usize)
            .map(|inner| Self { inner })
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// Members currently queued. Can exceed `capacity`; see the core docs.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.capacity() as u32
    }

    /// Index of the front of the ring.
    ///
    /// Unbounded at `capacity === 0`, where upstream's `start === capacity`
    /// wrap check never fires — which is why this is a `u32` rather than
    /// something bounded by the capacity.
    #[napi(getter)]
    pub fn start(&self) -> u32 {
        self.inner.start() as u32
    }

    #[napi]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    #[napi]
    pub fn has(&self, member: u32) -> bool {
        self.inner.has(member as usize)
    }

    /// Upstream returns `this` for chaining.
    #[napi]
    pub fn enqueue<'a>(&mut self, this: This<'a>, member: u32) -> This<'a> {
        self.inner.enqueue(member as usize);

        this
    }

    #[napi]
    pub fn dequeue(&mut self) -> Either<u32, Undefined> {
        self.inner.dequeue().into()
    }

    /// A fresh cursor over the queued members, front to back.
    ///
    /// The *factory* half of D-07: every call constructs a new cursor object,
    /// so `[...queue]` works repeatedly while each cursor is individually
    /// non-restartable. `crate::cursor::install_iterator_factories` aliases
    /// `Symbol.iterator` onto this method, as upstream's last line does.
    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsSparseQueueSet>,
    ) -> Result<JsSparseQueueSetValues> {
        let source = this.share_with(env, |queue| Ok(&queue.inner))?;

        Ok(JsSparseQueueSetValues {
            cursor: BridgeCursor::open(source),
        })
    }

    /// Upstream's own `forEach`.
    ///
    /// **Unlike `SparseSet.forEach` and `SparseMap.forEach`, this one freezes.**
    /// The other two loop on `i < this.size`, re-reading the live size every
    /// iteration; this one captures `c`, `l` and `i` before the loop:
    ///
    /// ```js
    /// var c = this.capacity, l = this.size, i = this.start, j = 0;
    /// while (j < l) { callback.call(scope, this.dense[i], j, this); … }
    /// ```
    ///
    /// So a callback that dequeues does **not** shorten this loop, where the
    /// equivalent callback shortens `SparseSet`'s. That inconsistency is
    /// upstream's, and it is reproduced by driving the same `CursorState` the
    /// `values()` cursor uses rather than by writing a second loop that might
    /// drift from it.
    ///
    /// The callback takes three arguments here, not two: `(member, index,
    /// queue)`. `member` can be `undefined` once the walk runs off the end of
    /// `dense`, which at `capacity === 0` is every step.
    ///
    /// `scope` carries the same `arguments.length > 1 ? scope : this` blind
    /// spot as the other two: `forEach(cb, undefined)` binds the queue here
    /// where upstream binds `undefined`. The omitted-argument case — the only
    /// one the original suite uses — is exact.
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<ForEachArgs, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut walk = CursorState::open(&self.inner);
        let mut ordinal = 0u32;

        loop {
            let member: Either<u32, Undefined> = match walk.step(&self.inner) {
                Step::Item(member) => Either::A(member),
                Step::Gap => Either::B(()),
                Step::Done => return Ok(()),
            };

            let args = (member, ordinal, this.object);

            match &scope {
                Some(scope) => callback.apply(*scope, args.into())?,
                None => callback.apply(this, args.into())?,
            };

            ordinal += 1;
        }
    }
}

/// The cursor `SparseQueueSet.prototype.values()` hands out.
///
/// `#[napi(iterator)]` supplies the identity half of D-07 for free: this
/// object's own `Symbol.iterator` returns itself, so it is non-restartable.
#[napi(iterator, js_name = "SparseQueueSetValues")]
pub struct JsSparseQueueSetValues {
    cursor: BridgeCursor<JsSparseQueueSet, CoreQueue>,
}

impl Generator for JsSparseQueueSetValues {
    /// `Either<u32, Undefined>`, not `Option<u32>`: napi renders `None` as
    /// `null`, and a walk over a zero-capacity queue yields real `undefined`s.
    type Yield = Either<u32, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step())
    }

    /// Upstream cursors have no `return` method, so a `break` out of a `for…of`
    /// leaves the cursor where it stopped and a later `next()` resumes. napi's
    /// default is the same observable behaviour; overriding it to `None` keeps
    /// that explicit rather than inherited.
    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}
