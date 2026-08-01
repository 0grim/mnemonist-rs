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

use std::time::Duration;

use difffuzz::modules::hashed_array_tree::{HashedArrayTreeSpec, REGRESSIONS as HAT_REGRESSIONS};
use difffuzz::modules::sparse_set::{SparseSetSpec, REGRESSIONS as SPARSE_REGRESSIONS};
use difffuzz::modules::static_disjoint_set::{StaticDisjointSetSpec, REGRESSIONS};
use difffuzz::Campaign;

#[test]
fn hashed_array_tree_matches_upstream() {
    let campaign = Campaign::cases(0x4A70, 96, HAT_REGRESSIONS);

    let report = difffuzz::run(&HashedArrayTreeSpec, &campaign)
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

#[test]
fn sparse_set_matches_upstream() {
    let campaign = Campaign::cases(0x5A2E, 96, SPARSE_REGRESSIONS);

    let report = difffuzz::run(&SparseSetSpec, &campaign)
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

/// Every batch must generate NEW cases.
///
/// `TestRunner` counts successes for its lifetime and loops
/// `while successes < config.cases`, so re-running one runner executes nothing
/// — and the campaign driver then spun at 100% CPU until its deadline while
/// still reporting a case count, because proptest replays the persisted
/// regression corpus before each (empty) main loop. A 120-second campaign
/// booked tens of thousands of "cases" that were two saved seeds repeated.
///
/// Pinned with **no corpus**, which is what makes the assertion sharp: with
/// nothing to replay, the only way past `batch` cases is a batch that really
/// generated. Duration mode rather than a case budget, because the broken
/// version does not terminate under a case budget it can never reach.
#[test]
fn every_batch_generates_new_cases() {
    const NO_CORPUS: &str = "/tmp/difffuzz-batch-regression-test-corpus.txt";
    const BATCH: u32 = 8;

    let _ = std::fs::remove_file(NO_CORPUS);

    let campaign = Campaign {
        seed: 0x0B47C4,
        cases: None,
        duration: Some(Duration::from_secs(2)),
        batch: BATCH,
        regressions: NO_CORPUS,
    };

    let report = difffuzz::run(&StaticDisjointSetSpec, &campaign)
        .expect("oracle must be reachable; `node` is required for differential tests");

    assert!(
        report.cases > u64::from(BATCH),
        "a {}s campaign executed {} cases with a batch of {BATCH}: batches after \
         the first generated nothing. {}",
        2,
        report.cases,
        report.log_line()
    );
}

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

/// A campaign that cannot execute anything must fail, not pass.
///
/// Both shapes below were live defects found in review. `batch: 0` is the
/// dangerous one: proptest's runner loops `while successes < cases`, so a zero
/// case count returns `Ok` without ever calling the property — which produced
/// a full-length run reporting "zero divergences" over zero work. Pinned here
/// because that is the single worst thing this crate could do.
#[test]
fn a_campaign_that_runs_nothing_is_an_error() {
    let empty_batch = Campaign {
        batch: 0,
        ..Campaign::cases(1, 10, REGRESSIONS)
    };

    assert!(
        difffuzz::run(&StaticDisjointSetSpec, &empty_batch).is_err(),
        "a zero batch must not be reportable as a clean campaign"
    );

    let unbounded = Campaign {
        cases: None,
        duration: None,
        ..Campaign::cases(1, 10, REGRESSIONS)
    };

    assert!(
        difffuzz::run(&StaticDisjointSetSpec, &unbounded).is_err(),
        "a campaign with neither budget would never terminate"
    );
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
