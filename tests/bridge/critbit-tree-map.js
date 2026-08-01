// Shim standing in for upstream `critbit-tree-map.js`.
//
// The original test file does `require('../critbit-tree-map.js')`, so a
// module has to exist at this path inside the assembled work tree. It
// resolves the native addon through `@port/addon`, published into the work
// tree's node_modules so the lookup is depth-independent.
module.exports = require('@port/addon').CritBitTreeMap;
