// Shim standing in for upstream `vp-tree.js`.
//
// The original test file does `require('../vp-tree.js')` plus `require('leven')`
// and `require('pandemonium/random')` for its distance function and random
// data generator -- both real harness dependencies (`tests/harness-package.json`),
// not something this shim needs to touch. Resolves the native addon through
// `@port/addon`, published into the work tree's node_modules so the lookup is
// depth-independent.
//
// `VPTree` has no `Symbol.iterator` upstream, so there is nothing for
// `crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES` to install for
// this class, unlike every T3 shim in this directory.
module.exports = require('@port/addon').VPTree;
