//! Task 4 Step 4: `QuadIndex::candidates()` measured against the real
//! `.psqidx`/`.psidx` pair, against the milestone's ~8ms/lookup budget (5s
//! blind-solve target / ~600 image quads per frame, before verification --
//! see `quad_reader.rs`'s `candidates` doc and `blind_grid.rs`'s module
//! doc for the full derivation and the kd-tree prototype this was measured
//! against).
//!
//! Follows `psolve-cli/tests/real_frames.rs`'s convention: no `#[ignore]`,
//! just a runtime existence check that skips (with an `eprintln!`) on a
//! machine that does not have `~/astroops/data/gaia-dr3-g16-dec45-
//! nside64.psqidx`, so this runs for real as part of `cargo test
//! --workspace` here without breaking the suite elsewhere.

use psolve_index::quad_reader::QuadIndex;
use psolve_index::reader::Index;
use std::path::Path;
use std::time::Instant;

const CODE_TOL: f64 = 0.02;
const PER_LOOKUP_BUDGET_MS: f64 = 8.0;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

fn open_real_star_index() -> Option<Index> {
    let psidx = Path::new(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psidx"));
    let psqidx =
        Path::new(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psqidx"));
    if !psidx.exists() || !psqidx.exists() {
        eprintln!("skipping: real gaia-dr3-g16-dec45-nside64 index not present");
        return None;
    }
    Some(Index::open(psidx).unwrap())
}

/// Routine (non-`#[ignore]`) real-index check: band 3 (2.0 deg, 63,211
/// quads) is real production data but small enough that even an
/// unoptimized debug build's grid construction is sub-second, so this runs
/// as part of ordinary `cargo test --workspace` without slowing it down --
/// unlike the full band-0 benchmark below, which needs `--release`.
#[test]
fn candidates_meets_the_blind_solve_per_lookup_budget_on_a_real_band() {
    let psqidx =
        Path::new(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psqidx"));
    let Some(star_index) = open_real_star_index() else { return };
    let qidx = QuadIndex::open(psqidx, &star_index).unwrap();

    let band = 3usize; // 2.0 deg
    let n = qidx.band_len(band);
    assert!(n > 1000, "expected the real band 3 to hold a meaningful number of quads, got {n}");
    run_budget_check(&qidx, band, n, "band 3 (2.0 deg)");
}

/// The scale that actually matters: band 0 (0.25 deg) holds 92.8% of the
/// index (17,343,044 / 18,692,947 quads, Task 2/3's measured per-band
/// counts). `#[ignore]`d because its grid build alone takes ~38s in an
/// unoptimized debug build (vs. the 1.875s measured in release -- see
/// `blind_grid.rs`'s module doc); run explicitly with:
///   cargo test --release -p psolve-index --test blind_candidates_real_index -- --ignored --nocapture
#[test]
#[ignore]
fn candidates_meets_the_blind_solve_per_lookup_budget_on_the_full_dominant_band() {
    let psqidx =
        Path::new(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psqidx"));
    let Some(star_index) = open_real_star_index() else { return };
    let qidx = QuadIndex::open(psqidx, &star_index).unwrap();

    // Band 0 (0.25 deg) holds 92.8% of the index (17,343,044 / 18,692,947
    // quads, per Task 2/3's measured per-band counts) -- the band whose
    // grid-build and query cost matter most in practice.
    let band = 0usize;
    let n = qidx.band_len(band);
    assert!(n > 1000, "expected the real band 0 to hold many quads, got {n}");
    run_budget_check(&qidx, band, n, "band 0 (0.25 deg, full scale)");
}

fn run_budget_check(qidx: &QuadIndex, band: usize, n: usize, label: &str) {
    // First call triggers this band's lazy grid build (blind_grid.rs) --
    // time it separately from steady-state lookups, since it is a one-time
    // per-process cost, not a per-lookup one.
    let seed_code = qidx.quad(band, 0).unwrap().code_f64();
    let t0 = Instant::now();
    let first: Vec<_> = qidx.candidates(seed_code, CODE_TOL, band).collect();
    let first_call_ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert!(!first.is_empty(), "querying a record's own code must find at least itself");

    // Steady-state: 1000 lookups, alternating between real (jittered)
    // codes and far-off misses -- the same mix a blind solve would present
    // (some image quads correspond to real catalogue quads, many don't).
    let n_queries = 1000usize;
    let mut latencies_ms = Vec::with_capacity(n_queries);
    let mut candidate_sizes = Vec::with_capacity(n_queries);
    for q_i in 0..n_queries {
        let query = if q_i % 2 == 0 {
            let base = qidx.quad(band, (q_i * 97) % n).unwrap().code_f64();
            let j = (q_i as f64 % 7.0 - 3.0) * (CODE_TOL * 0.3);
            [base[0] + j, base[1] - j, base[2] + j * 0.5, base[3]]
        } else {
            [10.0 + q_i as f64 * 0.01, -10.0, 5.0, -5.0]
        };
        let t0 = Instant::now();
        let got = qidx.candidates(query, CODE_TOL, band).count();
        latencies_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        candidate_sizes.push(got);
    }
    latencies_ms.sort_unstable_by(f64::total_cmp);
    let mean_ms = latencies_ms.iter().sum::<f64>() / latencies_ms.len() as f64;
    let p50_ms = percentile(&latencies_ms, 0.50);
    let p99_ms = percentile(&latencies_ms, 0.99);
    let mean_candidates =
        candidate_sizes.iter().sum::<usize>() as f64 / candidate_sizes.len() as f64;

    eprintln!(
        "blind-solve candidates() on real index, {label}: \
         grid_build(first call)={first_call_ms:.3}ms n_quads={n} \
         steady_state mean={mean_ms:.4}ms p50={p50_ms:.4}ms p99={p99_ms:.4}ms \
         mean_candidates={mean_candidates:.1} budget={PER_LOOKUP_BUDGET_MS}ms"
    );

    assert!(
        mean_ms < PER_LOOKUP_BUDGET_MS,
        "mean per-lookup latency {mean_ms:.4}ms exceeds the ~{PER_LOOKUP_BUDGET_MS}ms budget"
    );
    assert!(
        p99_ms < PER_LOOKUP_BUDGET_MS,
        "p99 per-lookup latency {p99_ms:.4}ms exceeds the ~{PER_LOOKUP_BUDGET_MS}ms budget"
    );
}
