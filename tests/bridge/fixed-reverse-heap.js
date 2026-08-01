// Shim standing in for upstream `fixed-reverse-heap.js`.
//
// The original test file does `require('../fixed-reverse-heap.js')`, so a
// module has to exist at this path inside the assembled work tree. It resolves
// the native addon through `@port/addon`, which is published into the work
// tree's node_modules so the lookup is depth-independent.
//
// FixedReverseHeap has no iterator and no statics, so unlike `heap.js` there is
// nothing for the addon to install onto the constructor at load time -- the
// export is the class and nothing else.
module.exports = require('@port/addon').FixedReverseHeap;
