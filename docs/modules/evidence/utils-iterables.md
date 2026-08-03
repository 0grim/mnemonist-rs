# utils/iterables — evidence

Gate artifacts for `docs/modules/utils-iterables.md`: full falsification record.

## Falsification record (gate 6, on `tests/boundary/iterables.js`)

Gate 6 asks that sabotaging the port turns the *original mocha suite* red; this file has no
original suite, so the gate has no target in that form. What was performed instead, named before
it was run: the sabotage had to break `getPointerArray throws BEFORE new Array(l) does.`, the one
spec whose subject is the *order of two statements* rather than the result of either.

**The sabotage:** `js_to_array_with_indices` allocating the value array before choosing the index
width — the tidier reading, and the one a port written from the function's description rather than
from its source would produce.

**Confirmed red**, and red only there: `18 passing, 1 failing`, that spec, with the port raising
`RangeError: Invalid array length` where upstream raises
`Error: mnemonist: Pointer Array of size > 4294967295 is not supported.`. Reverted; **confirmed
green again**: `19 passing`.

The spec that checks the inline reference against the vendored upstream source is a second guard of
a different kind: it pins seven distinguishing lines, so any edit to those four upstream bodies
moves one of them and the copy cannot silently drift.
