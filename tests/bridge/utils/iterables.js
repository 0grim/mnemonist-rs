// Shim standing in for upstream `utils/iterables.js`.
//
// `test/_utils.js` does `require('../utils/iterables.js')`. All four exports
// were already bridged (at the addon's top level, under their own upstream
// names) for `fixed-stack`/`fixed-deque`/`circular-buffer`'s `.from()`
// statics, well before this unit existed -- see
// `crates/mnemonist-napi/src/iterables.rs` and `docs/modules/utils-iterables.md`.
// This file only gives `test/_utils.js` a path to require; no new bridging
// happens here, cut from `../_utils.js` like this unit's other four leaves.
module.exports = require('../_utils.js').iterables;
