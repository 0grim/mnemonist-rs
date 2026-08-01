// Shim standing in for upstream `utils/binary-search.js`.
//
// `test/_utils.js` does `require('../utils/binary-search.js')`, so a module
// has to exist at this exact path. The export shape is assembled once in
// `../_utils.js`; this file only cuts the slice that belongs here, so the
// leaf cannot drift from the hub -- same convention `sort/insertion.js` and
// `sort/quick.js` already use for `../sort.js`.
module.exports = require('../_utils.js').binarySearch;
