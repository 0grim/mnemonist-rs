// Shim standing in for upstream `heap.js`.
//
// The original test file does `require('../heap.js')`, so a module has to exist
// at this path inside the assembled work tree. It resolves the native addon
// through `@port/addon`, which is published into the work tree's node_modules
// so the lookup is depth-independent.
//
// Note what is NOT here. Upstream's `heap.js` ends with a block of load-time
// assignments -- `Heap.siftUp`, `Heap.push`, …, `MaxHeap.prototype =
// Heap.prototype`, `Heap.MinHeap`, `Heap.MaxHeap` -- and every one of them is
// performed by the addon itself when it loads. The raw-array statics are
// `#[napi]` static methods on the class; `MaxHeap` and the two aliases are
// installed from `crates/mnemonist-napi/src/heap.rs`, because
// `MaxHeap.prototype = Heap.prototype` is a semantic (it is what makes every
// Heap an `instanceof MaxHeap`) and a shim that added semantics would mean the
// addon was incomplete without the test harness.
module.exports = require('@port/addon').Heap;
