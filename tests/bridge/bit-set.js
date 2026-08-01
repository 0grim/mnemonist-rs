// Shim standing in for upstream `bit-set.js`.
//
// The original test file does `require('../bit-set.js')`, so a module has to
// exist at this path inside the assembled work tree. It resolves the native
// addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// As with `sparse-set.js`, `BitSet.prototype[Symbol.iterator]` is NOT set here.
// Upstream sets it on the last line of its module and so do we -- in Rust, from
// crates/mnemonist-napi/src/cursor.rs, when the addon loads.
module.exports = require('@port/addon').BitSet;
