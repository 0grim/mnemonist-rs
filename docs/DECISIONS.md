# Decisions

This document records every deliberate divergence between this port and upstream `mnemonist`:
places where the port's observable behaviour differs from the JavaScript library's on purpose, and
places where it reproduces something upstream does that a Rust implementation would not naturally
do. It is the companion to `docs/BUGS.md`, which records defects found *in* upstream; this document
records where the port knowingly departs from upstream, or knowingly keeps a departure upstream
itself would not recognise as a bug.

Every entry below traces to a specific module and a specific mechanism. Divergences are grouped by
theme rather than listed in the order they were found, because the order they were found in has no
bearing on what a reader needs to understand the port.

---

## Iteration order and cursors

Upstream's iteration primitives, from the `obliterator` package, have several properties an
idiomatic Rust iterator does not: a collection's own iterator restarts on every call but a stored
cursor does not, some cursors freeze a length at creation while a structurally identical one
re-reads it on every step, and a cursor can observe live mutation to the elements it walks while
being blind to a change in how many elements there are. Reproducing this needed a cursor type built
around upstream's actual semantics rather than around `Iterator`.

**A cursor is a stateful, non-restartable object, not `IntoIterator`.** `obliterator`'s iterator
type returns itself from `Symbol.iterator`, so a second drain of the same stored cursor yields
nothing. The port never implements `IntoIterator` for a structure's cursor type, which would hand
out a fresh iterator per loop and silently restart it. A collection's own `Symbol.iterator`,
however, *does* construct a fresh cursor on every call — `[...stack]` twice gives the same sequence
twice, while `const it = stack.values(); [...it]` twice gives the sequence and then nothing. Both
halves are ported: the cursor type is a one-shot walk: the collection-level `Symbol.iterator` is a
factory installed from Rust that builds a new one each time it is asked.

**A cursor's captured length is frozen for most structures, but not all.** `Stack.prototype.values`
freezes `l = items.length` at creation, so a value pushed after the cursor is built is invisible to
it. `Queue.prototype.values`, four files away and structurally identical in every other respect,
re-reads `items.length` on every step, so a queue cursor that has already reported completion
*resumes* if the queue grows afterward. One uniform cursor shape would have silently reproduced only
one of these and terminated the other early. The port's cursor abstraction takes the walk's limit as
an overridable quantity, defaulting to the frozen length so every ordinary source is unaffected, with
`Queue` supplying the live one.

**Element mutation is visible through an open cursor; a length change is not, for the frozen case.**
A hybrid capture: the length is fixed once, the elements are read fresh on each step through a live
borrow of the structure. In pure Rust this distinction is unobservable — the borrow checker forbids
holding a cursor and mutating the structure at the same time, so the question never arises from a
Rust caller. It becomes observable only through the bridge, where a JavaScript caller can hold a
cursor and still call a mutating method, and the differential fuzzer's `mutate element → iter_next`
grammar exercises exactly that path.

**A cursor's state is detached from the borrow it walks, and the two are separate types.** The
natural Rust shape for a cursor is a struct holding `&'a Structure`. It is the wrong shape here,
because two different callers need a cursor that outlives any single borrow: the bridge, where a
cursor is a JavaScript object with its own lifetime and a Rust reference exists only for the
duration of one `next()` call; and the differential fuzzer, whose harness holds both the structure
and a live cursor over it in one struct, which cannot compile if the cursor carries a borrow. The
core splits a cursor into closure state alone (the frozen payload, the current position) and a
convenience wrapper that pairs that state with a borrow for a Rust caller who wants a normal
`Iterator`. The detached form is the faithful primitive; the borrow-carrying form is built on it for
ergonomics. This shape does not cover pointer-chasing walks — a linked list or a trie, where a
cursor's position is a place in the structure's graph rather than an index — which need their own
walk type built on the same split.

**`Stack` keeps a captured length and a live size as genuinely separate tracked quantities**, because
upstream does: `values()` reads `items.length`, everywhere else reads `this.size`. The two coincide
on every path either test suite exercises, and normalising them to one field would be an unforced
assumption about behaviour nothing tests.

**The `Map`-backed structures — `default-map`, `bi-map`, `fuzzy-map`, `set`, `lru-map` and others —
are all built on one generic ordered map, and its cursor is deliberately not the same abstraction as
the sequence cursor above.** An `obliterator` cursor freezes a length and reads lazily; a `Map`
cursor owns its own entry list, skips tombstoned slots and sees appends made after it opened. Both
are faithful to different things a JavaScript object can do, and one abstraction over both would get
one of them wrong. The map's cursor locates its position by a monotonic slot id rather than a
physical array index, so that deleting an entry — which tombstones its slot and eventually compacts
the entry list, moving live entries — cannot invalidate a cursor holding an index into a table that
has since moved. Every slot carries an id that is never reused, the slot table stays sorted by id
across any number of compactions, and a cursor resolves its id with a validated index hint for the
common case and a binary search as the fallback.

Two alternatives were rejected. The first is **V8's own approach** — chain the old table to the new
and transition live iterators through a hole list. It is correct, and it is strictly more
bookkeeping: a slot id needs no communication between a map and its cursors at all, which is
precisely what leaves `MapCursor` `Copy` and impossible to invalidate. The second is **never
compacting**, which removes the problem by letting the entry list grow without bound under exactly
the delete-then-insert churn `lru-map` performs by design.

**`MultiMap`'s flattened cursor snapshots each bucket rather than reading it live**, and this is the
one place in the family where the port's behaviour and upstream's are known to disagree. Upstream
obtains, per key, either a genuinely live `Set` iterator or an array walk with the length frozen at
entry, so a mutation to the *very bucket the cursor is currently inside* is, in principle, sometimes
visible and sometimes not, depending on which container type backs that key. The port clones a
bucket's contents once, when the outer walk reaches that key, and iterates the clone — which
correctly reproduces the outer map's own liveness (a key deleted ahead of the cursor is skipped) but
not a mutation to the same bucket mid-inner-walk. Every case either original test file performs is
reproduced exactly; this one gap is untested by both suites and is stated rather than silently
accepted.

**`forEach` is a different kind of walk from the three lazy iterators it sits next to, because its
timing is different.** The lazy iterators — `keys`, `values`, `entries` — advance their internal
position and *then* hand control back to the caller. `forEach`'s loop body does the opposite: it
calls the callback first and only afterward reads the next pointer. The difference is invisible until
a callback mutates the very entry the walk is about to visit next — for instance, promoting an entry
to the front of an LRU list from inside its own `forEach` callback — at which point a walk built on
the lazy-iterator abstraction has already captured the old "next" pointer before the callback ran,
and reports a stale successor. One generic cursor cannot serve both timings, so `forEach` is its own
walk type with `current()` and `advance()` as two separate calls, letting a caller's mutation land
between them exactly where upstream's own loop body allows it to.

**A trie's lazy walk re-navigates by token path on every step, rather than holding a live reference
to the node it is about to visit.** This is the port's most structurally significant iteration
divergence, forced by the FFI boundary rather than chosen for its own sake. Upstream's `values`,
`prefixes`, `keys` and `entries` close over arrays of actual JavaScript node objects it has
discovered but not yet visited; deleting a word prunes a *parent's* reference to a node, which can
leave the node object itself, and any value still attached to it, completely untouched — an open
walk already holding a reference to that object keeps reporting its stale content. A Rust cursor
cannot hold a live reference across the FFI boundary at all: a JavaScript-side cursor object outlives
the single call that produced it, and the trie underneath it stays mutable and is handed back in as a
fresh `&TrieMap` on every subsequent call. The port's walk therefore stores the *token path* to each
pending node rather than a reference, and re-resolves that path from the root on every step; a path
that no longer resolves, because the node it names or an ancestor of it was pruned since the frame
was queued, is simply skipped. The two designs agree on every sequence either original test file
performs, and on every delete that does not happen to prune something an open cursor has already
queued — the case the two designs disagree on turned out to be reachable, not merely theoretical: an
early, ungated differential fuzz run diverged inside a few hundred operations on both `trie` and
`trie-map`, so the fuzz grammar now explicitly keeps a `delete`/`clear` operation out of the same
generated program as a persistent, still-open cursor, disclosed as a grammar split rather than
presented as coincidental coverage. What the two designs do agree on is pinned by a dedicated test: a
new entry added inside a branch an open walk has already queued *is* visible to that walk, matching
upstream.

---

## Re-entrancy

Upstream's comparators, factories, hash functions and distance functions are ordinary JavaScript
closures, and several of the original test suites already call back into the very structure that
invoked them — from a comparator, from a factory, from a `forEach` callback. Rust's usual aliasing
discipline (an exclusive `&mut` while an operation runs) makes that pattern impossible to express, so
each structure that admits it needed a specific design that permits the callback to observe or mutate
the structure mid-operation, rather than one that merely tries to survive it.

**A JavaScript array is a reference, and `clear()` rebinds it rather than emptying it in place.**
`Stack.prototype.clear` is `this.items = []`, a new array, not `items.length = 0`; `Queue`'s
compaction is likewise a fresh array via `slice`. Both cursors, though, captured the array *object*
at creation, so a rebinding after the cursor opened leaves it walking the old, now-detached contents,
while an in-place mutation like `pop()` is visible to it as an element becoming `undefined`. The
port's backing store is `Rc<RefCell<Vec<T>>>` specifically so that rebinding and in-place mutation
are distinguishable operations — a plain `Vec<T>` would make `clear()` and "remove everything" the
same operation, and both would have shortened an already-open walk, which is not what upstream does.
Mutators still take `&mut self`; this is not interior mutability adopted for convenience.

**Every bridge structure holds its core state behind a `RefCell`, because a bare `&self` is not
actually true at this boundary.** napi hands the same object to JavaScript as both `&self` and
`&mut self`, and JavaScript re-enters from inside a callback. Rust marks `&T` as `noalias readonly`
whenever `T: Freeze`, and the optimiser used exactly that license: a `forEach` callback that
compacted a queue mid-walk was invisible to the walk's remaining iterations, because LLVM had hoisted
the read of the backing array out of the loop on the assumption that nothing aliased through an
immutable reference could change it. A `RefCell` is not `Freeze`, so the assumption disappears; the
fix is the type holding the state, not a barrier instruction papering over one codegen decision.

**The rule that follows from that fix — no borrow may be alive across a call that can run
JavaScript — is necessary but was not, on its own, sufficient.** `self.items.borrow().allocate(0)?`
keeps the borrow alive for the whole statement, because a temporary lives to the end of the
*statement* it appears in, not the end of the call. Where that call itself reads a JavaScript
constructor property and invokes it, a re-entrant `clear()` reaching the following `borrow_mut()`
aborted the whole Node process with `SIGABRT` — a Rust panic across the FFI boundary is not a
catchable JavaScript exception, and napi does not catch a panicking synchronous call. Every method
that can call back into JavaScript now binds its borrow to a local first and clones out of it before
making that call, rather than chaining a borrow directly into one.

**`heap` and `fixed-reverse-heap`'s sift and consume algorithms take an abstract store rather than
`&mut Vec<T>`, because upstream's own comparator is arbitrary code invoked from inside the loop, with
no defence against it calling back into the heap it is sorting.** A comparator can legally call
`heap.push()` or `heap.clear()` mid-sift, and upstream has no error path for this — it simply
proceeds, reading whatever the array now contains. An exclusive `&mut Vec<T>` is precisely the thing
such a re-entrant call would have to violate, so the natural Rust signature makes upstream's own
behaviour inexpressible rather than merely awkward to write. The store the algorithms address
instead is borrowed and released around each individual comparison, so a comparator that grows,
shrinks or clears the backing array between comparisons is legal, matching upstream rather than
panicking. The heap's `items` is, at the bridge, a real JavaScript array reached through an owning
napi reference rather than a materialised `Vec` — forced independently by three things at once:
upstream's own static `heapify` mutates the caller's array in place, `FixedReverseHeap` must return
something satisfying `instanceof` for whatever array class it was built with, and a comparison is a
call into JavaScript regardless, so the boundary is already being crossed on every comparison. This
also buys the typed-array store-narrowing semantics (`push(300)` on a `Uint8Array` keeping `44`) for
free, and it extends the re-entrancy guarantee to the array itself: a `Proxy` trap or accessor on the
array runs exactly where upstream's would.

**`BitVector` and `Vector`'s growth policy is the one place the "release the borrow before calling
JavaScript" rule cannot be met structurally, and the port refuses the call rather than serving it
half-built state.** The growth policy is a JavaScript function that core calls from *inside* `grow`,
so the methods that can trigger growth genuinely hold the vector's state for the whole policy call.
A policy that calls back into the same vector meets that outstanding borrow and receives a named,
catchable error instead of a result. Upstream would instead serve such a call from a vector mid-grow
and return whatever half-built state it finds — this is narrower than upstream's behaviour, and is
recorded as a narrowing rather than presented as equivalent. Resolving the policy before taking the
borrow was considered and rejected: core's `grow` calls the policy again even when handed an explicit
target capacity, so avoiding a second call would mean the bridge deciding whether to grow at all,
duplicating logic that is supposed to live only in core. Taking the vector out of its cell for the
policy's duration was also rejected: a re-entrant read would then see an empty vector and return a
silently wrong answer, which is worse than a loud, catchable one.

**`bk-tree`'s distance function meets the identical shape and is refused the identical way.** The
distance function is called from inside both `add`'s descent and `search`'s traversal, holding the
bridge's borrow for the whole call; a distance function that calls back into the same tree gets a
catchable error rather than being served a half-traversed tree.

**A factory or a `forEach` callback that calls back into the same `DefaultMap` was, for a time,
explicitly unsupported — and then it stopped needing to be, once the `RefCell` redesign landed for an
unrelated reason.** The original position was that supporting it would mean interior mutability
throughout the bridge, which was a decision for the whole crate rather than one module. That decision
was then forced anyway, by the soundness fix above, which turned out to be the identical exposure
manifesting as a miscompilation rather than merely as an aliasing hazard. Once every bridge structure
held a `RefCell` and re-borrowed per step as a matter of course, `forEach` re-borrowing between
callback invocations and `get` running its factory between the read and the write — which is where
upstream runs it too — came along for free. A re-entrant factory or callback now behaves like
upstream's.

**`VPTree` needs none of this, because nothing about it has shared mutable state to protect in the
first place**, and the contrast is itself informative: see "Where the port is more correct than
upstream" below for what that buys and what it costs.

---

## JavaScript number semantics

JavaScript has one numeric type, and its relational operators, its bitwise operators and its
typed-array element stores each apply their own implicit coercion. A comparator can legally return
`NaN` or a fractional value; a bitwise operator's first step is always a 32-bit truncation nobody
writes out; a `Uint8Array` store silently narrows whatever is assigned to it mod 256. None of these
map onto a single obvious Rust type, and each site where the port had to choose is a place a literal
translation would have quietly repaired something upstream leaves alone.

**A comparator returns `f64`, never `Ordering`.** Upstream performs exactly three tests on a
comparator's return value — `< 0`, `> 0`, `>= 0` — on whatever came back. `Ordering` has three values
and cannot represent an inconsistent answer; collapsing to it would silently *repair* a comparator
upstream would happily misbehave with. `Comparator::compare` returns a numeric result, coerced at the
bridge exactly as upstream's relational operators coerce it — including a `BigInt`, whose sign is
read directly rather than going through `ToNumber`, which would reject it.

**`DEFAULT_COMPARATOR`'s two relational tests are ported directly; the tests themselves are
delegated to the engine rather than reimplemented.** Comparing two arbitrary JavaScript values with
`<` runs `ToPrimitive`, which can call a user's `valueOf` or `toString` and can throw. Reimplementing
that in Rust would be a port of V8's coercion rules, not of `mnemonist`, and would be wrong in ways
nothing in this repository could detect. Native number-against-number and string-against-string
comparisons — including `NaN` and UTF-16 code-unit ordering — are handled directly; anything
involving an object, a symbol, or a mixed pair is delegated to a two-line comparison compiled once
and cached.

**Indices and counters that JavaScript holds as a plain number are `i64`, not `usize`, wherever the
arithmetic upstream performs on them can go negative or fractional.** `bit-set`'s indices are a
worked example beyond the general principle: every use upstream makes of an index is inside a
bitwise expression, so a negative index produces a negative word index and the operation is silently
dropped — a `usize` coercion would instead turn `set(-1)` into `set(4294967295)`, which happens to
match upstream's outcome for `set` by accident and does not match it at all inside `rank`'s
134-million-iteration inner loop. `bit-set`'s own `size` field is `i64` for the same reason as
`FibonacciHeap`'s (see "Where the port is deliberately less correct"): an upstream defect can drive
it negative, and a `usize` cannot hold the state upstream actually reaches.

**Typed-array *write* truncation has to be modelled in addition to typed-array *width* selection.**
Choosing the right backing width for a typed array is only half of upstream's semantics; writing to
it silently narrows the stored value mod the array's element width, and this is independently
reachable: a rank-comparison bug in `StaticDisjointSet.union` (recorded separately as an upstream
defect) leaves non-root ranks permanently at zero, so the tie-break branch that increments a rank
fires on nearly every merge — far past what the array's width was originally sized for — and a
naive `Vec<u32>` backing would have silently disagreed with upstream the first time a rank wrapped.
The port's typed backing masks every write to the selected width, and a regression test pins the
exact input (a 300-element set, unioned so the wrap is reachable) against Node's own answer.

**A non-integral capacity always raises the plain-`Array` form of upstream's error, even for a typed
array class where upstream itself does not raise at all.** Upstream passes a capacity straight to
`new this.ArrayClass(capacity)` and lets the class decide: `new FixedStack(Array, 2.5)` throws, but
`new FixedStack(Uint8Array, 2.5)` succeeds, leaving a `capacity` of `2.5` compared against every
following index for the deque's wrap arithmetic. A capacity that is not an integer cannot be a
`usize`, and carrying an `f64` capacity through wrap arithmetic that a `usize` was built to make
exact would buy nothing no test asks for; the `Array` case, which every original test exercises, is
reproduced exactly, and the typed-class case is the one that is not.

**The typed-array pointer-width helper takes an `f64`, because upstream itself coerces its one
argument two different ways at two different call sites.** `getPointerArray` compares `length - 1`
as a double, while the class constructor it eventually reaches truncates through `ToIndex`, so a
length of `256.5` produces a `Uint16Array` — one width wider than 256 elements actually need — and
a length of `-0.5` produces an empty `Uint8Array` while `-1` throws. A first draft that took a
`usize` and let the caller truncate produced the narrower, wrong width; the fuzzer's own
falsification seed pins the corrected behaviour now.

**Tracked counts are `f64`, including the exact repeat-count a fractional value produces.**
`MultiSet`'s only guard on a stored count is `typeof count !== 'number'`, so a fractional count is
legal and left as-is; the loop that repeats a value `multiplicity` times compares an integer step
counter against the raw float with `<`, which yields `ceil(multiplicity)` repeats for a non-integer
bound. The port stores counts as `f64` throughout and compares the same way, rather than rejecting a
fractional count or rounding it to something upstream never produces.

**`heap`'s `n` parameter to `nsmallest`/`nlargest` is carried as the raw `f64` it is and never
validated up front**, because upstream never validates it either — it is compared, sliced with, and
used directly as a loop counter, so a fractional `n` makes the scan read fractional indices, which
are always `undefined`, and the loop does nothing. The one guard upstream does have —
`new Array(n)` on its one array-allocating path — is raised from the same place, as the same real
error, so a caller who reaches that specific path sees upstream's own rejection.

**The k-way merge and union functions pick their next value with a real Fibonacci heap, not a linear
scan, once a Fibonacci heap existed to build on.** Upstream's own `kWayMergeArrays` uses a real heap,
and which of several equal-valued array heads it extracts first is a genuine artifact of that heap's
internal tree shape — driven by push's tie-break rule and, after the first extraction, by
consolidation's degree-bucket merging — not by insertion order alone. An earlier version of the port
used a plain linear scan that kept the earliest array on a tie, which matched on every case the
original suite reaches but diverged on a three-array case a differential campaign found once the
grammar was widened to a small, repetitive, tie-producing value pool. Once the Fibonacci heap was
itself a ported structure, the k-way scan was rebuilt to drive a real instance of it, holding array
indices and comparing through upstream's own inline tie-break closure translated directly — which is
upstream's algorithm, not a second substitute for it.

---

## Errors in place of undefined-driven cascades

A recurring situation: upstream reads or writes past the end of an internal array, gets `undefined`,
and lets that `undefined` propagate through further arithmetic until it produces a crash, a silently
wrong answer, or nothing observable at all, several calls away from the actual cause. Rust's own
bounds checking makes the same *mechanism* impossible without `#![forbid(unsafe_code)]` giving way,
so each such site was judged on whether upstream's behaviour is expressible at all. Where every step
is a well-defined typed-array read, a truncating store, or a silently dropped store — `SparseSet`'s
out-of-range membership, for instance — the port reproduces the corruption exactly, because Rust can
express it directly. Where the cascade instead runs through `NaN` arithmetic on an array index, there
is no honest Rust value that plays the role `NaN` plays there, and the port raises a named, catchable
error instead of reproducing a mechanism that would otherwise require an actual out-of-bounds panic
crossing the FFI boundary — which aborts the whole Node process, a worse outcome than the JavaScript
exception it would stand in for. `StaticDisjointSet`'s out-of-range union arguments, `SparseMap` and
`SparseQueueSet`'s constructor length overflow, `StaticIntervalTree`'s and `VPTree`'s and `KDTree`'s
queries against an empty structure, `FixedCritBitTreeMap`'s capacity overflow, and the k-way
merge/union path's stale-length condition all take this shape: the *outcome* — construction or the
operation fails, with a message that is upstream's own wording where upstream's own wording exists —
is reproduced; the *mechanism* — an actual read of `undefined`, an actual `NaN` propagating through
array-index arithmetic — is not, because Rust has no honest way to produce it without a panic
upstream does not have.

---

## Absence: `undefined`, `null`, and unreachable identity

JavaScript's `undefined`, `null` and "no such key" are three states a `Map` can distinguish and a
plain value cannot, and the boundary between Rust and JavaScript has to decide, per field, which of
Rust's narrower vocabulary — `Option`, an enum, a rejection — stands in for which.

**Primitive values are stored by value; objects, functions and symbols are stored by reference**,
forced twice over. `napi_create_reference` rejects a bare number at the Node-API version this addon
declares — measured directly, it was what made two of seven upstream assertions fail on the bridge's
first working run — and it is independently the right design regardless: a reference is a V8 global
handle, and one per stored value would mean a million live handles for a million-entry cache, against
upstream's inline small integers. Nothing is observable either way, because a JavaScript primitive
has no identity: `0 === 0` and `'a' === 'a'` regardless of where either came from, and `-0` and `NaN`
survive a round trip verbatim because only *key* identity, not stored values, is ever normalised.

**`undefined` is spelled `None`, consistently, across every structure that can hold an absent
value.** This is what makes an upstream defect expressible at all in one prominent case: a `Map`'s
own `get` cannot distinguish "no such key" from "this key's stored value happens to be `undefined`",
and several of upstream's own methods — `peek`, `has` used alongside `get` — inherit that same
inability. Storing `Option<V>` and letting `None` mean `undefined` reproduces that limitation for
free rather than requiring a redesign to introduce it.

**A cursor step that must yield `undefined` without ending the walk is `Either<T, Undefined>`, never
`Option<T>`.** The natural Rust choice, `Option<T>`, does not work: napi renders `Option::None` as a
JavaScript `null`, and `null` is not `undefined` under `assert.deepStrictEqual`. `Either`'s
`undefined`-carrying variant is a real `undefined`, and using it frees `Option<Step>` to keep its own
ordinary meaning elsewhere, where `None` genuinely means "the walk is done." This shows up wherever a
structure's internal length can outrun what a cursor was told to expect: `SparseSet` and, for most of
its cursors, `SparseMap` (its `entries()` cursor is the one exception — it yields a whole
`[key, value]` array, so a missing half is `undefined` *inside* the yielded array rather than a gap
in the step itself, and `Option::None` keeps its plain meaning there), `bit-set`'s `select`,
`hashed-array-tree`'s `get`/`pop`, and `sparse-queue-set`'s `dequeue`. Every Rust-side `Iterator`
implementation built on one of these steps skips the gap rather than stopping at it, because Rust has
no `undefined` value to hand a caller, and stopping early would turn a shrink into an early
termination — exactly the divergence this design exists to avoid. The three-state primitive is the
faithful one; the ordinary `Iterator` is a convenience layered on top of it.

**Object identity as a `Map` key, a `Set` member or a `WeakMap` key is refused loudly rather than
modelled.** A `Map` compares object keys by identity, and no identity hash for a JavaScript object is
reachable from Rust; two designs would work — a hidden tag written onto the object via a private
symbol, or a linear-scan association list of references probed with strict-equals — and each costs
something real: the first mutates a caller's object and fails on a frozen one, the second is O(n) per
lookup and retains a strong reference to every key it has ever seen. No test anywhere in the entire
`Map`-backed family constructs an object key, audited across every test file in that family, so
building either mechanism would be machinery no test could exercise, judged worse than a stated
limit. `default-weak-map` inverts the same argument in the one place it has to: there, object
identity is the entire point, so it is a real linear scan over live weak references, and it is
*function* and *symbol* keys — which a genuine `WeakMap` also accepts — that are refused instead,
because no test constructs a key any way but a bare object literal. `set.js`'s own members and
`trie-map`'s array-mode tokens (coerced with a plain string conversion, not upstream's full
property-key coercion, so a symbol token is rejected rather than silently accepted unchanged) make
the identical call for the identical reason, in each case audited against the specific suite that
would have to exercise it for the limit to matter.

**An omitted trailing argument and one explicitly passed as `undefined` are indistinguishable to a
typed napi signature, where upstream's `arguments.length` tells them apart.** napi-derive generates
constructors and methods that do not enforce arity, so a missing argument and an explicit `undefined`
argument both arrive as the same value — where upstream, checking `arguments.length` directly,
raises a different error for each. What is *not* lost in every case this affects: an explicit `null`
is still distinguished correctly, because napi maps `null` to a genuinely distinct value at the type
level and the original suites separately assert that a `null` in the same position raises upstream's
type error, not its arity error. This shows up in two shapes. The first is `forEach`'s optional
`this`-binding argument, which upstream binds only when a second argument is actually supplied
(`arguments.length > 1`): the port always binds `this` to the collection when the argument is
omitted, which is the only form any original test uses, across `bit-set`, `default-map`, `stack`,
`fixed-stack`, `queue`, `sparse-set`, `sparse-map`, `sparse-queue-set` and `linked-list`. The second
is a handful of constructors and one method whose behaviour genuinely depends on whether a particular
argument was supplied at all rather than on its value: the whole fixed-capacity family's arity
guard, `SparseMap`'s constructor (which upstream reads differently depending on whether a second
argument exists), `HashedArrayTree`'s constructor (which upstream leaves an `ArrayClass` of
`undefined` and only fails later, if the tree ever allocates), `FixedReverseHeap`'s constructor, and
`heap`'s `nsmallest`/`nlargest`, whose two- versus three-argument forms upstream distinguishes the
same way.

**`default-weak-map`'s `get` checks its key's type before running the factory; upstream runs the
factory first and only fails afterward, inside a real `WeakMap.prototype.set`.** Verified directly:
`get(1)` on a fresh map calls the factory — with whatever side effects it has — and only then throws,
because `this.items.get(key)` never throws for any key shape and only the eventual `set` does.
Reproducing that exact order would mean calling the port's typed factory closure with a value its own
signature has no slot for. No test anywhere calls `get` with a non-object key, so no test — original
or the port's own — reaches the ordering distinction; every other path (`peek`, `has`, `delete`,
which genuinely never throw for any key type, and `get`/`set`, which eventually throw the identical
message) matches exactly.

---

## Fidelity costing idiom

`docs/ARCHITECTURE.md` names six specific sites where reproducing upstream cost the port real Rust
idiom — a counter tracked rather than derived, a signed size where an unsigned one would be tidier,
an arena that never recycles a freed slot, a linear scan in place of a hash table, error text pinned
to upstream's exact wording, and a trie node's children kept in one insertion-ordered list rather
than a map — each described there alongside why the idiomatic alternative would have silently
repaired something upstream leaves broken. They are not repeated here; the section below extends the
same posture to entries that table does not cover.

---

## Where the port is deliberately less correct than it could be

**`BiMap` carries two real, independently stored size counters, not one value derived from its
backing maps**, and this is the entry that makes the point most clearly, because it took three
attempts to land. Upstream's `clear()` resets only the one counter belonging to whichever side calls
it, leaving the other stale until the next real mutation resynchronises it — except a subsequent
no-op delete of an absent key, which returns before touching either counter and leaves the staleness
in place for one extra operation. A first draft derived both counters directly from the underlying
maps' own lengths, which incidentally healed the desync on every `clear()` — more correct than
upstream, and therefore a defect under a bug-for-bug contract — caught by the differential fuzzer in
eighteen generated cases. A second draft added real stored counters but resynchronised them
unconditionally after every mutation, which re-healed the specific no-op-delete-after-clear case the
first draft had also (differently) gotten wrong; caught on the very next campaign, in a different
handful of cases. The version that shipped resynchronises only when a mutation actually removed
something, matching upstream's own conditional exactly rather than a tidier approximation of it.

**`LRUCache`'s eviction report drops a falsy evicted key on purpose, at the bridge, though the core
does not.** Upstream's `setpop` decides whether to report an eviction with `if (oldKey)` — plain
JavaScript truthiness, not a check for whether an eviction happened — so a key of `0`, `''`, `false`,
`NaN`, `null` or `undefined` that really was evicted is reported identically to no eviction at all.
The core's own `set_pop` has no concept of JavaScript truthiness and correctly reports every eviction
regardless of key, which is more correct than upstream and is therefore, under this port's
bug-for-bug contract, itself a defect rather than an improvement. The bridge reintroduces the bug
deliberately, with an explicit truthiness check on the evicted key gating whether the eviction is
reported at all — the one place in the port where a JavaScript-shaped falsity check is written by
hand specifically to make the Rust core's more-correct answer wrong again.

**`LRUCache`'s freed pointer slots are left stale on delete, not defensively cleared, and an early
draft that cleared them was actively worse.** Upstream's `delete`/`remove` only splice the linked
list and record the freed slot for reuse; neither ever touches the freed slot's stored key or value.
An early version of the port cleared both on unlink, which is the instinctively "safer" thing to do
and is exactly the kind of change this project's rules call out as a different, undocumented
contract rather than an improvement: a `keys`/`values`/`entries`/`forEach` walk whose frozen bound
had not yet reached a pointer, when a `delete` unlinked precisely that pointer out from underneath
it, then hit an internal invariant the port had asserted — a pointer reachable from the list head is
always live — which was no longer true, and panicked. The shipped version does not null either slot,
matching upstream, and a return method that used to move the value out now clones it instead so a
second read of the same stale slot does not observe a double-take. The result is that a stale walk
racing a delete can observe debris or a reused pointer's new occupant, exactly as upstream's own
algorithm cannot tell "stale" from "reused" apart either — the divergence in the *earlier* draft was
the one closer to a defect, and reproducing upstream's carelessness is what removed it.

---

## The `intersectionUnique` `NaN` gap

`intersectionUnique`'s k-way path is a known, currently open gap in the port's own correctness, not
a reproduction of anything upstream does. Upstream's `kWayIntersectionUniqueArrays` seeds its running
lower and upper bounds from JavaScript's `-Infinity`/`Infinity` sentinels; the port seeds the
equivalent accumulator from `Option<T>` instead, since there is no generic `-Infinity` to seed a
running fold from without a sentinel mechanism built for a running accumulator rather than a
per-slot value, which is the shape every sentinel type this port already has was built for. The
practical effect: where a `NaN`-headed array leaves upstream's sentinel bound untouched until a
later, non-`NaN` array supplies a real one, the port's `Option`-seeded accumulator is set by the
*first* array scanned regardless of whether it is `NaN`, and the two can disagree — confirmed
directly: `intersectionUnique([-1], [NaN], [-5])` returns `[-5]` from the port and `[]` from
upstream. This gap predates and is unrelated to the port's later work rebuilding the k-way
merge/union tie-break on a real Fibonacci heap, which `intersectionUnique` never used a heap for in
the first place and which that work therefore has nothing to say about. Because the gap was already
known, the differential fuzz campaign for `intersectionUnique` specifically excludes `NaN`
generation for this one function's k-way path, so its campaign is green over a region that
deliberately excludes a known disagreement rather than being green because the disagreement was
never generated. Every other k-way-capable function's campaign generates `NaN` normally.

---

## Where the port is more correct than upstream, and why that stays disclosed

A port that is more correct than upstream is, under this project's own bug-for-bug contract, a
defect rather than an improvement — but three cases are unavoidable or so nearly free that reproducing
upstream's version would cost real Rust soundness or idiom for a state nothing exercises. Each is
recorded rather than silently kept.

**`VPTree`'s distance function, if it recursively calls back into the same tree, sees independent,
correct state on each call — where upstream's own single, reused heap and counter fields would
interleave the outer call's in-progress state with the inner call's and produce whatever upstream's
un-arbitrated interleaving happens to leave behind.** Every query in the port builds its own heap and
traversal stack locally, and the core type's query methods take a shared reference rather than an
exclusive one, because there is no mutable tree-wide state to protect in the first place — the direct
consequence being that no bridge-side `RefCell` is needed for this structure at all, unlike `bk-tree`,
whose `add` does need exclusive access and does need one. No test, upstream's or this port's fuzz
grammar, inspects the shared fields directly, so this is unreachable through any instrument currently
in place; it is recorded here anyway rather than left implicit.

**`PassjoinIndex`'s inverted-index key is a real tuple, not upstream's string concatenation, which
can in principle collide on inputs neither the original suite nor the port's own differential fuzz
campaign has ever produced.** Upstream builds its key by concatenating a segment string directly with
a segment index, so two distinct pairs could in principle produce the same key — segment `"1"` at
index `2`, for instance, alongside a segment `"12"` at an index whose digits happen to complete the
same string. A tuple key cannot collide the way string concatenation can, so the port's candidate set
can only ever be a superset of upstream's on the inputs where upstream's own scheme is ambiguous,
never a subset — meaning this cannot manifest as a match the port misses that upstream would have
found. It is disclosed because this is exactly the failure mode this port's own porting rules warn
is easiest to introduce unnoticed: the one place a superset is even theoretically possible, named
rather than left to be discovered.

**`sort`'s helpers accept only numeric elements, which sidesteps two of upstream's own re-entrancy
defects by refusing the only inputs that could ever reach them, rather than fixing them.** Upstream's
sort routines are duck-typed over anything indexable, comparing elements with plain relational
operators that coerce through `valueOf`/`toString` — which means calling into arbitrary user code
from inside the sort loop. Two real defects live behind that: an undeclared loop counter that a
re-entrant comparator corrupts, and a partition stack held as shared module state that a re-entrant
sort corrupts a concurrent one's bounds with. Every element in every original test, and every element
either of `mnemonist`'s own callers ever pass, is a number. With only numeric elements, no user code
ever runs during a comparison, so neither defect's trigger condition — a comparison that runs
arbitrary code — exists in the port at all. This is not upstream's bug fixed; it is the input that
could observe it refused, and reproducing either defect bug-for-bug would mean building the callback
machinery needed to reach arbitrary comparisons in the first place, purely to reintroduce a hazard
nothing in scope needs.

---

## Reproducing JavaScript's own irregularities

A handful of upstream's own behaviours are irregular by JavaScript's own standards — a constructor
and its prototype sharing one object, a static method that exists only because `arguments` does —
and reproducing them exactly needed a JavaScript-shaped mechanism rather than a native Rust one.

**`X.of` and `.from(iterable)` for the fixed-capacity family are installed as evaluated JavaScript
literals, run once at module load, rather than native methods**, because napi-rs has no variadic
parameter and no representation for `arguments`. A native implementation would behave identically for
every case any test reaches — deleting the branch that specifically detects an `arguments` object and
letting it fall through the ordinary iterable path leaves every original assertion green, since a
modern `arguments` object is itself iterable — so the reason to keep the evaluated-JavaScript form is
that it is upstream's own literal definition and keeps the addon self-contained, not a coverage
argument.

**`MaxHeap` and `MaxFibonacciHeap` are installed the same way, prototype-sharing included, so
`instanceof` still cannot distinguish the min-heap from the max-heap variant, matching upstream's own
confusion rather than repairing it.** `MaxHeap.prototype = Heap.prototype` is the same object, not a
derived one, so `new Heap() instanceof MaxHeap` is true and the two constructors are indistinguishable
by type at runtime; `MaxFibonacciHeap` does the identical thing one file over. A second native class
for either would have its own distinct prototype object and would silently repair the blur — bug-for-
bug fidelity here specifically means reproducing the type confusion, and sharing one prototype object
is the only way to reproduce a shared prototype object.

**`Heap`'s ten static factory methods are declared on a separate shadow class, copied onto `Heap` at
module load, and then deleted from the addon's own exports.** Upstream carries both
`Heap.push(compare, heap, item)`, a static, and `Heap.prototype.push(item)`, an instance method, with
no conflict in JavaScript, where a constructor and its prototype are different objects. napi-rs
registers a class's statics and its prototype methods through one shared name table, so declaring
both under one class silently drops the prototype half — measured directly: nine of fourteen original
test cases failed with `heap.push is not a function` before this was found. Two residual statics
survive on the constructor and cannot be deleted, because napi declares a class's own properties
non-configurable: they are non-enumerable and are the bridge's only addition to upstream's surface,
noted rather than hidden.

**napi's generator `#.return` is deleted from the five sequence classes' cursors, because upstream's
own cursors have none — and it is *not* deleted from the other 47, which is a known gap.**
`obliterator`'s iterator type defines a constructor and a self-returning `Symbol.iterator` and
nothing else, so breaking out of a `for...of` loop over one leaves it exactly where it stopped,
resumable by a later `next()`. napi's generated iterator installs `return` as an own property, and
that `return` sets an internal generator-state flag *before* any Rust-side completion logic runs, so
nothing on the Rust side can prevent it from taking effect. Deleting the property is the only fix,
because `IteratorClose` does `GetMethod(iterator, "return")` and skips one that is absent.

The addon applies that deletion through a table in `crates/mnemonist-napi/src/statics.rs`, and the
table lists ten cursor factories: `values` and `entries` on `Stack`, `Queue`, `FixedStack`,
`FixedDeque` and `CircularBuffer`. The port hands out **57** generator-backed cursors in total. On
the remaining 47 — `SparseSet`, `BitSet`, `SparseMap`, the `Map`-backed family and the rest — a
`break` runs `IteratorClose`, latches the generator, and a later `next()` answers `{done: true}`
where upstream would resume. Measured, not inferred:

```
Stack.values       return absent   after break, next() -> 2           (matches upstream)
SparseSet.values   return present  after break, next() -> done: true  (diverges)
```

The fix is two rows per class in that table and no new mechanism. It was applied to the five classes
that were in scope when the latching was discovered and not extended afterwards, which is an
omission rather than a decision. It is recorded here because no gate detects it: the upstream suites
never resume a cursor after `break`, and the differential fuzzer compares the native crate, where no
`return` exists to latch — the layer gap described in `docs/METHODOLOGY.md`, with a live instance.

---

## Scope cuts

Each row below is upstream behaviour the port does not implement, because no test in either the
original suite or this port's own campaigns reaches the state where the two would visibly disagree.
Every one was a choice, not an omission: implementing machinery nothing exercises is worse than a
stated limit, and answering silently and wrongly is worse than both.

| What is cut | Where | Why |
|---|---|---|
| A structure's internal typed-array backing (`dense`/`sparse`/`vals`, `blocks`, `tree`/`augmentations`, `items`) is handed to JavaScript only as a copy, never as a live, write-through reference | `sparse-set`, `sparse-map`, `sparse-queue-set`, `hashed-array-tree`, `static-interval-tree`, `fixed-deque`, `fixed-stack` | napi can only hand out a copy of a Rust `Vec`; exposing one would silently break the write-through a real caller could otherwise rely on. Verified instead by the differential fuzzer, which compares the real backing store slot for slot after every operation. `bit-set`'s `array` is the one such field an original test reads directly, and is exposed, as a copy. |
| `.inspect()` (and, for `linked-list` and `circular-buffer`, the accompanying constructor-name trick for Node's REPL) is not ported | Nearly every module with one upstream | A Node display convenience with no upstream assertion anywhere. |
| A structure's own mutating method returns `bool`/`usize` in core (whether an insert was new, whether a merge happened) but `this` at the bridge, matching upstream's chaining API | `sparse-set`'s `add`, `sparse-map`'s `set`, `sparse-queue-set`'s `enqueue`, `static-disjoint-set`'s `union` | Upstream exposes the underlying fact only indirectly, through `size`/`dimension`; the bridge drops the richer core return value so the JS-visible surface matches exactly. |
| Only a subset of upstream's constructible `ArrayClass`/`Container`/`Values` combinations are modelled; the rest are refused with a message naming the supported set | `vector` (4 of roughly 15 classes upstream would accept), `hashed-array-tree` (3 typed-array widths), `sparse-map`'s `Values` (4 unsigned widths), `multi-array` (2 of 4 `(Container, capacity)` combinations — the other two are unsupported in any useful sense upstream either) | The original suites exercise exactly the modelled combinations. Silently reinterpreting an unmodelled one would be a wrong answer dressed as a right one. |
| A growth/reallocation policy is a boxed JS-callable closure returning `Option<f64>`, with a thrown exception parked and re-raised at the bridge and a `NaN`/`Infinity` result refused rather than attempted as an allocation | `bit-vector`, `vector` | Upstream's own guard (`typeof !== 'number'`) does not catch `NaN`, which then flows into an allocation of `NaN` elements — there is no honest Rust expression of that allocation, so it is refused catchably instead, the same call made for out-of-range reads elsewhere in this document. |
| A diagnostic property with no test reading it is not exposed | `vp-tree`'s per-query distance-call counter, `kd-tree`'s `axes`/`labels`/visited-node counter, `heap`'s and `fixed-reverse-heap`'s `this.comparator` | No upstream test reads any of these; synthesising a JS value to satisfy an unread getter would be a fabrication rather than an answer. Equivalent measurements, where useful, are taken directly inside the fuzz harness instead. |
| A constructor overload upstream exposes is not reachable directly, only through its documented factory methods | `kd-tree`'s raw `(dimensions, build)` constructor | No test or call site anywhere constructs one directly; every use goes through `.from`/`.fromAxes`. Exposing it honestly would require a caller to hand-assemble an internal shape nothing else in the ecosystem produces. |
| String indexing is over Unicode scalar values (Rust `char`), not UTF-16 code units | `symspell`, `passjoin-index` | The two schemes agree exactly within the Basic Multilingual Plane — the whole alphabet either suite or this port's fuzz grammar uses — and diverge only for characters outside it, which neither exercises. |
| Keys are truncated to one byte per UTF-16 code unit before reaching the generic critical-bit core | `critbit-tree-map`, `fixed-critbit-tree-map` | Correct for every key either suite supplies (all below code point 256, where the truncation is a no-op); upstream's own masking already mishandles wider code units in a way this port does not attempt to reproduce, since nothing tests it. |
| Only `String` input is accepted; upstream's array-like-of-characters form is not | `passjoin-index`'s `add`/`search` | No test passes anything but a string. |
| An object key given to `.from`/index construction is rejected rather than run through full property-key coercion | `lru-cache`'s object-backed pair, `trie-map`'s array-mode tokens (a `Symbol` token is rejected rather than accepted unchanged) | No test in either family ever supplies anything but a string or number in that position. |
| `Container` values other than exactly `Array` or the global `Set` are all treated as list-like, and a list bucket always materialises as a plain `Array`, never a caller's custom class | `multi-map` | Upstream's own write path takes the identical line for every non-`Set` container; no test asserts on the returned bucket's class, only on its contents. |
| The mismatched return-value convention on the sign-flip delegation branch (`add(x, -3)` returning `undefined` where `add`'s normal path returns `this`) is not modelled at the bridge | `multi-set` | No test reads either method's return value; the differential fuzz spec, which does compare raw return values, models the real asymmetry instead — required once a generated case turned it up. |
| Object-identity deduplication for `Set`-kind buckets is not exercised by the differential fuzzer | `fuzzy-multi-map` | Comparing against real upstream JavaScript is a core-vs-upstream protocol; object identity is entirely a bridge concern one layer outside that comparison. Covered instead by the original suite and a dedicated bridge-level test. |
| The three-argument boolean-shift form of `.from` (`Container` silently reinterpreted as a `useSet` flag when the third argument is a boolean and no fourth is given) is detected by argument shape, not by counting real JavaScript arguments | `fuzzy-multi-map` | Indistinguishable from upstream's own check for every call any test makes; the only constructible disagreement is not exercised anywhere. |
| Upstream's three reference-identity shortcuts (skipping a membership check when two arguments are literally the same object) are implemented in core, reachable from a Rust caller, and unreachable from JavaScript | `set`'s `intersection`/`isSubset`/`intersectionSize` and siblings | The bridge reads two JavaScript arguments into two independent Rust structures, so identity between the two originals is lost before core ever sees them. Demonstrated rather than merely asserted: passing one object twice through the bridge is shown to be unobservably different from the shortcut firing. |
| A feature-detection flag from upstream's own support-detection module is hardcoded true rather than checked at runtime | `obliterator`'s `ARRAY_BUFFER_SUPPORT`/`SYMBOL_SUPPORT` | Both hold on every Node version this project's harness supports; the branch they guard is dead code here. |
| `iter`'s narrower iterability than `forEach`'s is reproduced as upstream's own asymmetry | boundary layer shared by every structure | `iter.js` has no `forEach`-delegation branch and no plain-object branch, so the same input throws through one helper and iterates through the other. Fixing the inconsistency would be a silent behavioural change to a real upstream quirk, not a bug in this port. |

---

## What this document does not cover

Divergences that are purely about the test harness or the differential fuzzer itself — how the Node
oracle is driven, how percentiles are computed, how a JSON float parser's own rounding was found to
manufacture a false divergence, how a fuzz batch's case count was found to be counting replays rather
than new programs — are process and tooling decisions, not divergences between the port and
upstream `mnemonist`.

They are documented, in two places rather than this one. `docs/METHODOLOGY.md` covers the gates and
the instruments' own failures, including the float parser and the replay-inflated case count.
`bench/methodology.md` covers the benchmark harness end to end: what is compared and what is
deliberately not, how percentiles are computed once in the driver over both sides rather than twice
and hoped to agree, and the checksum both sides must produce before any result is written.
