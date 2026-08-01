// HARNESS SCAFFOLDING, NOT VENDORED SOURCE.
//
// Every other file in this directory is upstream mnemonist, byte for byte. This
// one is ours, and it is here for a single mechanical reason: `fuzz/oracle.js`
// addresses a module by `require`-ing `bench/upstream/<key>.js` and calling
// `new` on whatever that file exports. `GeneralizedSuffixArray` is a *second*
// export of `suffix-array.js`, reachable only as a property of the first, so it
// has no file of its own for the oracle to name.
//
// Two lines of re-export, no logic. The class below is upstream's, unmodified,
// loaded from upstream's own file.
module.exports = require('./suffix-array.js').GeneralizedSuffixArray;
