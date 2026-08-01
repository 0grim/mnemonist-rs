// Shim standing in for upstream `bk-tree.js`.
//
// The original test file does `require('../bk-tree.js')` and, separately,
// `require('leven')` for its distance function -- `leven` is a real harness
// dependency (`tests/harness-package.json`), not something this shim needs to
// touch. Resolves the native addon through `@port/addon`, published into the
// work tree's node_modules so the lookup is depth-independent.
//
// `BKTree` has no `Symbol.iterator` upstream, so there is nothing for
// `crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES` to install for
// this class, unlike every T3 shim in this directory.
module.exports = require('@port/addon').BKTree;
