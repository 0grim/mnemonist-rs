// Shim standing in for upstream `circular-buffer.js`.
//
// Upstream's file builds CircularBuffer by copying FixedDeque.prototype key by
// key and then replacing `push` and `unshift`. The port does the same thing in
// Rust -- `CircularBuffer` holds a `FixedDeque` and delegates -- so this shim,
// like the others, carries no semantics of its own.
module.exports = require('@port/addon').CircularBuffer;
