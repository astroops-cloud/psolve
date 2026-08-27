//! The blind-solve code-space search structure: an equal-population
//! (quantile) grid over a `.psqidx` band's 4-vector codes.
//!
//! ## Why a grid, and why quantile edges, not equal-width ones
//!
//! The spike (`docs/superpowers/2026-08-15-blind-solve-spike.md` section 5)
//! measured the code space as clustered, not uniform: coefficient of
//! variation 0.74-1.51 across coordinate-pair histograms, hotspot bins
//! holding 3-9.3x the mean occupancy, and up to 39% of a naive equal-WIDTH
//! grid's cells sitting empty. An equal-width grid hash was therefore
//! already known to be the wrong shape before this module was written.
//! **Equal-population (quantile) bin edges** fix this: bin boundaries are
//! chosen so each bin holds roughly the same COUNT of points, not the same
//! WIDTH of code space -- a hotspot gets many narrow bins, a sparse region
//! gets a few wide ones, so no bin absorbs an outsized share of the band.
//!
//! ## Prototyped against a kd-tree; the grid won by measurement, not taste
//!
//! Task 4 prototyped both candidates the spike named -- this quantile grid
//! and a hand-rolled kd-tree (median-split, cycling axis, hypersphere-vs-
//! splitting-plane pruning) -- against the real 18,692,947-quad index
//! (`~/astroops/data/gaia-dr3-g16-dec45-nside64.psqidx`), at `code_tol =
//! 0.02` (the hinted matcher's own default, `match_.rs`'s `MatchParams`).
//! Both passed the superset-of-brute-force correctness check on every
//! sampled query at every band size tried. Measured (release build, this
//! machine, band 0 = 0.25 deg = 17,343,044 quads, 92.8% of the index):
//!
//! | structure | build (band 0, full) | query mean | query p99 | bytes/quad (band 0) |
//! |---|---|---|---|---|
//! | quantile grid (chosen) | 1.875 s | 0.0122 ms | 0.0443 ms | 8 (packed key+local idx) |
//! | kd-tree (rejected) | 2.230 s | 0.0085 ms | 0.0331 ms | ~16 (idx+axis+2 child ptrs) |
//!
//! The kd-tree's query latency was consistently lower (~30-40%, every band
//! size tested from 1,117 to 17,343,044 quads) but **both are 180-700x
//! under the ~8ms/lookup budget** (5s blind-solve target / ~600 image
//! quads/frame, before verification -- see the module doc on `candidates`),
//! so that margin has no bearing on whether the milestone target is met.
//! What decided it:
//! - **Build cost at the band that matters most.** Band 0 holds 92.8% of
//!   the index; there the grid built FASTER (1.875s vs 2.230s) despite
//!   being slower to build at smaller bands -- the grid's O(n log n) sorts
//!   scale better than the kd-tree's recursive `select_nth_unstable_by` at
//!   this band's actual size.
//! - **Roughly half the memory.** The grid's structure is one `(u32, u32)`
//!   pair per quad (8 bytes); the kd-tree's node (index + axis + two child
//!   pointers) is ~16 bytes/quad even before alignment padding. At band 0's
//!   scale that is ~139 MB vs ~277 MB, and it only gets bigger on a future
//!   deeper (G<=18) rebuild.
//! - **A more mechanically obvious correctness argument.** The grid's
//!   candidate set is, by construction, "every quad whose per-axis
//!   quantile bin overlaps `[code[d]-tol, code[d]+tol]`" -- an axis-aligned
//!   box that trivially contains the Euclidean tolerance ball, so the
//!   superset property follows from the bin-edge search alone, not from a
//!   recursive pruning invariant that has to be gotten right on every
//!   branch. `candidates()` (`quad_reader.rs`) then filters this by real
//!   Euclidean distance, so what it returns is not merely a superset but
//!   exactly the brute-force answer -- the grid only narrows *which*
//!   records get that check, it never decides membership itself.
//!
//! Bin count (`BINS_PER_DIM`) is a fixed constant, not tuned per band: it
//! was measured at every band size from 1,117 to 17,343,044 quads and
//! performed well (sub-25-candidate mean sets, sub-millisecond queries)
//! at all of them without per-band adjustment, so there was nothing to
//! gain from making it adaptive.

use crate::quad_format::QuadRecord;
use crate::quad_reader::QuadIndex;

/// Bins per code-vector dimension. See the module doc: measured to work
/// well across every band size in the real index without per-band tuning.
/// 48^4 (~5.3M) possible cells comfortably fits `u32`'s packed key.
const BINS_PER_DIM: usize = 48;

/// Total order comparator for `f64` bin edges/keys that never panics on
/// NaN, so this never reaches for `partial_cmp().unwrap()` on values
/// decoded from an untrusted `.psqidx` file. `QuadRecord::code_f64` cannot
/// itself produce NaN/inf from a finite `u16` (the transform is a fixed
/// affine map), but this stays defensive rather than relying on that.
fn bin_of(edges: &[f64], v: f64) -> usize {
    debug_assert!(edges.len() >= 2);
    match edges.binary_search_by(|e| e.total_cmp(&v)) {
        Ok(i) => i.min(edges.len() - 2),
        Err(i) => i.saturating_sub(1).min(edges.len() - 2),
    }
}

fn pack_key(bins: [usize; 4]) -> u32 {
    let mut k = 0u32;
    for b in bins {
        k = k * (BINS_PER_DIM as u32) + b as u32;
    }
    k
}

/// One band's quantile grid: per-dimension bin edges plus every quad's
/// (packed 4D bin key, local index within the band) pair, sorted by key so
/// a candidate cell's members are a contiguous run findable by binary
/// search.
pub(crate) struct BandGrid {
    /// `BINS_PER_DIM + 1` non-decreasing edges per dimension.
    edges: [Vec<f64>; 4],
    /// Sorted by packed key. `(key, local_idx_within_band)`.
    sorted: Vec<(u32, u32)>,
}

impl BandGrid {
    /// Builds the grid for `band` from `qidx`, decoding each of the band's
    /// `n` quads once via `QuadIndex::quad` (already bounds-checked,
    /// panic-free on truncated/corrupt input). Does not retain a separate
    /// copy of the decoded codes -- only the compact packed keys survive
    /// past this call, so a query re-decodes the (few) candidate records it
    /// actually needs to distance-check.
    ///
    /// Callers must not call this with `n == 0`: `candidates()` on
    /// `QuadIndex` guards that case before it ever reaches here, since an
    /// empty band has no values to draw quantile edges from.
    pub(crate) fn build(qidx: &QuadIndex, band: usize, n: usize) -> BandGrid {
        // Pair each decoded code with its TRUE band position up front.
        // `qidx.quad` is panic-free but can in principle return `None` for
        // a corrupt record slice; using `filter_map` alone (without
        // carrying the original `i`) would silently compact the surviving
        // codes and desync every later index from the band position it is
        // supposed to name -- `filter_exact` would then decode the wrong
        // record for a given local index. Carrying `(i, code)` through
        // keeps that mapping correct even if some positions are skipped.
        let codes: Vec<(u32, [f64; 4])> =
            (0..n).filter_map(|i| qidx.quad(band, i).map(|r| (i as u32, r.code_f64()))).collect();
        let n = codes.len();

        let mut edges: [Vec<f64>; 4] = Default::default();
        for (d, edge) in edges.iter_mut().enumerate() {
            if n == 0 {
                *edge = vec![0.0; BINS_PER_DIM + 1];
                continue;
            }
            let mut vals: Vec<f64> = codes.iter().map(|&(_, c)| c[d]).collect();
            vals.sort_unstable_by(f64::total_cmp);
            let mut e = Vec::with_capacity(BINS_PER_DIM + 1);
            for i in 0..=BINS_PER_DIM {
                let pos = (i * (n - 1)) / BINS_PER_DIM;
                e.push(vals[pos]);
            }
            *edge = e;
        }

        let mut sorted: Vec<(u32, u32)> = codes
            .iter()
            .map(|&(band_idx, c)| {
                let bins = [
                    bin_of(&edges[0], c[0]),
                    bin_of(&edges[1], c[1]),
                    bin_of(&edges[2], c[2]),
                    bin_of(&edges[3], c[3]),
                ];
                (pack_key(bins), band_idx)
            })
            .collect();
        sorted.sort_unstable_by_key(|&(k, _)| k);

        BandGrid { edges, sorted }
    }

    fn bin_range(&self, d: usize, lo: f64, hi: f64) -> (usize, usize) {
        (bin_of(&self.edges[d], lo), bin_of(&self.edges[d], hi))
    }

    fn run_for_key(&self, key: u32) -> &[(u32, u32)] {
        let start = self.sorted.partition_point(|&(k, _)| k < key);
        let end = self.sorted.partition_point(|&(k, _)| k <= key);
        &self.sorted[start..end]
    }

    /// Local (within-band) indices of every quad whose packed quantile-bin
    /// key falls in the axis-aligned box `[code[d]-tol, code[d]+tol]` for
    /// every dimension `d`. This box always contains the Euclidean
    /// tolerance ball of the same radius, so this is a superset of a
    /// brute-force Euclidean scan by construction -- `QuadIndex::
    /// candidates` narrows it to an exact match with a real distance
    /// check. A degenerate (negative or zero-width after clamping) `tol`
    /// yields an empty range per axis rather than panicking: `lo..=hi`
    /// with `lo > hi` is simply an empty iterator in Rust.
    pub(crate) fn candidate_local_indices(&self, code: [f64; 4], tol: f64) -> Vec<u32> {
        let (x0, x1) = self.bin_range(0, code[0] - tol, code[0] + tol);
        let (y0, y1) = self.bin_range(1, code[1] - tol, code[1] + tol);
        let (z0, z1) = self.bin_range(2, code[2] - tol, code[2] + tol);
        let (w0, w1) = self.bin_range(3, code[3] - tol, code[3] + tol);
        let mut out = Vec::new();
        if x0 > x1 || y0 > y1 || z0 > z1 || w0 > w1 {
            return out;
        }
        for a in x0..=x1 {
            for b in y0..=y1 {
                for c in z0..=z1 {
                    for e in w0..=w1 {
                        for &(_, idx) in self.run_for_key(pack_key([a, b, c, e])) {
                            out.push(idx);
                        }
                    }
                }
            }
        }
        out
    }
}

/// Squared Euclidean distance between two 4-vector codes.
pub(crate) fn code_dist2(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    let mut s = 0.0;
    for i in 0..4 {
        let d = a[i] - b[i];
        s += d * d;
    }
    s
}

/// Exact (not just superset) matches for `code`/`tol`/`band`: every
/// `QuadRecord` in the band whose real Euclidean distance to `code` is
/// `<= tol`. `grid.candidate_local_indices` narrows the search to a small
/// set of records; this does the definitive distance check on each,
/// against the record's own decoded code (not the quantized bin it landed
/// in), so the result is identical to what a brute-force scan of the whole
/// band would return -- not merely a superset of it.
pub(crate) fn filter_exact(
    qidx: &QuadIndex,
    band: usize,
    grid: &BandGrid,
    code: [f64; 4],
    tol: f64,
) -> Vec<QuadRecord> {
    let t2 = tol * tol;
    grid.candidate_local_indices(code, tol)
        .into_iter()
        .filter_map(|local| qidx.quad(band, local as usize))
        .filter(|r| code_dist2(&r.code_f64(), &code) <= t2)
        .collect()
}
