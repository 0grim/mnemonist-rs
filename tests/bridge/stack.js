// Shim standing in for upstream `stack.js`.
//
// The original test file does `require('../stack.js')`, so a module has to
// exist at this path inside the assembled work tree. It resolves the native
// addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// Note what is NOT here. Upstream's `stack.js` ends with three load-time
// assignments -- `Stack.prototype[Symbol.iterator] = Stack.prototype.values`,
// `Stack.of = function () { return Stack.from(arguments); }`, and the
// inspect symbol -- and the first two are performed by the addon itself when
// it loads (`crates/mnemonist-napi/src/cursor.rs` and `statics.rs`). A shim
// that added semantics would mean the addon was incomplete without the test
// harness, which is exactly backwards.
module.exports = require('@port/addon').Stack;
