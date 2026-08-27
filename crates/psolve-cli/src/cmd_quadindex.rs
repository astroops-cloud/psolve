//! `psolve quad-index build`: sweeps the sky at six doubling angular bands,
//! forms geometric quads from the paired star `.psidx`'s catalogue via
//! `psolve_core::quad`, and writes a `.psqidx` blind-solve quad index
//! (`psolve_index::quad_format` / `quad_builder`).
//!
//! This is the one place in the workspace allowed to depend on both
//! `psolve-index` and `psolve-core` (the milestone's dependency budget:
//! `psolve-index` gets memmap2 only, `psolve-cli` gets `psolve-index`,
//! `psolve-core`, `rayon`). Accordingly, everything geometry-shaped --
//! tiling, star fetch, quad forming, band assignment, and the per-tile
//! selection rule -- lives HERE, not in `psolve-index::quad_builder`, which
//! only knows how to accumulate already-computed quad records and write the
//! file (see that module's own doc for the split).
//!
//! ## Pipeline, per tile
//!
//! 1. Fetch the brightest `TILE_STAR_BUDGET` catalogue stars within the
//!    tile's disc (`Index::brightest_in_disc_indexed`, which -- unlike the
//!    hinted solve path's `brightest_in_disc` -- also returns each star's
//!    global index into the paired `.psidx`'s flat record array, exactly
//!    the reference a `QuadRecord.star_idx` needs).
//! 2. Project each star to the tangent plane at the tile centre
//!    (`psolve_core::project::radec_to_tangent`) -- `quad_code` operates on
//!    a flat 2-D point set, not spherical coordinates.
//! 3. Form quads from that point set (`psolve_core::quad::build_quads`) and
//!    select at most `TILE_QUAD_CAP` of them (`select_tile_quads`, this
//!    module's stated selection rule -- see its own doc).
//! 4. Assign each selected quad to a band BY ITS ACTUAL DIAGONAL
//!    (`band_for_diag`), not by which band's tile swept it -- see that
//!    function's doc for why those can differ and why that's correct.
//!
//! ## Determinism
//!
//! Every fact this milestone can measure downstream (search-structure
//! benchmarks, storage, the null-test false-positive rate) depends on the
//! index being reproducible. `--jobs` may change wall-clock; it must never
//! change the output. Two properties make that true here:
//! - Tiles are visited in a fixed order per band (`tiles_for_band`'s own
//!   nested loop, independent of any parallelism), and `rayon`'s
//!   `par_iter().map(..).collect::<Vec<_>>()` over that fixed `Vec`
//!   reassembles results in the SAME order regardless of which thread
//!   finished which tile first (a documented property of Rayon's indexed
//!   `collect`) -- so the per-tile results are pushed into the builder in a
//!   thread-count-independent sequence.
//! - Every per-tile computation is itself a pure function of its inputs:
//!   `build_quads`'s own total-order tie-breaks (see its module doc),
//!   `select_tile_quads`'s explicit `idx`-based tie-break on ties, and no
//!   randomness or hash-iteration order anywhere in this file.
//!
//! `QuadIndexBuilder::push`'s own doc states the other half of this
//! contract: push order IS output order, so preserving push order here is
//! sufficient, not just necessary.

use crate::cmd_index::{json_escape, json_number};
use crate::flag;
use psolve_core::project::radec_to_tangent;
use psolve_core::quad::{self, Quad};
use psolve_index::quad_builder::QuadIndexBuilder;
use psolve_index::quad_format::{QuadHeader, HEADER_BYTES};
use psolve_index::quad_reader::QuadIndex;
use psolve_index::reader::Index;
use psolve_index::sha256::hex;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Read};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

/// The spec's six doubling bands, degrees, shallowest first. §4a's spike
/// (`docs/superpowers/2026-08-15-blind-solve-spike.md`) measured this exact
/// sequence's storage and build cost against G<=16 -- ~18.67M quads / 448MB
/// / 12-15 min single-threaded, embarrassingly parallel -- which this
/// module's own build is measured against (see the Task 2 report).
pub const BAND_SCALES_DEG: [f32; 6] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];

/// Per-tile emission cap. "A per-tile emission cap is mandatory, not an
/// optimisation" (spike): the 1.0 deg band alone offers a measured median
/// 11,183 formable quads per tile against a 10-30/tile target: two to three
/// orders of magnitude more than needed, and uncapped storage runs to
/// 2-3x the budget for no matching benefit.
pub const TILE_QUAD_CAP: usize = 25;

/// Brightest stars fetched per tile before quad-forming. Slightly above the
/// spike's own "12-star budget" (used there purely for its storage/build-time
/// extrapolation, not for a selection rule) -- the extra headroom is what
/// gives `select_tile_quads`'s conditioning-aware reselection genuine
/// alternatives to choose among per seed star, rather than just enough
/// points to reach the cap once.
const TILE_STAR_BUDGET: usize = 14;

/// Nearest-neighbour count `build_quads` draws its combinations from.
/// Matches the spike's own production-shaped regime, which is what its
/// build-time/storage numbers were measured against.
const TILE_NEIGHBOURS: usize = 6;

/// Upper bound handed to `build_quads` so its own round-robin ordering is
/// never truncated before `select_tile_quads`'s reselection sees it -- an
/// untruncated pool is what makes recovering `build_quads`'s per-seed
/// grouping from `idx[0]` well-defined (see that function's doc). Worst
/// case for `TILE_STAR_BUDGET` seeds each contributing up to
/// C(`TILE_NEIGHBOURS`, 3) combinations is `14 * C(6,3) = 280`, comfortably
/// under this.
const TILE_QUAD_POOL_CAP: usize = 4096;

/// One sky tile: a centre and the disc radius its stars are drawn from.
#[derive(Debug, Clone, Copy)]
struct Tile {
    ra_deg: f64,
    dec_deg: f64,
    radius_deg: f64,
}

/// Restricts the full-sky sweep to a declination/right-ascension box.
/// Defaults to the whole sky. Two independent uses: (1) a real build
/// against a catalogue with known declination coverage (e.g. this
/// milestone's `dec<=45` source) can skip tiles over a region with no stars
/// at all, rather than spending a `cells_in_disc` scan on every one of them
/// only to find nothing; (2) tests can restrict a sweep to a few dozen
/// tiles instead of the full ~800K, without touching the tiling or
/// selection logic under test.
///
/// **RA does not wrap the 0/360 boundary** -- `min_ra_deg <= max_ra_deg` is
/// enforced by `validate`, same as declination. A caller who genuinely
/// needs a box that straddles 0 deg RA can express it as two separate
/// builds (or, for the whole-sky default, doesn't need RA bounds at all).
/// This is a deliberate scope limit, not an oversight: nothing in this
/// milestone needs a wrapping RA box, and a wrapping range comparison is
/// exactly the kind of edge case worth not inventing without a caller that
/// exercises it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SweepBounds {
    min_ra_deg: f64,
    max_ra_deg: f64,
    min_dec_deg: f64,
    max_dec_deg: f64,
}

impl Default for SweepBounds {
    fn default() -> Self {
        SweepBounds { min_ra_deg: 0.0, max_ra_deg: 360.0, min_dec_deg: -90.0, max_dec_deg: 90.0 }
    }
}

impl SweepBounds {
    fn validate(&self) -> Result<(), String> {
        if !(0.0..=360.0).contains(&self.min_ra_deg) || !(0.0..=360.0).contains(&self.max_ra_deg) {
            return Err(format!(
                "--min-ra/--max-ra {}..{} must be within 0..360",
                self.min_ra_deg, self.max_ra_deg
            ));
        }
        if self.min_ra_deg > self.max_ra_deg {
            return Err(format!(
                "--min-ra {} exceeds --max-ra {} (RA bounds do not wrap 0/360)",
                self.min_ra_deg, self.max_ra_deg
            ));
        }
        if !(-90.0..=90.0).contains(&self.min_dec_deg) || !(-90.0..=90.0).contains(&self.max_dec_deg) {
            return Err(format!(
                "--min-dec/--max-dec {}..{} must be within -90..90",
                self.min_dec_deg, self.max_dec_deg
            ));
        }
        if self.min_dec_deg > self.max_dec_deg {
            return Err(format!(
                "--min-dec {} exceeds --max-dec {}",
                self.min_dec_deg, self.max_dec_deg
            ));
        }
        Ok(())
    }

    fn contains(&self, ra_deg: f64, dec_deg: f64) -> bool {
        (self.min_ra_deg..=self.max_ra_deg).contains(&ra_deg)
            && (self.min_dec_deg..=self.max_dec_deg).contains(&dec_deg)
    }
}

/// Deterministic full-sky tiling at `scale_deg`: declination is divided
/// into `round(180/scale)` equal-height strips; within each strip, right
/// ascension is divided into `round(360*cos(dec_centre)/scale)` equal-width
/// slices, so a tile near the equator is close to `scale` x `scale` on the
/// sky, while near the poles -- where a fixed RA width subtends much less
/// true arc -- proportionally fewer, wider slices are used rather than many
/// needle-thin ones. This is the standard cos(dec)-scaled RA-strip scheme,
/// not HEALPix: HEALPix cells are equal-area but not sized to an arbitrary
/// caller-chosen angular scale, and a quad-index tile is never looked up by
/// pixel index the way the star index's cells are (a tile here is used
/// exactly once, to seed one disc query), so there is no format-level
/// reason to spend HEALPix's extra structure on it.
///
/// Each tile's disc radius is `scale_deg / 2`, matching the spike's own
/// measurement method exactly (its write-up: "Per tile: ... radius =
/// band/2") -- a disc inscribed in the tile's own square footprint. This
/// leaves two known, accepted gaps rather than silently covering them up:
/// it under-covers the tile's four corners (a corner sits
/// `scale/2 * sqrt(2)` from centre, beyond the disc), and near the poles a
/// strip can collapse to a single tile whose disc radius does not reach
/// the full RA range at that latitude. Neither is a correctness bug -- a
/// star simply outside every tile's disc contributes no quad, it never
/// contributes a WRONG one -- and the real catalogue this milestone builds
/// from is dec<=45, so there are no stars at the poles to miss. The corner
/// gap costs only a sliver of a tile's own quad-forming headroom, which the
/// spike already measured as two to three orders of magnitude oversupplied
/// at every band from 0.5 deg up.
fn tiles_for_band(scale_deg: f64, bounds: SweepBounds) -> Vec<Tile> {
    let n_dec = ((180.0 / scale_deg).round() as i64).max(1);
    let dec_step = 180.0 / n_dec as f64;
    let mut out = Vec::new();
    for dband in 0..n_dec {
        let dec_c = -90.0 + dec_step * (dband as f64 + 0.5);
        let n_ra = ((360.0 * dec_c.to_radians().cos() / scale_deg).round() as i64).max(1);
        let ra_step = 360.0 / n_ra as f64;
        for rband in 0..n_ra {
            let ra_c = ra_step * (rband as f64 + 0.5);
            if bounds.contains(ra_c, dec_c) {
                out.push(Tile { ra_deg: ra_c, dec_deg: dec_c, radius_deg: scale_deg / 2.0 });
            }
        }
    }
    out
}

/// Which band a quad's diagonal (degrees) belongs to, by geometric-mean
/// midpoint between successive doubling scales -- the standard scheme
/// multi-scale quad indexes use to split by quad diameter range (e.g. the
/// well-known open-source solver's own reference index does exactly this).
/// For six scales 0.25..8 deg doubling, the midpoints are
/// `sqrt(s_i * s_{i+1})`: 0.354, 0.707, 1.414, 2.828, 5.657 deg. Band 0
/// covers everything below the first midpoint; the last band covers
/// everything from the last midpoint up, with no upper bound -- a quad
/// wider than 8 deg cannot occur in practice here (every tile's disc radius
/// bounds its own possible diagonal at that band's own scale, and 8 deg is
/// the widest scale swept), but an open upper bound costs nothing and
/// avoids a spurious rejection if that ever changes.
///
/// This is independent of which band's TILE SWEEP produced the quad: a
/// quad found while sweeping the 8 deg band can easily have a much smaller
/// true diagonal (`TILE_STAR_BUDGET` stars spread thin across an 8 deg disc
/// often have their nearest few neighbours only 1-2 deg apart), and
/// correctly lands in a smaller band here. That is intended, not a bug --
/// what matters to a later solve is the quad's actual scale, not which
/// sweep happened to find it -- and it means a lookup by an image's own FOV
/// (Task 4) finds every quad whose geometry actually matches that FOV,
/// regardless of provenance.
///
/// **Consequence for anyone reasoning about band density from tile counts**:
/// because assignment is diagonal-true, band 0's own quad count is NOT
/// bounded by `(0.25 deg band's own tiles swept) x TILE_QUAD_CAP` -- it also
/// receives small-diagonal quads formed by every coarser band's sweep. The
/// real build measured exactly this: the 0.25 deg band swept 563,396 tiles
/// (a `563,396 x 25 = 14,084,900` sweep-provenance ceiling) but actually
/// holds 17,343,044 quads, 23% over that figure, entirely from smaller-than-
/// their-own-sweep quads landing there from the 0.5/1/2/4/8 deg sweeps. The
/// ceiling that DOES hold is global, across every band: total tiles swept
/// (751,094) x `TILE_QUAD_CAP` (25) = 18,777,350; the real build's actual
/// 18,692,947 is 99.55% of that. A per-band tile-sweep count is the wrong
/// number to divide a band's quad count by.
fn band_for_diag(diag_deg: f64, band_scales_deg: &[f32]) -> usize {
    let mut band = 0usize;
    for i in 0..band_scales_deg.len().saturating_sub(1) {
        let mid = (f64::from(band_scales_deg[i]) * f64::from(band_scales_deg[i + 1])).sqrt();
        if diag_deg < mid {
            return band;
        }
        band = i + 1;
    }
    band
}

/// Select at most `cap` quads from a tile's full geometric-code candidate
/// pool -- not brightest-N, not first-found. This is the design decision
/// the milestone brief calls out by name ("Which 25 matters and is a real
/// design decision"): at the 1.0 deg band the spike measured a median
/// 11,183 formable quads against a 25-quad cap, so which 25 survive is
/// nearly the whole effect this index has on match quality.
///
/// **What this function actually changes, measured**: at this build's own
/// parameters (`TILE_STAR_BUDGET` = 14 seed stars, `TILE_QUAD_CAP` = 25),
/// spatial coverage across seed stars is NOT something this function earns
/// -- it falls out of `build_quads`'s own round-robin interleave (rank 0 of
/// every seed, then rank 1, and so on) for free, before this function ever
/// runs: with at most 14 seeds and a cap of 25, the interleave's rank-0
/// pass alone already reaches every seed with a non-empty candidate group,
/// so ANY selection that preserves that order -- including a plain
/// first-25 prefix of the untruncated pool -- gets full seed coverage at
/// these parameters. Measured directly against real production tiles: the
/// set of distinct seeds/stars touched by the untouched pool, a naive
/// first-25 prefix, and this function's actual output were IDENTICAL in
/// every sampled tile. What this function changes is conditioning: within
/// each seed's own group -- and ONLY within it, this never reorders ACROSS
/// seeds, so it cannot improve on the interleave's already-free coverage --
/// quads are re-sorted by `conditioning_key` (best-first) before the
/// identical round-robin interleave is re-run. Measured mean
/// `conditioning_key` roughly DOUBLES versus the naive first-25 prefix on
/// real production tiles. The trade is not free: on the same tiles, mean
/// pairwise centroid spread among the selected quads was slightly LOWER
/// than the naive prefix's in most samples, not higher -- conditioning is
/// bought, in dense tiles, with a small amount of spatial dispersion,
/// rather than adding to it. That is still the right trade (a
/// well-conditioned quad that lands one seed closer to its neighbours
/// solves; a near-degenerate one from a slightly more distant seed does
/// not), but it is a conditioning optimisation, not a joint
/// conditioning-and-spread one -- see `select_tile_quads_improves_mean_conditioning_over_naive_prefix_truncation`
/// for the discriminating test, and this function's own recovery of
/// `build_quads`'s per-seed grouping from `idx[0]` (a field `build_quads`
/// never reorders -- set from the ORIGINAL `[seed, ...]` index array before
/// `quad_code`'s own A/B/C/D canonicalisation runs, so it survives intact)
/// for the mechanism.
///
/// A pool no larger than `cap` is returned unchanged: there is nothing to
/// select among, so sparse tiles see no reranking effect at all.
fn select_tile_quads(points: &[(f64, f64)], cap: usize) -> Vec<Quad> {
    if points.len() < 4 || cap == 0 {
        return Vec::new();
    }
    let pool = quad::build_quads(points, TILE_NEIGHBOURS, TILE_QUAD_POOL_CAP);
    if pool.len() <= cap {
        return pool;
    }

    let mut by_seed: Vec<Vec<Quad>> = vec![Vec::new(); points.len()];
    for q in pool {
        by_seed[q.idx[0]].push(q);
    }
    for group in &mut by_seed {
        group.sort_by(|a, b| {
            conditioning_key(b, points)
                .partial_cmp(&conditioning_key(a, points))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.idx.cmp(&b.idx))
        });
    }

    let deepest = by_seed.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::with_capacity(cap);
    'outer: for rank in 0..deepest {
        for seed in &by_seed {
            if let Some(q) = seed.get(rank) {
                out.push(*q);
                if out.len() >= cap {
                    break 'outer;
                }
            }
        }
    }
    out
}

/// A quad's numerical conditioning score: higher means further from either
/// near-degenerate failure the milestone brief names ("A quad whose four
/// stars are nearly collinear, or whose diagonal is barely longer than its
/// short side, gives a numerically weak transform"), lower means fragile.
/// The two are combined with `min`, not averaged -- a quad is only as
/// trustworthy as its WORST margin, and either failure alone weakens the
/// resulting transform regardless of how good the other margin looks.
///
/// 1. **Near-collinear inner points.** `quad_code`'s canonical frame
///    (`psolve_core::quad::to_frame`) places A at the origin and B at
///    (1,1) in a basis rotated 45 deg from the raw along/across-AB axes:
///    `x = along - perp`, `y = along + perp`. Inverting that, a point's
///    perpendicular offset from the AB line is `(y - x) / 2`. A quad whose
///    C or D sits almost exactly on that line carries almost no
///    information about the field's rotation -- numerically, four points
///    close to three-plus-a-duplicate. `min(|perp_C|, |perp_D|)` is 0 for
///    an exactly collinear point and grows with genuine spread off the
///    line; taking the min over the mean means ONE collinear-ish point is
///    enough to flag the whole quad.
/// 2. **A fragile A/B choice.** `quad_code` always names the two most
///    widely separated of the four points A and B. If the second-widest
///    pairwise distance among all six pairs is nearly as large as that
///    maximum, centroiding noise on the image side can flip which pair
///    wins, producing a different code for what is essentially the same
///    four stars -- the brief's "diagonal is barely longer than its short
///    side" case, with the runner-up pairwise distance standing in for
///    "short side" (a general 4-point set has no fixed vertex order, so a
///    quadrilateral's own diagonal-vs-side language doesn't apply
///    literally). `(d_max - d_second) / d_max` is the fractional margin by
///    which AB actually wins: 0 is a coin flip, 1 is total dominance.
///
/// Both margins land on comparable 0..~1 scales (one a half-unit frame
/// coordinate, the other a distance ratio), so a plain `min` is a
/// defensible conjunction without inventing a weighting this project has no
/// data to justify.
fn conditioning_key(q: &Quad, points: &[(f64, f64)]) -> f64 {
    let [x_c, y_c, x_d, y_d] = q.code;
    let perp_c = (y_c - x_c) / 2.0;
    let perp_d = (y_d - x_d) / 2.0;
    let collinearity_margin = perp_c.abs().min(perp_d.abs());

    let mut dists = [0.0f64; 6];
    let mut n = 0;
    for u in 0..4 {
        for v in (u + 1)..4 {
            let (pu, pv) = (points[q.idx[u]], points[q.idx[v]]);
            let (dx, dy) = (pv.0 - pu.0, pv.1 - pu.1);
            dists[n] = (dx * dx + dy * dy).sqrt();
            n += 1;
        }
    }
    dists.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let (d_max, d_second) = (dists[0], dists[1]);
    let ab_margin = if d_max > 0.0 { (d_max - d_second) / d_max } else { 0.0 };

    collinearity_margin.min(ab_margin)
}

/// One tile's contribution: the band it belongs in (by diagonal -- see
/// `band_for_diag`), its raw geometric code, and the four stars' global
/// references into the paired `.psidx`.
type TileQuadRecord = (usize, [f64; 4], [u32; 4]);

/// Everything one tile contributes: for each selected quad, its correct
/// band (by diagonal -- see `band_for_diag`) and its `.psqidx` record
/// payload (raw code, the four stars' global references into the paired
/// `.psidx`). Empty for a tile with fewer than 4 usable stars -- see this
/// function's own early returns -- never a panic.
fn process_tile(index: &Index, tile: Tile) -> Vec<TileQuadRecord> {
    let stars =
        index.brightest_in_disc_indexed(tile.ra_deg, tile.dec_deg, tile.radius_deg, TILE_STAR_BUDGET);
    if stars.len() < 4 {
        return Vec::new();
    }
    let mut points = Vec::with_capacity(stars.len());
    let mut global_idx = Vec::with_capacity(stars.len());
    for (gidx, rec) in &stars {
        // `radec_to_tangent` only fails for a point beyond 90 deg from the
        // tangent point; every tile's radius is at most 4 deg (half of
        // `BAND_SCALES_DEG`'s largest entry), so this is unreachable for a
        // star this same disc query just selected -- but a defensive skip
        // costs nothing and keeps `points`/`global_idx` in lockstep even if
        // that invariant is ever loosened.
        if let Some(xy) = radec_to_tangent(rec.ra_deg(), rec.dec_deg(), tile.ra_deg, tile.dec_deg) {
            points.push(xy);
            global_idx.push(*gidx);
        }
    }
    if points.len() < 4 {
        return Vec::new();
    }

    select_tile_quads(&points, TILE_QUAD_CAP)
        .into_iter()
        .map(|q| {
            let band = band_for_diag(q.diag, &BAND_SCALES_DEG);
            let star_idx = [
                global_idx[q.idx[0]],
                global_idx[q.idx[1]],
                global_idx[q.idx[2]],
                global_idx[q.idx[3]],
            ];
            (band, q.code, star_idx)
        })
        .collect()
}

fn hex_bytes(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// `--star-index` consumes a following token; without listing it here,
/// `info_positional`'s scan below would mistake that value for the
/// positional `<FILE>` argument -- the same defect class `cmd_index.rs`'s
/// own `QUERY_VALUED_FLAGS` comment names, now with a second subcommand
/// sharing the shape.
const INFO_VALUED_FLAGS: &[&str] = &["--star-index"];

/// The first token that is neither a flag nor a flag's own value. Mirrors
/// `cmd_index::query_positional` exactly, duplicated rather than shared
/// across the two `--star-index`- and `--ra`-shaped flag lists: the two
/// subcommands' valued-flag sets are unrelated and a shared helper would
/// have to take the list as a parameter anyway, which is what this already
/// is in miniature.
fn info_positional<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if INFO_VALUED_FLAGS.contains(&a) {
            i += 2;
        } else if a.starts_with("--") {
            i += 1;
        } else {
            return Some(a);
        }
    }
    None
}

/// `psolve quad-index info --star-index <FILE> <PSQIDX> [--verify]`
///
/// A `.psqidx` cannot be opened at all without its paired `.psidx` --
/// `QuadIndex::open`'s own doc explains why `star_index_fingerprint`
/// enforcement is built into `open` itself rather than a separate opt-in
/// step, and this command is the one place that constructor is called
/// outside the reader's own tests, so `--star-index` is required here, not
/// optional, mirroring `build`'s own required `--star-index`.
pub fn info(args: &[&str]) -> ExitCode {
    let Some(star_index_path) = flag(args, "--star-index") else {
        eprintln!("psolve quad-index info: --star-index <FILE> is required");
        return ExitCode::from(2);
    };
    let Some(path) = info_positional(args) else {
        eprintln!("psolve quad-index info: <FILE> is required");
        return ExitCode::from(2);
    };
    let verify = args.contains(&"--verify");

    let star_index = match Index::open(Path::new(star_index_path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("psolve quad-index info: cannot open --star-index {star_index_path}: {e}");
            return ExitCode::from(3);
        }
    };

    let qidx = match QuadIndex::open(Path::new(path), &star_index) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("psolve quad-index info: {e}");
            return ExitCode::from(3);
        }
    };
    let h = qidx.header();

    // Same "exit 3, print nothing to stdout" contract `cmd_index::info` uses
    // for its own `--verify`: a partial JSON object on a failed verify would
    // be two contradictory signals at once (exit 3 says "problem", a
    // parseable object on stdout says "here is a result").
    if verify {
        if let Err(e) = qidx.verify_digest() {
            eprintln!("psolve quad-index info: {e}");
            return ExitCode::from(3);
        }
    }

    // Same `verified`/`digest_ok` split `cmd_index::info` uses: `digest_ok`
    // is `null`, not `false`, whenever `--verify` did not run at all --
    // `false` would read as "checked and failed" when nothing was checked.
    let digest_ok = if verify { "true".to_string() } else { "null".to_string() };

    let band_scales_json: String =
        h.band_scales_deg().iter().map(|d| json_number(f64::from(*d))).collect::<Vec<_>>().join(",");
    let per_band_json: String = (0..h.n_bands as usize)
        .map(|b| format!("{{\"band\":{b},\"count\":{}}}", qidx.band_len(b)))
        .collect::<Vec<_>>()
        .join(",");

    println!(
        "{{\"name\":\"{}\",\"version\":{},\"nside\":{},\"epoch\":{},\"n_quads\":{},\
\"n_bands\":{},\"mag_limit\":{},\"band_scales_deg\":[{}],\"per_band\":[{}],\
\"sha256\":\"{}\",\"star_index_fingerprint\":\"{}\",\"verified\":{},\"digest_ok\":{}}}",
        json_escape(h.name_str()),
        h.version,
        h.nside,
        json_number(h.epoch),
        h.n_quads,
        h.n_bands,
        json_number(f64::from(h.mag_limit)),
        band_scales_json,
        per_band_json,
        hex(&h.records_sha256),
        hex_bytes(&h.star_index_fingerprint),
        verify,
        digest_ok
    );
    ExitCode::SUCCESS
}

/// `psolve quad-index build --star-index <FILE> --out <FILE> [OPTIONS]`
pub fn build(args: &[&str]) -> ExitCode {
    let Some(star_index_path) = flag(args, "--star-index") else {
        eprintln!("psolve quad-index build: --star-index <FILE> is required");
        return ExitCode::from(2);
    };
    let Some(out) = flag(args, "--out") else {
        eprintln!("psolve quad-index build: --out <FILE> is required");
        return ExitCode::from(2);
    };
    let name = flag(args, "--name").unwrap_or("blind-quad-index");
    // Mirrors `cmd_index::build`'s own `--name` guard: a short identifier
    // interpolated unescaped into the JSON result and stored in a fixed
    // 24-byte header field (`quad_format::NAME_BYTES`), so both need the
    // same restriction, not an escaping scheme.
    if name.is_empty()
        || name.len() > 24
        || !name.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
    {
        eprintln!(
            "psolve quad-index build: --name must be 1-24 printable ASCII characters \
             with no quotes or backslashes"
        );
        return ExitCode::from(2);
    }
    let mut bounds = SweepBounds::default();
    let parse_deg = |name: &str, default: f64| -> Result<f64, ()> {
        flag(args, name).unwrap_or(&default.to_string()).parse::<f64>().map_err(|_| ())
    };
    match (
        parse_deg("--min-ra", bounds.min_ra_deg),
        parse_deg("--max-ra", bounds.max_ra_deg),
        parse_deg("--min-dec", bounds.min_dec_deg),
        parse_deg("--max-dec", bounds.max_dec_deg),
    ) {
        (Ok(min_ra), Ok(max_ra), Ok(min_dec), Ok(max_dec)) => {
            bounds = SweepBounds {
                min_ra_deg: min_ra,
                max_ra_deg: max_ra,
                min_dec_deg: min_dec,
                max_dec_deg: max_dec,
            };
        }
        _ => {
            eprintln!(
                "psolve quad-index build: --min-ra/--max-ra/--min-dec/--max-dec must be degrees"
            );
            return ExitCode::from(2);
        }
    }
    if let Err(e) = bounds.validate() {
        eprintln!("psolve quad-index build: {e}");
        return ExitCode::from(2);
    }

    // Same "reject rather than launder" shape as `cmd_index::build`'s own
    // `--jobs`: rayon treats `num_threads(0)` as "use the default", which
    // would otherwise turn a bogus `--jobs 0` into silent success with the
    // default thread count instead of a usage error.
    if let Some(v) = flag(args, "--jobs") {
        match v.parse::<usize>() {
            Ok(0) => {
                eprintln!("psolve quad-index build: --jobs must be at least 1");
                return ExitCode::from(2);
            }
            Ok(j) => {
                let _ = rayon::ThreadPoolBuilder::new().num_threads(j).build_global();
            }
            Err(_) => {
                eprintln!("psolve quad-index build: --jobs must be a non-negative integer");
                return ExitCode::from(2);
            }
        }
    }

    let index = match Index::open(Path::new(star_index_path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("psolve quad-index build: cannot open {star_index_path}: {e}");
            return ExitCode::from(3);
        }
    };

    // First 8 bytes of the paired star index's own record digest -- the
    // tripwire Task 3's reader will compare against to catch an accidental
    // mispairing. See `quad_format.rs`'s module doc for the full rationale.
    let mut fingerprint = [0u8; 8];
    fingerprint.copy_from_slice(&index.header().records_sha256[..8]);

    // `QuadHeader::nside`'s own doc leaves its semantics up to the builder
    // ("provenance only ... this does not size anything in this file's
    // layout"). This tiling does not use HEALPix at all (`tiles_for_band`'s
    // own doc explains why a cos(dec)-scaled RA-strip grid was chosen
    // instead), so there is no "tiling nside" to store here. The paired
    // star index's own nside is stored instead: it is still meaningful
    // provenance (which star index shape this build queried against) and
    // costs nothing to carry, even though it plays no structural role in
    // `.psqidx` itself.
    let mut builder = match QuadIndexBuilder::new(
        index.header().nside,
        index.header().epoch,
        index.header().mag_limit,
        name,
        fingerprint,
        &BAND_SCALES_DEG,
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("psolve quad-index build: {e}");
            return ExitCode::from(2);
        }
    };

    let t0 = Instant::now();
    let mut tiles_swept: u64 = 0;
    for (sweep_band, &scale) in BAND_SCALES_DEG.iter().enumerate() {
        let tiles = tiles_for_band(f64::from(scale), bounds);
        tiles_swept += tiles.len() as u64;
        eprintln!(
            "psolve quad-index build: sweeping band {sweep_band} ({scale} deg): {} tile(s)",
            tiles.len()
        );
        // See this module's doc, "Determinism": `collect::<Vec<_>>()` over
        // an `IndexedParallelIterator` (a `par_iter()` on a `Vec`)
        // reassembles results in the tiles' own fixed order regardless of
        // which thread finished which tile first, so the push loop below
        // runs in a thread-count-independent sequence.
        let results: Vec<Vec<TileQuadRecord>> =
            tiles.par_iter().map(|&t| process_tile(&index, t)).collect();
        for recs in results {
            for (band, code, star_idx) in recs {
                if let Err(e) = builder.push(band, code, star_idx) {
                    eprintln!("psolve quad-index build: {e}");
                    return ExitCode::from(3);
                }
            }
        }
    }

    let per_band: Vec<usize> = (0..BAND_SCALES_DEG.len()).map(|b| builder.band_len(b)).collect();

    let mut f = match File::create(out) {
        Ok(f) => BufWriter::new(f),
        Err(e) => {
            eprintln!("psolve quad-index build: cannot create {out}: {e}");
            return ExitCode::from(2);
        }
    };
    let stats = match builder.finish(&mut f) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("psolve quad-index build: {e}");
            return ExitCode::from(3);
        }
    };
    drop(f);

    // Re-read just the header (128 bytes) to report the digest and confirm
    // what was actually written parses -- the same "prove it opens" check
    // `cmd_index::build` does with a full `Index::open`, scaled down since
    // Task 3's `.psqidx` reader doesn't exist yet for this crate to call.
    let header = match File::open(out).and_then(|mut f| {
        let mut buf = [0u8; HEADER_BYTES];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }) {
        Ok(buf) => match QuadHeader::from_bytes(&buf) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("psolve quad-index build: wrote an index that will not parse: {e}");
                return ExitCode::from(3);
            }
        },
        Err(e) => {
            eprintln!("psolve quad-index build: cannot re-read {out}: {e}");
            return ExitCode::from(3);
        }
    };

    let per_band_json: String = BAND_SCALES_DEG
        .iter()
        .zip(per_band.iter())
        .map(|(scale, count)| format!("{{\"scale_deg\":{scale},\"count\":{count}}}"))
        .collect::<Vec<_>>()
        .join(",");

    println!(
        "{{\"n_quads\":{},\"clamped\":{},\"tiles_swept\":{},\"n_bands\":{},\"per_band\":[{}],\
\"nside\":{},\"mag_limit\":{},\"epoch\":{},\"name\":\"{}\",\"sha256\":\"{}\",\
\"star_index_fingerprint\":\"{}\",\"seconds\":{:.1}}}",
        stats.written,
        stats.clamped,
        tiles_swept,
        BAND_SCALES_DEG.len(),
        per_band_json,
        header.nside,
        json_number(f64::from(header.mag_limit)),
        json_number(header.epoch),
        json_escape(name),
        hex(&header.records_sha256),
        hex_bytes(&header.star_index_fingerprint),
        t0.elapsed().as_secs_f64()
    );

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- tiles_for_band --

    #[test]
    fn tiles_for_band_covers_every_declination_strip() {
        let tiles = tiles_for_band(8.0, SweepBounds::default());
        assert!(!tiles.is_empty());
        for t in &tiles {
            assert!((-90.0..=90.0).contains(&t.dec_deg));
            assert!((0.0..360.0).contains(&t.ra_deg));
            assert_eq!(t.radius_deg, 4.0);
        }
    }

    #[test]
    fn a_finer_band_scale_produces_more_tiles() {
        let b = SweepBounds::default();
        assert!(tiles_for_band(0.25, b).len() > tiles_for_band(1.0, b).len());
        assert!(tiles_for_band(1.0, b).len() > tiles_for_band(8.0, b).len());
    }

    #[test]
    fn tile_generation_is_deterministic() {
        let b = SweepBounds::default();
        assert_eq!(
            tiles_for_band(1.0, b).iter().map(|t| (t.ra_deg, t.dec_deg)).collect::<Vec<_>>(),
            tiles_for_band(1.0, b).iter().map(|t| (t.ra_deg, t.dec_deg)).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn sweep_bounds_filters_tiles_outside_the_box() {
        let b = SweepBounds { min_ra_deg: 90.0, max_ra_deg: 92.0, min_dec_deg: 19.0, max_dec_deg: 21.0 };
        let tiles = tiles_for_band(0.25, b);
        assert!(!tiles.is_empty());
        for t in &tiles {
            assert!(b.contains(t.ra_deg, t.dec_deg));
        }
        assert!(tiles.len() < tiles_for_band(0.25, SweepBounds::default()).len());
    }

    #[test]
    fn sweep_bounds_rejects_a_flipped_range() {
        let b = SweepBounds { min_dec_deg: 40.0, max_dec_deg: -40.0, ..SweepBounds::default() };
        assert!(b.validate().is_err());
        let b2 = SweepBounds { min_ra_deg: 300.0, max_ra_deg: 10.0, ..SweepBounds::default() };
        assert!(b2.validate().is_err());
    }

    // -- band_for_diag --

    #[test]
    fn band_assignment_uses_geometric_mean_midpoints() {
        // Midpoints: 0.354, 0.707, 1.414, 2.828, 5.657.
        assert_eq!(band_for_diag(0.1, &BAND_SCALES_DEG), 0);
        assert_eq!(band_for_diag(0.35, &BAND_SCALES_DEG), 0);
        assert_eq!(band_for_diag(0.36, &BAND_SCALES_DEG), 1);
        assert_eq!(band_for_diag(0.7, &BAND_SCALES_DEG), 1);
        assert_eq!(band_for_diag(0.71, &BAND_SCALES_DEG), 2);
        assert_eq!(band_for_diag(1.4, &BAND_SCALES_DEG), 2);
        assert_eq!(band_for_diag(1.5, &BAND_SCALES_DEG), 3);
        assert_eq!(band_for_diag(2.8, &BAND_SCALES_DEG), 3);
        assert_eq!(band_for_diag(2.9, &BAND_SCALES_DEG), 4);
        assert_eq!(band_for_diag(5.6, &BAND_SCALES_DEG), 4);
        assert_eq!(band_for_diag(5.7, &BAND_SCALES_DEG), 5);
        assert_eq!(band_for_diag(100.0, &BAND_SCALES_DEG), 5, "no upper bound on the last band");
    }

    #[test]
    fn band_assignment_never_exceeds_the_configured_band_count() {
        for diag in [0.0, 0.001, 3.0, 8.0, 50.0, f64::MAX] {
            assert!(band_for_diag(diag, &BAND_SCALES_DEG) < BAND_SCALES_DEG.len());
        }
    }

    // -- select_tile_quads / conditioning_key --

    /// Deterministic scatter, matching `psolve_core::quad`'s own fixture
    /// style, so ties don't hide ordering bugs.
    fn scatter(n: usize, scale: f64, seed: u64) -> Vec<(f64, f64)> {
        let mut s = seed;
        let mut pts = Vec::with_capacity(n);
        let mut nxt = || {
            s = s.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..n {
            pts.push((nxt() * scale, nxt() * scale));
        }
        pts
    }

    #[test]
    fn select_tile_quads_honours_the_cap_exactly() {
        let pts = scatter(14, 1.0, 1);
        let got = select_tile_quads(&pts, TILE_QUAD_CAP);
        assert!(got.len() <= TILE_QUAD_CAP);
        // This fixture has plenty of stars/combinations, so the cap should
        // actually bind, not merely happen not to be exceeded.
        assert_eq!(got.len(), TILE_QUAD_CAP);
    }

    #[test]
    fn select_tile_quads_never_duplicates_a_star_set() {
        let pts = scatter(14, 1.0, 7);
        let got = select_tile_quads(&pts, TILE_QUAD_CAP);
        let mut keys: Vec<[usize; 4]> = got
            .iter()
            .map(|q| {
                let mut k = q.idx;
                k.sort_unstable();
                k
            })
            .collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(before, keys.len());
    }

    #[test]
    fn select_tile_quads_is_spread_across_many_seed_stars() {
        // Same property `build_quads`'s own
        // `the_cap_is_spread_across_seed_points_not_taken_from_the_first_few`
        // test checks -- this function must not lose that guarantee while
        // reselecting for conditioning.
        let pts = scatter(14, 1.0, 3);
        let got = select_tile_quads(&pts, TILE_QUAD_CAP);
        let mut seeds: Vec<usize> = got.iter().map(|q| q.idx[0]).collect();
        seeds.sort_unstable();
        seeds.dedup();
        assert!(seeds.len() >= 10, "expected wide seed spread, got {} distinct seeds", seeds.len());
    }

    #[test]
    fn select_tile_quads_on_too_few_points_is_empty_not_a_panic() {
        assert!(select_tile_quads(&[], TILE_QUAD_CAP).is_empty());
        assert!(select_tile_quads(&[(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)], TILE_QUAD_CAP).is_empty());
    }

    #[test]
    fn select_tile_quads_is_deterministic() {
        let pts = scatter(14, 1.0, 42);
        let a = select_tile_quads(&pts, TILE_QUAD_CAP);
        let b = select_tile_quads(&pts, TILE_QUAD_CAP);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.idx, y.idx);
            assert_eq!(x.code, y.code);
        }
    }

    /// The measured, discriminating effect of this module's selection rule
    /// (see `select_tile_quads`'s own doc for the full measurement this
    /// pins down): at these parameters seed coverage is already saturated
    /// by `build_quads`'s own interleave (14 seeds, cap 25), so a naive
    /// first-25 prefix of the untruncated pool gets the SAME seed coverage
    /// `select_tile_quads` does -- that comparison cannot discriminate
    /// between them (an earlier version of this test asserted exactly that
    /// non-discriminating property, and could not fail). What actually
    /// differs, and does discriminate, is conditioning: within each seed's
    /// own group this reorders to best-conditioned-first before
    /// re-interleaving, so the selected set's mean `conditioning_key` must
    /// be strictly better than the naive prefix's.
    #[test]
    fn select_tile_quads_improves_mean_conditioning_over_naive_prefix_truncation() {
        let pts = scatter(14, 1.0, 99);
        let full_pool = quad::build_quads(&pts, TILE_NEIGHBOURS, TILE_QUAD_POOL_CAP);
        assert!(full_pool.len() > TILE_QUAD_CAP, "fixture must actually exercise the cap");

        let mean_key = |qs: &[Quad]| -> f64 {
            qs.iter().map(|q| conditioning_key(q, &pts)).sum::<f64>() / qs.len() as f64
        };

        let naive_mean = mean_key(&full_pool[..TILE_QUAD_CAP]);
        let selected = select_tile_quads(&pts, TILE_QUAD_CAP);
        assert_eq!(selected.len(), TILE_QUAD_CAP);
        let selected_mean = mean_key(&selected);

        assert!(
            selected_mean > naive_mean,
            "selection must improve mean conditioning over a naive first-{TILE_QUAD_CAP} prefix \
             of the same pool (naive={naive_mean}, selected={selected_mean})"
        );

        // Seed coverage itself is unchanged by the reselection -- confirms
        // the doc's claim that spread is inherited from build_quads' own
        // interleave, not earned by this function, at these parameters.
        let seeds_of = |qs: &[Quad]| -> Vec<usize> {
            let mut s: Vec<usize> = qs.iter().map(|q| q.idx[0]).collect();
            s.sort_unstable();
            s.dedup();
            s
        };
        assert_eq!(
            seeds_of(&full_pool[..TILE_QUAD_CAP]),
            seeds_of(&selected),
            "at TILE_STAR_BUDGET < TILE_QUAD_CAP, seed coverage is identical between the naive \
             prefix and the reselected output -- this function changes WHICH quad each seed \
             contributes, not how many seeds contribute"
        );
    }

    /// A pool at or under the cap is returned unchanged -- no reranking, no
    /// truncation. This is the sparse-tile case `select_tile_quads`'s own
    /// doc says sees no reranking effect at all.
    #[test]
    fn select_tile_quads_returns_the_full_pool_unchanged_when_at_or_under_the_cap() {
        let pts = scatter(6, 1.0, 5);
        let pool = quad::build_quads(&pts, TILE_NEIGHBOURS, TILE_QUAD_POOL_CAP);
        assert!(!pool.is_empty() && pool.len() <= TILE_QUAD_CAP, "fixture must stay under the cap");
        let selected = select_tile_quads(&pts, TILE_QUAD_CAP);
        assert_eq!(selected, pool, "a pool at or under the cap must pass through unchanged");
    }

    #[test]
    fn conditioning_key_penalises_a_near_collinear_quad() {
        let square = [(0.0, 0.0), (10.0, 10.0), (3.0, 6.0), (7.0, 2.0)];
        let sq = quad::quad_code(square[0], square[1], square[2], square[3]).unwrap();
        let sq_quad = Quad { code: sq, idx: [0, 1, 2, 3], diag: 0.0 };

        // Same A/B, but C/D nudged onto the AB line -- near-collinear.
        let collinear = [(0.0, 0.0), (10.0, 10.0), (5.0, 5.0001), (5.0001, 5.0)];
        let cl = quad::quad_code(collinear[0], collinear[1], collinear[2], collinear[3]).unwrap();
        let cl_quad = Quad { code: cl, idx: [0, 1, 2, 3], diag: 0.0 };

        assert!(
            conditioning_key(&sq_quad, &square) > conditioning_key(&cl_quad, &collinear),
            "a well-spread quad must score higher than a near-collinear one"
        );
    }

    #[test]
    fn conditioning_key_penalises_a_fragile_ab_choice() {
        let points = [(0.0, 0.0), (10.0, 10.0), (3.0, 6.0), (7.0, 2.0)];
        let clear_ab = Quad {
            code: quad::quad_code(points[0], points[1], points[2], points[3]).unwrap(),
            idx: [0, 1, 2, 3],
            diag: 0.0,
        };

        // A near-square: the two diagonals are almost the same length, so
        // whichever pair "wins" as A/B is a near coin-flip.
        let near_square = [(0.0, 0.0), (10.0, 10.001), (10.0, 0.0), (0.0, 10.0)];
        let fragile = Quad {
            code: quad::quad_code(near_square[0], near_square[1], near_square[2], near_square[3])
                .unwrap(),
            idx: [0, 1, 2, 3],
            diag: 0.0,
        };

        assert!(
            conditioning_key(&clear_ab, &points) > conditioning_key(&fragile, &near_square),
            "a quad with a dominant AB pair must score higher than a near-tie"
        );
    }

    // -- process_tile --

    fn write_test_index(dir: &std::path::Path, stars: &[(f64, f64, f32)]) -> std::path::PathBuf {
        let mut b = psolve_index::builder::Builder::new(64, 20.0, 2016.0, "test").unwrap();
        for &(ra, dec, mag) in stars {
            b.push(ra, dec, mag, 0.0, 0.0);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        b.finish(&mut buf).unwrap();
        let p = dir.join("t.psidx");
        std::fs::write(&p, buf.into_inner()).unwrap();
        p
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir()
            .join(format!("psolve-quadindex-test-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_tile_with_fewer_than_four_stars_emits_nothing() {
        let d = tmpdir("sparse");
        let p = write_test_index(&d, &[(10.0, 20.0, 12.0), (10.01, 20.01, 12.5)]);
        let idx = Index::open(&p).unwrap();
        let tile = Tile { ra_deg: 10.0, dec_deg: 20.0, radius_deg: 0.125 };
        assert!(process_tile(&idx, tile).is_empty());
    }

    #[test]
    fn an_empty_tile_emits_nothing_not_a_panic() {
        let d = tmpdir("empty");
        let p = write_test_index(&d, &[(10.0, 20.0, 12.0)]);
        let idx = Index::open(&p).unwrap();
        let tile = Tile { ra_deg: 200.0, dec_deg: -60.0, radius_deg: 0.125 };
        assert!(process_tile(&idx, tile).is_empty());
    }

    /// A synthetic star field produces a deterministic quad set: pins the
    /// count AND a digest over the emitted (band, code, star_idx) triples,
    /// not just the count -- the same "pin contents, not just length"
    /// discipline `psolve_core::quad`'s own golden test uses.
    fn fnv1a_over_tile_output(recs: &[(usize, [f64; 4], [u32; 4])]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let prime: u64 = 0x100000001b3;
        for (band, code, star_idx) in recs {
            h ^= *band as u64;
            h = h.wrapping_mul(prime);
            for c in code {
                h ^= c.to_bits();
                h = h.wrapping_mul(prime);
            }
            for s in star_idx {
                h ^= u64::from(*s);
                h = h.wrapping_mul(prime);
            }
        }
        h
    }

    #[test]
    fn a_synthetic_star_field_produces_a_deterministic_pinned_quad_set() {
        let d = tmpdir("golden");
        // 30 stars in a deterministic scatter around a fixed centre, well
        // within one 1 deg tile.
        let mut s: u64 = 0xD1CE5EED;
        let mut nxt = || {
            s = s.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        let stars: Vec<(f64, f64, f32)> = (0..30)
            .map(|i| (100.0 + (nxt() - 0.5) * 0.8, 20.0 + (nxt() - 0.5) * 0.8, 8.0 + i as f32 * 0.2))
            .collect();
        let p = write_test_index(&d, &stars);
        let idx = Index::open(&p).unwrap();
        let tile = Tile { ra_deg: 100.0, dec_deg: 20.0, radius_deg: 0.5 };

        let recs = process_tile(&idx, tile);
        assert_eq!(recs.len(), TILE_QUAD_CAP, "this fixture must be dense enough to hit the cap");
        assert_eq!(
            fnv1a_over_tile_output(&recs),
            0xe667_f14e_c558_4cb4,
            "pinned digest over (band, code, star_idx) changed -- if this is an intentional \
             algorithm change, recompute and update the pin, don't just widen the assertion"
        );
    }

    #[test]
    fn every_emitted_quad_lands_in_the_band_its_diagonal_falls_in() {
        let d = tmpdir("band-consistency");
        let mut s: u64 = 12345;
        let mut nxt = || {
            s = s.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        let stars: Vec<(f64, f64, f32)> = (0..20)
            .map(|i| (50.0 + (nxt() - 0.5) * 3.0, 10.0 + (nxt() - 0.5) * 3.0, 9.0 + i as f32 * 0.3))
            .collect();
        let p = write_test_index(&d, &stars);
        let idx = Index::open(&p).unwrap();
        let tile = Tile { ra_deg: 50.0, dec_deg: 10.0, radius_deg: 2.0 };

        let recs = process_tile(&idx, tile);
        assert!(!recs.is_empty());
        for (band, _code, star_idx) in &recs {
            // Resolve the four stars this record actually references,
            // re-project them to the SAME tangent plane `process_tile`
            // used, and recompute the true diagonal (max pairwise
            // distance) independently of anything `process_tile` itself
            // computed -- then confirm `band_for_diag` assigns that exact
            // diagonal to the band this record was actually stored under.
            let pts: Vec<(f64, f64)> = star_idx
                .iter()
                .map(|&gi| {
                    let rec = idx.star_at(gi).expect("star_idx must resolve in this same index");
                    radec_to_tangent(rec.ra_deg(), rec.dec_deg(), tile.ra_deg, tile.dec_deg)
                        .expect("within the tile's own small radius, projection must succeed")
                })
                .collect();
            let mut diag: f64 = 0.0;
            for u in 0..4 {
                for v in (u + 1)..4 {
                    let (dx, dy) = (pts[v].0 - pts[u].0, pts[v].1 - pts[u].1);
                    diag = diag.max((dx * dx + dy * dy).sqrt());
                }
            }
            assert_eq!(
                band_for_diag(diag, &BAND_SCALES_DEG),
                *band,
                "record's own true diagonal {diag} deg must land in the band it was stored under"
            );
        }
    }

    #[test]
    fn band_for_diag_is_the_true_source_of_truth_for_band_assignment() {
        // Directly exercises the property the brief names ("a quad lands in
        // the band its diagonal falls in") against the real function used
        // by `process_tile`, with exact diagonals rather than an indirect
        // re-derivation.
        for (diag, expected) in [(0.05, 0usize), (0.5, 1), (1.0, 2), (2.0, 3), (4.0, 4), (8.0, 5)] {
            assert_eq!(band_for_diag(diag, &BAND_SCALES_DEG), expected, "diag {diag}");
        }
    }

    #[test]
    fn header_fingerprint_matches_the_source_psidx_records_digest() {
        let d = tmpdir("fingerprint");
        let stars: Vec<(f64, f64, f32)> =
            (0..10).map(|i| (10.0 + i as f64 * 0.01, 20.0 + i as f64 * 0.01, 10.0)).collect();
        let p = write_test_index(&d, &stars);
        let idx = Index::open(&p).unwrap();

        let mut fingerprint = [0u8; 8];
        fingerprint.copy_from_slice(&idx.header().records_sha256[..8]);

        let b = QuadIndexBuilder::new(64, 2016.0, 20.0, "x", fingerprint, &BAND_SCALES_DEG).unwrap();
        let mut buf = std::io::Cursor::new(Vec::new());
        b.finish(&mut buf).unwrap();
        let bytes = buf.into_inner();
        let h = QuadHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.star_index_fingerprint, fingerprint);
        assert_eq!(&h.star_index_fingerprint[..], &idx.header().records_sha256[..8]);
    }
}
