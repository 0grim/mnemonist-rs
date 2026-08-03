# heap — working log

Chronological. See `docs/modules/heap.md` for the current-state document and
`docs/modules/evidence/heap.md` for the gate artifacts.

## Three port defects found by independent review (found this series)

The three port defects now described in the document's "Bugs this found" section (the `RefCell`
borrow held across a JS call in `Heap::clear`/`Heap::peek`, `clear()`/`consume()` preserving a class
upstream discards, and `n` being validated before upstream would validate it) were all found by a
second, independent look at the code rather than by any of the gates. This is worth recording on its
own: the unit had 21 upstream assertions, 47 boundary cases, three fuzz campaigns and 5 M operations
all green when the review happened. It is the same category `docs/METHODOLOGY.md`'s "What these
instruments cannot see" collects, and, like PORTBUG-1, it was found by a person reading the code again
rather than by the machinery. None of the three was reachable by the fuzzer by construction: the
core-side store never calls JavaScript from `allocate`, it has a single array class, and
`nsmallest`/`nlargest` are outside the fuzz alphabet — so the earlier green fuzz campaigns were not
wrong, they were just never going to see these.

The napi-rs name-table conflict (statics and prototype methods sharing one registration, DIV-HEAP-6) and
the `#[napi(factory)]` bare-call bug (`MaxHeap`'s factory needing its receiver bound before the
temporary property is deleted) were found by the port's own machinery instead — nine of fourteen
boundary cases failing loudly with `heap.push is not a function`, and a `Failed to create instance of
class` error respectively. Both fixed; both now described in the document without further comment.

## Gap 16 correction (found this series)

The document's gap 16 (in "What upstream does NOT test") previously read "upstream's `new Array(n)`
refuses it too" — a claim that was true for one of `nsmallest`'s three code paths and false for the
other two, which is exactly the pair the bridge's overly-early validation intercepted (see defect 3,
above). The gap text has been corrected in place; no separate marker is needed since the corrected
text is now what the document carries.

## Bench cause investigation: RefCell + Comparator trait overhead (originally unconfirmed, confirmed 2026-08-02)

The `mixed-1e6` p50/min regression was first published with the mechanism (indirection per
comparison) named as a plausible cause but not isolated by profiling — no `perf`/`cargo flamegraph`
on this host, so the attribution was stated as unconfirmed.

**Confirmed 2026-08-02** with a bare counterfactual instead of a profiler.
`bench/runner/src/heap.rs::run_mixed_bare`, reachable via `bench-runner --heap-probe`, runs the
identical mixed op stream against a bare `Vec<f64>` binary min-heap — same sift-up/sift-down
algorithm, no `RefCell`, no `Cell`, no `Store`/`Comparator` trait, `<` inlined directly. The bare
heap's p50 (21.721 ns/op) is not merely faster than the wrapped one (31.781 ns/op) — it beats
*upstream's own* published p50 (24.316 ns) and min (19.457 ns) outright. **Verdict: confirmed, and
understated.** The indirection layer costs 10.06 ns/op in this isolated comparison, which is *more*
than the entire measured regression against upstream (7.68 ns/op) — removing it does not just close
the gap, it flips the comparison to a Rust win, the same direction every other metric in the table
already points. This finding is now stated as current fact in the document; this entry keeps the
history of how it went from unconfirmed to confirmed.

## Structural fix (RefCell removal / Comparator specialisation) declined, not costed

A fix that keeps re-entrancy safety for the general `Comparator` case while giving a non-re-entrant
concrete comparator (such as this benchmark's own `DefaultComparator`) a faster path — e.g.
specialising `Heap` for `S: Store` combinations known not to re-enter — was identified but not
attempted and not costed with an hour estimate the way `default-map`'s structural fix was. It is a
`crates/mnemonist-core` design change, not a local tweak, and would need heap's fuzz campaign and
bench figures re-run before it could stand. Recorded here rather than in the document because no
work toward it has happened and there is nothing dated to report; the document states only that it
has not been attempted and why.
