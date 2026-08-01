// Shim standing in for upstream `bloom-filter.js`.
//
// The original test file does `require('../bloom-filter.js')`, so a module has
// to exist at this path inside the assembled work tree. It resolves the native
// addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// Nothing else is here. Upstream's `bloom-filter.js` has no load-time
// assignments at all -- no `Symbol.iterator`, no `.of`, no inspect symbol --
// so unlike `suffix-array.js` there is not even a namespacing statement to
// carry.
module.exports = require('@port/addon').BloomFilter;
