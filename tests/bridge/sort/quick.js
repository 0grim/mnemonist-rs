// Shim standing in for upstream `sort/quick.js`.
//
// `test/sort.js` does `require('../sort/quick.js')`, so a module has to exist
// at this exact path inside the assembled work tree. The export shape is
// assembled once in `../sort.js`; this file only cuts the half that belongs
// here, so the two leaves cannot drift apart from the aggregate.
module.exports = require('../sort.js').quick;
