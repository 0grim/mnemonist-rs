//! Short differential campaigns, so `cargo test` exercises the fuzzer rather
//! than only compiling it.
//!
//! These are deliberately small — a few seconds each. The 60-second gate-9 runs
//! are launched from `fuzz/run.sh` and recorded in `fuzz/log.txt`; what these
//! guard is that the harness still *works*, which is the thing most likely to
//! rot silently while nobody is looking at it.
//!
//! They require `node` on `PATH`. That is intentional: a differential test that
//! passes when the oracle is missing would be worse than no test at all.

use difffuzz::modules::static_disjoint_set::{StaticDisjointSetSpec, REGRESSIONS};
use difffuzz::Campaign;

#[test]
fn static_disjoint_set_matches_upstream() {
    let campaign = Campaign::cases(0xD1FF, 96, REGRESSIONS);

    let report = difffuzz::run(&StaticDisjointSetSpec, &campaign)
        .expect("oracle must be reachable; `node` is required for differential tests");

    assert!(
        report.ops > 0,
        "campaign ran no operations, so it proved nothing: {}",
        report.log_line()
    );

    if let Some(divergence) = report.divergence {
        panic!("{divergence}");
    }
}

/// The persisted regression corpus is replayed by `TestRunner::run` before any
/// novel case, so the entry recorded in
/// `proptest-regressions/static-disjoint-set.txt` is covered by the test above.
/// This asserts the file is actually present, because a silently missing
/// corpus is indistinguishable from a passing one.
#[test]
fn regression_corpus_is_committed() {
    let corpus = std::fs::read_to_string(REGRESSIONS)
        .expect("proptest-regressions/static-disjoint-set.txt must be checked in");

    assert!(
        corpus.lines().any(|line| line.starts_with("cc ")),
        "regression corpus has no seeds"
    );
}
