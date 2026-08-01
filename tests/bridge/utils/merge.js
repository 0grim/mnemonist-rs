// Shim standing in for upstream `utils/merge.js`.
//
// `test/_utils.js` does `require('../utils/merge.js')`. The variadic
// dispatch (`merge`/`unionUnique`/`intersectionUnique`, each choosing between
// a two-array and a k-way addon function) is assembled once in
// `../_utils.js`; see that file's header for why it lives there and not
// here.
module.exports = require('../_utils.js').merge;
