// Shim standing in for upstream `kd-tree.js`.
//
// The original test file does `require('../kd-tree.js')` plus `require('lodash/fp/get')`
// and `require('../utils/comparators.js').createTupleComparator` -- both real
// harness dependencies/already-ported utilities, not something this shim
// needs to touch. Resolves the native addon through `@port/addon`, published
// into the work tree's node_modules so the lookup is depth-independent.
//
// `KDTree` has no direct `new KDTree(...)` in the original suite -- only
// `.from`/`.fromAxes` -- and no `Symbol.iterator` either, so there is nothing
// for `crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES` to install
// for this class, unlike every T3 shim in this directory.
module.exports = require('@port/addon').KDTree;
