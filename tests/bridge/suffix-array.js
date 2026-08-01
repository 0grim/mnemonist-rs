// Shim standing in for upstream `suffix-array.js`.
//
// The original test file does `require('../suffix-array.js')`, so a module has
// to exist at this path inside the assembled work tree. It resolves the native
// addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// The one line of assembly here is upstream's own last-but-one line:
//
//     SuffixArray.GeneralizedSuffixArray = GeneralizedSuffixArray;
//
// which is a CommonJS *namespacing* statement, not a behaviour: it says how one
// module presents two classes. The addon exports both classes at top level, so
// nothing is missing from it -- `require('@port/addon').GeneralizedSuffixArray`
// works on its own. Compare `stack.js`, where `Stack.of` deliberately is NOT
// here, because that one is behaviour and a shim that supplied it would make
// the addon incomplete without the harness.
//
// The alternative was to do the alias inside the addon's single
// `#[napi(module_exports)]` hook in `crates/mnemonist-napi/src/cursor.rs`. That
// hook is being edited by several agents at once, and a merge conflict landing
// inside a function tail has already broken this tree three times, so the alias
// lives here instead. See `docs/modules/suffix-array.md`.
var addon = require('@port/addon');

addon.SuffixArray.GeneralizedSuffixArray = addon.GeneralizedSuffixArray;

module.exports = addon.SuffixArray;
