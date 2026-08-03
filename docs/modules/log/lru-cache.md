# lru-cache — working log

Chronological. See `docs/modules/lru-cache.md` for the current-state document and
`docs/modules/evidence/lru-cache.md` for the gate artifacts.

## Port defect 1 (`unlink` nulling stale slots) found by design review, before any fuzz campaign existed

The panic described in the document's "Bugs this found" (delete/remove nulling
`this.K[pointer]`/`this.V[pointer]`, which upstream never does) was found by reading, before any
fuzz campaign for this unit had run at all: the shape — a hole-bearing `-with-delete` variant, a
walk left open across a mutation — was exactly the interesting territory named for this unit going
in, so it was checked directly with a scratch Rust probe rather than waiting for the fuzzer to reach
it.

## Port defect 2 (`forEach` eager-advance timing) found by the fuzzer's first, un-logged campaign

The `forEach` timing bug described in the document was found by the differential fuzzer's very
first campaign against this grammar. At the time, `fuzz/log.txt` had zero logged campaigns for this
module — the eight campaigns now recorded in the document's "Fuzz + bench" section all post-date the
fix. A `$forEach("set", "arg1,arg0", ...)` program — the very shape named as interesting territory
("interleaved with mutation") — disagreed on the third callback invocation, port seeing
`[undefined, 1]` where upstream re-saw `["w", true]`.

Both port defects were found by or before this same grammar/reading pass, before any of the eight
campaigns recorded in the document were run. Those eight therefore measure the grammar *after* the
interesting bugs were already fixed, not instead of finding them — worth recording explicitly since
"zero divergences across eight campaigns" could otherwise be misread as "the fuzzer found nothing
of note for this unit."
