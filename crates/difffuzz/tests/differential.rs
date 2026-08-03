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
// Appended rather than filed alphabetically: this list is edited from several
// worktrees at once, and a conflict boundary that lands inside it has already
// broken three merges. New imports go on the end.
use difffuzz::modules::set::{SetSpec, REGRESSIONS as SET_REGRESSIONS};
use difffuzz::modules::sort::{SortSpec, REGRESSIONS as SORT_REGRESSIONS};
// Appended, never interleaved (CLAUDE.md, Git).
use difffuzz::modules::bloom_filter::{BloomFilterSpec, REGRESSIONS as BLOOM_REGRESSIONS};
use difffuzz::modules::suffix_array::{
    GeneralizedSuffixArraySpec, SuffixArraySpec, GENERALIZED_REGRESSIONS,
    REGRESSIONS as SUFFIX_ARRAY_REGRESSIONS,
};
// Appended at the end of the import list; see `modules/mod.rs`.
use difffuzz::modules::fixed_reverse_heap::{
    FixedReverseHeapSpec, REGRESSIONS as FIXED_REVERSE_HEAP_REGRESSIONS,
};
use difffuzz::modules::heap::{HeapSpec, REGRESSIONS as HEAP_REGRESSIONS};
use difffuzz::Campaign;
// Appended at the END of the file, never between two existing tests -- see
// the bottom of this file, where the actual #[test] fns for these live.
use difffuzz::modules::lru_cache::{
    LruCacheSpec, LruCacheWithDeleteSpec, LruMapSpec, LruMapWithDeleteSpec, MAP_REGRESSIONS,
    MAP_WITH_DELETE_REGRESSIONS, REGRESSIONS as LRU_CACHE_REGRESSIONS,
    WITH_DELETE_REGRESSIONS as LRU_CACHE_WITH_DELETE_REGRESSIONS,
};
// Appended at the end of the import run, never inserted.
use difffuzz::modules::circular_buffer::{
    CircularBufferSpec, REGRESSIONS as CIRCULAR_BUFFER_REGRESSIONS,
};
use difffuzz::modules::fixed_deque::{FixedDequeSpec, REGRESSIONS as FIXED_DEQUE_REGRESSIONS};
use difffuzz::modules::fixed_stack::{FixedStackSpec, REGRESSIONS as FIXED_STACK_REGRESSIONS};
// Appended at the end of the import run, never inserted (CLAUDE.md, Git).
use difffuzz::modules::static_interval_tree::{
    StaticIntervalTreeSpec, REGRESSIONS as STATIC_INTERVAL_TREE_REGRESSIONS,
};
use difffuzz::modules::vector::{VectorSpec, REGRESSIONS as VECTOR_REGRESSIONS};
// Appended at the end of the import run, never inserted: this file is edited
// by several agents at once and a conflict boundary landing mid-list has
// already broken merges here.
use difffuzz::modules::bi_map::{BiMapSpec, REGRESSIONS as BI_MAP_REGRESSIONS};
use difffuzz::modules::bk_tree::{BkTreeSpec, REGRESSIONS as BK_TREE_REGRESSIONS};
use difffuzz::modules::fuzzy_map::{FuzzyMapSpec, REGRESSIONS as FUZZY_MAP_REGRESSIONS};
// Appended at the end of the import run, never inserted (CLAUDE.md, Git).
use difffuzz::modules::fuzzy_multi_map::{
    FuzzyMultiMapSpec, REGRESSIONS as FUZZY_MULTI_MAP_REGRESSIONS,
};
use difffuzz::modules::multi_map::{MultiMapSpec, REGRESSIONS as MULTI_MAP_REGRESSIONS};
use difffuzz::modules::multi_set::{MultiSetSpec, REGRESSIONS as MULTI_SET_REGRESSIONS};
// Appended at the end of the import run, never inserted (CLAUDE.md, Git).
use difffuzz::modules::fibonacci_heap::{
    FibonacciHeapSpec, REGRESSIONS as FIBONACCI_HEAP_REGRESSIONS,
};

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
    const BATCH: u32 = 8;

    // Per-process, not a fixed `/tmp` name. The point of this path is that
    // nothing is there to replay, and a name shared between two concurrent
    // test binaries -- two checkouts, two users, or one machine running the
    // suite twice at once -- is a path either of them may be writing to. A
    // unique name can only make the corpus emptier, never fuller, so it
    // strengthens the assertion rather than relaxing it.
    let no_corpus = std::env::temp_dir().join(format!(
        "difffuzz-batch-regression-{}.txt",
        std::process::id()
    ));
    // `Campaign::regressions` is `&'static str`, and this one is only known at
    // run time. Leaking a single path in a test process that is about to exit
    // is cheaper than widening the field's lifetime across every call site.
    let no_corpus: &'static str =
        Box::leak(no_corpus.to_string_lossy().into_owned().into_boxed_str());

    let _ = std::fs::remove_file(no_corpus);

    // The deadline escalates instead of being a single fixed budget, because a
    // 2-second wall clock is a proxy for "at least two batches ran" and the
    // proxy breaks under CPU contention: every case round-trips through the
    // Node oracle, so on a loaded machine two seconds buys exactly one batch
    // and the assertion fails while the implementation is correct. Observed
    // five times, and reproducible on demand by running this suite against 12
    // busy cores.
    //
    // Escalating cannot mask the defect this test exists to catch. The broken
    // version cannot exceed `BATCH` cases at ANY deadline -- its second and
    // later batches generate nothing at all, so more time yields more spinning
    // and no more cases. Extra time can therefore only rescue a slow machine,
    // never a broken implementation.
    let mut report = None;

    for seconds in [2u64, 10, 30] {
        let campaign = Campaign {
            seed: 0x0B47C4,
            cases: None,
            duration: Some(Duration::from_secs(seconds)),
            batch: BATCH,
            regressions: no_corpus,
        };

        let attempt = difffuzz::run(&StaticDisjointSetSpec, &campaign)
            .expect("oracle must be reachable; `node` is required for differential tests");

        let exceeded = attempt.cases > u64::from(BATCH);
        report = Some((seconds, attempt));

        if exceeded {
            break;
        }
    }

    let (seconds, report) = report.expect("the loop runs at least once");

    assert!(
        report.cases > u64::from(BATCH),
        "campaigns of up to {seconds}s executed {} cases with a batch of {BATCH}: \
         batches after the first generated nothing. {}",
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
        // Appended at the END of the list (CLAUDE.md, Git).
        SUFFIX_ARRAY_REGRESSIONS,
        GENERALIZED_REGRESSIONS,
        BLOOM_REGRESSIONS,
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

// Appended at the END of the file, never between existing tests (CLAUDE.md).

/// `suffix-array` is the first module here with no mutating method: the whole
/// computation is the constructor, so the campaign spends its budget on
/// constructions rather than on op sequences. See the spec's module docs.
#[test]
fn suffix_array_matches_upstream() {
    let campaign = Campaign::cases(0x5FFA, 96, SUFFIX_ARRAY_REGRESSIONS);

    let report = difffuzz::run(&SuffixArraySpec, &campaign)
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

// Appended at the END of the file, never between two existing tests.

/// `heap` — the first module whose comparator is a callback.
///
/// Six comparator factories, four of which mutate or throw from inside a sift.
/// The budgets they count in *comparisons* make this sharper than a black-box
/// grammar: a sift that reaches the right ordering by a different number of
/// comparisons diverges here.
#[test]
fn heap_matches_upstream() {
    let campaign = Campaign::cases(0x11EA9, 96, HEAP_REGRESSIONS);

    let report = difffuzz::run(&HeapSpec, &campaign)
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

/// The generalized variant, which is where `longestCommonSubsequence` -- the
/// only method in this unit with branching logic outside the constructor --
/// gets exercised.
#[test]
fn generalized_suffix_array_matches_upstream() {
    let campaign = Campaign::cases(0x65FA, 96, GENERALIZED_REGRESSIONS);

    let report = difffuzz::run(&GeneralizedSuffixArraySpec, &campaign)
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

// New tests go on the end of this file, never in the middle: a conflict
// boundary landing inside one has already broken three merges.

/// The first free-function module, and therefore the first exercise of the
/// `functions` mode in `fuzz/oracle.js`.
///
/// Worth its own note: this campaign compares almost nothing that the others
/// do. `sort` has no observable state, so `observe()` is `{}` for every op and
/// a bug in the state comparison would be invisible here. What it compares is
/// the return value *and the arguments after the call*, which is where every
/// effect of an in-place sort lives.
#[test]
fn sort_matches_upstream() {
    let campaign = Campaign::cases(0x5057, 96, SORT_REGRESSIONS);

    let report = difffuzz::run(&SortSpec, &campaign)
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

/// The second free-function module, and the one whose campaign leans hardest
/// on the argument echo: four of `set.js`'s fourteen functions return
/// `undefined` and do all their work to their first argument, so without it
/// they would be compared against nothing at all.
#[test]
fn set_matches_upstream() {
    let campaign = Campaign::cases(0x5E7, 96, SET_REGRESSIONS);

    let report = difffuzz::run(&SetSpec, &campaign)
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

/// `bloom-filter` is the only module whose op arguments are deliberately
/// ill-typed: `add` and `test` take numbers and booleans as well as strings,
/// because every non-string collapses onto the empty sequence upstream (B-98)
/// and that is only reachable if the grammar can express one.
#[test]
fn bloom_filter_matches_upstream() {
    let campaign = Campaign::cases(0xB100, 96, BLOOM_REGRESSIONS);

    let report = difffuzz::run(&BloomFilterSpec, &campaign)
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

// ---------------------------------------------------------------------------
// Appended at the end of the file, never in the middle: a conflict boundary
// landing inside an existing test has already broken three merges
// (CLAUDE.md, Git).
// ---------------------------------------------------------------------------

#[test]
fn vector_matches_upstream() {
    let campaign = Campaign::cases(0x7EC70, 96, VECTOR_REGRESSIONS);

    let report = difffuzz::run(&VectorSpec, &campaign)
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

/// The only module in this crate where every op is a query: nothing in the
/// public API mutates a built tree, so the whole signal is in each op's
/// result rather than in a changing `observe()`. See the spec's module docs.
#[test]
fn static_interval_tree_matches_upstream() {
    let campaign = Campaign::cases(0x517, 96, STATIC_INTERVAL_TREE_REGRESSIONS);

    let report = difffuzz::run(&StaticIntervalTreeSpec, &campaign)
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

/// `fixed-reverse-heap` — the same comparators through a different pair of
/// algorithms, plus a generated capacity that includes the `0` upstream's dead
/// guard lets through.
#[test]
fn fixed_reverse_heap_matches_upstream() {
    let campaign = Campaign::cases(0xF12ED, 96, FIXED_REVERSE_HEAP_REGRESSIONS);

    let report = difffuzz::run(&FixedReverseHeapSpec, &campaign)
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

// Appended at the end of the file, never inserted: several agents edit this
// file at once and a test added after the last one cannot land inside another
// agent's hunk.

/// `bi-map` — this campaign's own regression corpus carries two REAL
/// divergences (B-120), not sabotages; see the corpus file's provenance block
/// and `docs/modules/bi-map.md`.
#[test]
fn bi_map_matches_upstream() {
    let campaign = Campaign::cases(0xB1AA, 96, BI_MAP_REGRESSIONS);

    let report = difffuzz::run(&BiMapSpec, &campaign)
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

// Appended at the END of the file, never between two existing tests.

/// `lru-cache` -- the object-backed base class. Small capacities (`1..=6`)
/// and a 300-op ceiling, so eviction fires constantly rather than the
/// campaign only proving that a map stores things. See
/// `crates/difffuzz/src/modules/lru_cache.rs`'s module docs.
#[test]
fn lru_cache_matches_upstream() {
    let campaign = Campaign::cases(0x1_20126, 96, LRU_CACHE_REGRESSIONS);

    let report = difffuzz::run(&LruCacheSpec, &campaign)
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

/// `fuzzy-map` — the hash function travels as a named factory
/// (`fuzzyIdentity`/`fuzzyLower`); see the module doc for why those names are
/// prefixed.
#[test]
fn fuzzy_map_matches_upstream() {
    let campaign = Campaign::cases(0xF522, 96, FUZZY_MAP_REGRESSIONS);

    let report = difffuzz::run(&FuzzyMapSpec, &campaign)
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

/// `lru-cache-with-delete` -- the object-backed pair's freelist. `delete`/
/// `remove` are in the alphabet here and nowhere else in this family, which
/// is what actually exercises `holes` reuse in
/// `mnemonist_core::structures::lru_cache::LruCache::insert_new`, and what
/// found the port defect `docs/modules/lru-cache.md` calls "Bugs this
/// found" -- deleting a not-yet-visited pointer out from under an open walk.
#[test]
fn lru_cache_with_delete_matches_upstream() {
    let campaign = Campaign::cases(0x1_20127, 96, LRU_CACHE_WITH_DELETE_REGRESSIONS);

    let report = difffuzz::run(&LruCacheWithDeleteSpec, &campaign)
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

/// `bk-tree` — the first campaign over a genuine tree shape rather than an
/// `OrderedMap`. `search`'s return value stands in for the "root" observation
/// this module does not have; see the spec's module doc.
#[test]
fn bk_tree_matches_upstream() {
    let campaign = Campaign::cases(0xB711, 96, BK_TREE_REGRESSIONS);

    let report = difffuzz::run(&BkTreeSpec, &campaign)
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

/// `lru-map` -- the `Map`-backed base class. Same grammar as `lru-cache`,
/// over the same key pool, so the SameValueZero index (a number and its
/// string form are different keys here, unlike the object-backed pair) gets
/// exercised by construction rather than by a second grammar.
#[test]
fn lru_map_matches_upstream() {
    let campaign = Campaign::cases(0x1_20128, 96, MAP_REGRESSIONS);

    let report = difffuzz::run(&LruMapSpec, &campaign)
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

/// `lru-map-with-delete`.
#[test]
fn lru_map_with_delete_matches_upstream() {
    let campaign = Campaign::cases(0x1_20129, 96, MAP_WITH_DELETE_REGRESSIONS);

    let report = difffuzz::run(&LruMapWithDeleteSpec, &campaign)
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

// ---------------------------------------------------------------------------
// Appended at the end of the file, never inserted: several agents edit this
// file at once and a new test after the last one cannot land inside another
// agent's hunk (CLAUDE.md, Git).
// ---------------------------------------------------------------------------

/// `multi-map` — a three-key pool shared by `set`/`remove`, so a bucket
/// accumulates several values and drains back to zero constantly. See the
/// spec's module docs for what is deliberately out of this grammar (cursor
/// lifecycle ops).
#[test]
fn multi_map_matches_upstream() {
    let campaign = Campaign::cases(0x1_20130, 96, MULTI_MAP_REGRESSIONS);

    let report = difffuzz::run(&MultiMapSpec, &campaign)
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

/// `multi-set` — a three-item pool over `add`/`remove`/`set`/`edit`/`delete`,
/// small counts including zero and negative (the sign-flip delegation), and
/// a bounded `top`. See the spec's module docs for B-161/B-162's coverage
/// here.
#[test]
fn multi_set_matches_upstream() {
    let campaign = Campaign::cases(0x1_20131, 96, MULTI_SET_REGRESSIONS);

    let report = difffuzz::run(&MultiSetSpec, &campaign)
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

/// `fuzzy-multi-map` — `fuzzyLower` collapses `'Hello'`/`'HELLO'`/`'World'`
/// onto two hashed keys, so `add`ing all three is exactly the "one key,
/// several values" case this campaign exists to hit. See the spec's module
/// docs for why `Set`-kind object-identity dedup is out of scope here.
#[test]
fn fuzzy_multi_map_matches_upstream() {
    let campaign = Campaign::cases(0x1_20132, 96, FUZZY_MULTI_MAP_REGRESSIONS);

    let report = difffuzz::run(&FuzzyMultiMapSpec, &campaign)
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

/// `fibonacci-heap` — a short sanity campaign so `cargo test` exercises the
/// re-entrant comparator path (`fibPushy`/`fibPopper`/`fibClearer`) briefly,
/// not just at compile time. The 60-second gate-9 campaigns live in
/// `fuzz/log.txt`; this only guards that the harness still works.
#[test]
fn fibonacci_heap_matches_upstream() {
    let campaign = Campaign::cases(0x1_20133, 96, FIBONACCI_HEAP_REGRESSIONS);

    let report = difffuzz::run(&FibonacciHeapSpec, &campaign)
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
