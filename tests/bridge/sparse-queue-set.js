// Shim standing in for upstream `sparse-queue-set.js`.
//
// The original test file does `require('../sparse-queue-set.js')`, so a module
// has to exist at this path inside the assembled work tree. It resolves the
// native addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// As with the other two shims: `SparseQueueSet.prototype[Symbol.iterator]` is
// NOT set here. Upstream sets it -- to `values` -- on the last line of its
// module, and so does ours, in Rust, from `crates/mnemonist-napi/src/cursor.rs`
// when the addon loads.
module.exports = require('@port/addon').SparseQueueSet;
