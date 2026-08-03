# bloom-filter — working log

Chronological. See `docs/modules/bloom-filter.md` for the current-state document and
`docs/modules/evidence/bloom-filter.md` for the gate artifacts.

## Fuzz oracle harness bug: integral floats encoded ambiguously (found this series, fixed)

The spec's very first run reported `capacity: port 6.0, upstream 6` before a single operation ran.
JavaScript has one number type; `serde_json::Value` has two, and `json!(6.0_f64) != json!(6)`. An
encoding mismatch is a *false* divergence, and one that fired on a rare value instead of on every
value would have looked exactly like a port defect. Fixed by `js_number()`, which renders a whole
`f64` the way `JSON.stringify` does, and noted in the regression corpus header so the next module
spec does not rediscover it.
