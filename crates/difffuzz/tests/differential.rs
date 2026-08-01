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

use difffuzz::modules::bit_set::{BitSetSpec, REGRESSIONS as BIT_SET_REGRESSIONS};
use difffuzz::modules::bit_vector::{BitVectorSpec, REGRESSIONS as BIT_VECTOR_REGRESSIONS};
use difffuzz::modules::default_map::{DefaultMapSpec, REGRESSIONS as DEFAULT_MAP_REGRESSIONS};
use difffuzz::modules::hashed_array_tree::{HashedArrayTreeSpec, REGRESSIONS as HAT_REGRESSIONS};
use difffuzz::modules::sparse_map::{SparseMapSpec, REGRESSIONS as SPARSE_MAP_REGRESSIONS};
use difffuzz::modules::sparse_queue_set::{
    SparseQueueSetSpec, REGRESSIONS as SPARSE_QUEUE_REGRESSIONS,
};
use difffuzz::modules::sparse_set::{SparseSetSpec, REGRESSIONS as SPARSE_REGRESSIONS};
use difffuzz::modules::static_disjoint_set::{StaticDisjointSetSpec, REGRESSIONS};
use difffuzz::Campaign;
// Appended at the end of the import run, never inserted.
use difffuzz::modules::circular_buffer::{
    CircularBufferSpec, REGRESSIONS as CIRCULAR_BUFFER_REGRESSIONS,
};
use difffuzz::modules::fixed_deque::{FixedDequeSpec, REGRESSIONS as FIXED_DEQUE_REGRESSIONS};
use difffuzz::modules::fixed_stack::{FixedStackSpec, REGRESSIONS as FIXED_STACK_REGRESSIONS};

#[test]
fn bit_set_matches_upstream() {
    let campaign = Campaign::cases(0xB175, 96, BIT_SET_REGRESSIONS);

    let report = difffuzz::run(&BitSetSpec, &campaign)
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
fn bit_vector_matches_upstream() {
    let campaign = Campaign::cases(0xB1EC, 96, BIT_VECTOR_REGRESSIONS);

    let report = difffuzz::run(&BitVectorSpec, &campaign)
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
fn default_map_matches_upstream() {
    let campaign = Campaign::cases(0xDEFA, 96, DEFAULT_MAP_REGRESSIONS);

    let report = difffuzz::run(&DefaultMapSpec, &campaign)
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

#[test]
fn sparse_map_matches_upstream() {
    let campaign = Campaign::cases(0x5A99, 96, SPARSE_MAP_REGRESSIONS);

    let report = difffuzz::run(&SparseMapSpec, &campaign)
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
fn sparse_queue_set_matches_upstream() {
    let campaign = Campaign::cases(0x5A11, 96, SPARSE_QUEUE_REGRESSIONS);

    let report = difffuzz::run(&SparseQueueSetSpec, &campaign)
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

/// Every committed seed must carry a provenance note.
///
/// All eight seeds in this repo came from deliberate sabotages, not from real
/// port defects. An unlabelled `cc` line reads as "a bug was found and fixed
/// here", which is the opposite of what happened, and the labelling is only
/// worth anything if something notices when it goes missing.
#[test]
fn every_regression_corpus_explains_where_its_seeds_came_from() {
    for corpus in [
        REGRESSIONS,
        SPARSE_REGRESSIONS,
        SPARSE_MAP_REGRESSIONS,
        SPARSE_QUEUE_REGRESSIONS,
    ] {
        let text = std::fs::read_to_string(corpus)
            .unwrap_or_else(|_| panic!("{corpus} must be checked in"));

        let seeds = text.lines().filter(|line| line.starts_with("cc ")).count();

        assert!(seeds > 0, "{corpus} has no seeds");
        assert!(
            text.contains("PROVENANCE"),
            "{corpus} holds {seeds} seed(s) with no provenance block, so a reader \
             cannot tell a sabotage from a real defect"
        );
    }
}

// ---------------------------------------------------------------------------
// Appended at the end of the file, never inserted: several agents edit this
// file at once and a test added after the last one cannot land inside another
// agent's hunk.
// ---------------------------------------------------------------------------

#[test]
fn fixed_stack_matches_upstream() {
    let campaign = Campaign::cases(0xF15A, 96, FIXED_STACK_REGRESSIONS);

    let report = difffuzz::run(&FixedStackSpec, &campaign)
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

/// The corpora added by this wave carry provenance too.
///
/// `every_regression_corpus_explains_where_its_seeds_came_from` above pins the
/// four corpora that existed when it was written; extending its list in place
/// would be a merge conflict, so the same assertion is made here for the ones
/// added since. Every seed in them came from a deliberate sabotage, and an
/// unlabelled `cc` line would read as "a real defect was found and fixed here".
#[test]
fn wave_one_regression_corpora_explain_where_their_seeds_came_from() {
    // A slice rather than an array literal: the list grows as this wave lands
    // its remaining modules, and a one-element array is a clippy lint.
    const CORPORA: &[&str] = &[
        FIXED_STACK_REGRESSIONS,
        FIXED_DEQUE_REGRESSIONS,
        CIRCULAR_BUFFER_REGRESSIONS,
    ];

    for corpus in CORPORA {
        let text = std::fs::read_to_string(corpus)
            .unwrap_or_else(|_| panic!("{corpus} must be checked in"));

        let seeds = text.lines().filter(|line| line.starts_with("cc ")).count();

        assert!(seeds > 0, "{corpus} has no seeds");
        assert!(
            text.contains("PROVENANCE"),
            "{corpus} holds {seeds} seed(s) with no provenance block, so a reader \
             cannot tell a sabotage from a real defect"
        );
    }
}

#[test]
fn fixed_deque_matches_upstream() {
    let campaign = Campaign::cases(0xF1DE, 96, FIXED_DEQUE_REGRESSIONS);

    let report = difffuzz::run(&FixedDequeSpec, &campaign)
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
fn circular_buffer_matches_upstream() {
    let campaign = Campaign::cases(0xC18F, 96, CIRCULAR_BUFFER_REGRESSIONS);

    let report = difffuzz::run(&CircularBufferSpec, &campaign)
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
