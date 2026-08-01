// Shim standing in for upstream `inverted-index.js`.
//
// The original test file does `require('../inverted-index.js')`, so a
// module has to exist at this path inside the assembled work tree. It
// resolves the native addon through `@port/addon`, published into the work
// tree's node_modules so the lookup is depth-independent.
//
// Note what is NOT here: `InvertedIndex.prototype[Symbol.iterator]`.
// Upstream aliases it to `documents` on its last line, and so does ours --
// in Rust, from `crates/mnemonist-napi/src/cursor.rs`'s
// `ITERATOR_FACTORIES`, when the addon loads.
module.exports = require('@port/addon').InvertedIndex;
