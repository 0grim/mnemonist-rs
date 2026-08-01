// Shim standing in for upstream `multi-array.js`.
//
// The original test file does `require('../multi-array.js')` and
// `require('obliterator/take')` -- `obliterator` is a real harness dependency
// (`tests/harness-package.json`), not something this shim needs to touch.
// Resolves the native addon through `@port/addon`, published into the work
// tree's node_modules so the lookup is depth-independent.
//
// `MultiArray` has no `Symbol.iterator` upstream (`test/multi-array.js` never
// uses `for...of`), so there is nothing for
// `crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES` to install for
// this class.
module.exports = require('@port/addon').MultiArray;
