# vector — working log

Chronological. See `docs/modules/vector.md` for the current-state document and
`docs/modules/evidence/vector.md` for the gate artifacts.

## Fuzz oracle harness bug: `serde_json` float parsing not correctly rounded (found this series, fixed)

The oracle's response line is a full-precision JSON number for every non-truncating
`Float64Array` value this grammar generates. `serde_json`'s default float parser is not always
correctly rounded for such values — a scratch test parsing the literal `"38403.356486892444"`
recovered a value one ULP away from what Rust's own `f64::from_str` gives for the same text. The
wire log showed the port and the oracle's raw response text agreeing exactly; only the *parsed*
`Value` used for the comparison disagreed. Enabling `serde_json`'s `float_roundtrip` feature
(workspace `Cargo.toml`) fixed it — the same class of finding as `default-map`'s own number-encoding
fault: a harness defect that manufactures divergences rather than catching real ones. `vector` is
the first module whose grammar generates `f64` values wide enough to land in the affected range.
