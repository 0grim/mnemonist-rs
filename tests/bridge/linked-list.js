// Shim standing in for upstream `linked-list.js`.
//
// The original test file does `require('../linked-list.js')`, so a module
// has to exist at this path inside the assembled work tree. It resolves the
// native addon through `@port/addon`, published into the work tree's
// node_modules so the lookup is depth-independent.
//
// Note what is NOT here: `LinkedList.prototype[Symbol.iterator]`. Upstream
// aliases it to `values` on its last line, and so does ours -- in Rust, from
// `crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES`, when the
// addon loads. A shim that added semantics would mean the addon was
// incomplete without the test harness, which is exactly backwards.
module.exports = require('@port/addon').LinkedList;
