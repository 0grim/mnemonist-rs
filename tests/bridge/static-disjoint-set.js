// Shim standing in for upstream `static-disjoint-set.js`.
//
// The original test file does `require('../static-disjoint-set.js')`, so a
// module has to exist at this path inside the assembled work tree. It resolves
// the native addon through `@port/addon`, which is published into the work
// tree's node_modules so the lookup is depth-independent.
module.exports = require('@port/addon').StaticDisjointSet;
