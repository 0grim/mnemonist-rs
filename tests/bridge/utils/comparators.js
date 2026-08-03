// Shim standing in for upstream `utils/comparators.js`.
//
// `test/heap.js` does `require('../utils/comparators.js').DEFAULT_COMPARATOR`,
// so the module has to exist one directory *below* the specs -- which is why
// this shim lives in a subdirectory. `tests/run.sh` copies `tests/bridge/.`
// into the work tree root, so the path is preserved. The default spec glob is
// non-recursive (DIV-PROJ-22), so nothing here is mistaken for a spec.
//
// All four upstream exports are re-exported. Two are `#[napi]` functions backed
// by `mnemonist_core::utils::comparators`; the other two *return* functions,
// which napi cannot hand back from a `#[napi]` signature, so their
// closure-making half is installed onto the addon's exports at load time
// (`crates/mnemonist-napi/src/comparators.rs`). Their comparison logic is still
// the Rust port -- only the closure is JavaScript, and it is upstream's own.
var addon = require('@port/addon');

exports.DEFAULT_COMPARATOR = addon.DEFAULT_COMPARATOR;
exports.DEFAULT_REVERSE_COMPARATOR = addon.DEFAULT_REVERSE_COMPARATOR;
exports.reverseComparator = addon.reverseComparator;
exports.createTupleComparator = addon.createTupleComparator;
