# static-disjoint-set — working log

Chronological. See `docs/modules/static-disjoint-set.md` for the current-state document and
`docs/modules/evidence/static-disjoint-set.md` for the gate artifacts.

## Fuzz campaign counts corrected: proptest `TestRunner` reuse under-ran every batch after the first (fixed 3120085)

The first run of this gate logged `cases=16666` / `ops=2837506`. The op count was real and every
comparison in it was real, but the *diversity* was not: proptest's `TestRunner` counts successes for
its whole lifetime and loops `while successes < config.cases`, so the campaign driver's reuse of one
runner meant every batch after the first executed no new cases at all. What it did still execute was
the persisted regression corpus, replayed before the (empty) main loop and counted as cases. So
"16,666 cases" was 32 genuinely new programs plus two saved seeds re-run about 8,300 times each; the
rest of the two minutes was the driver spinning.

Measured decisively before the fix: with the corpus file removed, a 120-second campaign dropped
from 16,666 cases to **32**. Fixed in `3120085` (a fresh runner per batch, seeded from
`(seed, batch)`), pinned by `every_batch_generates_new_cases`, and disclosed in `fuzz/log.txt`
above the superseded lines, which are kept rather than deleted. The op counts fell because the
repeated corpus programs were short and cheap; the coverage rose by two orders of magnitude. The
numbers now in the current document (2.10 M operations across 6,984 distinct programs) are the
corrected figures.

It was found while porting `sparse-set`, whose corpus did not exist yet — so instead of quietly
repeating two programs the driver spun visibly and a 20-second run reported 32 cases. This belongs
among the confident-but-empty green signals documented in `docs/METHODOLOGY.md`'s "What these
instruments cannot see" section: the number was large, the run took the full 120 seconds, and
nothing looked wrong. See also `docs/modules/log/sparse-set.md`, which records the same defect from
the discovering unit's side.

## Bench: PointerVec regression at 4e6 traced and fixed; footprint hypothesis refuted

An earlier revision of this port **lost the tail badly at 4e6: p99 275.0 ns/op against upstream's
102.1, a 2.7× regression**, reproducible across repeats and a full harness re-run, while p50 stayed
1.7× ahead. It was caused by `PointerVec` backing *every* logical width with a `Vec<u32>` — where
upstream's `ranks` is a `Uint8Array`, ours was four times as wide.

Giving `PointerVec` a real per-width backing store (`Vec<u8>` / `Vec<u16>` / `Vec<u32>`, where the
narrowing cast *is* the truncation, so the mask became unnecessary rather than merely correct) took
p99 from **275.0 → 43.6 ns/op**, turning a 2.7× loss into a 3.1× win — the figure now in the current
document.

**The mechanism first predicted was wrong, and the data said so — refuted.** The hypothesis was
footprint: 4e6 items meant 16 MB + 16 MB = 32 MB of structure against upstream's 4 MB + 16 MB =
20 MB, straddling this CPU's 32 MB L3. If that were the mechanism, resident memory should have
dropped by ~12 MB. **It did not: `structure_rss_delta_mb` moved 12.8 → 13.0.**

The reason is that `ranks` is `vec![0; n]` and, because of the rank bug (B-7), almost every entry is
*never written* — only roots are ever bumped. Linux does not fault in untouched zero pages, so the
extra 12 MB was never resident and never appeared in RSS in the first place. The footprint argument
was measuring something that did not exist. **Verdict: refuted.**

The address-space-stride/TLB-pressure hypothesis that replaced it is stated as the current
(unconfirmed) explanation in the document itself, since it has not been superseded by anything
since.

## Stale operation count in "Bugs this found" corrected to match the Fuzz section

The document's "Bugs this found" section previously read "Two campaigns, 4.23 M operations, zero
divergences" — the pre-correction total from before the proptest `TestRunner` reuse fix above, left
unupdated when the Fuzz section's own count was corrected to 2.10 M operations across 6,984 distinct
programs. Both sentences describe the same two campaigns; the current document now states the
corrected figure (2.10 M) consistently in both places.
