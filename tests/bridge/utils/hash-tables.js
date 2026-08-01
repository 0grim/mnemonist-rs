// Shim standing in for upstream `utils/hash-tables.js`.
//
// `test/_utils.js` does `require('../utils/hash-tables.js')`. Export shape
// assembled once in `../_utils.js`; see that file's header for why.
module.exports = require('../_utils.js').hashTables;
