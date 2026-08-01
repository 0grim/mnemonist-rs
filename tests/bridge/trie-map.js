// Shim standing in for upstream `trie-map.js`.
//
// The original test file does `require('../trie-map.js')`, so a module has
// to exist at this path inside the assembled work tree. It resolves the
// native addon through `@port/addon`, which is published into the work
// tree's node_modules so the lookup is depth-independent.
//
// Note what is NOT here: `TrieMap.prototype[Symbol.iterator]` and
// `TrieMap.SENTINEL`. Upstream sets both itself, and so does the addon --
// the first from `crates/mnemonist-napi/src/cursor.rs`, the second from
// `crates/mnemonist-napi/src/trie_map.rs`'s `install_trie_statics`, both at
// load time. A shim that added either would mean
// `require('@port/addon').TrieMap` was incomplete without the test harness,
// which is exactly backwards.
module.exports = require('@port/addon').TrieMap;
