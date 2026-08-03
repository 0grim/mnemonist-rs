# passjoin-index — working log

Chronological. See `docs/modules/passjoin-index.md` for the current-state document and
`docs/modules/evidence/passjoin-index.md` for the gate artifacts.

## Bench: workload size reduced after `add`'s non-deduplication made a larger size unworkable

`add` never deduplicates (upstream's own behaviour, reproduced exactly): every re-add of an
already-present word appends another entry to every matching segment's candidate list. An early
attempt at this workload drew `add` targets with replacement from a domain smaller than the op
count at `size` 2,000 and `ops` 20,000, so the same word got re-added repeatedly over the run, and
each re-add made future `search` calls touching that segment slower. Measured directly: a single
pass took **7.5 seconds**, and the index — which should have stayed near 2,000 entries — grew past
3,500 from duplicate re-adds alone.

At `ops` 5,000 the same shape stays honest but fast: the index grows from a 1,000-entry prefill to
3,551 by the run's end, `search` still returns a match 74.9% of the time (avg 0.88 matches/call in
that standalone measurement), and a full pass completes in under a second. The current document
states this smaller, workable size as the workload definition.
