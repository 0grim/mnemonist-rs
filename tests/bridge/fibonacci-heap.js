// Shim standing in for upstream `fibonacci-heap.js`.
//
// The original test file does `require('../fibonacci-heap.js')`, so a module
// has to exist at this path inside the assembled work tree. It resolves the
// native addon through `@port/addon`, published into the work tree's
// node_modules so the lookup is depth-independent regardless of how deep a
// shim sits (see `tests/bridge/heap.js` and `tests/bridge/utils/*.js`).
//
// Note what is NOT here. Upstream's `fibonacci-heap.js` ends with its own
// load-time assignments -- `FibonacciHeap.MinFibonacciHeap`,
// `FibonacciHeap.MaxFibonacciHeap`, and `MaxFibonacciHeap.prototype =
// FibonacciHeap.prototype` -- and every one is performed by the addon itself
// when it loads, from `crates/mnemonist-napi/src/fibonacci_heap.rs`'s
// `install_fibonacci_heap_statics`. The prototype assignment is a semantic
// (it is what makes `new FibonacciHeap() instanceof MaxFibonacciHeap` true
// upstream, BUG-FIBONACCI-HEAP-2) and a shim that added it separately would mean
// `require('@port/addon').FibonacciHeap` was incomplete without the test
// harness -- exactly backwards.
module.exports = require('@port/addon').FibonacciHeap;
