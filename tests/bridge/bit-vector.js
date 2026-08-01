// Shim standing in for upstream `bit-vector.js`.
//
// The original test file does `require('../bit-vector.js')`, so a module has to
// exist at this path inside the assembled work tree. It resolves the native
// addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// `BitVector.prototype[Symbol.iterator]` is installed in Rust, from
// crates/mnemonist-napi/src/cursor.rs, when the addon loads -- exactly where
// upstream installs it, on the last line of its own module.
module.exports = require('@port/addon').BitVector;
